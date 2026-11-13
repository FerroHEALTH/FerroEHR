// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_ACCESS` policy gate — the spec-grounded access-decision layer.
//!
//! `EHR_ACCESS` is a mandatory, versioned RM object and the openEHR
//! access-decision authority: "All access decisions to data in the EHR must be
//! made in accordance with the policies and rules in this object" (RM
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class). This gate is
//! therefore the foundational, always-on layer, and the crate's RBAC and ABAC
//! gates — our own enterprise extensions, since no openEHR spec governs them —
//! compose on top of it as additive restrictions, never the other way round. It
//! runs first in the pre-dispatch chain and is never config-gated: every EHR
//! carries an `EHR_ACCESS`, and the `open` default keeps every flow working
//! (BASE `architecture_overview/master07-security.adoc` §Access Control).
//!
//! The gate decides each EHR an operation reaches by the `default_access` of
//! its `access_directives` settings: on every authenticated request to an
//! `/ehr/{ehr_id}`-scoped route, on every EHR a manifest export names
//! (`enforce_each`), and for AQL (`query_scope`), where every EHR the caller
//! may not reach is withheld from the query's sources. Who writes and reads
//! the settings themselves is the service's rule
//! (`ferroehr::service::ehr::access_rule`). The scheme is our own design.

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction); the carriers here are \
              cfg(test)-only, so #[expect] would be unfulfilled in the non-test build"
)]

use axum::response::{IntoResponse, Response};
use ferroehr::access_terminology::AccessDefault;
use ferroehr::aql::sql::EhrAccessScope;
use ferroehr::config::authz::EhrAccessDefault;
use ferroehr::ids::EhrId;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::ehr::access_settings::EhrAccessSettings;
use openehr_its::rest::runtime::ApiError;
use uuid::Uuid;

use crate::api::RequestParts;
use crate::extensions::access::authn::{Principal, current_principal};
use crate::extensions::access::authz::AuthzHandle;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The `EHR_ACCESS` policy engine: the per-EHR decision over the read settings
/// and the authenticated caller.
///
/// `Ok(())` permits; `Err(reason)` is a denial with its reason. Absent
/// settings (`None`) fall back to the server-wide `authz.rbac.ehr_access_default`
/// — `open` by default, the disposition that keeps every existing EHR working
/// (`master07` "sensible defaults").
#[derive(Debug)]
pub struct EhrAccessGate;

/// Who asks the gate: the roles the caller holds and who it is to the EHR's
/// settings.
#[derive(Debug)]
pub struct GateCaller<'a> {
    /// The caller's roles.
    pub roles: &'a [String],
    /// The caller as the settings' rules see it.
    pub settings: &'a SettingsCaller,
}

impl EhrAccessGate {
    /// The per-EHR gate: `open` (or no settings under an `open` server
    /// default) permits; `restricted` and `consent_required` admit the
    /// configured `admin_role`, the subject and a gate-keeper, and leave every
    /// permit directive to the access evaluator.
    ///
    /// # Errors
    /// `Err(reason)` when the EHR admits the caller under none of these — the
    /// reason is the human-readable denial detail.
    pub fn ehr_gate(
        settings: Option<&EhrAccessSettings>,
        caller: &GateCaller<'_>,
        server_default: EhrAccessDefault,
        admin_role: &str,
    ) -> Result<(), String> {
        let admin = caller
            .roles
            .iter()
            .any(|r| r.eq_ignore_ascii_case(admin_role));
        let Some(read) = settings else {
            // A newly created EHR carries no settings, so this branch governs
            // most records — which is why the disposition is configurable at
            // all rather than hardcoded permit.
            return match server_default {
                EhrAccessDefault::Open => Ok(()),
                EhrAccessDefault::Restricted if admin => Ok(()),
                EhrAccessDefault::Restricted => Err(format!(
                    "this EHR carries no ACCESS_CONTROL_SETTINGS and the server default is \
                     restricted; only the '{admin_role}' role may reach it"
                )),
            };
        };
        match read.default_access() {
            AccessDefault::Open => Ok(()),
            // TODO(#5513): the access evaluator admits the callers a permit directive names.
            AccessDefault::Restricted | AccessDefault::ConsentRequired => {
                if admin || caller.settings.subject() || caller.settings.is_gate_keeper(settings) {
                    Ok(())
                } else {
                    Err(
                        "EHR access is restricted; the caller is neither the subject nor a \
                         gate-keeper"
                            .to_owned(),
                    )
                }
            }
        }
    }
}

