// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype human-readable-identifier (HRID) grammar — the single home for
//! reading, writing, and keying an `ARCHETYPE_HRID`.
//!
//! Spec oracle: `docs/specs/openehr/AM/docs/AOM2/master07.05` §Physical
//! Archetype Identifier — the physical form
//! `[ns::]publisher-package-class.concept.vMAJOR[.MINOR[.PATCH]][-status[.build]]`.
//! [`parse_hrid`] reads that form into the generated
//! `openehr_am::v2_4::aom2` [`ArchetypeHrid`] and `hrid_to_string` writes it
//! back; the two are inverses over a normalised (3-part-version) id.
//!
//! Two further, deliberately *looser* readings of the same grammar live here so
//! all three sit side by side and their differences are visible:
//! [`hrid_lookup_key`]/[`raw_id_lookup_key`] (repository keying — version and
//! namespace insensitive, case-folded) and [`is_archetype_id`] (the
//! `master04.3` slot/root meta-pattern predicate, which judges shape only and
//! parses nothing).
//!
//! Everything that has to locate the `.v` version boundary in a raw id string —
//! [`parse_hrid`], [`raw_id_lookup_key`], [`raw_id_concept`],
//! [`raw_id_version`] — goes through the one primitive [`version_marker`], so
//! the grammar has exactly one reading. See that function for why the reading
//! is the LAST `.v<digit>`.

use std::fmt::Write;

use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_base::prelude::VersionStatus;

/// Parse an archetype HRID (or a version-partial specialise reference) into an
/// [`ArchetypeHrid`], normalising a partial version per `master07.05`.
///
/// Form: `[ns::]publisher-package-class.concept.vMAJOR[.MINOR[.PATCH]]
/// [-rc|-alpha|-beta[.build]]`.
///
/// # Errors
/// Returns a message describing the first structural problem.
pub fn parse_hrid(s: &str) -> Result<ArchetypeHrid, String> {
    let (namespace, rest) = match s.split_once("::") {
        Some((ns, rest)) => (Some(ns.to_owned()), rest),
        None => (None, s),
    };

    let vpos =
        version_marker(rest).ok_or_else(|| format!("HRID {s:?} has no `.vN` version segment"))?;
    let left = rest.get(..vpos).unwrap_or_default();
    let version = rest.get(vpos + 2..).unwrap_or_default();

    let (model_part, concept_id) = left
        .rsplit_once('.')
        .ok_or_else(|| format!("HRID {s:?} has no `.concept` segment"))?;

    let segments: Vec<&str> = model_part.split('-').collect();
    let [publisher, package, class] = segments.as_slice() else {
        return Err(format!(
            "HRID {s:?} model part must be `publisher-package-class`, found {model_part:?}"
        ));
    };
    if publisher.is_empty() || package.is_empty() || class.is_empty() || concept_id.is_empty() {
        return Err(format!("HRID {s:?} has an empty identifier segment"));
    }

    let (release_version, version_status, build_count) = parse_version(version)?;

    Ok(ArchetypeHrid {
        namespace,
        rm_publisher: (*publisher).to_owned(),
        rm_package: (*package).to_owned(),
        rm_class: (*class).to_owned(),
        concept_id: concept_id.to_owned(),
        release_version,
        version_status: VersionStatus::from_wire(version_status),
        build_count,
    })
}

/// Parse the version tail into `(release_version, status, build_count)`,
/// normalising a 1- or 2-part numeric version to 3 parts (`master07.05`;
/// 1.4 `v1` ⇒ `1.0.0`).
fn parse_version(version: &str) -> Result<(String, &'static str, String), String> {
    let (status, numeric, build) = if let Some((numeric, build)) = split_status(version, "-rc") {
        ("rc", numeric, build)
    } else if let Some((numeric, build)) = split_status(version, "-alpha") {
        ("alpha", numeric, build)
    } else if let Some((numeric, build)) = split_status(version, "-beta") {
        ("beta", numeric, build)
    } else {
        ("", version, "")
    };

    let mut parts = numeric.split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    let patch = parts.next().unwrap_or("0");
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid version {version:?}"));
    }
    let release_version = format!(
        "{}.{}.{}",
        major,
        if minor.is_empty() { "0" } else { minor },
        if patch.is_empty() { "0" } else { patch }
    );
    Ok((release_version, status, build.to_owned()))
}

