// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[cohort]` section — the allow-list of demographic predicates a cohort
//! query may select on, and the two limits that bound what it may serve.
//!
//! **No openEHR spec governs this — our own design/extension.** openEHR defines
//! no cross-domain cohort query: AQL runs over the clinical domain and knows
//! nothing of the demographic one, and the SM has no linkage component at all.
//! What this section states is which demographic leaves a deployment is willing
//! to have a cohort selected on, named by the archetype and at-code that carry
//! them — because that is a deployment fact about the archetypes in use, never
//! something a server can infer.
//!
//! The list is EMPTY by default, which leaves the surface off: a deployment
//! that has not stated what may be selected on has not decided, and a default
//! guess would be a selection criterion nobody chose.
//!
//! A field of the one config tree ([`crate::config::FerroEhrConfig`]); no
//! loader of its own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The predicate keys a deployment may bind.
///
/// Closed, and closed deliberately: the whole point of the allow-list is that
/// no caller can name a demographic leaf the deployment did not choose, and a
/// free-form key set would make the config file the place a new selection
/// criterion is invented rather than declared.
pub const PREDICATE_KEYS: &[&str] = &["age_band", "city", "organisation", "postcode_area", "sex"];

/// The key whose binding must be — and whose binding alone may be — a
/// [`PredicateKind::BirthDate`].
pub const BIRTH_DATE_KEY: &str = "age_band";

/// How a bound predicate's value is matched against the stored leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateKind {
    /// Exact equality against a `DV_TEXT`-shaped leaf's `value/value`.
    Text,
    /// Prefix match against a `DV_TEXT`-shaped leaf's `value/value`.
    TextPrefix,
    /// Exact equality against a `DV_CODED_TEXT` leaf's
    /// `value/defining_code/code_string`.
    Coded,
    /// An inclusive age band in whole years (`"40-49"`) against a `DV_DATE`
    /// leaf's `value/value`.
    BirthDate,
}

impl PredicateKind {
    /// The kind's configuration spelling, for boot logs and refusals.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::TextPrefix => "text_prefix",
            Self::Coded => "coded",
            Self::BirthDate => "birth_date",
        }
    }
}

/// One bound predicate: the archetype and at-code whose ELEMENT carries the
/// value, and how to match it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateBinding {
    /// The archetype HRID of the nearest archetyped ancestor of the ELEMENT
    /// (`openEHR-DEMOGRAPHIC-ADDRESS.address.v1`), matched case-insensitively
    /// against the stored, case-folded column (BASE `base_types`
    /// `master05-identification_package.adoc` §"Composite Identifiers and
    /// Case").
    pub archetype: String,
    /// The ELEMENT's `archetype_node_id` at-code (`at0012`).
    pub node: String,
    /// How the caller's value is matched against the leaf.
    pub kind: PredicateKind,
}

/// The `[cohort]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CohortConfig {
    /// The distinct-EHR floor a result set must reach to be served
    /// (`FERROEHR__COHORT__SMALL_CELL_THRESHOLD`); `0` disables suppression.
    ///
    /// Below it the rows are withheld and the response says so, because a
    /// cohort narrow enough to name one person re-identifies that person out
    /// of aggregate content the caller was never entitled to as an individual
    /// record.
    pub small_cell_threshold: u32,
    /// The largest cohort a predicate may select
    /// (`FERROEHR__COHORT__MAX_COHORT_SIZE`).
    ///
    /// A predicate matching more parties is refused rather than truncated: a
    /// silently shortened cohort is a wrong denominator, and a denominator
    /// nobody can see is worse than a refusal.
    pub max_cohort_size: u32,
    /// The bound predicates, keyed by the name a caller uses
    /// (`[cohort.predicates]`). Empty — the default — leaves the surface off.
    pub predicates: BTreeMap<String, PredicateBinding>,
}

impl Default for CohortConfig {
    fn default() -> Self {
        Self {
            small_cell_threshold: 5,
            max_cohort_size: 100_000,
            predicates: BTreeMap::new(),
        }
    }
}

impl CohortConfig {
    /// Whether this deployment serves cohort queries at all.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.predicates.is_empty()
    }
}

/// Whether `value` reads as a demographic archetype HRID
/// (`openEHR-DEMOGRAPHIC-<CLASS>.<concept>.v<n>`).
///
/// The grammar is BASE `base_types` `master05-identification_package.adoc`
/// §Archetype Identifiers, narrowed to the DEMOGRAPHIC rm-package: a cohort
/// predicate that named a clinical archetype would run on the demographic
/// pool and match nothing, silently.
#[must_use]
pub fn is_demographic_archetype_hrid(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("openehr-demographic-") else {
        return false;
    };
    let Some((entity, tail)) = rest.split_once('.') else {
        return false;
    };
    let Some((concept, version)) = tail.rsplit_once(".v") else {
        return false;
    };
    !entity.is_empty()
        && entity
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !concept.is_empty()
        && concept
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !version.is_empty()
        && version.chars().all(|c| c.is_ascii_digit())
}

/// Whether `value` reads as an at-code (`at0012`, `at0012.1`).
#[must_use]
pub fn is_at_code(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("at") else {
        return false;
    };
    let mut segments = rest.split('.');
    segments
        .next()
        .is_some_and(|first| !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()))
        && segments
            .all(|segment| !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_section_leaves_the_surface_off() {
        let config = CohortConfig::default();
        assert!(!config.is_enabled(), "no predicate is bound by default");
        assert_eq!(config.small_cell_threshold, 5);
        assert_eq!(config.max_cohort_size, 100_000);
    }

    #[test]
    fn archetype_hrids_are_recognized_case_insensitively() {
        assert!(is_demographic_archetype_hrid(
            "openEHR-DEMOGRAPHIC-ADDRESS.address.v1"
        ));
        assert!(is_demographic_archetype_hrid(
            "openehr-demographic-person.person-patient.v2"
        ));
        // A clinical archetype would run on the demographic pool and match
        // nothing, so it is not a legal binding.
        assert!(!is_demographic_archetype_hrid(
            "openEHR-EHR-OBSERVATION.blood_pressure.v2"
        ));
        assert!(!is_demographic_archetype_hrid("at0012"));
        assert!(!is_demographic_archetype_hrid(
            "openEHR-DEMOGRAPHIC-ADDRESS.address"
        ));
        assert!(!is_demographic_archetype_hrid(
            "openEHR-DEMOGRAPHIC-ADDRESS.address.vx"
        ));
    }

    #[test]
    fn at_codes_are_recognized() {
        assert!(is_at_code("at0012"));
        assert!(is_at_code("at0012.1.3"));
        assert!(!is_at_code("at"));
        assert!(!is_at_code("at0012."));
        assert!(!is_at_code("id12"));
        assert!(!is_at_code("openEHR-DEMOGRAPHIC-ADDRESS.address.v1"));
    }

    #[test]
    fn an_unknown_predicate_key_is_not_in_the_closed_set() {
        assert!(PREDICATE_KEYS.contains(&"city"));
        assert!(!PREDICATE_KEYS.contains(&"national_identifier"));
    }
}
