// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login binding of every authenticated request: the caller is looked up
//! in the party binding, read from the local party source with no cache, and
//! the request carries the outcome or is refused for it.
//!
//! A party that is `inactive` or `abandoned` (RM common
//! `master06-change_control_package.adoc` §Abandoned and Inactive States), a
//! login two parties carry, a deprovisioned key, a credential without a key,
//! a party without the marker the caller's kind needs, groups that cannot be
//! read and a person whose login no party carries are each refused with
//! `403` and their binding code, with no fallback to a login-only committer
//! (RM common `AUDIT_DETAILS.committer` names who did it). A source that
//! cannot answer is the server's failure, never the person's, and answers
//! `503`. No openEHR spec governs resolving a
//! login to a party: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use ferroehr::config::binding::ProviderId;
use ferroehr::error::ServiceError;
use ferroehr::party_binding::grants::capabilities::Capabilities;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::versioning::committer::bound::BoundDetails;

use ferrohealth_smart::assurance::AssuranceSection;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::Outcome;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::resource::{Access, Caller};

use super::AuthError;

pub(super) mod closed;
mod jit;
pub(crate) mod own;

/// The party binding of the deployment, with what the committer of a bound
/// caller carries beside it: the provider, the namespace and the source of
/// the `PERSON`'s name.
pub(crate) struct Binding {
    /// The shared crate's binding.
    shared: PartyBinding,
    /// The source logins are looked up in, which also names a `PERSON`.
    source: Arc<dyn BindingSource>,
    /// The provider the deployment acts for.
    deployment: Deployment,
    /// `[authz.party_binding] namespace`.
    namespace: String,
    /// Each issuer's `identifier_issuer`s, which a `refused-unbound` problem
    /// names so the operator can provision the person.
    identifier_issuers: BTreeMap<String, Vec<String>>,
    /// The just-in-time creation of each bound issuer, or why the
    /// mapping cannot write a provisional `PERSON`.
    jit: Result<BTreeMap<String, jit::Jit>, Vec<String>>,
    /// `[authz.assurance]`, which an `admit_assurance` condition reads.
    assurance: AssuranceSection,
}

/// What a login-binding refusal's problem text names beside its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// The key claim the token lacked (`refused-key-claim-missing`).
    Claim(String),
    /// The identifier issuers under which no demographic party carries the
    /// login's identifier (`refused-unbound`).
    IdentifierIssuer(String),
}

impl Named {
    /// The key claim, for a [`Named::Claim`].
    #[must_use]
    pub fn claim(&self) -> Option<&str> {
        match self {
            Self::Claim(claim) => Some(claim),
            Self::IdentifierIssuer(_) => None,
        }
    }
}

impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Binding")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

/// Why the party binding of `[auth.issuer.<name>.party]` and `[authz.party_binding]`
/// cannot be built.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BindingRefused {
    /// `[auth.issuer.<name>.party]` tables bind logins, but no party source was given to
    /// look them up in, so every caller would be served unbound.
    #[error(
        "a party table binds logins to parties, but no party source was given to look them up in"
    )]
    NoSource,
    /// The shared crate refuses the binding.
    #[error("authz.party_binding")]
    Shared(#[source] ferroehr::config::binding::BindingError),
}

/// Returns the party binding of `config`'s `[auth.issuer.<name>.party]` tables and
/// `[authz.party_binding]`, looking logins up in `source`; `None` when
/// `[auth]` is off or binds no issuer.
///
/// # Errors
/// [`BindingRefused::NoSource`] for `[auth.issuer.<name>.party]` tables without a
/// `source`, and [`BindingRefused::Shared`] for a binding the shared crate
/// refuses.
pub(super) fn binding(
    config: &crate::config::AppConfig,
    resolved: &ferrohealth_smart::config::Resolved,
    source: Option<Arc<dyn BindingSource>>,
) -> Result<Option<Arc<Binding>>, BindingRefused> {
    if !resolved.enabled || resolved.party.is_empty() {
        return Ok(None);
    }
    let source = source.ok_or(BindingRefused::NoSource)?;
    let lookup: Arc<dyn ferrohealth_smart::party::source::PartySource> =
        Arc::<dyn BindingSource>::clone(&source);
    let binding = ferroehr::config::binding::party_binding(&config.party_binding, resolved, lookup)
        .map_err(BindingRefused::Shared)?;
    let mut identifier_issuers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for party in &resolved.party {
        identifier_issuers
            .entry(party.issuer.expose().to_owned())
            .or_default()
            .push(party.identifier_issuer.clone());
    }
    Ok(Some(Arc::new(Binding {
        shared: binding,
        source,
        deployment: Deployment::of(config),
        namespace: config.party_binding.namespace.clone(),
        identifier_issuers,
        jit: jit::tables(&config.party_binding, resolved),
        assurance: config.assurance.clone(),
    })))
}

