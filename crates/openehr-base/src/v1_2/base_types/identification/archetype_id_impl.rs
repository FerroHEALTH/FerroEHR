// @generated-from-template templates/openehr-base/base_types/identification/archetype_id_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written accessor functions for `ARCHETYPE_ID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.archetype_id.adoc`.
//! Lexical form:
//! `rm_originator '-' rm_name '-' rm_entity '.' concept_name {'-' specialisation}* '.v' version_id`
//! e.g. `openEHR-EHR-OBSERVATION.blood_pressure-cuff.v1`.

use super::archetype_id::ArchetypeId;
use super::lexical::IdError;
use std::str::FromStr;

/// Decomposition of an archetype-id value into `qualified_rm_entity`,
/// `domain_concept`, and `version_id` slices.
struct Parts<'a> {
    qualified: &'a str,
    domain: &'a str,
    version: &'a str,
}

/// Split an archetype-id string on the trailing `.vN` and the first `.` after
/// the RM qualifier. Returns `None` if either separator is absent.
fn parse(value: &str) -> Option<Parts<'_>> {
    let (head, version) = value.rsplit_once(".v")?;
    let (qualified, domain) = head.split_once('.')?;
    if qualified.is_empty() || domain.is_empty() || version.is_empty() {
        return None;
    }
    Some(Parts {
        qualified,
        domain,
        version,
    })
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

    /// Major version identifier, e.g. `1` — the part after the trailing `.v`
    /// (BASE `version_id`).
    #[must_use]
    pub fn version_id(&self) -> &str {
        parse(&self.value).map_or("", |p| p.version)
    }

    /// Major version number, e.g. `1` for both `.v1` and `.v1.2.3` — the leading
    /// numeric segment of `version_id`.
    ///
    /// BASE base_types master05 §Syntaxes defines only the single-part form
    /// (`version_id = '0' | non-zero-digit, [ number ]`); the three-part
    /// `major.minor.patch` belongs to the AM archetype HRID
    /// (`docs/specs/openehr/AM/docs/Identification/`), and only the major is
    /// significant to interface-reference matching (AM master07 §Referencing:
    /// an `ihrid_ref` carries the major version only, and a differing major is
    /// a hard boundary).
    #[must_use]
    pub fn major_version(&self) -> &str {
        self.version_id().split('.').next().unwrap_or("")
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
        assert_eq!("".parse::<ArchetypeId>(), Err(IdError::Empty));
    }
}

/// Lexical validity per BASE base_types master05 §Syntaxes:
/// `archetype_id = qualified_rm_entity '.' domain_concept '.v' version_id`,
/// `qualified_rm_entity = rm_originator '-' rm_name '-' rm_entity` (each an
/// `alphanum-str` = letter { letter | digit | '_' }), `domain_concept =
/// concept_name { '-' specialisation }`, `version_id = '0' | nz-digit [number]`.
#[must_use]
pub(crate) fn is_valid_archetype_id(value: &str) -> bool {
    fn alphanum_str(s: &str) -> bool {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    let mut sections = value.split('.');
    let (Some(entity), Some(concept), Some(version), None) = (
        sections.next(),
        sections.next(),
        sections.next(),
        sections.next(),
    ) else {
        return false;
    };
    let entity_ok = {
        let parts: Vec<&str> = entity.split('-').collect();
        parts.len() == 3 && parts.iter().all(|p| alphanum_str(p))
    };
    let concept_ok = concept.split('-').all(alphanum_str);
    let version_ok = version.strip_prefix('v').is_some_and(|n| {
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'))
    });
    entity_ok && concept_ok && version_ok
}

impl crate::validate::Validate for ArchetypeId {
    fn validate_invariants(&self, out: &mut Vec<crate::validate::InvariantViolation>) {
        if !is_valid_archetype_id(&self.value) {
            out.push(crate::validate::InvariantViolation::here(
                "Invariant Value_valid failed on type ARCHETYPE_ID (lexical form \
                 rm_originator-rm_name-rm_entity.domain_concept.vN, BASE base_types \
                 master05 §Syntaxes)",
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
        ] {
            assert!(is_valid_archetype_id(ok), "{ok} must be valid");
        }
        for bad in [
            "openEHR-EHR-COMPOSITION.encounter.v1draft",
            "openEHR-EHR.encounter.v1",
            "openEHR-EHR-COMPOSITION.encounter",
            "openEHR-EHR-COMPOSITION.1concept.v1",
            "openEHR-EHR-COMPOSITION.encounter.v01",
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
