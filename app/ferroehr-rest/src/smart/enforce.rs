// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SMART scope + launch-context enforcement
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//! §Scopes ¶2 + §Resource Scopes; master07 §Context Selection).
//!
//! This module is pure: [`evaluate`] takes the caller's parsed [`SmartScope`]s
//! and the operation axes the ABAC PEP already resolves, and returns an
//! allow/deny decision plus the compartment binding the PEP must additionally
//! enforce. It holds no state and issues no responses — the `403` mapping and
//! the patient-compartment `ehrId` binding stay in the PEP, AND-composed after
//! RBAC and Cedar.
//!
//! `crate::extensions::access::pep` calls it after the RBAC/Cedar decision
//! succeeds: the PEP parses `Principal.scopes` with [`SmartScope::parse_all`],
//! maps the op with [`family_of_op`] and [`permission_of_op`], feeds the
//! resolved id and the `GateConfig` in, routes a [`ScopeDecision::Deny`] to a
//! `403`, and on `bind_patient_compartment` binds the [`launch_context_ehr_id`]
//! against the target EHR through the ABAC subject gate (master07 §Context
//! Selection).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 7): RFC 7519 leaves the claim set open; \
              decided-on claims lift into typed fields"
)]

use crate::extensions::access::authz::request::{AccessMode, ResourceKind};

use openehr_sdt::smart_scopes::{Compartment, Permission, ResourceFamily, SmartScope};

/// Configuration the gate needs (a slice of `ferroehr::config::smart::SmartConfig`).
#[derive(Debug, Clone, Copy)]
pub struct GateConfig {
    /// Fail-closed when the token carries no matching SMART resource scope for a
    /// scope-governed operation (master08 §Scopes ¶2). When `false`, the gate is
    /// advisory: it engages only when the token actually carries resource scopes
    /// for the operation's family.
    pub require_smart_scopes: bool,
}

/// The gate verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeDecision {
    /// The SMART resource-scope grammar permits the operation.
    Allow,
    /// Denied; the string is the audit/diagnostic reason.
    Deny(String),
}

/// The result of [`evaluate`]: the decision plus whether the PEP must bind the
/// patient compartment (i.e. verify the launch-context `ehrId` claim against the
/// target EHR, master07 §Context Selection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeOutcome {
    /// The allow/deny decision.
    pub decision: ScopeDecision,
    /// `true` when the operation is permitted **only** through a `patient`
    /// compartment scope, so the PEP must additionally bind the launch context.
    /// `false` when a broader (`user`/`system`) scope permits, or when no SMART
    /// resource family governs the operation.
    pub bind_patient_compartment: bool,
}