/// The healthcare provider the deployment acts for:
/// `[authz.party_binding] provider`, whatever issuer binds a caller.
#[derive(Debug, Clone, Default)]
pub(crate) struct Deployment {
    /// `[authz.party_binding] provider`.
    provider: Option<ProviderId>,
}

impl Deployment {
    /// The provider of `config`.
    pub(super) fn of(config: &crate::config::AppConfig) -> Self {
        Self {
            provider: config.party_binding.provider.clone(),
        }
    }

    /// The healthcare provider the deployment acts for, as `(issuer, id)`.
    pub(super) fn provider(&self) -> Option<(String, String)> {
        self.provider
            .as_ref()
            .map(|one| (one.issuer.clone(), one.id.clone()))
    }

    /// [`Self::provider`], borrowed from the deployment's configuration.
    pub(super) fn provider_named(&self) -> Option<(&str, &str)> {
        self.provider
            .as_ref()
            .map(|one| (one.issuer.as_str(), one.id.as_str()))
    }
}

impl Binding {
    /// What the committer of the bound caller of `principal` carries beside
    /// the binding; `None` for a caller the binding did not bind.
    ///
    /// # Errors
    /// [`AuthError::PartyUnavailable`] when the source cannot name the
    /// `PERSON`.
    pub(super) async fn details(
        &self,
        principal: &super::Principal,
    ) -> Result<Option<BoundDetails>, AuthError> {
        let Some(caller) = principal.access.caller() else {
            return Ok(None);
        };
        let Some(bound) = caller.party().and_then(Outcome::bound) else {
            return Ok(None);
        };
        let name = self
            .source
            .person_name(bound.party.id())
            .await
            .map_err(|error| {
                tracing::warn!(
                    error = %ferroehr::error::ErrorChain::new(&error),
                    "the party source could not name the bound PERSON"
                );
                AuthError::PartyUnavailable { retry_after: None }
            })?;
        Ok(Some(BoundDetails {
            name,
            provider: self.deployment.provider(),
            namespace: self.namespace.clone(),
            portal: None,
        }))
    }

    /// The outcome of the unbound `caller`, holding `roles`, after
    /// just-in-time creation, read again from the source once a provisional
    /// `PERSON` is committed; `None` when the issuer creates no party for
    /// the caller.
    ///
    /// A commit that loses the key race to a concurrent first request, or
    /// whose `userName` another party holds, is a conflict: the source is
    /// read again, and binds the winner's `PERSON` or still nothing.
    ///
    /// # Errors
    /// [`AuthError::PartyUnavailable`] when the commit or the source fails
    /// for the server's own reason.
    async fn just_in_time(
        &self,
        caller: &Caller,
        roles: &[String],
    ) -> Result<Option<Outcome>, AuthError> {
        let issuer = caller.issuer().unwrap_or("-");
        if !jit::candidate(caller) {
            return Ok(None);
        }
        let admissions = self.source.admissions(issuer).await.map_err(|error| {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the admission rules of just-in-time creation could not be read"
            );
            AuthError::PartyUnavailable { retry_after: None }
        })?;
        if admissions.is_empty() {
            return Ok(None);
        }
        let jit = match &self.jit {
            Ok(tables) => match tables.get(issuer) {
                Some(jit) => jit,
                None => return Ok(None),
            },
            Err(reasons) => {
                tracing::warn!(
                    issuer,
                    reasons = %reasons.join("; "),
                    "a create_at_login grant admits this issuer's logins, and the party mapping \
                     cannot write a provisional PERSON"
                );
                return Ok(None);
            }
        };
        let person = match jit.person(
            caller,
            &admissions,
            (roles, &self.assurance),
            (&self.namespace, self.deployment.provider()),
        ) {
            Ok(person) => person,
            Err(reason) => {
                tracing::info!(
                    issuer,
                    reason = reason.as_str(),
                    "just-in-time creation does not admit the login"
                );
                return Ok(None);
            }
        };
        match self.source.create_provisional(person).await {
            Ok(_) | Err(ServiceError::Conflict(_)) => {}
            Err(
                error @ (ServiceError::BadRequest(_)
                | ServiceError::Unprocessable { .. }
                | ServiceError::ValidationFailed(_)
                | ServiceError::Forbidden(_)
                | ServiceError::NotFound(_)),
            ) => {
                tracing::warn!(
                    issuer,
                    error = %ferroehr::error::ErrorChain::new(&error),
                    "just-in-time creation refused the login's PERSON"
                );
                return Ok(None);
            }
            Err(error) => {
                tracing::warn!(
                    issuer,
                    error = %ferroehr::error::ErrorChain::new(&error),
                    "just-in-time creation could not commit the login's PERSON"
                );
                return Err(AuthError::PartyUnavailable { retry_after: None });
            }
        }
        self.shared
            .resolve(caller)
            .await
            .map_err(|error| closed::unanswered(&error))
    }
}

