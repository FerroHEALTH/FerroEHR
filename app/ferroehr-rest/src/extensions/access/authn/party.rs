// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login binding of every authenticated request: the caller is looked up
//! in the party binding, read from the local party source with no cache, and
//! the request carries the outcome or is refused for it.
//!
//! A party that is `inactive` or `abandoned` (RM common
//! `master06-change_control_package.adoc` §Abandoned and Inactive States), a
//! login two parties carry, a deprovisioned key, a token without the key
//! claim, a party without the marker the caller's kind needs and groups that
//! cannot be read are each refused with `403` and their binding code. A
//! source or an identity provider that cannot answer is the server's failure,
//! never the person's, and answers `503`. No openEHR spec governs resolving a
//! login to a party: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ferroehr::config::binding::Providers;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::versioning::committer::bound::BoundDetails;

use ferrohealth_smart::party::adapter::AdapterError;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::{GroupsCause, Outcome};
use ferrohealth_smart::resource::{Access, Caller};

use super::AuthError;

/// The party binding of the deployment, with what the committer of a bound
/// caller carries beside it: the provider, the namespace and the source of
/// the `PERSON`'s name.
pub(crate) struct Binding {
    /// The shared crate's binding.
    pub(super) shared: PartyBinding,
    /// The source logins are looked up in, which also names a `PERSON`.
    source: Arc<dyn BindingSource>,
    /// `[authz.party_binding] provider`.
    provider: Option<Providers>,
    /// `[authz.party_binding] namespace`.
    namespace: String,
    /// Each issuer's `tenant_claim`, which picks a per-tenant provider.
    tenant_claims: BTreeMap<String, String>,
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
    let binding = ferroehr::config::binding::party_binding(
        &config.auth,
        &config.party_binding,
        resolved,
        lookup,
    )
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
    Ok(Some(Arc::new(Binding {
        shared: binding,
        source,
        provider: config.party_binding.provider.clone(),
        namespace: config.party_binding.namespace.clone(),
        tenant_claims,
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
        let provider = match &self.provider {
            None => None,
            Some(Providers::One(one)) => Some((one.issuer.clone(), one.id.clone())),
            Some(Providers::ByTenant(tables)) => {
                let tenant = caller
                    .issuer()
                    .and_then(|issuer| self.tenant_claims.get(issuer))
                    .and_then(|claim| principal.claims.get(claim))
                    .and_then(serde_json::Value::as_str);
                tables
                    .iter()
                    .find(|table| Some(table.tenant.as_str()) == tenant)
                    .map(|table| (table.issuer.clone(), table.id.clone()))
            }
        };
        Ok(Some(BoundDetails {
            name,
            provider,
            namespace: self.namespace.clone(),
        }))
    }
}

/// Looks the caller of `access` up in `binding`, returning the access whose
/// caller carries the outcome ([`Caller::party`]).
///
/// # Errors
/// [`AuthError::PartyRefused`] for an outcome the request is refused for,
/// and [`AuthError::PartyUnavailable`] when the source or the identity
/// provider could not answer.
pub(super) async fn bind(binding: &PartyBinding, access: Access) -> Result<Access, AuthError> {
    let Some(caller) = access.caller() else {
        return Ok(access);
    };
    let outcome = match binding.resolve(caller).await {
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
    if let Some(refusal) = refusal(&outcome) {
        tracing::warn!(
            issuer = caller.issuer().unwrap_or("-"),
            outcome = outcome.as_str(),
            "the login binding refused the request"
        );
        return Err(refusal);
    }
    Ok(Access::Caller(Arc::new(
        Caller::clone(caller).with_party(outcome),
    )))
}

/// The refusal `outcome` answers, or `None` for an outcome the request
/// proceeds under.
fn refusal(outcome: &Outcome) -> Option<AuthError> {
    let code = match outcome {
        // TODO(#5006): refuse an unbound person, once no fallback serves them.
        Outcome::Party(_) | Outcome::Unbound => return None,
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
        // NOTE: `Outcome` is non-exhaustive; an outcome added later binds no
        // party until it is read here.
        _ => "refused-unbound",
    };
    Some(AuthError::PartyRefused {
        code,
        principal: None,
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
/// can act on and never a claim value.
pub(super) fn detail(code: &str) -> &'static str {
    match code {
        "refused-ambiguous" => "two or more demographic parties carry this login's identifier",
        "refused-inactive" => "the demographic party this login is bound to is inactive",
        "refused-abandoned" => "the demographic party this login is bound to is abandoned",
        "refused-deprovisioned" => "this login's identifier belongs to a deprovisioned party",
        "refused-key-claim-missing" => "the credential carries no binding key",
        "refused-wrong-marker" => {
            "the demographic party this login is bound to may not act in this role"
        }
        "refused-groups-unavailable" => "the groups this token asserts cannot be read",
        _ => "this login is bound to no demographic party",
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
            assert_ne!(detail(code), detail("refused-unbound"), "{code}");
        }
        assert!(refusal(&Outcome::Unbound).is_none());
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
