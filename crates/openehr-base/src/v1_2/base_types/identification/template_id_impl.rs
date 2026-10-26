// @generated-from-template templates/openehr-base/base_types/identification/template_id_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written accessor functions for `TEMPLATE_ID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.template_id.adoc`
//! and `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
//! §Template Identifiers. A template identifier in the human-readable (HRID)
//! form is `namespace '::' rm_publisher '-' rm_package '-' rm_class '.'
//! concept_id '.v' version_id` (e.g.
//! `org.example::openEHR-EHR-COMPOSITION.discharge_summary.v1.0.0`); a legacy
//! free-text template name stays a valid `TEMPLATE_ID`, and every accessor
//! answers the empty string for it.

use super::template_id::TemplateId;

/// The parts of an HRID template identifier: namespace, qualified RM class
/// name, concept id and the version after `.v`.
struct Parts<'a> {
    namespace: &'a str,
    qualified: &'a str,
    concept: &'a str,
    version: &'a str,
}

/// Split a template identifier into its HRID parts, or `None` when it is not
/// an HRID.
///
/// NOTE: BASE artefact_identification master03 §Formal Model reference expression, with the namespace mandatory as master05 §Syntaxes requires of a template id.
fn parse(value: &str) -> Option<Parts<'_>> {
    let (namespace, rest) = value.split_once("::")?;
    let (qualified, remainder) = rest.split_once('.')?;
    let (concept, version) = remainder.split_once('.')?;
    let version = version.strip_prefix('v')?;
    let mut classes = qualified.split('-');
    let well_formed = reverse_domain_name(namespace)
        && classes
            .by_ref()
            .take(3)
            .filter(|c| alphanumeric_name(c))
            .count()
            == 3
        && classes.next().is_none()
        && segmented_alphanumeric_name(concept)
        && version_id(version);
    well_formed.then_some(Parts {
        namespace,
        qualified,
        concept,
        version,
    })
}