impl ScopeOutcome {
    fn allow(bind_patient_compartment: bool) -> Self {
        Self {
            decision: ScopeDecision::Allow,
            bind_patient_compartment,
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Self {
            decision: ScopeDecision::Deny(reason.into()),
            bind_patient_compartment: false,
        }
    }
}

/// Map an operation id onto the SMART resource family it accesses (master08
/// §Resource Scopes: `template-…`, `composition-…`, `aql-…`).
///
/// `None` for operation families the master08 grammar defines **no** resource
/// scope for — EHR, `EHR_STATUS`, CONTRIBUTION, DIRECTORY. NOTE: master08
/// §Resource Scopes lists exactly three resource types, so those operations
/// are governed by the compartment binding + the existing RBAC/ABAC layers,
/// not by a SMART resource scope; the SMART gate does not deny them
/// (adjudicated — the out-of-noun silence).
#[must_use]
pub fn family_of_op(op: &str) -> Option<ResourceFamily> {
    if op.starts_with("composition_") || op.starts_with("versioned_composition_") {
        Some(ResourceFamily::Composition)
    } else if op.starts_with("query_execute_") || op.starts_with("definition_query_") {
        // master08 §Resource Scopes: `aql-<queryName>` covers both *executing*
        // queries (`query_execute_*`, permission `s`) and the AQL *definitions*
        // (`definition_query_*`, CRUD — the maximal table's
        // "AQL definitions or ad-hoc queries" row).
        Some(ResourceFamily::Aql)
    } else if op.starts_with("definition_template_") {
        // Template ops are RBAC-only in `classify.rs` (no `ResourceKind`), so the
        // family must come from the op id, not the kind.
        Some(ResourceFamily::Template)
    } else {
        None
    }
}

/// Map an operation id onto the CRUDS [`Permission`] it exercises (master08
/// §Resource Scopes permission list).
///
/// Total; mirrors `classify::access_of` but also covers the template ops
/// (upload/store → create).
#[must_use]
pub fn permission_of_op(op: &str) -> Permission {
    if op.starts_with("query_execute_") {
        Permission::Search
    } else if op.contains("_create") || op.contains("_upload") || op.contains("_store") {
        Permission::Create
    } else if op.contains("_update") {
        Permission::Update
    } else if op.contains("_delete") {
        Permission::Delete
    } else {
        Permission::Read
    }
}

/// The [`ResourceFamily`] for an ABAC [`ResourceKind`], where one exists —
/// the alternative entry point when the PEP already has the resolved kind.
///
/// Note that `Template` has no `ResourceKind` (template ops are RBAC-only),
/// so prefer [`family_of_op`] to also catch templates.
#[must_use]
pub const fn family_of_kind(kind: ResourceKind) -> Option<ResourceFamily> {
    match kind {
        ResourceKind::Composition => Some(ResourceFamily::Composition),
        ResourceKind::Query => Some(ResourceFamily::Aql),
        ResourceKind::Ehr
        | ResourceKind::EhrStatus
        | ResourceKind::Contribution
        | ResourceKind::Directory => None,
    }
}

/// The [`Permission`] for an ABAC [`AccessMode`].
#[must_use]
pub const fn permission_of_access(access: AccessMode) -> Permission {
    match access {
        AccessMode::Create => Permission::Create,
        AccessMode::Read => Permission::Read,
        AccessMode::Update => Permission::Update,
        AccessMode::Delete => Permission::Delete,
        AccessMode::Execute => Permission::Search,
    }
}

/// Evaluate the SMART resource-scope grammar for one operation.
///
/// - `family` is the operation's SMART resource family ([`family_of_op`]);
///   `None` means no master08 resource scope governs it → [`ScopeDecision::Allow`]
///   with no compartment binding (the NOTE on [`family_of_op`]).
/// - `permission` is the CRUDS operation ([`permission_of_op`]).
/// - `resource_id` is the resolved template id / query name, or `None` when it
///   could not be resolved (e.g. an ad-hoc query, or a body the PEP did not
///   parse). A `None` id matches only a broad `*`/`**` pattern; a specific
///   pattern cannot match an unknown id, which is denied fail-closed.
///
/// Advisory vs fail-closed (master08 §Scopes ¶2): when
/// `cfg.require_smart_scopes` is `false` and the caller holds **no** resource
/// scope for this family, the gate defers ([`ScopeDecision::Allow`], no
/// binding). When `true`, or when the caller does hold family scopes, at least
/// one must match or the operation is denied.
#[must_use]
pub fn evaluate(
    scopes: &[SmartScope],
    family: Option<ResourceFamily>,
    permission: Permission,
    resource_id: Option<&str>,
    cfg: GateConfig,
) -> ScopeOutcome {
    let Some(family) = family else {
        // No SMART resource scope governs this operation family.
        return ScopeOutcome::allow(false);
    };

    // The resource scopes the caller holds for this family.
    let family_scopes: Vec<_> = scopes
        .iter()
        .filter_map(|s| match s {
            SmartScope::Resource(r) if r.resource.family() == family => Some(r),
            _ => None,
        })
        .collect();

    if family_scopes.is_empty() {
        return if cfg.require_smart_scopes {
            ScopeOutcome::deny(format!(
                "no SMART resource scope granted for {family:?} (require_smart_scopes)"
            ))
        } else {
            // Advisory mode: no SMART scope for this family → defer to RBAC/ABAC.
            ScopeOutcome::allow(false)
        };
    }

    // At least one family scope is held: at least one must permit this
    // (permission, resource_id). Track the broadest permitting compartment so the
    // PEP binds the patient context only when *nothing broader* permitted.
    let mut broadest: Option<Compartment> = None;
    for scope in &family_scopes {
        if scope.permissions.contains(permission) && scope_matches_id(scope, resource_id) {
            broadest = Some(broaden(broadest, scope.compartment));
        }
    }

    match broadest {
        Some(compartment) => ScopeOutcome::allow(compartment == Compartment::Patient),
        None => ScopeOutcome::deny(format!(
            "no granted {family:?} scope permits {permission:?} on {}",
            resource_id.unwrap_or("<unresolved>")
        )),
    }
}

/// The launch-context openEHR `EHR` id for the current request (master07
/// §Context Selection token-response table): the `ehrId` claim, falling back
/// to the standard SMART `patient` context claim.
///
/// Returns `None` when neither is present.
#[must_use]
pub fn launch_context_ehr_id(
    claims: &serde_json::Map<String, serde_json::Value>,
    ehr_id_claim: &str,
    patient_claim: &str,
) -> Option<String> {
    claim_str(claims, ehr_id_claim).or_else(|| claim_str(claims, patient_claim))
}

/// Whether the caller requested a `launch/patient` context (master07).
///
/// Advisory: context *selection* is an Authorization-Server/Launcher duty
/// (out of CDR scope — master07 §SMART Authorization Flow); the CDR only
/// observes the marker.
#[must_use]
pub fn requests_patient_context(scopes: &[SmartScope]) -> bool {
    use openehr_sdt::smart_scopes::LaunchContext;
    scopes
        .iter()
        .any(|s| matches!(s, SmartScope::LaunchContext(LaunchContext::Patient)))
}

fn claim_str(claims: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    claims
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Whether one held scope's resource pattern covers the addressed resource.
///
/// With no resolved id, only a broad `*`/`**` pattern can permit — a specific
/// pattern is fail-closed against an unknown id.
fn scope_matches_id(
    scope: &openehr_sdt::smart_scopes::ResourceScope,
    resource_id: Option<&str>,
) -> bool {
    let Some(id) = resource_id else {
        let p = scope.resource.pattern().as_str();
        return p == "*" || p == "**";
    };
    scope.resource.pattern().matches(id)
}

/// The broader of two compartments (breadth: System > User > Patient). Used so
/// that if any `user`/`system` scope permits, the patient binding is not forced.
fn broaden(current: Option<Compartment>, candidate: Compartment) -> Compartment {
    let rank = |c: Compartment| match c {
        Compartment::Patient => 0,
        Compartment::User => 1,
        Compartment::System => 2,
    };
    match current {
        Some(cur) if rank(cur) >= rank(candidate) => cur,
        _ => candidate,
    }
}

#[cfg(test)]
mod tests {
    /// Every ITS-REST operation maps to the SMART family master08 §Resource
    /// Scopes names for it (`composition-`, `aql-`, `template-`), and only the
    /// operation families the grammar names no resource for (EHR, `EHR_STATUS`,
    /// CONTRIBUTION, DIRECTORY, the demographic, admin and system groups) map to
    /// none.
    #[test]
    fn every_route_has_its_family_or_none_by_adjudication() {
        use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
        type Group = (
            &'static str,
            &'static [(&'static str, &'static str, &'static str)],
        );
        let groups: [Group; 6] = [
            ("admin", admin::ROUTES),
            ("definition", definition::ROUTES),
            ("demographic", demographic::ROUTES),
            ("ehr", ehr::ROUTES),
            ("query", query::ROUTES),
            ("system", system::ROUTES),
        ];
        for (group, routes) in groups {
            for (_, _, op) in routes {
                let family = family_of_op(op);
                match group {
                    "query" => assert_eq!(family, Some(ResourceFamily::Aql), "{op}"),
                    "definition" if op.starts_with("definition_query_") => {
                        assert_eq!(family, Some(ResourceFamily::Aql), "{op}");
                    }
                    "definition" => assert_eq!(family, Some(ResourceFamily::Template), "{op}"),
                    "ehr" if op.contains("composition") => {
                        assert_eq!(family, Some(ResourceFamily::Composition), "{op}");
                    }
                    _ => assert_eq!(family, None, "{op} ({group})"),
                }
            }
        }
    }

