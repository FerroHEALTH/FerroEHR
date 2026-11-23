// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype human-readable-identifier grammar (`AOM2/master07.05`
//! §Physical Archetype Identifier).
//!
//! [`parse_hrid`] and `Display` are inverses over a normalised id;
//! [`hrid_lookup_key`] and [`is_archetype_id`] are looser readings. Every reader
//! of the `.v` boundary goes through [`version_marker`].

use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_am::v2_4::aom2::archetype::archetype_hrid_impl::HridError;

/// Parse an archetype HRID, or a version-partial specialise reference, into an
/// [`ArchetypeHrid`], normalising a partial version (`master07.05`).
///
/// # Errors
/// Returns a message describing the first structural problem.
pub fn parse_hrid(s: &str) -> Result<ArchetypeHrid, String> {
    s.parse().map_err(|e: HridError| e.to_string())
}

/// The `publisher-package-class.concept` lookup key of an [`ArchetypeHrid`].
///
/// ASCII-lowercased: archetype-id matching is case-insensitive on the RM entity
/// names (`ADL2/master07.05`; `master03` §Lexical Conventions).
#[must_use]
pub fn hrid_lookup_key(h: &ArchetypeHrid) -> String {
    format!(
        "{}-{}-{}.{}",
        h.rm_publisher, h.rm_package, h.rm_class, h.concept_id
    )
    .to_ascii_lowercase()
}

/// The `publisher-package-class.concept` lookup key of a raw archetype-id string,
/// namespace and version stripped, as [`hrid_lookup_key`].
#[must_use]
pub fn raw_id_lookup_key(raw: &str) -> String {
    strip_namespace_and_version(raw).to_ascii_lowercase()
}

/// The `concept` segment of a raw archetype id, or the whole namespace-stripped
/// id when it has none (BASE `master05-identification_package.adoc` §Syntaxes).
#[must_use]
pub fn raw_id_concept(raw: &str) -> &str {
    let core = raw.rsplit("::").next().unwrap_or(raw);
    let before_version = split_at_version(core).0;
    before_version.rsplit_once('.').map_or(core, |(_, c)| c)
}

/// The specialisation depth a raw archetype id states: the number of `-`
/// specialisations in its `domain_concept` (`CLUSTER.exam-foot` is depth 1).
///
/// Grammar: `domain_concept = concept_name, { '-', specialisation }` (BASE
/// `base_types` `master05-identification_package.adoc` §Syntaxes).
#[must_use]
pub fn raw_id_specialisation_depth(raw: &str) -> usize {
    raw_id_concept(raw).matches('-').count()
}

/// The `major[.minor[.patch]]` release segment of a raw archetype id, empty
/// without a `.v<digit>` marker (ADL2 `master07.05` §Version Identifiers).
#[must_use]
pub fn raw_id_version(raw: &str) -> &str {
    let core = raw.rsplit("::").next().unwrap_or(raw);
    match split_at_version(core).1 {
        // Skip the `.v`; take up to the pre-release status marker (`-`).
        Some(marker) => {
            let tail = marker.get(2..).unwrap_or_default();
            tail.split('-').next().unwrap_or(tail)
        }
        None => "",
    }
}

/// A raw archetype id with its optional `ns::` prefix and its `.vN…` version
/// tail removed — i.e. `qualified_rm_entity '.' domain_concept`.
fn strip_namespace_and_version(raw: &str) -> &str {
    let no_ns = raw.rsplit("::").next().unwrap_or(raw);
    split_at_version(no_ns).0
}

/// Splits a namespace-stripped archetype id at its [`version_marker`]; `None`
/// for the tail when there is none.
#[must_use]
pub fn split_at_version(core: &str) -> (&str, Option<&str>) {
    let Some(idx) = version_marker(core) else {
        return (core, None);
    };
    match core.split_at_checked(idx) {
        Some((before, marker)) => (before, Some(marker)),
        None => (core, None),
    }
}

/// The byte index of the version marker in a namespace-stripped archetype id:
/// the last `.v` followed by an ASCII digit.
///
/// A `concept_name` may itself start with `v` and a digit, while `version_id`
/// never contains a `v` (BASE `master05-identification_package.adoc` §Syntaxes;
/// ADL2 `master07.05` §Version Identifiers), so only the last marker is
/// unambiguous.
#[must_use]
pub fn version_marker(s: &str) -> Option<usize> {
    openehr_am::v2_4::aom2::archetype::archetype_hrid_impl::version_marker(s)
}