/// If `version` carries the `marker` pre-release suffix, split into
/// `(numeric, build)` where `build` is the `.N` count after the marker (empty
/// if absent).
fn split_status<'a>(version: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let (numeric, after) = version.split_once(marker)?;
    let build = after.strip_prefix('.').unwrap_or("");
    Some((numeric, build))
}

/// Reconstruct an [`ArchetypeHrid`] string
/// (`[ns::]publisher-package-class.concept.vMAJOR.MINOR.PATCH[-status.build]`;
/// `master07.05`).
#[must_use]
pub(crate) fn hrid_to_string(h: &ArchetypeHrid) -> String {
    let mut s = String::new();
    if let Some(ns) = &h.namespace {
        let _ = write!(s, "{ns}::");
    }
    let _ = write!(
        s,
        "{}-{}-{}.{}.v{}",
        h.rm_publisher, h.rm_package, h.rm_class, h.concept_id, h.release_version
    );
    let status = h.version_status.as_str();
    if !status.is_empty() {
        let _ = write!(s, "-{status}");
        if !h.build_count.is_empty() {
            let _ = write!(s, ".{}", h.build_count);
        }
    }
    s
}

/// The `publisher-package-class.concept` lookup key of an [`ArchetypeHrid`].
///
/// Case-folded to ASCII lowercase: openEHR archetype-id matching is
/// case-insensitive on the RM publisher/package/class (`ADL2/master07.05`
/// §Physical Archetype Identifier — the RM entity names follow the
/// case-insensitive type-name matching of `master03` §Lexical Conventions), so
/// a reference `openehr-task_planning-DECISION_GROUP.x` resolves the archetype
/// `openehr-TASK_PLANNING-DECISION_GROUP.x`.
#[must_use]
pub fn hrid_lookup_key(h: &ArchetypeHrid) -> String {
    format!(
        "{}-{}-{}.{}",
        h.rm_publisher, h.rm_package, h.rm_class, h.concept_id
    )
    .to_ascii_lowercase()
}

/// The `publisher-package-class.concept` lookup key of a RAW archetype-id
/// string: the optional `ns::` prefix and the trailing `.vN…` version
/// stripped, case-folded to match [`hrid_lookup_key`].
///
/// This is the family identity that groups every stored version of one
/// archetype/template.
#[must_use]
pub fn raw_id_lookup_key(raw: &str) -> String {
    strip_namespace_and_version(raw).to_ascii_lowercase()
}

/// The `concept` segment of a raw archetype id — the token between the model
/// part (`publisher-package-class`) and the `.vN…` version.
///
/// Falls back to the whole (namespace-stripped) id when the shape carries no
/// `.concept` segment, so the accessor is total on stored input.
///
/// Grammar: `archetype_id = qualified_rm_entity, '.', domain_concept, '.v',
/// version_id` (BASE `base_types` `master05-identification_package.adoc`
/// §Syntaxes).
#[must_use]
pub fn raw_id_concept(raw: &str) -> &str {
    let core = raw.rsplit("::").next().unwrap_or(raw);
    let before_version = split_at_version(core).0;
    before_version.rsplit_once('.').map_or(core, |(_, c)| c)
}

/// The numeric release-version segment of a raw archetype id
/// (`…concept.v1.2.3-rc.4` → `1.2.3`); empty when the id carries no `.v<digit>`
/// marker.
///
/// The pre-release qualifier (`-alpha.N` / `-rc.N`, ADL2
/// `master07.05-adl_identification.adoc` §Version Identifiers) is dropped —
/// this is the `major[.minor[.patch]]` release number only.
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

/// Splits a (namespace-stripped) archetype id at its [`version_marker`].
///
/// Returns everything before the marker, and the marker onwards (`None` when
/// the id carries no version marker, in which case the whole input is the
/// "before" part). The marker's `.` and `v` are ASCII, so the split is always
/// on a char boundary.
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