    use super::*;

    fn scopes(s: &str) -> Vec<SmartScope> {
        SmartScope::parse_all(s)
    }

    fn advisory() -> GateConfig {
        GateConfig {
            require_smart_scopes: false,
        }
    }

    fn fail_closed() -> GateConfig {
        GateConfig {
            require_smart_scopes: true,
        }
    }

    // ── op-id → axes mapping ──────────────────────────────────────────────────

    #[test]
    fn family_mapping() {
        assert_eq!(
            family_of_op("composition_create"),
            Some(ResourceFamily::Composition)
        );
        assert_eq!(
            family_of_op("versioned_composition_get"),
            Some(ResourceFamily::Composition)
        );
        assert_eq!(
            family_of_op("query_execute_adhoc_query"),
            Some(ResourceFamily::Aql)
        );
        // The cohort extension takes the `query_execute_` prefix rule.
        assert_eq!(
            family_of_op("query_execute_cohort"),
            Some(ResourceFamily::Aql)
        );
        // AQL definitions are aql-family too (master08 maximal table).
        assert_eq!(
            family_of_op("definition_query_store.yaml"),
            Some(ResourceFamily::Aql)
        );
        assert_eq!(
            family_of_op("definition_query_list"),
            Some(ResourceFamily::Aql)
        );
        assert_eq!(
            family_of_op("definition_template_adl1.4_upload"),
            Some(ResourceFamily::Template)
        );
        // No SMART resource family for EHR/EHR_STATUS/CONTRIBUTION/DIRECTORY.
        assert_eq!(family_of_op("ehr_create"), None);
        assert_eq!(family_of_op("ehr_status_update"), None);
        assert_eq!(family_of_op("contribution_create"), None);
        assert_eq!(family_of_op("directory_create"), None);
    }

