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
//! There are three evaluation points: the per-EHR gate on every authenticated
//! request to an `/ehr/{ehr_id}`-scoped route (`master07` access list), the
//! composition privacy ceiling on Composition reads (`master07` privacy levels),
//! and the gate-keeper preflight on CONTRIBUTION commits carrying an
//! `EHR_ACCESS` version (`master07` gate-keeper; RM ehr
//! `master04-ehr_package.adoc` §EHR Access). The concrete scheme evaluated,
//! `ferroehr.access_control.v1`, is our own design.
//!
//! The per-EHR gate also holds for AQL (`query_scope`): every EHR the caller
//! may not read is withheld from the query's sources, whether the query names
//! the EHR in its `WHERE` clause, an `EHR` predicate, the `ehr_id` parameter,
//! or not at all. Privacy-level filtering of individual Composition rows is out
//! of scope for queries.

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction); the carriers here are \
              cfg(test)-only, so #[expect] would be unfulfilled in the non-test build"
)]

use axum::response::{IntoResponse, Response};
use ferroehr::aql::sql::EhrAccessScope;
use ferroehr::config::authz::EhrAccessDefault;
use ferroehr::ids::EhrId;
use ferroehr::service::ehr::access_types::{AccessLevel, principal_matches};
use ferroehr::service::ehr::access_types::{DefaultAccess, EhrAccessSettings};
use openehr_its::rest::runtime::ApiError;
use uuid::Uuid;

use crate::api::RequestParts;
use crate::extensions::access::authn::{Principal, current_principal};
use crate::extensions::access::authz::AuthzHandle;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The `EHR_ACCESS` policy engine: the three spec-grounded evaluation points,
/// as pure decisions over the parsed scheme settings + the authenticated
/// caller.
///
/// `Ok(())` permits; `Err(reason)` is a denial with its reason. Absent
/// settings (`None`) fall back to the server-wide `authz.rbac.ehr_access_default`
/// — `open` by default, the disposition that keeps every existing EHR working
/// (`master07` "sensible defaults").
#[derive(Debug)]
pub struct EhrAccessGate;

impl EhrAccessGate {
    /// The per-EHR gate: `open` (or no settings) permits; `restricted` requires
    /// the caller to match the access list (`master07` §Access Control).
    ///
    /// # Errors
    /// `Err(reason)` when the EHR is restricted and the caller matches no
    /// access-list entry — the reason is the human-readable denial detail.
    pub fn ehr_gate(
        settings: Option<&EhrAccessSettings>,
        subject: Option<&str>,
        roles: &[String],
        server_default: EhrAccessDefault,
        admin_role: &str,
    ) -> Result<(), String> {
        let Some(settings) = settings else {
            // A newly created EHR carries no settings, so this branch governs
            // most records — which is why the disposition is configurable at
            // all rather than hardcoded permit.
            return match server_default {
                EhrAccessDefault::Open => Ok(()),
                EhrAccessDefault::Restricted => {
                    if roles.iter().any(|r| r.eq_ignore_ascii_case(admin_role)) {
                        Ok(())
                    } else {
                        Err(format!(
                            "this EHR carries no ACCESS_CONTROL_SETTINGS and the server default \
                             is restricted; only the '{admin_role}' role may reach it"
                        ))
                    }
                }
            };
        };
        match settings.default_access {
            DefaultAccess::Open => Ok(()),
            DefaultAccess::Restricted => {
                if settings.match_principal(subject, roles).is_some() {
                    Ok(())
                } else {
                    Err("EHR access is restricted; the caller is not on the access list".to_owned())
                }
            }
        }
    }

    /// The Composition privacy ceiling: the effective level of the target
    /// versioned Composition must be strictly below the caller's ceiling
    /// (`full` → unbounded; `restricted_below` → `max_level`; no access-list
    /// entry → `default_level + 1`, i.e. the default level stays readable unless
    /// raised) — `master07` §Access Control privacy levels.
    ///
    /// # Errors
    /// `Err(reason)` when the target Composition's privacy level is at or above
    /// the caller's ceiling.
    pub fn privacy_gate(
        settings: Option<&EhrAccessSettings>,
        subject: Option<&str>,
        roles: &[String],
        target_vo_id: &str,
    ) -> Result<(), String> {
        let Some(settings) = settings else {
            return Ok(());
        };
        let level = settings.privacy.level_for(target_vo_id);
        let ceiling = match settings.match_principal(subject, roles) {
            Some(entry) => match entry.access {
                AccessLevel::Full => i64::MAX,
                // `max_level` is present for `restricted_below`; a malformed
                // absent one fails closed at level 0.
                AccessLevel::RestrictedBelow => entry.max_level.unwrap_or(0),
            },
            None => settings.privacy.default_level.saturating_add(1),
        };
        if level >= ceiling {
            Err(format!(
                "composition privacy level {level} is at or above the caller's ceiling {ceiling}"
            ))
        } else {
            Ok(())
        }
    }