/// Looks the caller of `access` up in `binding`, with the roles
/// `role_issuers` honours for it,
/// returning the access whose caller carries the outcome
/// ([`Caller::party`]).
///
/// A caller no party carries gets a provisional `PERSON` where its issuer
/// creates one at a first login, and is bound to it.
///
/// # Errors
/// [`AuthError::PartyRefused`] for an outcome the request is refused for,
/// and [`AuthError::PartyUnavailable`] when the source or the identity
/// provider could not answer.
pub(super) async fn bind(
    binding: &Binding,
    access: Access,
    role_issuers: &super::build::RoleIssuers,
) -> Result<Access, AuthError> {
    let Some(caller) = access.caller() else {
        return Ok(access);
    };
    if matches!(caller.kind(), CallerKind::Patient) {
        return own::own_patient(binding, caller)
            .await
            .map(|bound| match bound {
                Some(bound) => Access::Caller(Arc::new(bound)),
                None => access.clone(),
            });
    }
    let outcome = match binding.shared.resolve(caller).await {
        Ok(None) => return Ok(access),
        Ok(Some(outcome)) => outcome,
        Err(error) => return Err(closed::unanswered(&error)),
    };
    let outcome = match outcome {
        Outcome::Unbound => binding
            .just_in_time(caller, &role_issuers.roles(caller))
            .await?
            .unwrap_or(Outcome::Unbound),
        outcome => outcome,
    };
    if let Some(mut refusal) = refusal(&outcome).filter(|_| {
        !own::binds_the_person(caller, &outcome) && !own::binds_a_representative(caller, &outcome)
    }) {
        let issuer = caller.issuer().unwrap_or("-");
        // NOTE: GDPR Art. 5(1)(c), the line names the issuer and the outcome, never the claim
        // value, which the access record of the request carries under its retention.
        tracing::warn!(
            issuer,
            outcome = outcome.as_str(),
            "the login binding refused the request"
        );
        ferroehr::telemetry::metrics::metrics()
            .binding_refusals
            .add(
                1,
                &[
                    opentelemetry::KeyValue::new("issuer", issuer.to_owned()),
                    opentelemetry::KeyValue::new("outcome", outcome.as_str()),
                ],
            );
        if let AuthError::PartyRefused { named, .. } = &mut refusal {
            // NOTE: OpenID Connect Core 1.0 §5.7, a login is bound by its token's `sub`.
            *named = match outcome {
                Outcome::KeyClaimMissing
                    if caller.via() == ferrohealth_smart::resource::Via::Bearer =>
                {
                    Some(Named::Claim("sub".to_owned()))
                }
                Outcome::Unbound => binding
                    .identifier_issuers
                    .get(issuer)
                    .map(|issuers| Named::IdentifierIssuer(issuers.join(", "))),
                _ => None,
            };
        }
        return Err(refusal);
    }
    Ok(Access::Caller(Arc::new(
        Caller::clone(caller).with_party(outcome),
    )))
}

/// The refusal of a representative whose asserted subject the party source
/// records no live representation of.
pub(super) const fn unrecorded() -> AuthError {
    AuthError::PartyRefused {
        code: "refused-representation-unrecorded",
        principal: None,
        party: None,
        named: None,
    }
}

/// The refusal `outcome` answers, or `None` for an outcome the request
/// proceeds under.
fn refusal(outcome: &Outcome) -> Option<AuthError> {
    let code = match outcome {
        Outcome::Party(_) => return None,
        Outcome::Ambiguous { .. } => "refused-ambiguous",
        Outcome::Inactive { .. } => "refused-inactive",
        Outcome::Abandoned { .. } => "refused-abandoned",
        Outcome::Deprovisioned => "refused-deprovisioned",
        Outcome::KeyClaimMissing => "refused-key-claim-missing",
        Outcome::WrongMarker { .. } => "refused-wrong-marker",
        Outcome::GroupsUnavailable(_) => "refused-groups-unavailable",
        // NOTE: `Outcome::Unbound`, and an outcome added later to the non-exhaustive
        // `Outcome` binds no party until it is read here, so both are unbound.
        _ => "refused-unbound",
    };
    Some(AuthError::PartyRefused {
        code,
        principal: None,
        party: outcome
            .party()
            .map(|party| Box::new(super::record::triple(party))),
        named: None,
    })
}

