// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written accessor functions and the `Value_valid` grammar for
//! `ARCHETYPE_ID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.archetype_id.adoc`
//! and `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
//! §Syntaxes. Lexical forms: the legacy ADL 1.4
//! `rm_originator '-' rm_name '-' rm_entity '.' concept_name {'-' specialisation} '.v' major_version`
//! (e.g. `openEHR-EHR-OBSERVATION.blood_pressure-cuff.v1`) and the ADL 2
//! `namespace '::' … '.v' version_id` (e.g.
//! `org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v2.1.4`).

use super::archetype_id::ArchetypeId;
use super::lexical::IdError;
use std::str::FromStr;

/// Decomposition of an archetype-id value into its `namespace`,
/// `qualified_rm_entity`, `domain_concept`, and `version_id` slices.
struct Parts<'a> {
    namespace: &'a str,
    qualified: &'a str,
    domain: &'a str,
    version: &'a str,
}

/// Split an archetype-id string into its namespace (before `::`, empty when
/// absent), the RM qualifier (up to the first `.`), the domain concept (up to
/// the next `.`) and the version after its `v`. Returns `None` if a separator
/// is absent or a part is empty.
fn parse(value: &str) -> Option<Parts<'_>> {
    let (namespace, rest) = value.split_once("::").unwrap_or(("", value));
    let (qualified, remainder) = rest.split_once('.')?;
    let (domain, version) = remainder.split_once('.')?;
    let version = version.strip_prefix('v')?;
    if qualified.is_empty() || domain.is_empty() || version.is_empty() {
        return None;
    }
    Some(Parts {
        namespace,
        qualified,
        domain,
        version,
    })
}

/// The numeric part of a version identifier: everything before a pre-release
/// modifier (`2.0.0` of `2.0.0-rc.4`).
fn numeric_version(version: &str) -> &str {
    version.split_once('-').map_or(version, |(numeric, _)| numeric)
}

impl ArchetypeId {
    /// Globally-qualified RM entity, e.g. `openEHR-EHR-OBSERVATION` — the part
    /// before the first `.` (BASE `qualified_rm_entity`).
    #[must_use]
    pub fn qualified_rm_entity(&self) -> &str {
        parse(&self.value).map_or("", |p| p.qualified)
    }

    /// Concept name including any specialisations, e.g. `blood_pressure-cuff` —
    /// the part between the first `.` and the trailing `.vN` (BASE
    /// `domain_concept`).
    #[must_use]
    pub fn domain_concept(&self) -> &str {
        parse(&self.value).map_or("", |p| p.domain)
    }

    /// Organisation originating the RM, e.g. `openEHR` (BASE `rm_originator`).
    #[must_use]
    pub fn rm_originator(&self) -> &str {
        self.qualified_rm_entity().split('-').next().unwrap_or("")
    }

    /// Reference-model name, e.g. `EHR` (BASE `rm_name`).
    #[must_use]
    pub fn rm_name(&self) -> &str {
        self.qualified_rm_entity().split('-').nth(1).unwrap_or("")
    }

    /// Ontological RM entity, e.g. `OBSERVATION` (BASE `rm_entity`) — the third
    /// and remaining `-`-delimited segments of the RM qualifier.
    #[must_use]
    pub fn rm_entity(&self) -> &str {
        // rm_originator '-' rm_name '-' rm_entity: take everything after the
        // second '-'. `splitn(3, ..)` leaves any further '-' inside the third
        // segment, which is exactly the `rm_entity` remainder.
        let q = self.qualified_rm_entity();
        let mut segments = q.splitn(3, '-');
        match (segments.next(), segments.next(), segments.next()) {
            (Some(_), Some(_), Some(rest)) => rest,
            _ => "",
        }
    }

    /// Name of the concept specialisation, if any, e.g. `cuff` for
    /// `blood_pressure-cuff` — the last `-`-delimited segment of the domain
    /// concept when it is specialised, else the empty string (BASE
    /// `specialisation`).
    #[must_use]
    pub fn specialisation(&self) -> &str {
        let d = self.domain_concept();
        match d.rsplit_once('-') {
            Some((_, last)) => last,
            None => "",
        }
    }

    /// Namespace of the identifier, e.g. `org.openehr`: the part before `::`,
    /// empty for a legacy identifier, which has none (BASE `namespace`).
    #[must_use]
    pub fn namespace(&self) -> &str {
        parse(&self.value).map_or("", |p| p.namespace)
    }

