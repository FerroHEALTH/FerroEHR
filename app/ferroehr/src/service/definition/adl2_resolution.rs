// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The repository of stored ADL2 artefacts an artefact compiles against.
//!
//! The `openehr-adl` [`ArchetypeRepository`] holds every stored release under
//! its namespace, family and major version and resolves each reference to the
//! release BASE Artefact Identification
//! (`BASE/docs/artefact_identification/master07-referencing.adoc`
//! §Referencing) names, so every stored artefact goes in and the engine
//! applies the one resolution rule.

use openehr_adl::artefact::{ArchetypeRepository, resolve_hrid};
use openehr_adl::assemble::parse_artefact;
use openehr_adl::hrid::{parse_hrid, split_at_version};
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;

/// One stored ADL2 source, parsed, or `None` when it no longer parses.
///
/// NOTE: a stored row that no longer parses is left out, as the engine
/// repository always left it out; it was engine-valid at its upload.
pub(super) fn parse_stored(adl: &str) -> Option<Archetype> {
    parse_artefact(adl, Dialect::Adl2).ok()
}

/// The repository holding every one of `stored`.
pub(super) fn repository_of(stored: impl IntoIterator<Item = Archetype>) -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::default();
    for artefact in stored {
        repo.insert(artefact);
    }
    repo
}

/// The archetype reference a wire `template_id` and its optional `version`
/// make: the `version` replaces any version the `template_id` writes.
pub(super) fn wire_reference(template_id: &str, version: Option<&str>) -> String {
    let Some(version) = version else {
        return template_id.to_owned();
    };
    let (namespace, core) = match template_id.rsplit_once("::") {
        Some((namespace, core)) => (Some(namespace), core),
        None => (None, template_id),
    };
    let family = split_at_version(core).0;
    match namespace {
        Some(namespace) => format!("{namespace}::{family}.v{version}"),
        None => format!("{family}.v{version}"),
    }
}

/// The stored HRID among `stored` that `reference` resolves to, by the
/// engine's rule ([`resolve_hrid`], BASE Artefact Identification
/// `master07-referencing.adoc` §Referencing): a release outranks its
/// pre-releases, and a pre-release is read only by its full identifier.
///
/// NOTE: a stored HRID that no longer parses is left out, as
/// [`parse_stored`] leaves out a stored source; it parsed at its upload.
pub(super) fn resolve_stored(reference: &str, stored: Vec<String>) -> Option<String> {
    let any_namespace = !reference.contains("::");
    let held: Vec<(ArchetypeHrid, String)> = stored
        .into_iter()
        .filter_map(|raw| parse_hrid(&raw).ok().map(|h| (h, raw)))
        .map(|(mut h, raw)| {
            // NOTE: ITS-REST `template_id_adl2.yaml` reads the partial id
            // `openEHR-EHR-COMPOSITION.t_vital_signs.v1` as its namespaced
            // example template, so a wire id without a namespace reads every one.
            if any_namespace {
                h.namespace = None;
            }
            (h, raw)
        })
        .collect();
    let chosen = resolve_hrid(reference, None, held.iter().map(|(h, _)| h))?;
    held.iter()
        .find(|(h, _)| std::ptr::eq(h, chosen))
        .map(|(_, raw)| raw.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wire_version_replaces_the_template_id_version() {
        let family = "openEHR-EHR-COMPOSITION.c";
        assert_eq!(
            wire_reference(&format!("{family}.v1"), None),
            "openEHR-EHR-COMPOSITION.c.v1"
        );
        assert_eq!(
            wire_reference(family, Some("1.0.0-rc.1")),
            format!("{family}.v1.0.0-rc.1")
        );
        assert_eq!(
            wire_reference(&format!("{family}.v2"), Some("1.2")),
            format!("{family}.v1.2")
        );
        assert_eq!(
            wire_reference(&format!("org.example::{family}.v2"), Some("1")),
            format!("org.example::{family}.v1")
        );
    }

    #[test]
    fn a_stored_release_outranks_its_candidate_in_either_order() {
        let release = "openEHR-EHR-COMPOSITION.c.v1.0.0".to_owned();
        let candidate = "openEHR-EHR-COMPOSITION.c.v1.0.0-rc.1".to_owned();
        for stored in [
            vec![release.clone(), candidate.clone()],
            vec![candidate.clone(), release.clone()],
        ] {
            let read = |reference: &str| resolve_stored(reference, stored.clone());
            assert_eq!(read("openEHR-EHR-COMPOSITION.c.v1"), Some(release.clone()));
            assert_eq!(
                read("openEHR-EHR-COMPOSITION.c.v1.0.0"),
                Some(release.clone())
            );
            assert_eq!(read(&candidate), Some(candidate.clone()));
            assert_eq!(read("openEHR-EHR-COMPOSITION.c.v2"), None);
        }
    }

    #[test]
    fn a_wire_id_without_a_namespace_reads_a_namespaced_template() {
        let stored = vec!["org.highmed::openEHR-EHR-COMPOSITION.t.v1.0.0".to_owned()];
        assert_eq!(
            resolve_stored("openEHR-EHR-COMPOSITION.t.v1", stored.clone()),
            stored.first().cloned()
        );
        assert_eq!(
            resolve_stored("org.other::openEHR-EHR-COMPOSITION.t.v1", stored),
            None
        );
    }
}