/// Enforce the `EHR_ACCESS` policy for an EHR-scoped request, after
/// authentication and before dispatch. `Err(response)` short-circuits with a
/// ready `403` (deny) or `500` (settings unavailable — fail-closed, consistent
/// with the ABAC PEP); `Ok(())` lets the request proceed. A route without an
/// `ehr_id` path param is not EHR-scoped and always proceeds.
pub(crate) async fn enforce(state: &AppState, parts: &RequestParts) -> Result<(), Box<Response>> {
    let Some(ehr_id_raw) = parts.path.get("ehr_id") else {
        return Ok(());
    };
    let Ok(ehr_id) = Uuid::parse_str(ehr_id_raw) else {
        // A malformed ehr_id is not ours to reject — the dispatch returns 400/404.
        return Ok(());
    };
    let principal = current_principal();
    enforce_ehr(state, principal.as_ref(), EhrId(ehr_id)).await
}

/// Enforce the per-EHR gate on every EHR an operation names in its body, such
/// as the manifest of `POST /message/export`, before any of their content is
/// read: the first EHR the caller may not reach refuses the whole operation
/// with the `403` or fail-closed `500` its path route would answer.
pub(crate) async fn enforce_each(state: &AppState, ehr_ids: &[EhrId]) -> Result<(), Box<Response>> {
    let principal = current_principal();
    for ehr_id in ehr_ids {
        enforce_ehr(state, principal.as_ref(), *ehr_id).await?;
    }
    Ok(())
}

/// The per-EHR gate for one EHR. A setting-less EHR falls back to the
/// configured server-wide disposition, whether or not the RBAC gate is on; no
/// handle means that disposition is `open`. Settings that cannot be read fail
/// closed with `500`.
async fn enforce_ehr(
    state: &AppState,
    principal: Option<&Principal>,
    ehr_id: EhrId,
) -> Result<(), Box<Response>> {
    let settings = state
        .backend()
        .current_ehr_access_settings(ehr_id)
        .await
        .map_err(|e| Box::new(unavailable(principal, &e)))?;
    Decider::of(state, principal)
        .decide((*settings).as_ref())
        .map_err(|reason| Box::new(forbidden(principal, &reason)))
}

/// The per-EHR decision for one caller, its inputs read once.
struct Decider {
    /// The caller's roles.
    roles: Vec<String>,
    /// The caller as the settings' rules see it.
    settings_caller: SettingsCaller,
    /// The server-wide disposition of a setting-less EHR.
    server_default: EhrAccessDefault,
    /// The role a restricted EHR admits.
    admin_role: String,
}

impl Decider {
    /// The decision for `principal` under `state`'s configuration.
    fn of(state: &AppState, principal: Option<&Principal>) -> Self {
        let authz = state.authz();
        let (server_default, admin_role) = server_disposition(authz.as_deref());
        Self {
            roles: principal.map_or_else(Vec::new, |p| p.roles.clone()),
            settings_caller: state.backend().settings_caller(),
            server_default,
            admin_role: admin_role.to_owned(),
        }
    }

    /// [`EhrAccessGate::ehr_gate`] over an EHR's `settings`.
    fn decide(&self, settings: Option<&EhrAccessSettings>) -> Result<(), String> {
        let caller = GateCaller {
            roles: &self.roles,
            settings: &self.settings_caller,
        };
        EhrAccessGate::ehr_gate(settings, &caller, self.server_default, &self.admin_role)
    }
}

/// The fail-closed `500` of settings that cannot be read.
fn unavailable(principal: Option<&Principal>, error: &impl std::fmt::Display) -> Response {
    server_error(
        principal,
        &format!("EHR_ACCESS settings unavailable: {error}"),
    )
}

/// The per-EHR decision for an AQL execution: the EHRs the authenticated caller
/// may read, by the same [`EhrAccessGate::ehr_gate`] the path routes apply.
///
/// An EHR with no settings follows the server-wide default, so the scope lists
/// the settings-bearing EHRs only: the ones withheld when setting-less EHRs are
/// readable, or the ones permitted when they are not. `Err(response)` is a
/// fail-closed `500` when the settings cannot be read.
pub(crate) async fn query_scope(state: &AppState) -> Result<EhrAccessScope, Box<Response>> {
    let principal = current_principal();
    let index = state
        .backend()
        .ehr_access_settings_index()
        .await
        .map_err(|e| Box::new(unavailable(principal.as_ref(), &e)))?;
    let decider = Decider::of(state, principal.as_ref());
    let readable = |settings: Option<&EhrAccessSettings>| decider.decide(settings).is_ok();
    let scope = if readable(None) {
        let withheld: Vec<EhrId> = index
            .iter()
            .filter(|(_, settings)| !readable(Some(settings)))
            .map(|(ehr_id, _)| *ehr_id)
            .collect();
        if withheld.is_empty() {
            EhrAccessScope::Unrestricted
        } else {
            EhrAccessScope::Excluding(withheld)
        }
    } else {
        EhrAccessScope::Only(
            index
                .iter()
                .filter(|(_, settings)| readable(Some(settings)))
                .map(|(ehr_id, _)| *ehr_id)
                .collect(),
        )
    };
    Ok(scope)
}

