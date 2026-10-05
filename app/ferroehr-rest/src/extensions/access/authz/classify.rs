// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Operation classification: a generated ITS-REST operation id → its coarse
//! [`OperationClass`].
//!
//! Keyed by the ITS-REST operation ids. Every generated operation is explicitly
//! classified; one that is not returns `None` from [`class_of`], which the
//! total-coverage guard test turns into a build failure.
//!
//! Only the two `admin_*` operations are [`OperationClass::Admin`]; every other
//! generated route is [`OperationClass::Clinical`], requiring any authenticated
//! principal with a role. [`OperationClass::Public`] classifies the non-generated
//! surface, which never reaches [`class_of`], and the management surface is
//! classified by its own per-endpoint guard against `[management.endpoints]`.

use crate::extensions::access::authz::request::{AccessMode, ResourceKind};

/// The coarse authorization class of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationClass {
    /// No authorization check (root/status/health per the router).
    Public,
    /// Any authenticated principal with at least one role.
    Clinical,
    /// Administrative operations — require `rbac.admin_role`. The management
    /// surface reaches this class through its own per-endpoint guard, which
    /// maps an `admin_only` endpoint level onto it; `[management]` owns that
    /// decision, and there is no second RBAC key spelling it.
    Admin,
}

/// Classify a generated ITS-REST operation id. `None` means **unknown** — a
/// generated operation that has not been classified (fails the coverage guard).
///
/// Only the generated route surface is covered here; the non-generated surface
/// (status/health/swagger/management) is classified by route in the REST layer.
#[must_use]
#[expect(
    clippy::match_same_arms,
    reason = "the arms are grouped by resource family, so naming every operation \
              explicitly is the point — merging equal arms would hide which \
              family an operation classifies as"
)]
pub fn class_of(op: &str) -> Option<OperationClass> {
    use OperationClass::{Admin, Clinical};
    let class = match op {
        // ── ADMIN API (the only Admin-class generated routes) ────────────────
        "admin_ehr_delete" | "admin_ehr_delete_all" => Admin,

        // ── SYSTEM (the OPTIONS-and-Conformance manifest) ────────────────────
        // NOTE: unreachable — `crate::router` mounts the live manifest above the
        // auth layer, so it answers uncredentialed (ITS-REST system
        // `Description.md`, service discovery); kept for totality (#2072).
        "options" => Clinical,

        // ── EHR ──────────────────────────────────────────────────────────────
        "ehr_get_by_subject" | "ehr_create" | "ehr_get_by_id" | "ehr_create_with_id" => Clinical,

        // ── EHR_STATUS ────────────────────────────────────────────────────────
        "ehr_status_get_by_version_id"
        | "ehr_status_get_at_time"
        | "ehr_status_update"
        | "versioned_ehr_status_get"
        | "versioned_ehr_status_revision_history"
        | "versioned_ehr_status_version_get_at_time"
        | "versioned_ehr_status_version_get_by_id" => Clinical,

        // ── COMPOSITION ──────────────────────────────────────────────────────
        "composition_create"
        | "composition_get"
        | "composition_update"
        | "composition_delete"
        | "versioned_composition_get"
        | "versioned_composition_revision_history"
        | "versioned_composition_version_get_at_time"
        | "versioned_composition_version_get_by_id" => Clinical,

        // ── DIRECTORY (FOLDER) ───────────────────────────────────────────────
        "directory_get_at_time"
        | "directory_update"
        | "directory_create"
        | "directory_delete"
        | "directory_get_by_version_id" => Clinical,

        // The released OAS reuses these CONTRIBUTION ids in both the ehr and
        // demographic bundles, and Clinical is correct for both (RM change
        // control governs demographic content identically); the RBAC route map
        // is (method, path)-keyed, so the families never collide.
        "contribution_create" | "contribution_get" => Clinical,

        // ── Item tags (clinical-resource metadata) ───────────────────────────
        "ehr_tags_get"
        | "composition_tags_get"
        | "composition_tags_update"
        | "composition_tags_delete"
        | "ehr_status_tags_get"
        | "ehr_status_tags_update"
        | "ehr_status_tags_delete" => Clinical,

        // ── QUERY execution ──────────────────────────────────────────────────
        "query_execute_adhoc_query"
        | "query_execute_adhoc_query_body"
        | "query_execute_stored_query"
        | "query_execute_stored_query_body"
        | "query_execute_stored_query_version"
        | "query_execute_stored_query_version_body"
        | "query_execute_cohort" => Clinical,

        // ── DEFINITION: templates + stored queries (v1: any authenticated) ───
        "definition_template_adl1.4_list"
        | "definition_template_adl1.4_upload"
        | "definition_template_adl1.4_get"
        | "definition_template_adl1.4_example_get"
        | "definition_template_adl2_list"
        | "definition_template_adl2_upload"
        | "definition_template_adl2_get"
        | "definition_template_adl2_example_get"
        | "definition_template_adl2_version_get"
        | "definition_query_list"
        | "definition_query_store.yaml"
        | "definition_query_version_get"
        | "definition_query_version_store.yaml" => Clinical,

        // ── DEMOGRAPHIC (coarse RBAC only — no ABAC resource kind) ──────────
        "agent_create" | "agent_get" | "agent_update" | "agent_delete" => Clinical,
        "group_create" | "group_get" | "group_update" | "group_delete" => Clinical,
        "organisation_create"
        | "organisation_get"
        | "organisation_update"
        | "organisation_delete" => Clinical,
        "person_create" | "person_get" | "person_update" | "person_delete" => Clinical,
        "role_create" | "role_get" | "role_update" | "role_delete" => Clinical,
        "versioned_party_get"
        | "versioned_party_revision_history"
        | "versioned_party_version_get_at_time"
        | "versioned_party_version_get_by_id" => Clinical,
        "demographic_tags_get"
        | "agent_tags_get"
        | "agent_tags_update"
        | "agent_tags_delete"
        | "group_tags_get"
        | "group_tags_update"
        | "group_tags_delete"
        | "organisation_tags_get"
        | "organisation_tags_update"
        | "organisation_tags_delete"
        | "person_tags_get"
        | "person_tags_update"
        | "person_tags_delete"
        | "role_tags_get"
        | "role_tags_update"
        | "role_tags_delete" => Clinical,

        _ => return None,
    };
    Some(class)
}