    /// The gate-keeper preflight: once a gate-keeper is set, only that principal
    /// may commit a new `EHR_ACCESS` version (`master07` §Access Control; the
    /// settings stay CONTRIBUTION-wrapped + audited — RM ehr
    /// `master04-ehr_package.adoc` §EHR Access).
    ///
    /// # Errors
    /// `Err(reason)` when a gate-keeper is set and the caller is not it.
    pub fn gate_keeper_gate(
        settings: Option<&EhrAccessSettings>,
        subject: Option<&str>,
        roles: &[String],
    ) -> Result<(), String> {
        let Some(settings) = settings else {
            return Ok(());
        };
        let Some(gate_keeper) = &settings.gate_keeper else {
            return Ok(());
        };
        if principal_matches(gate_keeper, subject, roles) {
            Ok(())
        } else {
            Err(format!(
                "only the gate-keeper ({gate_keeper}) may change the EHR_ACCESS settings"
            ))
        }
    }
}

/// Whether `op` reads a Composition (subject to the privacy ceiling). Mirrors
/// the composition-read set the ABAC post-check uses.
fn is_composition_read(op: &str) -> bool {
    op == "composition_get" || op.starts_with("versioned_composition_")
}

/// The target versioned Composition uid for a read op, from the path params
/// (the versioned-object uid, a version uid, or the uid-based id — all
/// normalised to the versioned-object head by `Privacy::level_for`).
fn composition_target(parts: &RequestParts) -> Option<&str> {
    ["versioned_object_uid", "version_uid", "uid_based_id"]
        .into_iter()
        .find_map(|k| parts.path.get(k).map(String::as_str))
}

/// Whether a CONTRIBUTION body carries a version whose data targets `EHR_ACCESS`
/// (the only `EHR_ACCESS` write path — there is no dedicated ITS-REST endpoint).
/// Parsed minimally: an `EHR_ACCESS` create/modify carries the full object, so a
/// `versions[].data._type == "EHR_ACCESS"` is sufficient detection.
fn contribution_targets_ehr_access(parts: &RequestParts) -> bool {
    let Ok(value) = crate::overview::negotiate::json_value(&parts.headers, &parts.body) else {
        // A malformed body: the dispatch will return the 400; nothing to gate.
        return false;
    };
    value
        .get("versions")
        .and_then(|v| v.as_array())
        .is_some_and(|versions| {
            versions.iter().any(|v| {
                v.get("data")
                    .and_then(|d| d.get("_type"))
                    .and_then(|t| t.as_str())
                    == Some("EHR_ACCESS")
            })
        })
}

/// Enforce the `EHR_ACCESS` policy for an EHR-scoped request, after
/// authentication and before dispatch. `Err(response)` short-circuits with a
/// ready `403` (deny) or `500` (settings unavailable — fail-closed, consistent
/// with the ABAC PEP); `Ok(())` lets the request proceed. A route without an
/// `ehr_id` path param is not EHR-scoped and always proceeds.
pub(crate) async fn enforce(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
) -> Result<(), Box<Response>> {
    let Some(ehr_id_raw) = parts.path.get("ehr_id") else {
        return Ok(());
    };
    let Ok(ehr_id) = Uuid::parse_str(ehr_id_raw) else {
        // A malformed ehr_id is not ours to reject — the dispatch returns 400/404.
        return Ok(());
    };

    let principal = current_principal();
    let subject = principal.as_ref().map(|p| p.subject.as_str());
    let roles = principal.as_ref().map_or(&[][..], |p| p.roles.as_slice());

    let settings = match state
        .backend()
        .current_ehr_access_settings(EhrId(ehr_id))
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return Err(Box::new(server_error(
                principal.as_ref(),
                &format!("EHR_ACCESS settings unavailable: {e}"),
            )));
        }
    };
    let settings = (*settings).as_ref();

    // 1. The per-EHR gate (every EHR-scoped route). A setting-less EHR falls
    // back to the server-wide disposition; with authz absent entirely there is
    // no configured default to consult, so the historical permit stands.
    let authz = state.authz();
    let rbac = authz.as_deref().and_then(AuthzHandle::rbac_rules);
    EhrAccessGate::ehr_gate(
        settings,
        subject,
        roles,
        rbac.map_or(EhrAccessDefault::Open, |r| r.ehr_access_default),
        rbac.map_or("ADMIN", |r| r.admin_role.as_str()),
    )
    .map_err(|reason| Box::new(forbidden(principal.as_ref(), &reason)))?;

    // 2. The Composition privacy ceiling (Composition read routes).
    if is_composition_read(op)
        && let Some(target) = composition_target(parts)
    {
        EhrAccessGate::privacy_gate(settings, subject, roles, target)
            .map_err(|reason| Box::new(forbidden(principal.as_ref(), &reason)))?;
    }

    // 3. The gate-keeper preflight (CONTRIBUTION commits touching EHR_ACCESS).
    if op == "contribution_create" && contribution_targets_ehr_access(parts) {
        EhrAccessGate::gate_keeper_gate(settings, subject, roles)
            .map_err(|reason| Box::new(forbidden(principal.as_ref(), &reason)))?;
    }

    Ok(())
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
    let subject = principal.as_ref().map(|p| p.subject.as_str());
    let roles = principal.as_ref().map_or(&[][..], |p| p.roles.as_slice());
    let index = state
        .backend()
        .ehr_access_settings_index()
        .await
        .map_err(|e| {
            Box::new(server_error(
                principal.as_ref(),
                &format!("EHR_ACCESS settings unavailable: {e}"),
            ))
        })?;
    let authz = state.authz();
    let rbac = authz.as_deref().and_then(AuthzHandle::rbac_rules);
    let server_default = rbac.map_or(EhrAccessDefault::Open, |r| r.ehr_access_default);
    let admin_role = rbac.map_or("ADMIN", |r| r.admin_role.as_str());
    let readable = |settings: Option<&EhrAccessSettings>| {
        EhrAccessGate::ehr_gate(settings, subject, roles, server_default, admin_role).is_ok()
    };
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

