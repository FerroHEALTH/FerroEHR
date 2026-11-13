// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Building the [`Authenticator`]: the issuers of `[auth]`, read at start,
//! the `DPoP` proof check of `[auth.dpop]`, the Basic store, and the issuers
//! whose role claims may grant a privileged role (`[authz.rbac]
//! role_issuers`). No openEHR spec governs authentication mechanics: our own
//! design.

use std::sync::Arc;

use ferroehr::party_binding::source::BindingSource;
use ferrohealth_smart::guard::Gate;
use ferrohealth_smart::resource::described::{Candidate, DescribeError, describe};
use ferrohealth_smart::resource::introspection::{IntrospectionSettings, Introspector};
use ferrohealth_smart::resource::{Caller, Honoured, IssuerSet, PatientClaim, ResourceServer};
use ferrohealth_smart::sender::proof::ProofCheck;
use ferrohealth_smart::sender::replay::ReplayStore;

use super::{Authenticator, issuer, upper_cased};
use crate::config::AppConfig;
use ferrohealth_secret::secret_url::SecretUrl;

/// The check of the described issuer, whose metadata the SMART discovery
/// document is built from.
pub(super) enum Primary {
    /// A JWT access token checked against the issuer's keys.
    Jwt(Arc<ResourceServer>),
    /// An opaque access token checked by the issuer's introspection endpoint
    /// (RFC 7662).
    Introspection(Arc<Introspector>),
}

