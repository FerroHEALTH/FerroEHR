// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The role model: JWT-claim role extraction and the RBAC gate decision.
//!
//! Roles are plain upper-cased strings (`USER`, `ADMIN`, …). They come from a
//! Bearer token's configured claim paths ([`extract_roles`]) or a Basic user's
//! configured role list (resolved in `ferroehr-rest`). [`authorize`] gates an
//! [`OperationClass`] against a principal's roles and the [`RbacConfig`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 7): RFC 7519 leaves the claim set open; \
              decided-on claims lift into typed fields"
)]

use serde_json::{Map, Value};

use crate::extensions::access::authz::classify::OperationClass;
use ferroehr::config::authz::RbacConfig;

/// Extract roles from a validated JWT claim map given dotted claim paths.
///
/// A path resolves through nested JSON objects (`realm_access.roles`). The value
/// at a path is normalized to a role list: an array yields each string element
/// (the RFC 9068 §2.2.3.1 / RFC 7643 §4.1.2 shape); a plain string is split on
/// whitespace, for issuers that carry a single space-delimited role list. Every
/// role is trimmed, upper-cased, and de-duplicated with first-seen order
/// preserved.
#[must_use]
pub fn extract_roles(claims: &Map<String, Value>, paths: &[String]) -> Vec<String> {
    let mut roles: Vec<String> = Vec::new();
    for path in paths {
        let Some(value) = lookup(claims, path) else {
            continue;
        };
        match value {
            Value::Array(items) => {
                for item in items.iter().filter_map(Value::as_str) {
                    push_role(&mut roles, item);
                }
            }
            Value::String(s) => {
                for token in s.split_whitespace() {
                    push_role(&mut roles, token);
                }
            }
            _ => {}
        }
    }
    roles
}

/// Appends one normalized role — trimmed and upper-cased — unless it is empty
/// or already present (first-seen order is preserved).
fn push_role(roles: &mut Vec<String>, raw: &str) {
    let norm = raw.trim().to_ascii_uppercase();
    if !norm.is_empty() && !roles.contains(&norm) {
        roles.push(norm);
    }
}

