// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Building the [`Authenticator`]: the issuers of `[auth]`, each read in the
//! background from the start and asking the policy grants for a client's
//! standing, the `DPoP` proof
//! check of `[auth.dpop]`, the Basic store, the issuers whose role claims may
//! grant a privileged role (`[authz.rbac] role_issuers`) and the first
//! administrator (`[authz.rbac] first_admin`). No openEHR spec governs
//! authentication mechanics: our own design.

use std::sync::Arc;

use ferroehr::party_binding::source::BindingSource;
use ferrohealth_smart::guard::Gate;
use ferrohealth_smart::kind::standing::Standing;
use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::resource::described::{Candidate, DescribeError, Metadata, describe};
use ferrohealth_smart::resource::introspection::{IntrospectionSettings, Introspector};
use ferrohealth_smart::resource::{
    Caller, Honoured, IssuerSet, LazyResourceServer, ResourceServer, Settings, Via,
};
use ferrohealth_smart::sender::proof::ProofCheck;
use ferrohealth_smart::sender::replay::ReplayStore;

use super::verdicts::TokenVerdicts;
use super::{Authenticator, issuer, upper_cased};
use crate::config::AppConfig;

/// The check of the described issuer, whose metadata the SMART discovery
/// document is built from.
pub(super) enum Primary {
    /// A JWT access token checked against the issuer's keys, read at start.
    Jwt(Arc<ResourceServer>),
    /// A JWT access token checked against the issuer's keys, read in the
    /// background, so the document waits for that read.
    Lazy(Arc<LazyResourceServer>),
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
    /// The identifiers of the issuers `role_issuers` names.
    issuers: Vec<String>,
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
    /// Returns the rule of `config`'s `[authz.rbac]`, its issuer names read
    /// through `resolved`.
    ///
    /// # Errors
    /// A message naming a role issuer that names no `persons` issuer of
    /// `[auth]`.
    fn of(config: &AppConfig, resolved: &Resolved) -> Result<Self, String> {
        let issuers = resolved
            .role_issuers("authz.rbac.role_issuers", &config.rbac.role_issuers)
            .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?;
        Ok(Self {
            privileged: config.rbac.privileged_roles(),
            issuers,
            first_admin: config.rbac.first_admin.clone().map(|user| FirstAdmin {
                user,
                role: config.rbac.admin_role.trim().to_ascii_uppercase(),
                scope: ferroehr::config::auth::product_scope(&config.auth.shared).to_owned(),
            }),
        })
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
            && !self.issuers.iter().any(|listed| listed == issuer)
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
    /// Builds the authenticator of `config`. A JWT issuer read from its
    /// metadata is read in the background from the start: until it is, its
    /// tokens are refused with `503` (`issuer-unread`) and the readiness
    /// route reports the server not ready ([`Authenticator::issuers_ready`]),
    /// so the server starts whether or not its authorization server is up. A
    /// `jwks_file` issuer and an introspected issuer are read here, from a
    /// local file and the introspection endpoint, and one that cannot be
    /// read refuses the start.
    ///
    /// `[auth.smart] issuer_metadata_file` is the metadata of the issuer it names,
    /// whose key set is a `jwks_file`, so its discovery document can name a
    /// `token_endpoint`. When the SMART discovery document is served, it
    /// describes the issuer `ferrohealth-smart`'s `describe` chooses:
    /// `[auth] described_issuer`, else the one issuer of a user-token kind whose
    /// metadata is available, else, with only `machines` issuers trusted, the one
    /// `machines` issuer, for SMART Backend Services alone. The `DPoP` proof
    /// check of `[auth.dpop]` compares a proof's `htu` with the server's
    /// public root (`[server] public_base_url` with `server.base_path`) and
    /// the request path, and records each accepted proof in `replay`, which
    /// the server builds over the clinical pool every replica shares.
    ///
    /// With a party `source` and an `[auth.issuer.<name>.party]` table, every
    /// caller is looked up in the party binding the tables and
    /// `[authz.party_binding]` describe, on every request. `standing` answers
    /// which clients a person's token comes through, which actors are
    /// delegates and which system subjects need no scope, as the token is
    /// classified.
    ///
    /// # Errors
    /// A message naming the refused setting, the `jwks_file` or introspected
    /// issuer that could not be read, the metadata file that could not be
    /// read, names no `[auth]` issuer or does not fit the issuer it names, a
    /// served document whose issuer `describe` refuses (an `[auth]
    /// described_issuer` that names no issuer the document can describe, or
    /// no `described_issuer` and several or no issuers it can describe), an
    /// `[auth.dpop]` table without a public root, a role issuer that names no
    /// `persons` issuer, or a party binding the shared crate refuses or that
    /// has party tables and no `source`.
    pub async fn new(
        config: &AppConfig,
        source: Option<Arc<dyn BindingSource>>,
        replay: Arc<dyn ReplayStore>,
        standing: Arc<dyn Standing>,
    ) -> Result<Arc<Self>, String> {
        let resolved = ferroehr::config::auth::resolve_with(
            &config.auth.shared,
            standing,
            &config.audience_bases(),
        )
        .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?;
        let party = super::party::binding(config, &resolved, source)
            .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?;
        let deployment = super::party::Deployment::of(config, &resolved);
        let role_issuers = RoleIssuers::of(config, &resolved)?;
        let patient = patient_context(&resolved);
        let mut gate = Gate::open();
        let mut primary = None;
        let mut bearer = None;
        let mut reads = BackgroundReads::default();
        if resolved.enabled {
            let mut issuers = IssuerSet::new();
            let mut candidates = Vec::new();
            issuers = introspected(resolved.introspection, issuers, &mut candidates).await?;
            let mut supplied = match config.auth.smart.issuer_metadata_file.as_deref() {
                Some(path) => Some(issuer::issuer_metadata(path).await?),
                None => None,
            };
            for settings in resolved.issuers {
                if settings.jwks_file.is_none() {
                    let lazy = Arc::new(
                        ResourceServer::lazy(settings)
                            .map_err(|e| ferroehr::error::ErrorChain::new(&e).to_string())?,
                    );
                    let settings = lazy.settings();
                    candidates.push(Candidate::new(
                        settings.issuer.clone(),
                        settings.kind,
                        Metadata::Available,
                        Primary::Lazy(Arc::clone(&lazy)),
                    ));
                    issuers = issuers.with_lazy(lazy);
                    continue;
                }
                let server = Arc::new(local(settings, &mut supplied).await?);
                candidates.push(Candidate::of_server(
                    &server,
                    Primary::Jwt(Arc::clone(&server)),
                ));
                issuers = issuers.with(server);
            }
            if let Some(metadata) = supplied {
                return Err(format!(
                    "auth.smart.issuer_metadata_file names the issuer {}, which is no [auth] \
                     issuer with a jwks_file",
                    metadata.issuer
                ));
            }
            if config.auth.smart.discovery {
                let described = describe(candidates, resolved.described_issuer.as_ref())
                    .map_err(|e| refusal(&e))?;
                let purpose = described.purpose();
                primary = Some((described.into_check(), purpose));
            }
            if !issuers.is_empty() {
                let issuers = Arc::new(issuers);
                reads = BackgroundReads(issuers.read_in_background());
                bearer = Some(TokenVerdicts(Arc::clone(&issuers)));
                gate = gate.with_bearer(issuers);
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
                .with_honoured(honoured(patient.clone()));
        }
        Ok(Arc::new(Self {
            enabled: config.auth.shared.enabled,
            gate,
            bearer,
            realm: ferroehr::config::auth::realm(&config.auth.shared).to_owned(),
            primary,
            patient_context: !patient.is_empty(),
            reads,
            role_issuers,
            party,
            declared: super::declared::Declared::new(config.rate_clock.clone()),
            namespace: config.party_binding.namespace.clone(),
            local_issuer: config.api.basic_issuer().into(),
            deployment,
        }))
    }
}

/// The background reads of the lazily read issuers, stopped when the
/// authenticator that started them is dropped.
#[derive(Debug, Default)]
pub(super) struct BackgroundReads(Vec<tokio::task::JoinHandle<()>>);

impl Drop for BackgroundReads {
    fn drop(&mut self) {
        for read in &self.0 {
            read.abort();
        }
    }
}

/// Reads the `jwks_file` issuer of `settings` from its local file, with the
/// metadata of `[auth.smart] issuer_metadata_file` when `supplied` names it.
///
/// # Errors
/// A message naming the issuer whose key set could not be read, or the
/// metadata file that does not fit it.
async fn local(
    settings: Settings,
    supplied: &mut Option<ferrohealth_smart::metadata::ServerMetadata>,
) -> Result<ResourceServer, String> {
    let issuer = settings.issuer.clone();
    let server = ResourceServer::discover(settings)
        .await
        .map_err(|e| issuer::unreadable_issuer(&issuer, &e))?;
    let named = supplied.as_ref().is_some_and(|m| m.issuer == issuer);
    match supplied.take_if(|_| named) {
        Some(metadata) => server.with_metadata(metadata).map_err(|e| {
            format!(
                "auth.smart.issuer_metadata_file does not fit the [auth] issuer {issuer}: {}",
                ferroehr::error::ErrorChain::new(&e)
            )
        }),
        None => Ok(server),
    }
}

/// Adds the introspection check of each of `settings` to `issuers`, offering
/// each issuer to the discovery document as one of `candidates`.
///
/// # Errors
/// A message naming the issuer whose introspection endpoint could not be read.
async fn introspected(
    settings: Vec<IntrospectionSettings>,
    mut issuers: IssuerSet,
    candidates: &mut Vec<Candidate<Primary>>,
) -> Result<IssuerSet, String> {
    for settings in settings {
        let issuer = settings.issuer.clone();
        let introspector = Introspector::discover(settings).await.map_err(|e| {
            format!(
                "the introspection endpoint of {issuer} could not be read: {}",
                ferroehr::error::ErrorChain::new(&e)
            )
        })?;
        let introspector = Arc::new(introspector);
        candidates.push(Candidate::of_introspector(
            &introspector,
            Primary::Introspection(Arc::clone(&introspector)),
        ));
        issuers = issuers.with_introspector(introspector);
    }
    Ok(issuers)
}

/// Returns the identifiers of the issuers trusted to assert a patient
/// context: those whose profile sets `patient_context` (SMART App Launch
/// 2.2.0, Scopes and Launch Context, `launch/patient`).
fn patient_context(resolved: &Resolved) -> Vec<String> {
    let jwt = resolved
        .issuers
        .iter()
        .filter(|settings| settings.profile.patient_context)
        .map(|settings| settings.issuer.clone());
    let introspected = resolved
        .introspection
        .iter()
        .filter(|settings| settings.profile.patient_context)
        .map(|settings| settings.issuer.clone());
    jwt.chain(introspected).collect()
}

/// Returns what FerroEHR honours for a caller: a `patient/` grant of an
/// issuer trusted to assert a patient context, confined to the `ehrId` of
/// that context. Whether it is a backend client is its `query_system_wide`
/// grant, which the middleware reads from the CDR once the caller's grants
/// resolve.
fn honoured(patient: Vec<String>) -> impl Fn(&Caller) -> Honoured + Send + Sync + 'static {
    move |caller| Honoured {
        backend: false,
        patient: caller
            .issuer()
            .is_some_and(|issuer| patient.iter().any(|listed| listed == issuer)),
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