    #[test]
    fn permission_mapping() {
        assert_eq!(permission_of_op("composition_create"), Permission::Create);
        assert_eq!(permission_of_op("composition_get"), Permission::Read);
        assert_eq!(permission_of_op("composition_update"), Permission::Update);
        assert_eq!(permission_of_op("composition_delete"), Permission::Delete);
        assert_eq!(
            permission_of_op("query_execute_stored_query"),
            Permission::Search
        );
        assert_eq!(permission_of_op("query_execute_cohort"), Permission::Search);
        assert_eq!(
            permission_of_op("definition_template_adl1.4_upload"),
            Permission::Create
        );
        assert_eq!(
            permission_of_op("definition_query_store.yaml"),
            Permission::Create
        );
    }

    // ── no-family ops are not SMART-gated ─────────────────────────────────────

    #[test]
    fn no_family_always_allows() {
        let out = evaluate(
            &scopes("patient/composition-*.r"),
            None,
            Permission::Read,
            None,
            fail_closed(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);
        assert!(!out.bind_patient_compartment);
    }

    // ── advisory vs fail-closed when no family scope is held ──────────────────

    #[test]
    fn advisory_defers_without_family_scope() {
        // A plain OIDC token (no SMART resource scopes) is untouched in advisory
        // mode.
        let out = evaluate(
            &scopes("openid profile"),
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("MyTemplate.v1"),
            advisory(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);
        assert!(!out.bind_patient_compartment);
    }

    #[test]
    fn fail_closed_denies_without_family_scope() {
        let out = evaluate(
            &scopes("openid profile"),
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("MyTemplate.v1"),
            fail_closed(),
        );
        assert!(matches!(out.decision, ScopeDecision::Deny(_)));
    }

    // ── permission matching (master08 permission list) ────────────────────────

    #[test]
    fn permission_mismatch_denies() {
        // A read-only scope cannot create.
        let out = evaluate(
            &scopes("patient/composition-*.r"),
            Some(ResourceFamily::Composition),
            Permission::Create,
            Some("MyTemplate.v1"),
            advisory(),
        );
        assert!(matches!(out.decision, ScopeDecision::Deny(_)));
    }

    #[test]
    fn permission_match_allows() {
        let out = evaluate(
            &scopes("patient/composition-*.r"),
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("MyTemplate.v1"),
            advisory(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);
        // patient compartment → the PEP must bind the launch context.
        assert!(out.bind_patient_compartment);
    }

    // ── pattern matching threads through (master08 pattern table) ─────────────

    #[test]
    fn pattern_scopes_the_id() {
        let held = scopes("user/composition-MyHospital::*.r");
        let allow = evaluate(
            &held,
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("MyHospital::Vitals.v1"),
            advisory(),
        );
        assert_eq!(allow.decision, ScopeDecision::Allow);

        let deny = evaluate(
            &held,
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("OtherHospital::Vitals.v1"),
            advisory(),
        );
        assert!(matches!(deny.decision, ScopeDecision::Deny(_)));
    }

    // ── compartment breadth → patient binding ─────────────────────────────────

    #[test]
    fn user_compartment_does_not_force_patient_binding() {
        let out = evaluate(
            &scopes("user/composition-*.r"),
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("MyTemplate.v1"),
            advisory(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);
        assert!(!out.bind_patient_compartment);
    }

    #[test]
    fn broadest_compartment_wins_binding() {
        // Both a patient and a user scope permit; the broader (user) means no
        // forced patient binding.
        let out = evaluate(
            &scopes("patient/composition-*.r user/composition-*.r"),
            Some(ResourceFamily::Composition),
            Permission::Read,
            Some("MyTemplate.v1"),
            advisory(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);
        assert!(!out.bind_patient_compartment);
    }

    #[test]
    fn system_compartment_grants_broadly() {
        let out = evaluate(
            &scopes("system/aql-*.rs"),
            Some(ResourceFamily::Aql),
            Permission::Search,
            Some("any::query.v1"),
            fail_closed(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);
        assert!(!out.bind_patient_compartment);
    }

    // ── unresolved id fail-closed unless the pattern is broad ─────────────────

    #[test]
    fn unresolved_id_needs_broad_pattern() {
        // Ad-hoc query, no name resolved: a `*` scope permits.
        let out = evaluate(
            &scopes("user/aql-*.s"),
            Some(ResourceFamily::Aql),
            Permission::Search,
            None,
            advisory(),
        );
        assert_eq!(out.decision, ScopeDecision::Allow);

        // A specific-pattern scope cannot cover an unknown id.
        let out = evaluate(
            &scopes("user/aql-org.openehr::bp.v1.s"),
            Some(ResourceFamily::Aql),
            Permission::Search,
            None,
            advisory(),
        );
        assert!(matches!(out.decision, ScopeDecision::Deny(_)));
    }

    // ── launch-context claim binding (master07) ───────────────────────────────

    #[test]
    fn launch_context_prefers_ehr_id_then_patient() {
        let mut claims = serde_json::Map::new();
        claims.insert("ehrId".to_owned(), serde_json::json!("ehr-123"));
        claims.insert("patient".to_owned(), serde_json::json!("pat-9"));
        assert_eq!(
            launch_context_ehr_id(&claims, "ehrId", "patient").as_deref(),
            Some("ehr-123")
        );

        let mut only_patient = serde_json::Map::new();
        only_patient.insert("patient".to_owned(), serde_json::json!("pat-9"));
        assert_eq!(
            launch_context_ehr_id(&only_patient, "ehrId", "patient").as_deref(),
            Some("pat-9")
        );

        let empty = serde_json::Map::new();
        assert!(launch_context_ehr_id(&empty, "ehrId", "patient").is_none());
    }

    #[test]
    fn detects_patient_context_request() {
        assert!(requests_patient_context(&scopes("launch/patient openid")));
        assert!(!requests_patient_context(&scopes("launch/episode openid")));
        assert!(!requests_patient_context(&scopes("openid profile")));
    }
}