/// The ABAC [`ResourceKind`] of a generated operation, derived from its op-id
/// prefix.
///
/// `None` for operations ABAC does not model as resources (`definition_*`,
/// `demographic_*`, `admin_*`) — those are RBAC-only.
///
/// Order matters: `ehr_status_*` and `versioned_ehr_status_*` are tested before
/// the generic `ehr_*` fallthrough.
#[must_use]
pub fn kind_of(op: &str) -> Option<ResourceKind> {
    let kind = if op.starts_with("ehr_status_") || op.starts_with("versioned_ehr_status_") {
        ResourceKind::EhrStatus
    } else if op.starts_with("composition_") || op.starts_with("versioned_composition_") {
        ResourceKind::Composition
    } else if op.starts_with("contribution_") {
        ResourceKind::Contribution
    } else if op.starts_with("query_execute_") {
        ResourceKind::Query
    } else if op.starts_with("directory_") {
        ResourceKind::Directory
    } else if op.starts_with("ehr_") {
        // The generic EHR family (create/get/get-by-subject/tags) — after the
        // ehr_status guard above.
        ResourceKind::Ehr
    } else {
        return None;
    };
    Some(kind)
}

/// Returns whether a generated ITS-REST operation id is a mutating operation.
///
/// Total over the same universe [`class_of`] covers. AQL execution is a read
/// despite its POST verb, so a read-only principal keeps it; every other
/// GET-semantics op is a read and everything else a write.
///
/// The classification derives from the op-id verb (`write_verb`), and an op-id
/// with no recognized verb is treated as a write, so the read-only restriction
/// can never be bypassed by an unclassified future op; the total-coverage guard
/// test turns any such fall-through into a build failure.
///
/// No openEHR spec governs role semantics — our own design/extension.
#[must_use]
pub fn is_write(op: &str) -> bool {
    write_verb(op).unwrap_or(true)
}

/// Classify an op-id by its verb: `Some(true)` = write, `Some(false)` = read,
/// `None` = no recognized verb (the coverage guard fails on any generated op
/// that lands here; [`is_write`] treats it as a write at runtime).
fn write_verb(op: &str) -> Option<bool> {
    // AQL execution is a read despite the POST wire verb (Query API).
    if op.starts_with("query_execute_") {
        return Some(false);
    }
    // Write markers: the mutating verbs across every family, plus the
    // GENERATED-table admin ops, which all physically delete. The read-only
    // admin surfaces (config, reports, the parity sweep) are extension routes
    // outside the generated tables, so this prefix never sees them —
    // `extension_is_write` + EXTENSION_READ_ROUTES classify those.
    if op.starts_with("admin_")
        || op.contains("_create")
        || op.contains("_update")
        || op.contains("_delete")
        || op.contains("_upload")
        || op.contains("_store")
    {
        return Some(true);
    }
    // Read markers: the GET-semantics verbs.
    if op.contains("_get") || op.contains("_list") || op.contains("_revision_history") {
        return Some(false);
    }
    None
}