    /// Version identification, e.g. `1` for a legacy identifier and `1.8.2` or
    /// `2.0.0-rc.4` for an ADL 2 one: the part after `.v` (BASE `version_id`).
    #[must_use]
    pub fn version_id(&self) -> &str {
        parse(&self.value).map_or("", |p| p.version)
    }

    /// Major version, e.g. `1` for both `.v1` and `.v1.2.3`: the first numeric
    /// segment of `version_id` (BASE `major_version`).
    #[must_use]
    pub fn major_version(&self) -> &str {
        numeric_version(self.version_id())
            .split('.')
            .next()
            .unwrap_or("")
    }

    /// Minor version, the second numeric segment of `version_id`; empty for a
    /// major-only identifier (BASE `minor_version`).
    #[must_use]
    pub fn minor_version(&self) -> &str {
        numeric_version(self.version_id())
            .split('.')
            .nth(1)
            .unwrap_or("")
    }

    /// Patch version, the third numeric segment of `version_id`; empty for an
    /// identifier without one (BASE `patch_version`).
    #[must_use]
    pub fn patch_version(&self) -> &str {
        numeric_version(self.version_id())
            .split('.')
            .nth(2)
            .unwrap_or("")
    }
}

impl FromStr for ArchetypeId {
    type Err = IdError;

