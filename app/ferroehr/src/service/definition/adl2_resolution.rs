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

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;

/// One stored ADL2 source, parsed, or `None` when it no longer parses.
///
/// NOTE: a stored row that no longer parses is left out, as the engine
/// repository always left it out; it was engine-valid at its upload.
pub(super) fn parse_stored(adl: &str) -> Option<Archetype> {
    parse_artefact(adl, Dialect::Adl2).ok()
}

/// The repository holding every one of `stored`.
pub(super) fn repository_of(stored: impl IntoIterator<Item = Archetype>) -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    for artefact in stored {
        repo.insert(artefact);
    }
    repo
}
