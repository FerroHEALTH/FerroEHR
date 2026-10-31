// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stored release of an ADL2 artefact that a reference resolves to.
//!
//! The `openehr-adl` [`ArchetypeRepository`] holds one artefact per
//! `publisher-package-class.concept` family and ignores the version, so the
//! store chooses the release each reference names before the engine sees it.
//! BASE Artefact Identification
//! (`BASE/docs/artefact_identification/master07-referencing.adoc`
//! §Referencing): an interface reference (`ihrid_ref`, major version only)
//! "will match the latest release available of that major version", a
//! reference with more version parts narrows the match to them, and "a
//! pre-release version is referenced by its full identifier".

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::hrid::{hrid_lookup_key, parse_hrid, raw_id_lookup_key, raw_id_version};
use openehr_adl::opt::unresolved_constituents;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_base::v1_3::base_types::definitions::version_status::VersionStatus;

/// One stored artefact a reference can resolve to.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    /// The stored `ARCHETYPE_HRID`, parsed.
    hrid: ArchetypeHrid,
    /// The parsed stored source.
    artefact: Archetype,
}

impl Candidate {
    /// One stored `(hrid, adl)` row, or `None` when either no longer parses.
    ///
    /// NOTE: a stored row that no longer parses is left out, as the engine
    /// repository always left it out; it was engine-valid at its upload.
    pub(super) fn parse(hrid: &str, adl: &str) -> Option<Self> {
        Some(Self {
            hrid: parse_hrid(hrid).ok()?,
            artefact: parse_artefact(adl, Dialect::Adl2).ok()?,
        })
    }

    /// The parsed stored source.
    pub(super) fn artefact(&self) -> &Archetype {
        &self.artefact
    }
}

/// Every stored `(hrid, adl)` row that still parses, in the order of `rows`.
pub(super) fn parse_stored(rows: &[(String, String)]) -> Vec<Candidate> {
    rows.iter()
        .filter_map(|(hrid, adl)| Candidate::parse(hrid, adl))
        .collect()
}

