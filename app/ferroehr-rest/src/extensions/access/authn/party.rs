// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login binding of every authenticated request: the caller is looked up
//! in the party binding, read from the local party source with no cache, and
//! the request carries the outcome or is refused for it.
//!
//! A party that is `inactive` or `abandoned` (RM common
//! `master06-change_control_package.adoc` §Abandoned and Inactive States), a
//! login two parties carry, a deprovisioned key, a token without the key
//! claim, a party without the marker the caller's kind needs, groups that
//! cannot be read and a person whose login no party carries are each refused
//! with `403` and their binding code, with no fallback to a login-only
//! committer (RM common `AUDIT_DETAILS.committer` names who did it). A
//! source or an identity provider that cannot answer is the server's failure,
//! never the person's, and answers `503`. No openEHR spec governs resolving a
//! login to a party: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ferroehr::config::binding::Providers;
use ferroehr::config::clients::SystemClient;
use ferroehr::error::ServiceError;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::versioning::committer::bound::BoundDetails;

use ferrohealth_smart::assurance::AssuranceSection;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::adapter::AdapterError;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::{GroupsCause, Outcome, PartyRef, Represented};
use ferrohealth_smart::resource::{Access, Caller};

use super::{AuthError, RepresentedEhr};

mod jit;

/// The party binding of the deployment, with what the committer of a bound
/// caller carries beside it: the provider, the namespace and the source of
/// the `PERSON`'s name.
pub(crate) struct Binding {
    /// The shared crate's binding.
    shared: PartyBinding,
    /// The source logins are looked up in, which also names a `PERSON`.
    source: Arc<dyn BindingSource>,
    /// `[authz.party_binding] provider`.
    provider: Option<Providers>,
    /// `[authz.party_binding] namespace`.
    namespace: String,
    /// Each issuer's `tenant_claim`, which picks a per-tenant provider.
    tenant_claims: BTreeMap<String, String>,
    /// Each issuer's `person_key`, the claim a `refused-key-claim-missing`
    /// record names.
    key_claims: BTreeMap<String, String>,
    /// Each issuer's `identifier_issuer`s, which a `refused-unbound` problem
    /// names so the operator can provision the person.
    identifier_issuers: BTreeMap<String, Vec<String>>,
    /// The just-in-time creation of each issuer that sets `jit = true`.
    jit: BTreeMap<String, jit::Jit>,
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

/// Returns the party binding of `config`'s `[[auth.party]]` tables and
/// `[authz.party_binding]`, looking logins up in `source`; `None` without a
/// source, or when `[auth]` is off or binds no issuer.
///
/// # Errors
/// A message naming what the shared crate refuses in the binding.
pub(super) fn binding(
    config: &crate::config::AppConfig,
    resolved: &ferrohealth_smart::config::Resolved,
    source: Option<Arc<dyn BindingSource>>,
) -> Result<Option<Arc<Binding>>, String> {
    let Some(source) = source.filter(|_| resolved.enabled && !resolved.party.is_empty()) else {
        return Ok(None);
    };
    let lookup: Arc<dyn ferrohealth_smart::party::source::PartySource> =
        Arc::<dyn BindingSource>::clone(&source);
    let binding = ferroehr::config::binding::party_binding(&config.party_binding, resolved, lookup)
        .map_err(|e| {
            format!(
                "authz.party_binding: {}",
                ferroehr::error::ErrorChain::new(&e)
            )
        })?;
    let tenant_claims = resolved
        .issuers
        .iter()
        .filter_map(|settings| {
            let claim = settings.profile.tenant_claim.clone()?;
            Some((settings.issuer.clone(), claim))
        })
        .collect();
    let key_claims = resolved
        .issuers
        .iter()
        .map(|settings| (settings.issuer.clone(), settings.profile.person_key.clone()))
        .collect();
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
        provider: config.party_binding.provider.clone(),
        namespace: config.party_binding.namespace.clone(),
        tenant_claims,
        key_claims,
        identifier_issuers,
        jit: jit::tables(&config.party_binding, resolved),
        assurance: config.assurance.clone(),
    })))
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
            provider: self.provider_of(caller),
            namespace: self.namespace.clone(),
        }))
    }

    /// The healthcare provider `caller` acts for, as `(issuer, id)`: the one
    /// provider, or the table of the tenant its token names.
    fn provider_of(&self, caller: &Caller) -> Option<(String, String)> {
        match self.provider.as_ref()? {
            Providers::One(one) => Some((one.issuer.clone(), one.id.clone())),
            Providers::ByTenant(tables) => {
                let tenant = caller
                    .issuer()
                    .and_then(|issuer| self.tenant_claims.get(issuer))
                    .and_then(|claim| caller.claim(claim))
                    .and_then(serde_json::Value::as_str);
                tables
                    .iter()
                    .find(|table| Some(table.tenant.as_str()) == tenant)
                    .map(|table| (table.issuer.clone(), table.id.clone()))
            }
        }
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
        let Some(jit) = self.jit.get(issuer).filter(|jit| jit.creates(caller)) else {
            return Ok(None);
        };
        let person = match jit.person(
            caller,
            roles,
            &self.assurance,
            (&self.namespace, self.provider_of(caller)),
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
        self.shared.resolve(caller).await.map_err(|error| {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the party source could not answer"
            );
            AuthError::PartyUnavailable { retry_after: None }
        })
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
    let outcome = match binding.shared.resolve(caller).await {
        // NOTE: GDPR Art. 9(3) and Art. 29, a guest acts only under this controller's
        // responsibility, which `[[auth.party]] guests` declares; our own design.
        Ok(None) if matches!(caller.kind(), CallerKind::Guest(_)) => {
            return Err(AuthError::PartyRefused {
                code: "refused-guest",
                principal: None,
                party: None,
                named: None,
            });
        }
        Ok(None) if matches!(caller.kind(), CallerKind::Representative { .. }) => {
            return Err(unrecorded());
        }
        Ok(None) => return Ok(access),
        Ok(Some(outcome)) => outcome,
        Err(error) => {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the party source could not answer"
            );
            return Err(AuthError::PartyUnavailable { retry_after: None });
        }
    };
    let outcome = match outcome {
        Outcome::Unbound => binding
            .just_in_time(caller, &role_issuers.roles(caller))
            .await?
            .unwrap_or(Outcome::Unbound),
        outcome => outcome,
    };
    if let Some(mut refusal) = refusal(&outcome) {
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
            *named = match outcome {
                Outcome::KeyClaimMissing => {
                    binding.key_claims.get(issuer).cloned().map(Named::Claim)
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
    if let CallerKind::Representative { represented, .. } = caller.kind()
        && represented_subject(&outcome, represented).is_none()
    {
        tracing::warn!(
            issuer = caller.issuer().unwrap_or("-"),
            "the party source records no live representation of the asserted subject"
        );
        return Err(unrecorded());
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

/// The subject among the bound party's live representations that the token's
/// `represented` claim names, matched on the party id without regard to case.
// NOTE: no openEHR spec governs resolving a login to a party, our own design: the
// claim is a bare string, and the source names a subject by its container id.
fn represented_subject<'a>(outcome: &'a Outcome, represented: &str) -> Option<&'a PartyRef> {
    let represented = represented.trim();
    outcome
        .bound()?
        .represents
        .iter()
        .map(Represented::subject)
        .find(|subject| subject.id().eq_ignore_ascii_case(represented))
}

/// The EHR the representative of `access` is confined to, `None` for any
/// other caller: the EHR whose `EHR_STATUS.subject.external_ref` names the
/// represented subject (#5086), looked up through `service`, which records the
/// resolution on the access log.
///
/// # Errors
/// [`AuthError::PartyRefused`] when the bound party represents no such
/// subject, and [`AuthError::PartyUnavailable`] when the store cannot answer.
pub(super) async fn represented_ehr(
    service: &ferroehr::service::FerroEhrService,
    access: &Access,
) -> Result<Option<RepresentedEhr>, AuthError> {
    let Some(caller) = access.caller() else {
        return Ok(None);
    };
    let CallerKind::Representative { represented, .. } = caller.kind() else {
        return Ok(None);
    };
    let subject = caller
        .party()
        .and_then(|outcome| represented_subject(outcome, represented))
        .ok_or_else(unrecorded)?;
    let found = service
        .ehr_id_for_subject(subject.id(), subject.namespace())
        .await
        .map_err(|error| {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the EHR of the represented subject could not be read"
            );
            AuthError::PartyUnavailable { retry_after: None }
        })?;
    Ok(Some(RepresentedEhr {
        ehr_id: found.map(|ehr| ehr.to_string()),
    }))
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
        Outcome::GroupsUnavailable(GroupsCause::Adapter(error)) => {
            if let Some(retry_after) = upstream(error) {
                return Some(AuthError::PartyUnavailable { retry_after });
            }
            "refused-groups-unavailable"
        }
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

/// The `Retry-After` of an adapter failure that is the identity provider's
/// (a status, throttling or a timeout), or `None` for one that is the
/// token's.
#[expect(
    clippy::option_option,
    reason = "the outer None is a failure of the token, the inner one an upstream failure \
              that gave no Retry-After"
)]
fn upstream(error: &AdapterError) -> Option<Option<Duration>> {
    match error {
        AdapterError::Status { retry_after, .. } => Some(*retry_after),
        AdapterError::Timeout { .. } | AdapterError::Unreadable { .. } => Some(None),
        _ => None,
    }
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
        "refused-guest" => "a guest of this login's issuer is not admitted",
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
/// §Resource Scopes; RFC 9068 §2.2), and someone acting for a patient on a
/// `patients` issuer (RFC 8693 §4.1 `act`), whom FerroEHR binds to no party
/// of its own. No openEHR spec governs what a caller kind may do: our own
/// design.
pub(super) fn kind_refusal(caller: &Caller) -> Option<AuthError> {
    let inner = match caller.kind() {
        CallerKind::Guest(inner) => inner.as_ref(),
        kind => kind,
    };
    if matches!(inner, CallerKind::PatientRepresentative { .. }) {
        return Some(AuthError::KindRefused {
            code: "refused-patient-representative",
            message: "a token acting for a patient is not admitted: the representative is bound \
                      to no party of this deployment"
                .to_owned(),
            principal: None,
        });
    }
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
/// system caller carries the `[[authz.system_client]]` table of `clients`
/// that declares its issuer and client, when one does.
pub(super) fn committer_identity(
    principal: &super::Principal,
    clients: &[Arc<SystemClient>],
) -> ferroehr::versioning::committer::CommitterIdentity {
    let caller = principal.access.caller();
    let system = caller.is_some_and(|caller| matches!(caller.kind(), CallerKind::System));
    let client = super::client_of(principal);
    let system_client = caller.filter(|_| system).and_then(|caller| {
        let named = caller.client_id().or(client.as_deref())?;
        clients
            .iter()
            .find(|table| {
                Some(table.issuer.expose()) == caller.issuer() && table.client_id == named
            })
            .cloned()
    });
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
        client,
        person: crate::smart::acts_as_person(principal),
        party: principal
            .access
            .caller()
            .and_then(Caller::party)
            .and_then(Outcome::bound)
            .cloned(),
        representative: caller
            .is_some_and(|caller| matches!(caller.kind(), CallerKind::Representative { .. })),
        bound: None,
        system,
        system_client,
        external: super::declared::committing(&principal.access),
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::party::PartyRef;

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

    #[test]
    fn an_upstream_adapter_failure_is_unavailable_with_its_retry_after() {
        let throttled = Outcome::GroupsUnavailable(GroupsCause::Adapter(AdapterError::Status {
            status: http::StatusCode::TOO_MANY_REQUESTS,
            retry_after: Some(Duration::from_secs(7)),
            body: None,
        }));
        assert!(matches!(
            refusal(&throttled),
            Some(AuthError::PartyUnavailable { retry_after: Some(d) }) if d == Duration::from_secs(7)
        ));
        let claim = Outcome::GroupsUnavailable(GroupsCause::Adapter(AdapterError::Claim {
            claim: "oid".to_owned(),
        }));
        assert!(matches!(
            refusal(&claim),
            Some(AuthError::PartyRefused {
                code: "refused-groups-unavailable",
                ..
            })
        ));
    }
}