/// Whether `id` has the archetype-id shape of the slot and root meta-pattern
/// `^.+-.+-.+\..*\..+$` (master04.3); a shape test, looser than [`parse_hrid`].
#[must_use]
pub fn is_archetype_id(id: &str) -> bool {
    let body = id.rsplit("::").next().unwrap_or(id);
    let Some((prefix, rest)) = body.split_once('.') else {
        return false;
    };
    // publisher-package-class (three hyphen-separated non-empty parts)
    let hyphen_parts: Vec<&str> = prefix.split('-').collect();
    if hyphen_parts.len() < 3 || hyphen_parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    // concept.version — must have a version segment starting with a digit or 'v'
    rest.contains('.') && rest.split('.').next_back().is_some_and(|_| true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::prelude::VersionStatus;

    #[test]
    fn hrid_forms() {
        let h = parse_hrid("openEHR-EHR-OBSERVATION.blood_pressure.v1.2.3").expect("full");
        assert_eq!(h.namespace, None);
        assert_eq!(h.rm_publisher, "openEHR");
        assert_eq!(h.rm_class, "OBSERVATION");
        assert_eq!(h.release_version, "1.2.3");
        // No modifier names a released version (`org.openehr.am.aom2.archetype_hrid.adoc`).
        assert_eq!(h.version_status, VersionStatus::Released);

        // 1.4 single-number version normalises to 3 parts.
        let h = parse_hrid("openehr-TASK_PLANNING-TASK_PLAN.good_include.v0").expect("partial");
        assert_eq!(h.release_version, "0.0.0");

        // namespaced + release-candidate with a build count.
        let h = parse_hrid("uk.gov::openEHR-EHR-CLUSTER.device.v1.0.0-rc.2").expect("ns+rc");
        assert_eq!(h.namespace.as_deref(), Some("uk.gov"));
        // The `rc` modifier names `release_candidate`.
        assert_eq!(h.version_status, VersionStatus::ReleaseCandidate);
        assert_eq!(h.build_count, "2");

        // alpha with no build count.
        let h = parse_hrid("openEHR-EHR-OBSERVATION.x.v0.0.1-alpha").expect("alpha");
        assert_eq!(h.version_status, VersionStatus::Alpha);
        assert_eq!(h.build_count, "");

        assert!(parse_hrid("not-an-hrid").is_err());
    }

    /// Each HRID modifier reads as the `VERSION_STATUS` it names and writes
    /// back as the same modifier, so parse and write are inverse
    /// (`org.openehr.am.aom2.archetype_hrid.adoc`, the `version_status` row).
    #[test]
    fn every_version_status_round_trips() {
        for (hrid, status) in [
            ("openEHR-EHR-CLUSTER.device.v1.0.0", VersionStatus::Released),
            (
                "openEHR-EHR-CLUSTER.device.v1.0.0-rc.2",
                VersionStatus::ReleaseCandidate,
            ),
            (
                "openEHR-EHR-CLUSTER.device.v1.0.0-alpha.1",
                VersionStatus::Alpha,
            ),
            (
                "openEHR-EHR-CLUSTER.device.v1.0.0-beta.3",
                VersionStatus::Beta,
            ),
        ] {
            let parsed = parse_hrid(hrid).expect("a well-formed HRID");
            assert_eq!(parsed.version_status, status, "{hrid}");
            assert_eq!(parsed.to_string(), hrid, "{hrid} writes back unchanged");
        }
    }

    #[test]
    fn lookup_keys_ignore_namespace_version_and_case() {
        let h = parse_hrid("uk.gov::openEHR-EHR-OBSERVATION.bp.v1.2.3").expect("hrid");
        assert_eq!(hrid_lookup_key(&h), "openehr-ehr-observation.bp");
        assert_eq!(
            raw_id_lookup_key("uk.gov::openehr-ehr-OBSERVATION.bp.v1"),
            "openehr-ehr-observation.bp"
        );
        // A concept id starting with `v` is not mistaken for the version marker.
        assert_eq!(
            raw_id_lookup_key("openEHR-EHR-ENTRY.valc_parent"),
            "openehr-ehr-entry.valc_parent"
        );
    }

    /// The `.v` boundary is the last `.v<digit>` (BASE
    /// `master05-identification_package.adoc` §Syntaxes).
    #[test]
    fn the_version_marker_is_the_last_v_digit_on_a_concept_that_looks_like_one() {
        let pathological = "openEHR-EHR-OBSERVATION.v1_score.v1.2.3";

        // The parser: concept keeps its `v1…` prefix, version is the tail.
        let h = parse_hrid(pathological).expect("a `v<digit>`-leading concept is a legal HRID");
        assert_eq!(h.concept_id, "v1_score");
        assert_eq!(h.release_version, "1.2.3");

        // Every raw-string reader agrees with the parser.
        assert_eq!(
            version_marker(pathological),
            Some("openEHR-EHR-OBSERVATION.v1_score".len())
        );
        assert_eq!(
            split_at_version(pathological),
            ("openEHR-EHR-OBSERVATION.v1_score", Some(".v1.2.3"))
        );
        assert_eq!(raw_id_concept(pathological), "v1_score");
        assert_eq!(raw_id_version(pathological), "1.2.3");
        assert_eq!(
            raw_id_lookup_key(pathological),
            hrid_lookup_key(&h),
            "the raw-string key and the parsed key must be the same identity"
        );
    }

    /// The raw-string projections on the ordinary forms: namespace stripped,
    /// pre-release qualifier dropped from the release version, and an id with
    /// no `.v<digit>` marker left whole.
    #[test]
    fn raw_id_projections() {
        let full = "uk.gov::openEHR-EHR-CLUSTER.device.v1.0.0-rc.2";
        assert_eq!(raw_id_concept(full), "device");
        assert_eq!(raw_id_version(full), "1.0.0");
        assert_eq!(raw_id_lookup_key(full), "openehr-ehr-cluster.device");

        // ADL 1.4 single-number version.
        assert_eq!(raw_id_version("openEHR-EHR-OBSERVATION.bp.v1"), "1");

        // No version marker: nothing is stripped.
        let unversioned = "openEHR-EHR-ENTRY.valc_parent";
        assert_eq!(split_at_version(unversioned), (unversioned, None));
        assert_eq!(raw_id_version(unversioned), "");
        assert_eq!(raw_id_concept(unversioned), "valc_parent");
    }

    #[test]
    fn archetype_id_shape_predicate_is_looser_than_the_parser() {
        assert!(is_archetype_id("openEHR-EHR-OBSERVATION.bp.v1"));
        assert!(!is_archetype_id("not-an-hrid"));
        // Four model segments: the shape predicate admits it, `parse_hrid` does not.
        assert!(is_archetype_id("a-b-c-d.concept.v1"));
        assert!(parse_hrid("a-b-c-d.concept.v1").is_err());
    }
}