/// The server-wide disposition for a setting-less EHR and the role a
/// `restricted` one admits, read from the authorization handle whether or not
/// its RBAC gate is on.
///
/// [`AuthzHandle::build`] returns no handle only when the configured default is
/// `open`, and the binary wires none with authentication off, which the
/// configuration refuses beside a `restricted` default; so `None` reads as
/// `open`, and the admin role is then never consulted.
fn server_disposition(authz: Option<&AuthzHandle>) -> (EhrAccessDefault, &str) {
    authz.map_or((EhrAccessDefault::Open, "ADMIN"), |handle| {
        (handle.ehr_access_default(), handle.admin_role())
    })
}

/// A `403` carrying the principal (so the ATNA audit layer records the deny —
/// ITS-REST `401`/`403` discipline).
fn forbidden(principal: Option<&Principal>, detail: &str) -> Response {
    let mut resp =
        RestError::from(ApiError::Forbidden(format!("access denied: {detail}"))).into_response();
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal.clone());
    }
    resp
}

/// A `500` (fail-closed) when the settings cannot be read — an access decision
/// must never proceed on unknown policy.
fn server_error(principal: Option<&Principal>, detail: &str) -> Response {
    let mut resp = RestError::from(crate::overview::error::internal_fault(
        "read the EHR access-control settings",
        &detail,
    ))
    .into_response();
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal.clone());
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A term of the `access default` group.
    fn default_access(code: &str) -> serde_json::Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID",
                        "value": ferroehr::party_binding::policy_mark::TERMINOLOGY },
                    "code_string": code } })
    }

    /// Settings with the default `code` and the subject as their one
    /// gate-keeper.
    fn settings(code: &str) -> EhrAccessSettings {
        let access = json!({ "_type": "EHR_ACCESS", "settings": {
            "_type": "ACCESS_DIRECTIVE_SETTINGS",
            "default_access": default_access(code),
            "gate_keepers": [{ "_type": "PARTY_SELF" }]
        } });
        EhrAccessSettings::from_ehr_access(&access)
            .expect("the settings read")
            .expect("settings")
    }

    /// The gate's verdict for a caller holding `roles`, outside any request,
    /// so the caller is neither the subject nor a gate-keeper.
    fn gate(
        settings: Option<&EhrAccessSettings>,
        roles: &[String],
        default: EhrAccessDefault,
    ) -> bool {
        let nobody = SettingsCaller::default();
        let caller = GateCaller {
            roles,
            settings: &nobody,
        };
        EhrAccessGate::ehr_gate(settings, &caller, default, "ADMIN").is_ok()
    }

    #[test]
    fn absent_settings_follow_the_server_default() {
        assert!(gate(None, &[], EhrAccessDefault::Open));
        assert!(!gate(
            None,
            &["USER".to_owned()],
            EhrAccessDefault::Restricted
        ));
        assert!(gate(
            None,
            &["admin".to_owned()],
            EhrAccessDefault::Restricted
        ));
    }

    /// Explicit settings win over the server default, in both directions.
    #[test]
    fn explicit_settings_override_the_server_default() {
        let user = ["USER".to_owned()];
        assert!(gate(
            Some(&settings("open")),
            &user,
            EhrAccessDefault::Restricted
        ));
        assert!(!gate(
            Some(&settings("restricted")),
            &user,
            EhrAccessDefault::Open
        ));
        assert!(!gate(
            Some(&settings("consent_required")),
            &user,
            EhrAccessDefault::Open
        ));
    }

    /// A restricted EHR admits the administrator, and an EHR held closed
    /// admits no one else.
    #[test]
    fn a_restricted_ehr_admits_the_administrator() {
        let admin = ["ADMIN".to_owned()];
        assert!(gate(
            Some(&settings("restricted")),
            &admin,
            EhrAccessDefault::Open
        ));
        assert!(gate(
            Some(&EhrAccessSettings::HeldClosed),
            &admin,
            EhrAccessDefault::Open
        ));
        assert!(!gate(
            Some(&EhrAccessSettings::HeldClosed),
            &[],
            EhrAccessDefault::Open
        ));
    }
}