/// The privileged roles of `[authz.rbac]` and the issuers whose tokens may
/// grant them.
#[derive(Debug, Default)]
pub(super) struct RoleIssuers {
    /// `admin_role`, `emergency_access_role`, `import_role` and
    /// `identifier_reader_role`, upper-cased.
    privileged: Vec<String>,
    /// `role_issuers`.
    issuers: Vec<SecretUrl>,
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
            && !self.issuers.iter().any(|listed| listed.expose() == issuer)
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
    /// `[auth.smart] issuer_metadata_file` is the metadata of the issuer it names,
    /// whose key set is a `jwks_file`, so its discovery document can name a
    /// `token_endpoint`. When the SMART discovery document is served, it
    /// describes the issuer `ferrohealth-smart`'s `describe` chooses:
    /// `[auth] described_issuer`, else the one issuer that is not `machines`
    /// and whose metadata is available. Every
    /// caller an issuer admits carries `patient_claim`, the live ABAC patient
    /// claim, as the `ferrohealth-smart` `PatientClaim`. The `DPoP` proof
    /// check of `[auth.dpop]` compares a proof's `htu` with the server's
    /// public root (`[server] public_base_url` with `server.base_path`) and
    /// the request path, and records each accepted proof in `replay`, which
    /// the server builds over the clinical pool every replica shares.
    ///
    /// With a party `source` and an `[[auth.party]]` table, every caller is
    /// looked up in the party binding the tables and `[authz.party_binding]`
    /// describe, on every request.
    ///
    /// # Errors
    /// A message naming the refused setting, the issuer that could not be
    /// read, the metadata file that could not be read, names no `[auth]`
    /// issuer or does not fit the issuer it names, a served document whose
    /// issuer `describe` refuses (an `[auth] described_issuer` that names no
    /// issuer the document can describe, or no `described_issuer` and several
    /// or no issuers it can describe), an `[auth.dpop]` table without a public
    /// root, a `patient_claim` that names no claim,
    /// or a party binding the shared crate refuses or that has
    /// `[[auth.party]]` tables and no `source`.
    pub async fn new(
        config: &AppConfig,
        patient_claim: Option<&str>,
        source: Option<Arc<dyn BindingSource>>,
        replay: Arc<dyn ReplayStore>,
    ) -> Result<Arc<Self>, String> {
        let resolved = ferroehr::config::auth::resolve(&config.auth.shared)
            .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?;
        let party = super::party::binding(config, &resolved, source)
            .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?;
        let mut gate = Gate::open();
        let mut primary = None;
        let claim = patient_claim
            .map(PatientClaim::new)
            .transpose()
            .map_err(|e| format!("authz.abac.patient_claim is refused: {e}"))?;
        if resolved.enabled {
            let mut issuers = IssuerSet::new();
            let mut candidates = Vec::new();
            issuers = introspected(
                resolved.introspection,
                claim.as_ref(),
                issuers,
                &mut candidates,
            )
            .await?;
            let mut supplied = match config.auth.smart.issuer_metadata_file.as_deref() {
                Some(path) => Some(issuer::issuer_metadata(path).await?),
                None => None,
            };
            for settings in resolved.issuers {
                let issuer = settings.issuer.clone();
                let mut server = ResourceServer::discover(settings)
                    .await
                    .map_err(|e| issuer::unreadable_issuer(&issuer, &e))?;
                let named = supplied.as_ref().is_some_and(|m| m.issuer == issuer);
                if let Some(metadata) = supplied.take_if(|_| named) {
                    server = server.with_metadata(metadata).map_err(|e| {
                        format!(
                            "auth.smart.issuer_metadata_file does not fit the [auth] issuer \
                             {issuer}: {}",
                            ferroehr::error::ErrorChain::new(&e)
                        )
                    })?;
                }
                if let Some(claim) = &claim {
                    server = server.with_extension(claim.clone());
                }
                let server = Arc::new(server);
                candidates.push(Candidate::of_server(
                    &server,
                    Primary::Jwt(Arc::clone(&server)),
                ));
                issuers = issuers.with(server);
            }
            if let Some(metadata) = supplied {
                return Err(format!(
                    "auth.smart.issuer_metadata_file names the issuer {}, which is no [auth] issuer",
                    metadata.issuer
                ));
            }
            if config.auth.smart.discovery {
                let described = describe(candidates, resolved.described_issuer.as_ref())
                    .map_err(|e| refusal(&e))?;
                primary = Some(described.into_check());
            }
            if !issuers.is_empty() {
                gate = gate.with_bearer(Arc::new(issuers));
            }
            if let Some(dpop) = resolved.dpop {
                let root = config.public_root.as_ref().ok_or_else(|| {
                    "[auth.dpop] requires server.public_base_url, the URL a proof's htu names"
                        .to_owned()
                })?;
                let check = ProofCheck::new(dpop.settings(root.url().clone()), replay)
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
            gate = gate
                .with_realm(ferroehr::config::auth::realm(&config.auth.shared))
                .with_honoured(honoured(config));
        }
        Ok(Arc::new(Self {
            enabled: config.auth.shared.enabled,
            gate,
            realm: ferroehr::config::auth::realm(&config.auth.shared).to_owned(),
            primary,
            role_issuers: RoleIssuers::of(config),
            party,
            declared: super::declared::Declared::of(config),
        }))
    }
}

/// Adds the introspection check of each of `settings` to `issuers`, offering
/// each issuer to the discovery document as one of `candidates`, every caller
/// it admits carrying `claim`.
///
/// # Errors
/// A message naming the issuer whose introspection endpoint could not be read.
async fn introspected(
    settings: Vec<IntrospectionSettings>,
    claim: Option<&PatientClaim>,
    mut issuers: IssuerSet,
    candidates: &mut Vec<Candidate<Primary>>,
) -> Result<IssuerSet, String> {
    for settings in settings {
        let issuer = settings.issuer.clone();
        let mut introspector = Introspector::discover(settings).await.map_err(|e| {
            format!(
                "the introspection endpoint of {issuer} could not be read: {}",
                ferroehr::error::ErrorChain::new(&e)
            )
        })?;
        if let Some(claim) = claim {
            introspector = introspector.with_extension(claim.clone());
        }
        let introspector = Arc::new(introspector);
        candidates.push(Candidate::of_introspector(
            &introspector,
            Primary::Introspection(Arc::clone(&introspector)),
        ));
        issuers = issuers.with_introspector(introspector);
    }
    Ok(issuers)
}

/// Returns what `[auth.smart]` honours for a caller: a `patient_issuers`
/// issuer. Whether it is a backend client is its `query_system_wide` grant,
/// which the middleware reads from the CDR once the caller's grants resolve.
fn honoured(config: &AppConfig) -> impl Fn(&Caller) -> Honoured + Send + Sync + 'static {
    let patient = config.auth.smart.patient_issuers.clone();
    move |caller| Honoured {
        backend: false,
        patient: caller
            .issuer()
            .is_some_and(|issuer| patient.iter().any(|listed| listed.expose() == issuer)),
    }
}

/// The refusal of the issuer the served discovery document describes,
/// naming the key that chooses it and, for missing metadata, the key that
/// supplies it.
fn refusal(e: &DescribeError) -> String {
    let supply = match e {
        DescribeError::NoMetadata { .. } | DescribeError::NoneEligible => {
            "; auth.smart.issuer_metadata_file supplies the metadata of an issuer whose key set \
             is a jwks_file"
        }
        _ => "",
    };
    format!(
        "auth.smart.discovery = true cannot describe an issuer (auth.described_issuer): \
         {e}{supply}"
    )
}