/// A `403` carrying the principal (so the ATNA audit layer records the deny —
/// ITS-REST `401`/`403` discipline).
fn forbidden(principal: Option<&Principal>, detail: &str) -> Response {
    let mut resp =
        RestError(ApiError::Forbidden(format!("access denied: {detail}"))).into_response();
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal.clone());
    }
    resp
}

/// A `500` (fail-closed) when the settings cannot be read — an access decision
/// must never proceed on unknown policy.
fn server_error(principal: Option<&Principal>, detail: &str) -> Response {
    let mut resp = RestError(crate::overview::error::internal_fault(
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

    fn settings(scheme: &serde_json::Value) -> EhrAccessSettings {
        let access = json!({ "_type": "EHR_ACCESS", "settings": scheme });
        EhrAccessSettings::from_ehr_access(&access).expect("our scheme")
    }

    #[test]
    fn absent_settings_allow_everyone() {
        assert!(EhrAccessGate::ehr_gate(None, None, &[], EhrAccessDefault::Open, "ADMIN").is_ok());
        assert!(EhrAccessGate::privacy_gate(None, None, &[], "vo").is_ok());
        assert!(EhrAccessGate::gate_keeper_gate(None, None, &[]).is_ok());
    }

    /// The server-wide default governs an EHR that carries no settings — which
    /// is every newly created one, so this is the disposition most records
    /// actually run under.
    #[test]
    fn server_default_governs_a_setting_less_ehr() {
        // `open`: unchanged from the historical behaviour.
        assert!(
            EhrAccessGate::ehr_gate(None, None, &[], EhrAccessDefault::Open, "ADMIN").is_ok(),
            "the default posture must keep every existing EHR reachable"
        );

        // `restricted`: a clinical caller is refused …
        let denied = EhrAccessGate::ehr_gate(
            None,
            Some("bob"),
            &["USER".to_owned()],
            EhrAccessDefault::Restricted,
            "ADMIN",
        );
        assert!(denied.is_err(), "default-deny must refuse a clinical role");

        // … and so is a caller with no roles at all …
        assert!(
            EhrAccessGate::ehr_gate(None, None, &[], EhrAccessDefault::Restricted, "ADMIN")
                .is_err()
        );

        // … while the admin role still reaches it, so the operator can author
        // the settings that fix it. A default-deny nobody can climb out of is
        // an outage, not a control.
        assert!(
            EhrAccessGate::ehr_gate(
                None,
                Some("root"),
                &["ADMIN".to_owned()],
                EhrAccessDefault::Restricted,
                "ADMIN",
            )
            .is_ok()
        );

        // The admin role is the CONFIGURED one, matched case-insensitively like
        // every other role comparison.
        assert!(
            EhrAccessGate::ehr_gate(
                None,
                None,
                &["platform-admin".to_owned()],
                EhrAccessDefault::Restricted,
                "PLATFORM-ADMIN",
            )
            .is_ok()
        );
    }

    /// Explicit per-EHR settings win over the server default, in both
    /// directions — the server key is a fallback, never an override.
    #[test]
    fn explicit_settings_override_the_server_default() {
        let open =
            settings(&json!({ "_type": "FERROEHR_ACCESS_CONTROL_V1", "default_access": "open" }));
        assert!(
            EhrAccessGate::ehr_gate(
                Some(&open),
                Some("bob"),
                &["USER".to_owned()],
                EhrAccessDefault::Restricted,
                "ADMIN",
            )
            .is_ok(),
            "an EHR that says `open` stays open under a restricted server default"
        );

        let restricted = settings(&json!({
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "default_access": "restricted",
            "access_list": [{ "principal": "user:alice", "access": "full" }]
        }));
        assert!(
            EhrAccessGate::ehr_gate(
                Some(&restricted),
                Some("bob"),
                &["USER".to_owned()],
                EhrAccessDefault::Open,
                "ADMIN",
            )
            .is_err(),
            "an EHR that says `restricted` stays restricted under an open server default"
        );
    }

    #[test]
    fn open_ehr_admits_anonymous_and_named() {
        let s =
            settings(&json!({ "_type": "FERROEHR_ACCESS_CONTROL_V1", "default_access": "open" }));
        assert!(
            EhrAccessGate::ehr_gate(Some(&s), None, &[], EhrAccessDefault::Open, "ADMIN").is_ok()
        );
        assert!(
            EhrAccessGate::ehr_gate(Some(&s), Some("bob"), &[], EhrAccessDefault::Open, "ADMIN")
                .is_ok()
        );
    }

    #[test]
    fn restricted_ehr_gates_by_access_list() {
        let s = settings(&json!({
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "default_access": "restricted",
            "access_list": [
                { "principal": "user:bob", "access": "full" },
                { "principal": "role:nurse", "access": "restricted_below", "max_level": 2 }
            ]
        }));
        // Listed user permitted; listed role permitted; nobody denied.
        assert!(
            EhrAccessGate::ehr_gate(Some(&s), Some("bob"), &[], EhrAccessDefault::Open, "ADMIN")
                .is_ok()
        );
        assert!(
            EhrAccessGate::ehr_gate(
                Some(&s),
                Some("carol"),
                &["NURSE".to_owned()],
                EhrAccessDefault::Open,
                "ADMIN"
            )
            .is_ok()
        );
        assert!(
            EhrAccessGate::ehr_gate(
                Some(&s),
                Some("carol"),
                &[],
                EhrAccessDefault::Open,
                "ADMIN"
            )
            .is_err()
        );
        assert!(
            EhrAccessGate::ehr_gate(Some(&s), None, &[], EhrAccessDefault::Open, "ADMIN").is_err()
        );
    }

    #[test]
    fn privacy_ceiling_blocks_above_and_admits_below() {
        let s = settings(&json!({
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "default_access": "open",
            "access_list": [ { "principal": "role:nurse", "access": "restricted_below", "max_level": 2 } ],
            "privacy": {
                "default_level": 0,
                "composition_overrides": [ { "uid": "high-vo", "level": 3 } ]
            }
        }));
        let nurse = ["NURSE".to_owned()];
        // Nurse: ceiling 2 → level 0 (default) ok, level 3 (override) blocked.
        assert!(EhrAccessGate::privacy_gate(Some(&s), Some("x"), &nurse, "any-vo").is_ok());
        assert!(EhrAccessGate::privacy_gate(Some(&s), Some("x"), &nurse, "high-vo").is_err());
        // No access-list entry under open: ceiling default_level+1 = 1 → level 0
        // ok, the level-3 override blocked.
        assert!(EhrAccessGate::privacy_gate(Some(&s), Some("y"), &[], "any-vo").is_ok());
        assert!(EhrAccessGate::privacy_gate(Some(&s), Some("y"), &[], "high-vo").is_err());
    }

    #[test]
    fn full_access_has_no_privacy_ceiling() {
        let s = settings(&json!({
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "default_access": "open",
            "access_list": [ { "principal": "user:chief", "access": "full" } ],
            "privacy": { "default_level": 9, "composition_overrides": [ { "uid": "top", "level": 100 } ] }
        }));
        assert!(EhrAccessGate::privacy_gate(Some(&s), Some("chief"), &[], "top").is_ok());
    }

    #[test]
    fn gate_keeper_admits_only_the_keeper() {
        let s = settings(&json!({
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "gate_keeper": "user:alice",
            "default_access": "open"
        }));
        assert!(EhrAccessGate::gate_keeper_gate(Some(&s), Some("alice"), &[]).is_ok());
        assert!(EhrAccessGate::gate_keeper_gate(Some(&s), Some("bob"), &[]).is_err());
        assert!(EhrAccessGate::gate_keeper_gate(Some(&s), None, &[]).is_err());
    }
}