    /// Parse an `ARCHETYPE_ID` against the master05 §Syntaxes `archetype_id`
    /// production.
    ///
    /// The door is the SAME check the class's `Validate` impl runs. It used to
    /// be weaker — a `.v` anywhere, a `.` before it, three `-` segments — so
    /// `"1-2-3.@@@.vXYZ"` constructed and then failed its own invariants, and
    /// the "well-formed by construction" guarantee every sibling here
    /// establishes (`HierObjectId::new` calls `is_uid`, `VersionTreeId::new`
    /// calls `is_valid_version_tree`) did not hold for this type.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdError::Empty);
        }
        if !is_valid_archetype_id(s) {
            return Err(IdError::Archetype(s.to_owned()));
        }
        Ok(Self {
            value: s.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(v: &str) -> ArchetypeId {
        ArchetypeId {
            value: v.to_owned(),
        }
    }

    #[test]
    fn decomposes_specialised() {
        let a = aid("openEHR-EHR-OBSERVATION.blood_pressure-cuff.v1");
        assert_eq!(a.qualified_rm_entity(), "openEHR-EHR-OBSERVATION");
        assert_eq!(a.domain_concept(), "blood_pressure-cuff");
        assert_eq!(a.rm_originator(), "openEHR");
        assert_eq!(a.rm_name(), "EHR");
        assert_eq!(a.rm_entity(), "OBSERVATION");
        assert_eq!(a.specialisation(), "cuff");
        assert_eq!(a.version_id(), "1");
        assert_eq!(a.major_version(), "1");
    }

    #[test]
    fn decomposes_unspecialised() {
        let a = aid("openEHR-EHR-SECTION.vital_signs.v2");
        assert_eq!(a.domain_concept(), "vital_signs");
        assert_eq!(a.specialisation(), "");
        assert_eq!(a.version_id(), "2");
        assert_eq!(a.major_version(), "2");
        assert_eq!(a.rm_entity(), "SECTION");
    }

    #[test]
    fn major_version_ignores_minor_and_patch() {
        // A full hrid_ref (major.minor.patch) still yields the major only — the
        // significant part for interface-reference matching (AM master07).
        let a = aid("openEHR-EHR-OBSERVATION.pulse.v1.2.3");
        assert_eq!(a.version_id(), "1.2.3");
        assert_eq!(a.major_version(), "1");
    }

    #[test]
    fn decomposes_namespaced_adl2() {
        let a = aid("org.openehr::openEHR-EHR-OBSERVATION.blood_pressure-cuff.v2.1.4-rc.3");
        assert_eq!(a.namespace(), "org.openehr");
        assert_eq!(a.qualified_rm_entity(), "openEHR-EHR-OBSERVATION");
        assert_eq!(a.rm_originator(), "openEHR");
        assert_eq!(a.rm_entity(), "OBSERVATION");
        assert_eq!(a.domain_concept(), "blood_pressure-cuff");
        assert_eq!(a.specialisation(), "cuff");
        assert_eq!(a.version_id(), "2.1.4-rc.3");
        assert_eq!(a.major_version(), "2");
        assert_eq!(a.minor_version(), "1");
        assert_eq!(a.patch_version(), "4");
    }

    #[test]
    fn legacy_identifier_has_no_namespace_minor_or_patch() {
        let a = aid("openEHR-EHR-SECTION.vital_signs.v2");
        assert_eq!(a.namespace(), "");
        assert_eq!(a.minor_version(), "");
        assert_eq!(a.patch_version(), "");
    }

    #[test]
    fn from_str_strict() {
        assert!(
            "openEHR-EHR-OBSERVATION.x.v1"
                .parse::<ArchetypeId>()
                .is_ok()
        );
        assert!(matches!(
            "not-an-archetype".parse::<ArchetypeId>(),
            Err(IdError::Archetype(_))
        ));
        assert!(matches!(
            "openEHR-EHR.concept.v1".parse::<ArchetypeId>(),
            Err(IdError::Archetype(_))
        ));
        assert!(
            "org.openehr::openEHR-EHR-OBSERVATION.x.v1.0.0"
                .parse::<ArchetypeId>()
                .is_ok()
        );
        assert!(matches!(
            "org.openehr::openEHR-EHR-OBSERVATION.x.v1.0.0-gamma".parse::<ArchetypeId>(),
            Err(IdError::Archetype(_))
        ));
        assert_eq!("".parse::<ArchetypeId>(), Err(IdError::Empty));
    }
}

/// Lexical validity per BASE base_types master05 §Syntaxes (`Value_valid`).
///
/// `archetype_id = ( namespace '::' qualified_rm_entity '.' domain_concept '.v'
/// version_id ) | ( qualified_rm_entity '.' domain_concept '.v' major_version )`,
/// with `namespace = alphanum-ext-str { '.' alphanum-ext-str }`,
/// `qualified_rm_entity = rm_originator '-' rm_name '-' rm_entity` (each an
/// `alphanum-str`), `domain_concept = concept_name { '-' specialisation }`,
/// `version_id = major [ '.' minor [ '.' patch [ version_modifier ] ] ]` and
/// `version_modifier = ( '-rc' | '-alpha' | '-beta' ) [ '.' number ]`.
///
/// NOTE: a namespaceless identifier with a full `version_id` is accepted although BASE 1.3.0 refuses it, because AM 2.4.0 `ARCHETYPE_HRID.namespace` is optional and its artefacts carry that form (upstream conflict #3844, ruling #3845).
#[must_use]
pub(crate) fn is_valid_archetype_id(value: &str) -> bool {
    fn alphanum_str(s: &str) -> bool {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    fn alphanum_ext_str(s: &str) -> bool {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }
    fn number_part(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
    }
    fn modifier(s: &str) -> bool {
        let (kind, build) = s.split_once('.').map_or((s, None), |(k, b)| (k, Some(b)));
        matches!(kind, "rc" | "alpha" | "beta")
            && build.is_none_or(|b| !b.is_empty() && b.bytes().all(|d| d.is_ascii_digit()))
    }
    fn version_id(s: &str) -> bool {
        let (numeric, modifier_part) = s.split_once('-').map_or((s, None), |(n, m)| (n, Some(m)));
        let parts: Vec<&str> = numeric.split('.').collect();
        (1..=3).contains(&parts.len())
            && parts.iter().all(|p| number_part(p))
            && modifier_part.is_none_or(|m| parts.len() == 3 && modifier(m))
    }
    let (namespace, rest) = match value.split_once("::") {
        Some((namespace, rest)) => (Some(namespace), rest),
        None => (None, value),
    };
    if namespace.is_some_and(|ns| !ns.split('.').all(alphanum_ext_str)) {
        return false;
    }
    let mut sections = rest.splitn(3, '.');
    let (Some(entity), Some(concept), Some(version)) =
        (sections.next(), sections.next(), sections.next())
    else {
        return false;
    };
    let entity_ok = {
        let parts: Vec<&str> = entity.split('-').collect();
        parts.len() == 3 && parts.iter().all(|p| alphanum_str(p))
    };
    let concept_ok = concept.split('-').all(alphanum_str);
    let version_ok = version.strip_prefix('v').is_some_and(version_id);
    entity_ok && concept_ok && version_ok
}

impl crate::validate::Validate for ArchetypeId {
    fn validate_invariants(&self, out: &mut Vec<crate::validate::InvariantViolation>) {
        if !is_valid_archetype_id(&self.value) {
            out.push(crate::validate::InvariantViolation::here(
                "Invariant Value_valid failed on type ARCHETYPE_ID (lexical form \
                 [namespace::]rm_originator-rm_name-rm_entity.domain_concept.vVERSION, \
                 BASE base_types master05 §Syntaxes)",
            ));
        }
    }
}

#[cfg(test)]
mod validity_tests {
    use super::*;

    /// BASE master05 §Syntaxes `archetype_id` — accepted and rejected forms
    /// (incl. the L115 WARNING: no `.v1draft` lifecycle suffixes).
    #[test]
    fn archetype_id_lexical_form() {
        for ok in [
            "openEHR-EHR-COMPOSITION.encounter.v1",
            "openEHR-EHR-CLUSTER.laboratory_test_analyte.v2",
            "openEHR-EHR-OBSERVATION.blood_pressure-simple.v0",
            "openEHR-EHR-OBSERVATION.blood_pressure.v10",
            // The namespaced ADL 2 form (BASE 1.3.0 §Syntaxes examples).
            "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v2.1.4",
            "com.nedap::openEHR-EHR-OBSERVATION.pulse.v1.1.3",
            "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v2",
            "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v2.1",
            "org.openehr::openEHR-EHR-EVALUATION.problem.v2.0.0-rc.4",
            "org.openehr::openEHR-EHR-EVALUATION.problem.v2.0.0-alpha",
            "org.open-ehr::openEHR-EHR-EVALUATION.problem.v1.0.0-beta.12",
            // The recorded relaxation (#3845): namespaceless, full version.
            "openEHR-EHR-OBSERVATION.cbc.v1.0.0",
            "openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0",
        ] {
            assert!(is_valid_archetype_id(ok), "{ok} must be valid");
        }
        for bad in [
            "openEHR-EHR-COMPOSITION.encounter.v1draft",
            "openEHR-EHR.encounter.v1",
            "openEHR-EHR-COMPOSITION.encounter",
            "openEHR-EHR-COMPOSITION.1concept.v1",
            "openEHR-EHR-COMPOSITION.encounter.v01",
            "openEHR-EHR-COMPOSITION.encounter.v1.01.0",
            "openEHR-EHR-COMPOSITION.encounter.v1.0.0.0",
            "openEHR-EHR-COMPOSITION.encounter.v1..0",
            "openEHR-EHR-COMPOSITION.encounter.vx",
            "openEHR-EHR-COMPOSITION.encounter.v1.0-rc.1",
            "org.openehr::openEHR-EHR-COMPOSITION.encounter.v1.0.0-gamma",
            "org.openehr::openEHR-EHR-COMPOSITION.encounter.v1.0.0-rc.",
            "org.openehr::openEHR-EHR-COMPOSITION.encounter.v1.0.0-rc.x",
            "::openEHR-EHR-COMPOSITION.encounter.v1",
            "org..openehr::openEHR-EHR-COMPOSITION.encounter.v1",
            "1org.openehr::openEHR-EHR-COMPOSITION.encounter.v1",
            "org.openehr::openEHR-EHR.encounter.v1",
            "org.openehr::",
            "",
        ] {
            assert!(!is_valid_archetype_id(bad), "{bad} must be invalid");
        }
    }

    /// The construction door and the class's own invariants must agree, or the
    /// type's well-formed-by-construction guarantee is not one. They did not:
    /// the door checked for a `.v`, a `.` before it and three `-` segments, so
    /// these constructed and then failed `Validate`.
    #[test]
    fn construction_admits_exactly_what_the_invariants_accept() {
        for value in [
            "openEHR-EHR-COMPOSITION.encounter.v1",
            "openEHR-EHR-OBSERVATION.blood_pressure.v2.1.0",
            "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v2.1.0",
            "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v2.1.0-gamma",
            "1-2-3.@@@.vXYZ",
            "openEHR-EHR-COMPOSITION.encounter.v1draft",
            "openEHR-EHR.encounter.v1",
            "openEHR-EHR-COMPOSITION.encounter",
            "openEHR-EHR-COMPOSITION.encounter.v01",
        ] {
            assert_eq!(
                ArchetypeId::from_str(value).is_ok(),
                is_valid_archetype_id(value),
                "{value:?}: the door and the invariants disagree",
            );
        }
    }
}