/// The text of the refusal `code`, naming what the operator or the person
/// can act on and never a claim value: the claim the token lacked and the
/// identifier issuer an unbound login is provisioned under, when `named`.
pub(super) fn detail(code: &str, named: Option<&Named>) -> String {
    let text = match code {
        "refused-ambiguous" => "two or more demographic parties carry this login's identifier",
        "refused-inactive" => "the demographic party this login is bound to is inactive",
        "refused-abandoned" => "the demographic party this login is bound to is abandoned",
        "refused-deprovisioned" => "this login's identifier belongs to a deprovisioned party",
        "refused-key-claim-missing" => {
            return match named.and_then(Named::claim) {
                Some(claim) => format!("the credential carries no binding key claim {claim:?}"),
                None => "the credential carries no binding key".to_owned(),
            };
        }
        "refused-wrong-marker" => {
            "the demographic party this login is bound to may not act in this role"
        }
        "refused-groups-unavailable" => "the groups this token asserts cannot be read",
        "refused-representation-unrecorded" => {
            "no live representation of the asserted subject is recorded for this login"
        }
        _ => {
            return match named {
                Some(Named::IdentifierIssuer(issuers)) => format!(
                    "no demographic party carries this login's identifier for the identifier \
                     issuer {issuers}: provision the person"
                ),
                _ => "this login is bound to no demographic party".to_owned(),
            };
        }
    };
    text.to_owned()
}

/// Returns the refusal of a caller its kind does not admit to the request
/// at all: a system caller holding a `user/` or `patient/` scope, which names
/// a person the token does not carry (SMART on openEHR `master08-scopes`
/// §Resource Scopes; RFC 9068 §2.2). No openEHR spec governs what a caller
/// kind may do: our own design.
pub(super) fn kind_refusal(caller: &Caller) -> Option<AuthError> {
    let personal = caller
        .scope()
        .iter()
        .any(|scope| scope.starts_with("user/") || scope.starts_with("patient/"));
    (matches!(caller.kind(), CallerKind::System) && personal).then(|| AuthError::KindRefused {
        code: "refused-system-personal-scope",
        message: "a system caller acts for no person, so it holds system/ scopes only".to_owned(),
        principal: None,
    })
}

