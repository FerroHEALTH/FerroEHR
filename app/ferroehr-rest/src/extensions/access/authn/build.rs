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
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::resource::introspection::{IntrospectionSettings, Introspector};
use ferrohealth_smart::resource::{Caller, Honoured, IssuerSet, PatientClaim, ResourceServer};
use ferrohealth_smart::sender::proof::ProofCheck;
use ferrohealth_smart::sender::replay::ReplayStore;

use super::{Authenticator, Primary, issuer, upper_cased};
use crate::config::AppConfig;
use ferroehr::config::smart::SmartConfig;
use ferrohealth_secret::secret_url::SecretUrl;

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
    /// `token_endpoint`. The SMART discovery document describes
    /// `[auth.smart] described_issuer`, else the one
    /// issuer that is not `machines` and whose metadata is available. Every
    /// caller an issuer admits carries `patient_claim`, the live ABAC patient
    /// claim, as the `ferrohealth-smart` `PatientClaim`. The `DPoP` proof
    /// check of `[auth.dpop]` records each accepted proof in `replay`, which
    /// the server builds over the clinical pool every replica shares.
    ///
    /// With a party `source` and an `[[auth.party]]` table, every caller is
    /// looked up in the party binding the tables and `[authz.party_binding]`
    /// describe, on every request.
    ///
    /// # Errors
    /// A message naming the refused setting, the issuer that could not be
    /// read, the metadata file that could not be read, names no `[auth]`
    /// issuer or does not fit the issuer it names, an `[auth.smart]
    /// described_issuer` that names no issuer the document can describe, a
    /// served document with no `described_issuer` and several or no issuers
    /// it can describe, a
    /// `patient_claim` that names no claim, or a party binding the shared
    /// crate refuses.
    pub async fn new(
        config: &AppConfig,
        patient_claim: Option<&str>,
        source: Option<Arc<dyn BindingSource>>,
        replay: Arc<dyn ReplayStore>,
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
            let mut described = Described::default();
            issuers = introspected(
                resolved.introspection,
                claim.as_ref(),
                issuers,
                &mut described,
            )
            .await?;
            let mut supplied = match config.auth.smart.issuer_metadata_file.as_deref() {
                Some(path) => Some(issuer::issuer_metadata(path).await?),
                None => None,
            };
            for settings in resolved.issuers {
                let issuer = settings.issuer.clone();
                let read = settings.jwks_file.is_none();
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
                described.offer(Offer {
                    kind: resolved.kinds.get(&issuer).copied(),
                    issuer,
                    metadata: named || read,
                    primary: Primary::Jwt(Arc::clone(&server)),
                });
                issuers = issuers.with(server);
            }
            if let Some(metadata) = supplied {
                return Err(format!(
                    "auth.smart.issuer_metadata_file names the issuer {}, which is no [auth] issuer",
                    metadata.issuer
                ));
            }
            primary = described.chosen(&config.auth.smart)?;
            if !issuers.is_empty() {
                gate = gate.with_bearer(Arc::new(issuers));
            }
            if let Some(settings) = resolved.dpop {
                let check = ProofCheck::new(settings, replay)
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
/// each issuer to `described`, every caller it admits carrying `claim`.
///
/// # Errors
/// A message naming the issuer whose introspection endpoint could not be read.
async fn introspected(
    settings: Vec<IntrospectionSettings>,
    claim: Option<&PatientClaim>,
    mut issuers: IssuerSet,
    described: &mut Described,
) -> Result<IssuerSet, String> {
    for settings in settings {
        let issuer = settings.issuer.clone();
        let kind = Some(settings.kind);
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
        described.offer(Offer {
            issuer,
            kind,
            metadata: true,
            primary: Primary::Introspection(Arc::clone(&introspector)),
        });
        issuers = issuers.with_introspector(introspector);
    }
    Ok(issuers)
}

/// Returns what `[auth.smart]` honours for a caller: a `backend_clients` client
/// and a `patient_issuers` issuer.
fn honoured(config: &AppConfig) -> impl Fn(&Caller) -> Honoured + Send + Sync + 'static {
    let backend = config.auth.smart.backend_clients.clone();
    let patient = config.auth.smart.patient_issuers.clone();
    move |caller| Honoured {
        backend: caller
            .client_id()
            .is_some_and(|client| backend.iter().any(|listed| listed == client)),
        patient: caller
            .issuer()
            .is_some_and(|issuer| patient.iter().any(|listed| listed.expose() == issuer)),
    }
}

/// One `[auth]` issuer offered to the discovery document.
struct Offer {
    /// The issuer identifier.
    issuer: String,
    /// Its `kind`.
    kind: Option<IssuerKind>,
    /// Whether its metadata is available: introspected, read from it, or
    /// supplied by `[auth.smart] issuer_metadata_file`.
    metadata: bool,
    /// Its check, which builds the document.
    primary: Primary,
}

impl Offer {
    /// Whether the document may describe this issuer: one whose metadata is
    /// available and that serves a user launch, which every kind but
    /// `machines` does; `ferrohealth-smart` refuses every token of a
    /// `machines` issuer that is not app-only.
    fn eligible(&self) -> bool {
        self.metadata && self.kind != Some(IssuerKind::Machines)
    }
}

/// The issuers the SMART discovery document may describe. SMART App Launch
/// 2.2.0, Conformance, §Metadata requires `authorization_endpoint` when a
/// launch capability is advertised, so the document describes one issuer that
/// serves a user launch. Which issuer, when a deployment trusts several, is
/// not governed by SMART: our own design, under which the order of the
/// `[[auth.issuer]]` tables never decides.
#[derive(Default)]
struct Described {
    /// Every issuer offered.
    offers: Vec<Offer>,
}

impl Described {
    /// Offers `offer`.
    fn offer(&mut self, offer: Offer) {
        self.offers.push(offer);
    }

    /// Returns the check of the issuer `smart` describes: its
    /// `described_issuer`, when set, else the one eligible issuer. With the
    /// document not served and no name, several or no eligible issuers
    /// describe none.
    ///
    /// # Errors
    /// A message naming `auth.smart.described_issuer` when it names no
    /// eligible issuer; when the document is served with no name, a message
    /// naming the eligible issuers and the key when there are several, or
    /// saying no issuer can be described when there is none.
    fn chosen(self, smart: &SmartConfig) -> Result<Option<Primary>, String> {
        let served = smart.discovery;
        if let Some(named) = smart.described_issuer.as_ref().map(SecretUrl::expose) {
            let offer = self
                .offers
                .into_iter()
                .find(|offer| offer.issuer == named)
                .ok_or_else(|| {
                    format!("auth.smart.described_issuer names {named}, which is no [auth] issuer")
                })?;
            if offer.kind == Some(IssuerKind::Machines) {
                return Err(format!(
                    "auth.smart.described_issuer names the machines issuer {named}: a machines \
                     issuer signs client credentials tokens only, so the discovery document \
                     cannot send a user launch to it"
                ));
            }
            if !offer.metadata {
                return Err(format!(
                    "auth.smart.described_issuer names {named}, whose metadata FerroEHR does not \
                     have: its key set is a jwks_file, so auth.smart.issuer_metadata_file must \
                     supply the metadata"
                ));
            }
            return Ok(Some(offer.primary));
        }
        let mut eligible: Vec<Offer> = self.offers.into_iter().filter(Offer::eligible).collect();
        match (eligible.pop(), eligible.is_empty()) {
            (Some(offer), true) => Ok(Some(offer.primary)),
            (Some(last), false) if served => {
                let listed: Vec<&str> = eligible
                    .iter()
                    .chain(std::iter::once(&last))
                    .map(|offer| offer.issuer.as_str())
                    .collect();
                Err(format!(
                    "auth.smart.discovery = true can describe any of the issuers {}; set \
                     auth.smart.described_issuer to the one whose authorization server user \
                     launches start at",
                    listed.join(", ")
                ))
            }
            (None, _) if served => Err(
                "auth.smart.discovery = true can describe no issuer: the document needs an [auth] \
                 issuer that is not of kind machines and whose metadata FerroEHR has (read from \
                 it, introspected, or supplied by auth.smart.issuer_metadata_file)"
                    .to_owned(),
            ),
            _ => Ok(None),
        }
    }
}