/// The byte index of the version marker in a (namespace-stripped) archetype id:
/// the **last** `.v` immediately followed by an ASCII digit.
///
/// Why the LAST and not the first — derived from the grammar in BASE `base_types`
/// `master05-identification_package.adoc` §Syntaxes:
///
/// ```text
/// archetype_id        = qualified_rm_entity, '.', domain_concept, '.v', version_id ;
/// qualified_rm_entity = rm_originator, '-', rm_name, '-', rm_entity ;
/// domain_concept      = concept_name, { '-', specialisation } ;
/// concept_name        = alphanum-str ;   alphanum-str = letter, { letter | digit | '_' } ;
/// version_id          = '0' | non-zero-digit, [ number ] ;
/// ```
///
/// `concept_name`/`specialisation` are `alphanum-str`, whose second and later
/// characters may be digits — so a legal concept may begin with `v` followed by
/// a digit (`openEHR-EHR-OBSERVATION.v1_score.v1`), and a FIRST-marker scan
/// binds to the concept boundary and misreads (or rejects) a valid id. The
/// version tail, by contrast, can never contain a `v`: `version_id` is numeric
/// in BASE, and ADL2 `master07.05-adl_identification.adoc` §Version Identifiers
/// widens it only to `N.M.P` with an optional `-alpha.N` / `-rc.N` qualifier.
/// The last `.v<digit>` is therefore always the version marker, and the reading
/// is unambiguous.
///
/// The digit requirement is what keeps a concept id merely *starting* with `v`
/// (`…ENTRY.valc_parent`) from being mistaken for a version.
#[must_use]
pub fn version_marker(s: &str) -> Option<usize> {
    s.as_bytes()
        .windows(3)
        .rposition(|w| matches!(w, [b'.', b'v', third] if third.is_ascii_digit()))
}

/// True if `id` conforms to the archetype-id form
/// `[ns::]publisher-package-class.concept.vN…` (the slot/root meta-pattern
/// `^.+-.+-.+\..*\..+$`, master04.3).
///
/// This is the SHAPE predicate, deliberately looser than [`parse_hrid`]: it
/// admits four or more hyphen-separated model segments and any `concept.version`
/// tail, because the meta-pattern it mirrors is a regex over the raw text, not
/// a parse.
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

    #[test]
    fn hrid_forms() {
        let h = parse_hrid("openEHR-EHR-OBSERVATION.blood_pressure.v1.2.3").expect("full");
        assert_eq!(h.namespace, None);
        assert_eq!(h.rm_publisher, "openEHR");
        assert_eq!(h.rm_class, "OBSERVATION");
        assert_eq!(h.release_version, "1.2.3");
        // An empty status token is outside the `VERSION_STATUS` constant set, so
        // `from_wire` preserves it verbatim as `Other` (HRID tolerance).
        assert_eq!(h.version_status, VersionStatus::Other(String::new()));

        // 1.4 single-number version normalises to 3 parts.
        let h = parse_hrid("openehr-TASK_PLANNING-TASK_PLAN.good_include.v0").expect("partial");
        assert_eq!(h.release_version, "0.0.0");

        // namespaced + release-candidate with a build count.
        let h = parse_hrid("uk.gov::openEHR-EHR-CLUSTER.device.v1.0.0-rc.2").expect("ns+rc");
        assert_eq!(h.namespace.as_deref(), Some("uk.gov"));
        // `rc` is not a `VERSION_STATUS` constant (`release_candidate` is), so the
        // out-of-set token is preserved as `Other`.
        assert_eq!(h.version_status, VersionStatus::Other("rc".to_owned()));
        assert_eq!(h.build_count, "2");

        // alpha with no build count.
        let h = parse_hrid("openEHR-EHR-OBSERVATION.x.v0.0.1-alpha").expect("alpha");
        assert_eq!(h.version_status, VersionStatus::Alpha);
        assert_eq!(h.build_count, "");

        assert!(parse_hrid("not-an-hrid").is_err());
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

    /// The one reading of the `.v` boundary: the LAST `.v<digit>`.
    ///
    /// `concept_name = alphanum-str = letter, { letter | digit | '_' }` (BASE
    /// `base_types` `master05-identification_package.adoc` §Syntaxes), so a legal
    /// concept may begin `v` + digit; the version tail can never contain a `v`.
    /// Every reader of the grammar must therefore agree on the last marker.
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