/// The repository `root` resolves against: every specialisation parent and
/// archetype reference in its closure resolved to the stored release
/// [`choose`] picks for it, and nothing else.
///
/// A family the closure references at two major versions holds the first one
/// resolved, the engine repository holding one artefact per family.
pub(super) fn repository_for(root: &Archetype, stored: &[Candidate]) -> ArchetypeRepository {
    let hrids: Vec<&ArchetypeHrid> = stored.iter().map(|c| &c.hrid).collect();
    let mut repo = ArchetypeRepository::new();
    // Each round that inserts resolves at least one family for good, so the
    // walk ends within one round per stored artefact.
    for _ in 0..=stored.len() {
        let mut grew = false;
        for reference in unresolved_constituents(root, &repo) {
            if let Some(found) = choose(&reference, &hrids).and_then(|i| stored.get(i)) {
                repo.insert(found.artefact.clone());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    repo
}

/// What a reference asks of a stored version.
enum Wanted {
    /// The leading numeric version parts a release or release candidate must
    /// carry (`v1` → `[1]`, `v1.2` → `[1, 2]`).
    Prefix(Vec<u64>),
    /// A pre-release named by its full identifier.
    Exact(ArchetypeHrid),
}

impl Wanted {
    /// The version `reference` asks for, or `None` when its version is not
    /// numeric, so that nothing stored matches it.
    fn of(reference: &str) -> Option<Self> {
        if let Ok(h) = parse_hrid(reference)
            && h.version_status != VersionStatus::Released
        {
            return Some(Self::Exact(h));
        }
        let version = raw_id_version(reference);
        if version.is_empty() {
            return Some(Self::Prefix(Vec::new()));
        }
        version
            .split('.')
            .map(|part| part.parse().ok())
            .collect::<Option<Vec<u64>>>()
            .map(Self::Prefix)
    }

    /// The precedence of `stored` when it matches, higher first.
    fn rank(&self, stored: &ArchetypeHrid) -> Option<Precedence> {
        let precedence = precedence(stored)?;
        let admitted = match self {
            Self::Prefix(parts) => {
                precedence.status != Status::Other && precedence.release.starts_with(parts)
            }
            Self::Exact(h) => {
                h.release_version == stored.release_version
                    && h.version_status == stored.version_status
                    && h.build_count == stored.build_count
            }
        };
        admitted.then_some(precedence)
    }
}

/// The lifecycle of a stored version as reference matching ranks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    /// An alpha, beta or build version, matched by its full identifier only.
    Other,
    /// A release candidate.
    Candidate,
    /// A released version.
    Released,
}

/// The order of the stored versions one reference matches: released before
/// any release candidate, then the newest release number, then the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Precedence {
    /// The lifecycle, compared first.
    status: Status,
    /// `major.minor.patch`.
    release: [u64; 3],
    /// The pre-release build count, `0` when none.
    build: u64,
}

/// The [`Precedence`] of a stored version, or `None` when its release number
/// is not numeric.
///
/// NOTE: `master07-referencing.adoc` §Archetype External References (ADL/AOM 2)
/// resolves to "the most recent released version … OR the latest release
/// candidate" without an order; our own design takes the release first.
fn precedence(h: &ArchetypeHrid) -> Option<Precedence> {
    let number = |part: &str| part.parse::<u64>().ok();
    let status = match h.version_status {
        VersionStatus::Released => Status::Released,
        VersionStatus::ReleaseCandidate => Status::Candidate,
        _ => Status::Other,
    };
    let build = if h.build_count.is_empty() {
        0
    } else {
        number(&h.build_count)?
    };
    Some(Precedence {
        status,
        release: [
            number(h.major_version())?,
            number(h.minor_version())?,
            number(h.patch_version())?,
        ],
        build,
    })
}

/// The index in `stored` of the version `reference` resolves to: the
/// highest-ranked one of its family that matches it, the last of equals.
fn choose(reference: &str, stored: &[&ArchetypeHrid]) -> Option<usize> {
    let family = raw_id_lookup_key(reference);
    let wanted = Wanted::of(reference)?;
    stored
        .iter()
        .enumerate()
        .filter(|(_, h)| hrid_lookup_key(h) == family)
        .filter_map(|(i, h)| wanted.rank(h).map(|rank| (i, rank)))
        .max_by_key(|(_, rank)| *rank)
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id `choose` picks for `reference` out of `ids`.
    fn chosen<'a>(reference: &str, ids: &[&'a str]) -> Option<&'a str> {
        let parsed: Vec<ArchetypeHrid> = ids.iter().map(|id| parse_hrid(id).unwrap()).collect();
        let refs: Vec<&ArchetypeHrid> = parsed.iter().collect();
        choose(reference, &refs).map(|i| ids[i])
    }

    const OBS: &str = "openEHR-EHR-OBSERVATION.bp";

    fn id(version: &str) -> String {
        format!("{OBS}.v{version}")
    }

    #[test]
    fn an_interface_reference_takes_the_newest_release_of_its_major_version() {
        let ids = [id("1.0.0"), id("1.10.0"), id("1.9.3"), id("2.0.0")];
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(chosen(&id("1"), &ids), Some(ids[1]));
        assert_eq!(chosen(&id("2"), &ids), Some(ids[3]));
        assert_eq!(chosen(&id("3"), &ids), None);
        let mut reversed = ids.clone();
        reversed.reverse();
        assert_eq!(chosen(&id("1"), &reversed), Some(ids[1]));
    }

    #[test]
    fn more_version_parts_narrow_the_match() {
        let ids = [id("1.2.0"), id("1.2.7"), id("1.3.0")];
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(chosen(&id("1.2"), &ids), Some(ids[1]));
        assert_eq!(chosen(&id("1.2.0"), &ids), Some(ids[0]));
        assert_eq!(chosen(&id("1.4"), &ids), None);
    }

    #[test]
    fn a_release_wins_over_a_newer_release_candidate() {
        let ids = [id("1.3.0-rc.2"), id("1.2.0"), id("1.3.0-rc.10")];
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(chosen(&id("1"), &ids), Some(ids[1]));
        assert_eq!(chosen(&id("1"), &[ids[0], ids[2]]), Some(ids[2]));
    }

    #[test]
    fn alpha_and_beta_versions_match_only_their_full_identifier() {
        let ids = [id("1.0.0-alpha"), id("1.0.0-beta.2")];
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(chosen(&id("1"), &ids), None);
        assert_eq!(chosen(&id("1.0.0-beta.2"), &ids), Some(ids[1]));
    }

    #[test]
    fn the_family_is_case_insensitive_and_other_families_never_match() {
        let ids = [
            "openehr-ehr-observation.BP.v1.0.0",
            "openEHR-EHR-OBSERVATION.bp2.v1.1.0",
        ];
        assert_eq!(chosen(&id("1"), &ids), Some(ids[0]));
    }
}