/// The ABAC [`AccessMode`] of a generated operation (the Cedar action axis).
///
/// Returns `None` for operations without a [`ResourceKind`]; for a clinical
/// op it is always `Some`. Derived from the op-id verb.
#[must_use]
pub fn access_of(op: &str) -> Option<AccessMode> {
    kind_of(op)?;
    let mode = if op.starts_with("query_execute_") {
        AccessMode::Execute
    } else if op.contains("_create") {
        AccessMode::Create
    } else if op.contains("_update") {
        AccessMode::Update
    } else if op.contains("_delete") {
        AccessMode::Delete
    } else {
        // get / get_by_* / get_at_time / revision_history / tags_get → a read.
        AccessMode::Read
    };
    Some(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated ITS-REST route tables, all groups.
    fn all_route_ops() -> Vec<&'static str> {
        let mut ops = Vec::new();
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
        ] {
            for (_method, _path, op) in table {
                ops.push(*op);
            }
        }
        ops
    }

    /// Every generated operation id has an [`OperationClass`]. A new,
    /// unclassified generated operation fails this test.
    #[test]
    fn every_operation_is_classified() {
        let mut unclassified = Vec::new();
        for op in all_route_ops() {
            if class_of(op).is_none() {
                unclassified.push(op);
            }
        }
        assert!(
            unclassified.is_empty(),
            "unclassified ITS-REST operations (add to app/ferroehr-rest/src/extensions/access/authz/classify.rs): {unclassified:?}"
        );
    }

    /// The only Admin-class generated routes are the two admin-API deletes;
    /// everything else generated is Clinical.
    #[test]
    fn admin_routes_are_the_only_admin_class() {
        let mut admin: Vec<&str> = all_route_ops()
            .into_iter()
            .filter(|op| class_of(op) == Some(OperationClass::Admin))
            .collect();
        admin.sort_unstable();
        admin.dedup();
        assert_eq!(admin, vec!["admin_ehr_delete", "admin_ehr_delete_all"]);
    }

    #[test]
    fn known_mappings() {
        assert_eq!(class_of("ehr_create"), Some(OperationClass::Clinical));
        assert_eq!(class_of("admin_ehr_delete"), Some(OperationClass::Admin));
        assert_eq!(
            class_of("definition_template_adl1.4_upload"),
            Some(OperationClass::Clinical)
        );
        assert_eq!(class_of("person_get"), Some(OperationClass::Clinical));
        assert_eq!(class_of("no_such_operation"), None);
    }

    #[test]
    fn kind_derivation_prefix_rules() {
        assert_eq!(kind_of("ehr_create"), Some(ResourceKind::Ehr));
        assert_eq!(kind_of("ehr_get_by_subject"), Some(ResourceKind::Ehr));
        assert_eq!(kind_of("ehr_tags_get"), Some(ResourceKind::Ehr));
        assert_eq!(kind_of("ehr_status_update"), Some(ResourceKind::EhrStatus));
        assert_eq!(
            kind_of("versioned_ehr_status_get"),
            Some(ResourceKind::EhrStatus)
        );
        assert_eq!(
            kind_of("composition_create"),
            Some(ResourceKind::Composition)
        );
        assert_eq!(
            kind_of("versioned_composition_get"),
            Some(ResourceKind::Composition)
        );
        assert_eq!(
            kind_of("contribution_create"),
            Some(ResourceKind::Contribution)
        );
        assert_eq!(
            kind_of("query_execute_adhoc_query"),
            Some(ResourceKind::Query)
        );
        // The cohort extension takes the prefix rule, so ABAC covers it too.
        assert_eq!(kind_of("query_execute_cohort"), Some(ResourceKind::Query));
        assert_eq!(kind_of("directory_create"), Some(ResourceKind::Directory));
        // RBAC-only families have no ABAC resource kind.
        assert_eq!(kind_of("definition_template_adl1.4_upload"), None);
        assert_eq!(kind_of("person_get"), None);
        assert_eq!(kind_of("admin_ehr_delete"), None);
    }

    #[test]
    fn access_mode_derivation() {
        assert_eq!(access_of("composition_create"), Some(AccessMode::Create));
        assert_eq!(access_of("ehr_create_with_id"), Some(AccessMode::Create));
        assert_eq!(access_of("composition_update"), Some(AccessMode::Update));
        assert_eq!(access_of("composition_delete"), Some(AccessMode::Delete));
        assert_eq!(access_of("composition_get"), Some(AccessMode::Read));
        assert_eq!(
            access_of("query_execute_stored_query"),
            Some(AccessMode::Execute)
        );
        assert_eq!(access_of("definition_query_list"), None);
    }

    /// Every generated operation with an ABAC [`ResourceKind`] also has an
    /// [`AccessMode`], and every RBAC-only family (`definition_*`,
    /// `demographic_*`, `admin_*`) maps to no kind. Guards the derivation the
    /// same way the class guard does.
    #[test]
    fn kind_and_access_agree_over_all_routes() {
        for op in all_route_ops() {
            if kind_of(op).is_some() {
                assert!(
                    access_of(op).is_some(),
                    "op {op} has a ResourceKind but no AccessMode"
                );
            } else {
                assert!(
                    op.starts_with("definition_") || op.starts_with("admin_") || is_demographic(op),
                    "op {op} has no ResourceKind but is not a known RBAC-only family"
                );
                assert!(access_of(op).is_none());
            }
        }
    }

    /// Every generated operation id classifies write/read via a positive verb
    /// rule — no op falls through to [`is_write`]'s fail-safe default. A new,
    /// unrecognized generated op fails this test until [`write_verb`] handles it.
    #[test]
    fn every_operation_has_a_write_class() {
        let mut unclassified = Vec::new();
        for op in all_route_ops() {
            if write_verb(op).is_none() {
                unclassified.push(op);
            }
        }
        assert!(
            unclassified.is_empty(),
            "ops with no write/read verb rule (add to write_verb in app/ferroehr-rest/src/extensions/access/authz/classify.rs): {unclassified:?}"
        );
    }

    #[test]
    fn write_classification_exemplars() {
        // Writes: create / update / delete / upload / store, tag mutations, admin.
        assert!(is_write("ehr_create"));
        assert!(is_write("ehr_create_with_id"));
        assert!(is_write("composition_create"));
        assert!(is_write("composition_update"));
        assert!(is_write("composition_delete"));
        assert!(is_write("ehr_status_update"));
        assert!(is_write("contribution_create"));
        assert!(is_write("directory_create"));
        assert!(is_write("definition_template_adl1.4_upload"));
        assert!(is_write("definition_template_adl2_upload"));
        assert!(is_write("definition_query_store.yaml"));
        assert!(is_write("definition_query_version_store.yaml"));
        assert!(is_write("composition_tags_update"));
        assert!(is_write("composition_tags_delete"));
        assert!(is_write("ehr_status_tags_update"));
        assert!(is_write("person_create"));
        assert!(is_write("agent_tags_delete"));
        assert!(is_write("admin_ehr_delete"));
        assert!(is_write("admin_ehr_delete_all"));
        // Reads: GET-semantics ops, plus AQL execution (POST-but-read).
        assert!(!is_write("ehr_get_by_id"));
        assert!(!is_write("ehr_get_by_subject"));
        assert!(!is_write("composition_get"));
        assert!(!is_write("versioned_composition_get"));
        assert!(!is_write("versioned_composition_revision_history"));
        assert!(!is_write("versioned_composition_version_get_at_time"));
        assert!(!is_write("ehr_status_get_at_time"));
        assert!(!is_write("directory_get_by_version_id"));
        assert!(!is_write("contribution_get"));
        assert!(!is_write("ehr_tags_get"));
        assert!(!is_write("composition_tags_get"));
        assert!(!is_write("definition_template_adl1.4_list"));
        assert!(!is_write("definition_template_adl2_version_get"));
        assert!(!is_write("definition_query_list"));
        assert!(!is_write("query_execute_adhoc_query"));
        assert!(!is_write("query_execute_adhoc_query_body"));
        assert!(!is_write("query_execute_stored_query_version_body"));
        // The cohort extension selects; it never commits.
        assert!(!is_write("query_execute_cohort"));
        assert!(!is_write("person_get"));
        assert!(!is_write("versioned_party_revision_history"));
        // Fail-safe: an unrecognized op is treated as a write.
        assert!(is_write("no_such_operation"));
    }

    /// The demographic-API op families (RBAC-only; ABAC never covered them).
    fn is_demographic(op: &str) -> bool {
        const PREFIXES: [&str; 7] = [
            "agent_",
            "group_",
            "organisation_",
            "person_",
            "role_",
            "versioned_party_",
            "demographic_",
        ];
        PREFIXES.iter().any(|p| op.starts_with(p))
    }
}
