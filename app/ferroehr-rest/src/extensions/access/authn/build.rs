// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Building the [`Authenticator`]: the issuers of `[auth]`, read at start,
//! the `DPoP` proof check of `[auth.dpop]`, the Basic store, and the issuers
//! whose role claims may grant a privileged role (`[authz.rbac]
//! role_issuers`). No openEHR spec governs authentication mechanics: our own
//! design.

use std::sync::Arc;

use ferrohealth_smart::guard::Gate;
use ferrohealth_smart::party::source::PartySource;
use ferrohealth_smart::resource::introspection::Introspector;
use ferrohealth_smart::resource::{Caller, Honoured, IssuerSet, PatientClaim, ResourceServer};
use ferrohealth_smart::sender::proof::ProofCheck;
use ferrohealth_smart::sender::replay::MemoryReplayStore;

use super::{Authenticator, Primary, issuer, upper_cased};
use crate::config::AppConfig;

/// The privileged roles of `[authz.rbac]` and the issuers whose tokens may
/// grant them.
#[derive(Debug, Default)]
pub(super) struct RoleIssuers {
    /// `admin_role`, `emergency_access_role`, `import_role` and
    /// `identifier_reader_role`, upper-cased.
    privileged: Vec<String>,
    /// `role_issuers`.
    issuers: Vec<String>,
}

impl RoleIssuers {
    /// Returns the rule of `config`'s `[authz.rbac]`.
    fn of(config: &AppConfig) -> Self {
        let rbac = &config.rbac;
        let privileged = [
            Some(&rbac.admin_role),
            rbac.emergency_access_role.as_ref(),
            rbac.import_role.as_ref(),
            rbac.identifier_reader_role.as_ref(),
        ];
        Self {
            privileged: privileged
                .into_iter()
                .flatten()
                .map(|role| role.to_uppercase())
                .collect(),
            issuers: rbac.role_issuers.clone(),
        }
    }

    /// Returns the upper-cased roles `caller` holds: a token of an issuer
    /// `role_issuers` does not list grants no privileged role, while a Basic
    /// user keeps the roles its own table grants.
    pub(super) fn roles(&self, caller: &Caller) -> Vec<String> {
        let mut roles = upper_cased(caller.roles());
        if let Some(issuer) = caller.issuer()
            && !self.issuers.iter().any(|listed| listed == issuer)
        {
            roles.retain(|role| !self.privileged.contains(role));
        }
        roles
    }
}

impl Authenticator {
    /// Builds the authenticator of `config`: each `[auth]` issuer's metadata
    /// and key set are read here, so an authorization server that cannot be
    /// reached refuses the start rather than the first request.
    ///
    /// The first issuer takes `[smart] issuer_metadata_file` as its metadata
    /// when its key set is a `jwks_file`, so its discovery document can name a
    /// `token_endpoint`. Every caller an issuer admits carries `patient_claim`,
    /// the live ABAC patient claim, as the `ferrohealth-smart` `PatientClaim`.
    ///
    /// With a party `source` and an `[[auth.party]]` table, every caller is
    /// looked up in the party binding the tables and `[authz.party_binding]`
    /// describe, on every request.
    ///
    /// # Errors
    /// A message naming the refused setting, the issuer that could not be
    /// read, or the metadata file that could not be read or does not fit the
    /// first issuer, a `patient_claim` that names no claim, or a party
    /// binding the shared crate refuses.
    pub async fn new(
        config: &AppConfig,
        patient_claim: Option<&str>,
        source: Option<Arc<dyn PartySource>>,
    ) -> Result<Arc<Self>, String> {
        let resolved = ferroehr::config::auth::resolve(&config.auth.shared)
            .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?;
        let party = super::party::binding(config, &resolved, source)?;
        let mut gate = Gate::open();
        let mut primary = None;
        let claim = patient_claim
            .map(PatientClaim::new)
            .transpose()
            .map_err(|e| format!("authz.abac.patient_claim is refused: {e}"))?;
        if resolved.enabled {
            let mut issuers = IssuerSet::new();
            for settings in resolved.introspection {
                let issuer = settings.issuer.clone();
                let mut introspector = Introspector::discover(settings).await.map_err(|e| {
                    format!(
                        "the introspection endpoint of {issuer} could not be read: {}",
                        ferroehr::error::ErrorChain::new(&e)
                    )
                })?;
                if let Some(claim) = &claim {
                    introspector = introspector.with_extension(claim.clone());
                }
                let introspector = Arc::new(introspector);
                primary.get_or_insert_with(|| Primary::Introspection(Arc::clone(&introspector)));
                issuers = issuers.with_introspector(introspector);
            }
            let mut supplied = match config.smart.issuer_metadata_file.as_deref() {
                Some(path) => Some(issuer::issuer_metadata(path).await?),
                None => None,
            };
            for settings in resolved.issuers {
                let issuer = settings.issuer.clone();
                let mut server = ResourceServer::discover(settings)
                    .await
                    .map_err(|e| issuer::unreadable_issuer(&issuer, &e))?;
                if let Some(metadata) = supplied.take() {
                    server = server.with_metadata(metadata).map_err(|e| {
                        format!(
                            "smart.issuer_metadata_file does not fit the first [auth] issuer \
                             {issuer}: {}",
                            ferroehr::error::ErrorChain::new(&e)
                        )
                    })?;
                }
                if let Some(claim) = &claim {
                    server = server.with_extension(claim.clone());
                }
                let server = Arc::new(server);
                primary.get_or_insert_with(|| Primary::Jwt(Arc::clone(&server)));
                issuers = issuers.with(server);
            }
            if !issuers.is_empty() {
                gate = gate.with_bearer(Arc::new(issuers));
            }
            // NOTE: RFC 9449 §11.1, the replay store holds one process's proofs;
            // the shared store over PostgreSQL is #5072.
            if let Some(settings) = resolved.dpop {
                let check = ProofCheck::new(settings, Arc::new(MemoryReplayStore::new()))
                    .map_err(|e| format!("auth.dpop is refused: {e}"))?;
                gate = gate.with_dpop(Arc::new(check));
            }
            // NOTE: RFC 9110 §11.6.1, a challenge names only a scheme the server
            // implements, so an `[auth.basic]` table without users admits no Basic.
            if let Some(store) = resolved
                .basic
                .filter(|_store| ferroehr::config::auth::has_basic(&config.auth.shared))
            {
                gate = gate.with_basic(Arc::new(store));
            }
            let backend = config.smart.backend_clients.clone();
            let patient = config.smart.patient_issuers.clone();
            gate = gate
                .with_realm(ferroehr::config::auth::realm(&config.auth.shared))
                .with_honoured(move |caller| Honoured {
                    backend: caller
                        .client_id()
                        .is_some_and(|client| backend.iter().any(|listed| listed == client)),
                    patient: caller
                        .issuer()
                        .is_some_and(|issuer| patient.iter().any(|listed| listed == issuer)),
                });
        }
        Ok(Arc::new(Self {
            enabled: config.auth.shared.enabled,
            gate,
            realm: ferroehr::config::auth::realm(&config.auth.shared).to_owned(),
            primary,
            role_issuers: RoleIssuers::of(config),
            party,
        }))
    }
}