/// `[a-zA-Z0-9-]+(\.[a-zA-Z0-9-]+)+`: two or more dot-separated labels.
fn reverse_domain_name(s: &str) -> bool {
    s.split('.').count() >= 2
        && s.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// `V_ALPHANUMERIC_NAME`, `[a-zA-Z][a-zA-Z0-9_]+`.
fn alphanumeric_name(s: &str) -> bool {
    s.len() >= 2
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `V_SEGMENTED_ALPHANUMERIC_NAME`, `[a-zA-Z][a-zA-Z0-9_-]+`.
fn segmented_alphanumeric_name(s: &str) -> bool {
    s.len() >= 2
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// master04 §Version Numbering `version_id`: `major [ '.' minor [ '.' patch
/// [ ( '-rc' | '-alpha' | '-beta' ) [ '.' build_count ] ] ] ]`.
fn version_id(s: &str) -> bool {
    fn number(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
    }
    let (numeric, modifier) = s.split_once('-').map_or((s, None), |(n, m)| (n, Some(m)));
    let parts: Vec<&str> = numeric.split('.').collect();
    let modifier_ok = modifier.is_none_or(|m| {
        let (kind, build) = m.split_once('.').map_or((m, None), |(k, b)| (k, Some(b)));
        parts.len() == 3 && matches!(kind, "rc" | "alpha" | "beta") && build.is_none_or(number)
    });
    (1..=3).contains(&parts.len()) && parts.iter().all(|p| number(p)) && modifier_ok
}

/// The numeric part of a version identifier: everything before a pre-release
/// modifier (`2.0.0` of `2.0.0-rc.4`).
fn numeric_version(version: &str) -> &str {
    version
        .split_once('-')
        .map_or(version, |(numeric, _)| numeric)
}

impl TemplateId {
    /// Whether the value is an HRID template identifier; `false` for a legacy
    /// free-text name (BASE `is_hrid`).
    #[must_use]
    pub fn is_hrid(&self) -> bool {
        parse(&self.value).is_some()
    }

    /// Namespace of the identifier, e.g. `org.example`: the part before `::`
    /// (BASE `namespace`).
    #[must_use]
    pub fn namespace(&self) -> &str {
        parse(&self.value).map_or("", |p| p.namespace)
    }

    /// Qualified RM class name, e.g. `openEHR-EHR-COMPOSITION`
    /// (BASE `qualified_rm_class_name`).
    #[must_use]
    pub fn qualified_rm_class_name(&self) -> &str {
        parse(&self.value).map_or("", |p| p.qualified)
    }

    /// Template concept identifier, e.g. `discharge_summary`
    /// (BASE `concept_id`).
    #[must_use]
    pub fn concept_id(&self) -> &str {
        parse(&self.value).map_or("", |p| p.concept)
    }

    /// Version as it appears after `.v`, e.g. `1.0.0` (BASE `version_id`).
    #[must_use]
    pub fn version_id(&self) -> &str {
        parse(&self.value).map_or("", |p| p.version)
    }

    /// Major version, the first numeric segment of `version_id`
    /// (BASE `major_version`).
    #[must_use]
    pub fn major_version(&self) -> &str {
        numeric_version(self.version_id())
            .split('.')
            .next()
            .unwrap_or("")
    }

    /// Minor version, the second numeric segment of `version_id`; empty when
    /// absent (BASE `minor_version`).
    #[must_use]
    pub fn minor_version(&self) -> &str {
        numeric_version(self.version_id())
            .split('.')
            .nth(1)
            .unwrap_or("")
    }

    /// Patch version, the third numeric segment of `version_id`; empty when
    /// absent (BASE `patch_version`).
    #[must_use]
    pub fn patch_version(&self) -> &str {
        numeric_version(self.version_id())
            .split('.')
            .nth(2)
            .unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(value: &str) -> TemplateId {
        TemplateId {
            value: value.to_owned(),
        }
    }

    #[test]
    fn an_hrid_template_id_yields_every_part() {
        let id = tid("org.example::openEHR-EHR-COMPOSITION.discharge_summary.v1.0.0");
        assert!(id.is_hrid());
        assert_eq!(id.namespace(), "org.example");
        assert_eq!(id.qualified_rm_class_name(), "openEHR-EHR-COMPOSITION");
        assert_eq!(id.concept_id(), "discharge_summary");
        assert_eq!(id.version_id(), "1.0.0");
        assert_eq!(id.major_version(), "1");
        assert_eq!(id.minor_version(), "0");
        assert_eq!(id.patch_version(), "0");
    }

    #[test]
    fn a_reduced_precision_version_leaves_the_missing_segments_empty() {
        let id = tid("org.example::openEHR-EHR-COMPOSITION.vital_signs-ward.v2");
        assert!(id.is_hrid());
        assert_eq!(id.concept_id(), "vital_signs-ward");
        assert_eq!(id.major_version(), "2");
        assert_eq!(id.minor_version(), "");
        assert_eq!(id.patch_version(), "");
    }

    #[test]
    fn a_pre_release_modifier_is_not_a_version_segment() {
        let id = tid("org.example::openEHR-EHR-COMPOSITION.report.v1.2.3-rc.4");
        assert_eq!(id.version_id(), "1.2.3-rc.4");
        assert_eq!(id.patch_version(), "3");
    }

    #[test]
    fn a_legacy_template_name_answers_empty_parts() {
        for legacy in [
            "Vital Signs Encounter",
            "ehrbase_blood_pressure_simple.de.v0",
            "openEHR-EHR-COMPOSITION.report.v1",
            "org.example::not an hrid",
            "example::openEHR-EHR-COMPOSITION.report.v1",
            "org.example::openEHR-EHR-COMPOSITION-X.report.v1",
            "org.example::openEHR-EHR-COMPOSITION.report.v1.0-rc",
            "org.example::openEHR-EHR-COMPOSITION.report.v1.0.0-gamma",
            "",
        ] {
            let id = tid(legacy);
            assert!(!id.is_hrid(), "{legacy:?} is not an HRID");
            assert_eq!(id.namespace(), "", "{legacy:?}");
            assert_eq!(id.qualified_rm_class_name(), "", "{legacy:?}");
            assert_eq!(id.concept_id(), "", "{legacy:?}");
            assert_eq!(id.version_id(), "", "{legacy:?}");
            assert_eq!(id.major_version(), "", "{legacy:?}");
        }
    }
}
