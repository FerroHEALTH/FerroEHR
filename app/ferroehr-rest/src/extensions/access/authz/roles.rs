// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RBAC gate decision.
//!
//! Roles are plain upper-cased strings (`USER`, `ADMIN`, …) that
//! `ferrohealth-smart` reads from a token's `[auth] role_claims` or a Basic
//! user's `[[auth.basic.users]] roles`. [`authorize`] gates an
//! [`OperationClass`] against a principal's roles and the [`RbacConfig`].

use crate::extensions::access::authz::classify::OperationClass;
use ferroehr::config::authz::RbacConfig;

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