/// Returns the platform committer identity published for the service layer.
///
/// A write whose request carried no committal headers is attributed to the
/// authenticated principal rather than the system identity
/// (`AUDIT_DETAILS.committer` 1..1 — RM common master04 §Audit Details). A
/// Bearer subject was minted by the identity provider, so the audit records the
/// validated token issuer; Basic credentials are local and carry none. A
/// system caller's client `ACTOR` is `granted`'s client, the party its issuer
/// and `client_id` name in the policy parties, when one does. A
/// representative admitted on a stored representation commits as the
/// representative, their `PERSON` in the details.
pub(super) fn committer_identity(
    principal: &super::Principal,
    granted: &Capabilities,
    local_issuer: &Arc<str>,
) -> ferroehr::versioning::committer::CommitterIdentity {
    let caller = principal.access.caller();
    let system = caller.is_some_and(|caller| matches!(caller.kind(), CallerKind::System));
    let client = super::client_of(principal);
    let portal = principal
        .represented
        .as_ref()
        .and_then(|represented| represented.portal.as_ref());
    ferroehr::versioning::committer::CommitterIdentity {
        subject: principal.subject.clone(),
        id_type: match principal.method {
            super::AuthMethod::Basic => "basic",
            super::AuthMethod::Bearer => "oauth2",
        },
        issuer: match principal.method {
            super::AuthMethod::Basic => None,
            super::AuthMethod::Bearer => principal
                .access
                .caller()
                .and_then(Caller::issuer)
                .map(str::to_owned),
        },
        local_issuer: Arc::clone(local_issuer),
        client,
        person: crate::smart::acts_as_person(principal) || portal.is_some(),
        party: principal
            .access
            .caller()
            .and_then(Caller::party)
            .and_then(Outcome::bound)
            .cloned(),
        representative: portal.is_some(),
        bound: portal.map(|portal| BoundDetails {
            namespace: portal.namespace.clone(),
            portal: Some(ferroehr::versioning::committer::bound::PortalParty {
                person: portal.person,
                relationship: portal.relationship.clone(),
                login_issuer: portal.login_issuer.clone(),
                login: portal.login.clone(),
            }),
            ..BoundDetails::default()
        }),
        system,
        agent: granted
            .client()
            .filter(|_| system)
            .map(|client| client.party),
        external: super::declared::committing(&principal.access),
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::party::{GroupsCause, PartyRef};

    use super::*;

    #[test]
    fn each_refused_outcome_names_its_binding_code() {
        let party = PartyRef::new("demographic", "PERSON", "p");
        for (outcome, code) in [
            (Outcome::Ambiguous { count: 2 }, "refused-ambiguous"),
            (
                Outcome::Inactive {
                    party: party.clone(),
                },
                "refused-inactive",
            ),
            (Outcome::Abandoned { party }, "refused-abandoned"),
            (Outcome::Deprovisioned, "refused-deprovisioned"),
            (Outcome::KeyClaimMissing, "refused-key-claim-missing"),
            (
                Outcome::GroupsUnavailable(GroupsCause::Overage),
                "refused-groups-unavailable",
            ),
        ] {
            assert!(
                matches!(refusal(&outcome), Some(AuthError::PartyRefused { code: c, .. }) if c == code),
                "{outcome:?}"
            );
            assert_ne!(
                detail(code, None),
                detail("refused-unbound", None),
                "{code}"
            );
        }
        assert!(
            matches!(
                refusal(&Outcome::Unbound),
                Some(AuthError::PartyRefused {
                    code: "refused-unbound",
                    ..
                })
            ),
            "an unbound person is refused"
        );
    }

    #[test]
    fn a_refusal_for_the_partys_state_or_marker_names_the_party() {
        let party = PartyRef::new("demographic", "PERSON", "p");
        let refused = refusal(&Outcome::Inactive { party });
        let Some(AuthError::PartyRefused {
            party: Some(named), ..
        }) = refused
        else {
            panic!("an inactive party is refused naming it: {refused:?}");
        };
        assert_eq!(
            (
                named.namespace.as_str(),
                named.class.as_str(),
                named.id.as_str()
            ),
            ("demographic", "PERSON", "p")
        );
        assert!(matches!(
            refusal(&Outcome::Deprovisioned),
            Some(AuthError::PartyRefused { party: None, .. })
        ));
    }

    /// An `[auth]` section with one issuer, bound by its party
    /// table when `bound`.
    fn section(bound: bool) -> ferrohealth_smart::config::AuthSection {
        let issuer = ferrohealth_secret::secret_url::SecretUrl::new("https://login.example");
        let party = bound.then(|| ferrohealth_smart::party::config::PartySection {
            identifier_issuer: "urn:login".to_owned(),
            identifier_type: "guid".to_owned(),
            ..ferrohealth_smart::party::config::PartySection::default()
        });
        ferrohealth_smart::config::AuthSection {
            enabled: true,
            issuer: [(
                "login".to_owned(),
                ferrohealth_smart::config::IssuerEntry {
                    issuer,
                    audience: vec!["ferroehr".to_owned()],
                    party,
                    ..ferrohealth_smart::config::IssuerEntry::default()
                },
            )]
            .into(),
            ..ferrohealth_smart::config::AuthSection::default()
        }
    }

    #[test]
    fn party_tables_without_a_source_are_refused_naming_the_source() {
        let config = crate::config::AppConfig::default();
        let resolved = ferroehr::config::auth::resolve(&section(true), &[])
            .expect("the [auth] section resolves");
        let refused = binding(&config, &resolved, None);
        assert!(
            matches!(refused, Err(BindingRefused::NoSource)),
            "{refused:?}"
        );
        let resolved = ferroehr::config::auth::resolve(&section(false), &[])
            .expect("the [auth] section resolves");
        assert!(
            matches!(binding(&config, &resolved, None), Ok(None)),
            "no party table needs no source"
        );
    }

    #[tokio::test]
    async fn the_authenticator_refuses_party_tables_without_a_source() {
        let mut config = crate::config::AppConfig::default();
        *config.auth.shared = section(true);
        let replay: Arc<dyn ferrohealth_smart::sender::replay::ReplayStore> =
            Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new());
        let Err(message) = super::super::Authenticator::new(&config, None, replay).await else {
            panic!("a party table without a party source builds no authenticator");
        };
        assert!(message.contains("no party source"), "{message}");
    }
}
