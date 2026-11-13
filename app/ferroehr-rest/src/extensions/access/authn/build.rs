// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Building the [`Authenticator`]: the issuers of `[auth]`, read at start,
//! each asking the policy grants for a client's standing, the `DPoP` proof
//! check of `[auth.dpop]`, the Basic store, the issuers whose role claims may
//! grant a privileged role (`[authz.rbac] role_issuers`) and the first
//! administrator (`[authz.rbac] first_admin`). No openEHR spec governs
//! authentication mechanics: our own design.

use std::sync::Arc;

use ferroehr::party_binding::source::BindingSource;
use ferrohealth_smart::guard::Gate;
use ferrohealth_smart::kind::standing::Standing;
use ferrohealth_smart::resource::described::{Candidate, DescribeError, describe};
use ferrohealth_smart::resource::introspection::{IntrospectionSettings, Introspector};
use ferrohealth_smart::resource::{Caller, Honoured, IssuerSet, PatientClaim, ResourceServer, Via};
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

/// The privileged roles of `[authz.rbac]`, the issuers whose tokens may
/// grant them and the first administrator.
#[derive(Debug, Default)]
pub(super) struct RoleIssuers {
    /// `admin_role` and `emergency_access_role`, upper-cased.
    privileged: Vec<String>,
    /// `role_issuers`.
    issuers: Vec<SecretUrl>,
    /// `first_admin`, with what it holds.
    first_admin: Option<FirstAdmin>,
}

/// The Basic user `[authz.rbac] first_admin` names, and what it holds from
/// the configuration.
#[derive(Debug)]
pub(super) struct FirstAdmin {
    /// The Basic user.
    user: String,
    /// `admin_role`, upper-cased.
    pub(super) role: String,
    /// `[auth] product_scope`.
    pub(super) scope: String,
}

impl RoleIssuers {
    /// Returns the rule of `config`'s `[authz.rbac]`.
    fn of(config: &AppConfig) -> Self {
        Self {
            privileged: config.rbac.privileged_roles(),
            issuers: config.rbac.role_issuers.clone(),
            first_admin: config.rbac.first_admin.clone().map(|user| FirstAdmin {
                user,
                role: config.rbac.admin_role.trim().to_ascii_uppercase(),
                scope: ferroehr::config::auth::product_scope(&config.auth.shared).to_owned(),
            }),
        }
    }

    /// The first administrator, when `caller` is that Basic user.
    pub(super) fn first_admin(&self, caller: &Caller) -> Option<&FirstAdmin> {
        self.first_admin
            .as_ref()
            .filter(|admin| caller.via() == Via::Basic && caller.subject() == admin.user)
    }

    /// Whether `role`, upper-cased, is a privileged role.
    pub(super) fn is_privileged(&self, role: &str) -> bool {
        self.privileged.iter().any(|held| held == role)
    }

    /// Returns the upper-cased roles `caller` holds: a token of an issuer
    /// `role_issuers` does not list grants no privileged role, and the first
    /// administrator holds `admin_role`.
    pub(super) fn roles(&self, caller: &Caller) -> Vec<String> {
        let mut roles = upper_cased(caller.roles());
        if let Some(issuer) = caller.issuer()
            && !self.issuers.iter().any(|listed| listed.expose() == issuer)
        {
            roles.retain(|role| !self.privileged.contains(role));
        }
        if let Some(admin) = self.first_admin(caller)
            && !roles.contains(&admin.role)
        {
            roles.push(admin.role.clone());
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
    /// describe, on every request. `standing` answers which clients a
    /// person's token comes through, which actors are delegates and which
    /// system subjects need no scope, as the token is classified.
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
        standing: Arc<dyn Standing>,
    ) -> Result<Arc<Self>, String> {
        let resolved = ferroehr::config::auth::resolve_with(&config.auth.shared, standing)
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
            declared: super::declared::Declared::default(),
            namespace: config.party_binding.namespace.clone(),
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