/// Resolve a dotted claim path to a string value (the ABAC `organization` /
/// `patient` attribute lookup). Returns `None` when the path is absent or
/// the value is not a (non-empty) string.
#[must_use]
pub fn claim_string(claims: &Map<String, Value>, path: &str) -> Option<String> {
    lookup(claims, path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Resolve a dotted claim path through nested JSON objects.
fn lookup<'a>(claims: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = claims.get(first)?;
    for part in parts {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

/// The outcome of the RBAC gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RbacDecision {
    /// The caller may proceed.
    Allow,
    /// The caller is forbidden; the string is a human-readable reason for a 403.
    Deny(String),
}

/// Gate an operation class against the caller's roles and the RBAC config.
/// When `rbac.enabled` is `false` every class is allowed (auth-only
/// behaviour). Role matching is ASCII-case-insensitive.
#[must_use]
pub fn authorize(class: OperationClass, roles: &[String], rbac: &RbacConfig) -> RbacDecision {
    if !rbac.enabled {
        return RbacDecision::Allow;
    }
    let has = |name: &str| roles.iter().any(|r| r.eq_ignore_ascii_case(name));
    let require_authenticated = || {
        if roles.is_empty() {
            RbacDecision::Deny(
                "operation requires an authenticated principal with at least one role".to_owned(),
            )
        } else {
            RbacDecision::Allow
        }
    };
    let require_admin = || {
        if has(&rbac.admin_role) {
            RbacDecision::Allow
        } else {
            RbacDecision::Deny(format!("operation requires the '{}' role", rbac.admin_role))
        }
    };
    match class {
        OperationClass::Public => RbacDecision::Allow,
        OperationClass::Clinical => require_authenticated(),
        OperationClass::Admin => require_admin(),
    }
}

/// The read-only restriction: refuse a principal carrying the read-only role
/// on every write operation, even when it also holds granting roles (a
/// restriction overrides a grant).
///
/// Reads, non-read-only principals, and the rbac-disabled case all pass
/// through with [`RbacDecision::Allow`]. Role matching is
/// ASCII-case-insensitive, the same idiom as [`authorize`].
///
// NOTE: no openEHR spec governs this — our own design/extension; the SM places
// authorization out of band (SM `openehr_platform/master02-overview.adoc`
// §General Assumptions).
#[must_use]
pub fn authorize_readonly(is_write: bool, roles: &[String], rbac: &RbacConfig) -> RbacDecision {
    if !rbac.enabled || !is_write {
        return RbacDecision::Allow;
    }
    let carries_readonly = roles
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&rbac.readonly_role));
    if carries_readonly {
        RbacDecision::Deny(format!(
            "principal carries the read-only role '{}' — write operations are forbidden",
            rbac.readonly_role
        ))
    } else {
        RbacDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claims(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().expect("object")
    }

    /// An OAuth2 scope grants a CLIENT delegated authority (RFC 6749 §3.3); it
    /// asserts nothing about the subject's roles. Mining it made the
    /// at-least-one-role gate vacuous for every OIDC token, since `openid`
    /// alone satisfied it.
    #[test]
    fn scope_claim_does_not_grant_roles() {
        let c = claims(&json!({
            "realm_access": { "roles": ["user", "ferroehr-admin"] },
            "scope": "openid EHR_READ",
        }));
        let roles = extract_roles(&c, &RbacConfig::default().role_claims);
        assert!(roles.contains(&"USER".to_owned()));
        assert!(roles.contains(&"FERROEHR-ADMIN".to_owned()));
        assert!(
            !roles.contains(&"OPENID".to_owned()),
            "`openid` is a scope, not a role",
        );
        assert!(
            !roles.contains(&"EHR_READ".to_owned()),
            "a scope must not become a role",
        );
    }

    /// The RFC 9068 §2.2.3.1 carriers are read in order, so an issuer using the
    /// flat `roles`/`groups`/`entitlements` shape needs no configuration.
    #[test]
    fn rfc9068_role_claims_extracted() {
        for carrier in ["roles", "groups", "entitlements"] {
            let c = claims(&json!({ carrier: ["clinician"] }));
            assert_eq!(
                extract_roles(&c, &RbacConfig::default().role_claims),
                vec!["CLINICIAN".to_owned()],
                "the `{carrier}` claim must carry roles",
            );
        }
    }

    #[test]
    fn dedups_preserving_order() {
        // The same role from two configured carriers collapses to one entry.
        let c = claims(&json!({
            "roles": ["admin", "Admin"],
            "realm_access": { "roles": ["ADMIN"] },
        }));
        assert_eq!(
            extract_roles(&c, &RbacConfig::default().role_claims),
            vec!["ADMIN".to_owned()]
        );
    }

    #[test]
    fn missing_paths_yield_empty() {
        let c = claims(&json!({ "sub": "x" }));
        assert!(extract_roles(&c, &RbacConfig::default().role_claims).is_empty());
    }

    #[test]
    fn rbac_disabled_allows_everything() {
        let rbac = RbacConfig {
            enabled: false,
            ..RbacConfig::default()
        };
        assert_eq!(
            authorize(OperationClass::Admin, &[], &rbac),
            RbacDecision::Allow
        );
    }

    #[test]
    fn admin_class_requires_admin_role() {
        let rbac = RbacConfig::default();
        assert_eq!(
            authorize(OperationClass::Admin, &["USER".to_owned()], &rbac),
            RbacDecision::Deny("operation requires the 'ADMIN' role".to_owned())
        );
        assert_eq!(
            authorize(OperationClass::Admin, &["ADMIN".to_owned()], &rbac),
            RbacDecision::Allow
        );
    }

    #[test]
    fn clinical_requires_a_role() {
        let rbac = RbacConfig::default();
        assert!(matches!(
            authorize(OperationClass::Clinical, &[], &rbac),
            RbacDecision::Deny(_)
        ));
        assert_eq!(
            authorize(OperationClass::Clinical, &["USER".to_owned()], &rbac),
            RbacDecision::Allow
        );
    }

    #[test]
    fn readonly_role_denies_writes_allows_reads() {
        let rbac = RbacConfig::default();
        let readonly = &["READONLY".to_owned()];
        // A write is refused for a read-only principal.
        assert!(matches!(
            authorize_readonly(true, readonly, &rbac),
            RbacDecision::Deny(_)
        ));
        // A read is permitted.
        assert_eq!(
            authorize_readonly(false, readonly, &rbac),
            RbacDecision::Allow
        );
    }

    #[test]
    fn readonly_role_matches_case_insensitively() {
        let rbac = RbacConfig::default();
        assert!(matches!(
            authorize_readonly(true, &["readonly".to_owned()], &rbac),
            RbacDecision::Deny(_)
        ));
    }

    #[test]
    fn readonly_restriction_overrides_grants() {
        // The read-only role denies a write even alongside ADMIN + USER.
        let rbac = RbacConfig::default();
        let roles = &["ADMIN".to_owned(), "USER".to_owned(), "READONLY".to_owned()];
        assert!(matches!(
            authorize_readonly(true, roles, &rbac),
            RbacDecision::Deny(_)
        ));
    }

    #[test]
    fn readonly_gate_inert_when_rbac_disabled() {
        let rbac = RbacConfig {
            enabled: false,
            ..RbacConfig::default()
        };
        assert_eq!(
            authorize_readonly(true, &["READONLY".to_owned()], &rbac),
            RbacDecision::Allow
        );
    }

    #[test]
    fn non_readonly_principal_may_write() {
        let rbac = RbacConfig::default();
        assert_eq!(
            authorize_readonly(true, &["USER".to_owned()], &rbac),
            RbacDecision::Allow
        );
        // No roles at all is not read-only either (the class gate handles that).
        assert_eq!(authorize_readonly(true, &[], &rbac), RbacDecision::Allow);
    }

    #[test]
    fn custom_readonly_role_name() {
        let rbac = RbacConfig {
            readonly_role: "VIEWER".to_owned(),
            ..RbacConfig::default()
        };
        assert!(matches!(
            authorize_readonly(true, &["VIEWER".to_owned()], &rbac),
            RbacDecision::Deny(_)
        ));
        // The default READONLY name no longer restricts once reconfigured.
        assert_eq!(
            authorize_readonly(true, &["READONLY".to_owned()], &rbac),
            RbacDecision::Allow
        );
    }
}
