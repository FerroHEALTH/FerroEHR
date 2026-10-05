// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Identity law for archetype & template identifiers.
//!
//! Spec:
//! `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
//!
//! - §Archetype Identifiers — `ARCHETYPE_ID` is a multi-axial *immutable*
//!   identifier (`rm_originator-rm_name-rm_entity.domain_concept.vN`); the
//!   version segment is part of identity, so two versions are two distinct
//!   archetypes.
//! - `TEMPLATE_ID` (§Class Descriptions) — "Identifier for templates. Lexical
//!   form to be determined." The generated
//!   [`openehr_base::prelude::TemplateId`] carries only the opaque `value`, so a
//!   `TEMPLATE_ID` is treated as an opaque composite identifier governed by the
//!   case rule below rather than decomposed along axes the spec has not fixed.
//! - §Composite Identifiers and Case — composite identifiers are
//!   case-preserving ("not change case due to persistence, copying, transfer or
//!   other computation processes") and case-insensitive ("two identifiers
//!   identical apart from case are considered to be identical, and therefore to
//!   identify the same thing").
//!
//! This module is the in-process side of that law: [`canonical_key`] is the
//! comparison form the derived-runtime cache key uses, so case variants of one
//! stored template resolve to a single entry while the persisted `template_id`
//! stays case-preserved. Storage lookups and the SM `I_DEFINITION_ADL14`
//! provisioning surface enforce the same rule in SQL with
//! `lower(<column>) = lower($1)`.

/// The §Composite Identifiers and Case *comparison* form of an identifier, for
/// use as a map/cache KEY: surrounding whitespace trimmed off the wire token,
/// then the shared composite-identifier fold
/// ([`composite_id_key`](openehr_base::v1_3::base_types::identification::lexical::composite_id_key)).
/// Two ids are the same identifier iff their canonical keys are equal — the
/// keyed form of the same rule
/// [`composite_ids_equal`](openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal)
/// decides pairwise, so a cache hit can never disagree with a comparison.
///
/// Case-**preserving**: this is only the comparison/keying form — the original
/// string is what is stored and returned on the wire. Archetype/template ids are
/// ASCII by grammar (§Archetype Identifiers), so the ASCII fold is exact and
/// matches `PostgreSQL` `lower()` on the same values.
#[must_use]
pub(crate) fn canonical_key(id: &str) -> String {
    openehr_base::v1_3::base_types::identification::lexical::composite_id_key(id.trim())
}

/// The version axis of a `TEMPLATE_ID`, i.e. the numeric-dotted tail of a
/// trailing `.v<major>[.<minor>[.<patch>]]` segment, or `None` when the id
/// carries no such suffix.
///
/// # Spec basis
///
/// ADL 1.4 has no formal template-version field — the CNF schedule states
/// "versioning is not applicable for ADL 1.4"
/// (`docs/specs/openehr/CNF/docs/platform_test_schedule/master04-func_tc_definition_adl.adoc`
/// §Definition ADL). The ITS-REST DEFINITION API nevertheless carries an
/// (optional, `deprecated`) `TemplateMetadata.version`. The ITS-REST docs text
/// is silent on where that value comes from, so the RELEASED OAS grounds it
/// (the oracle order: docs text first, the released OAS fills docs-text
/// silence): its `version` query parameter documents the value as **"taken
/// from `template_id`"**
/// (`docs/specs/openehr/ITS-REST/specifications/parameters/query/filter_version.yaml`,
/// bundled as `computable/OAS/definition-codegen.openapi.yaml`
/// §`components.parameters.filter_version`). We therefore derive the reported
/// version from the id's version axis, per BASE §Archetype Identifiers
/// (`docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// — the `.vN` version segment). The spec is otherwise silent on the exact
/// provenance (it also permits `other_details`); this `template_id`-derived
/// reading is our own spec-permitted design choice, and the field is nullable
/// because a plain 1.4 `template_id` carries no version.
#[must_use]
pub(crate) fn template_version(template_id: &str) -> Option<String> {
    let (_, tail) = template_id.trim().rsplit_once(".v")?;
    // The version axis is `<major>[.<minor>[.<patch>]]`: it must start with a
    // digit and contain only digits and dots (so a concept ending in e.g.
    // ".verified" is not mistaken for a version).
    if !tail.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if tail.chars().all(|c| c.is_ascii_digit() || c == '.') {
        Some(tail.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_key, template_version};

    /// Two identifiers denote the *same* id under §Composite Identifiers and
    /// Case iff their canonical keys are equal (the comparison the store SQL
    /// boundary and the runtime cache key both apply).
    fn same_id(a: &str, b: &str) -> bool {
        canonical_key(a) == canonical_key(b)
    }

    #[test]
    fn case_insensitive_equality() {
        // §Composite Identifiers and Case: differ only in case → same id.
        assert!(same_id(
            "openEHR-EHR-COMPOSITION.encounter.v1",
            "OPENEHR-EHR-COMPOSITION.ENCOUNTER.V1",
        ));
        assert!(same_id("Vital signs.v1", "vital signs.v1"));
    }

    #[test]
    fn distinct_ids_stay_distinct() {
        // The version segment is part of identity (§Archetype Identifiers).
        assert!(!same_id("diagnosis.v1", "diagnosis.v2"));
        assert!(!same_id("encounter.v1", "vitals.v1"));
    }

    #[test]
    fn canonical_key_folds_case_and_trims() {
        assert_eq!(canonical_key("  Encounter.V1 "), "encounter.v1");
        // Idempotent — canonicalising a canonical key is a no-op.
        let once = canonical_key("MixedCase.V2");
        assert_eq!(canonical_key(&once), once);
    }

    #[test]
    fn version_axis_extracted_from_template_id() {
        // The trailing `.vN` axis is the reported version (ITS-REST
        // filter_version: "taken from template_id").
        assert_eq!(
            template_version("IDCR Allergies List.v0"),
            Some("0".to_owned())
        );
        assert_eq!(
            template_version("IDCR Problem List.v1"),
            Some("1".to_owned())
        );
        // A dotted `<major>.<minor>.<patch>` axis is kept whole.
        assert_eq!(
            template_version("openEHR-EHR-COMPOSITION.encounter.v1.0.2"),
            Some("1.0.2".to_owned())
        );
    }

    #[test]
    fn version_axis_absent_is_none() {
        // A plain 1.4 template_id with no version suffix.
        assert_eq!(template_version("Vital Signs"), None);
        // A `.v` not followed by a digit is not a version axis.
        assert_eq!(template_version("Encounter.verified"), None);
        assert_eq!(template_version(""), None);
    }
}
