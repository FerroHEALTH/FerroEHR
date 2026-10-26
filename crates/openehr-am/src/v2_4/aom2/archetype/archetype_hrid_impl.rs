// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `ARCHETYPE_HRID` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.archetype_hrid.adoc` §Functions,
//! `BASE/docs/artefact_identification/master03-artefact_source_id.adoc` §Formal Model (the
//! `artefact_hrid` / `local_hrid` / `hrid_root` grammar), and
//! `BASE/docs/artefact_identification/master04-versioning.adoc` §Version Numbering (the
//! `version_id` grammar). The [`FromStr`] reading follows the physical form of
//! `AM/docs/ADL2/master07.05-adl_identification.adoc`.

use std::fmt;
use std::str::FromStr;

use crate::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_base::v1_3::base_types::definitions::version_status::VersionStatus;

/// Why a string is no archetype HRID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HridError {
    /// No `.vN` version segment.
    NoVersion(String),
    /// No `.concept` segment before the version.
    NoConcept(String),
    /// The model part is not `publisher-package-class`.
    ModelPart {
        /// The whole input.
        hrid: String,
        /// The model part found.
        found: String,
    },
    /// A publisher, package, class or concept segment is empty.
    EmptySegment(String),
    /// The version tail is not `MAJOR[.MINOR[.PATCH]][-status[.build]]`.
    InvalidVersion(String),
}

impl fmt::Display for HridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVersion(s) => write!(f, "HRID {s:?} has no `.vN` version segment"),
            Self::NoConcept(s) => write!(f, "HRID {s:?} has no `.concept` segment"),
            Self::ModelPart { hrid, found } => write!(
                f,
                "HRID {hrid:?} model part must be `publisher-package-class`, found {found:?}"
            ),
            Self::EmptySegment(s) => write!(f, "HRID {s:?} has an empty identifier segment"),
            Self::InvalidVersion(v) => write!(f, "invalid version {v:?}"),
        }
    }
}

impl std::error::Error for HridError {}

impl FromStr for ArchetypeHrid {
    type Err = HridError;

    /// Reads the physical form
    /// `[ns::]publisher-package-class.concept.vMAJOR[.MINOR[.PATCH]][-rc|-alpha|-beta[.build]]`
    /// (ADL2 `master07.05-adl_identification.adoc`), normalising a 1- or
    /// 2-part version to 3 parts (`v1` reads as `1.0.0`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (namespace, rest) = match s.split_once("::") {
            Some((ns, rest)) => (Some(ns.to_owned()), rest),
            None => (None, s),
        };
        let vpos = version_marker(rest).ok_or_else(|| HridError::NoVersion(s.to_owned()))?;
        let left = rest.get(..vpos).unwrap_or_default();
        let version = rest.get(vpos + 2..).unwrap_or_default();
        let (model_part, concept_id) = left
            .rsplit_once('.')
            .ok_or_else(|| HridError::NoConcept(s.to_owned()))?;
        let segments: Vec<&str> = model_part.split('-').collect();
        let [publisher, package, class] = segments.as_slice() else {
            return Err(HridError::ModelPart {
                hrid: s.to_owned(),
                found: model_part.to_owned(),
            });
        };
        if publisher.is_empty() || package.is_empty() || class.is_empty() || concept_id.is_empty() {
            return Err(HridError::EmptySegment(s.to_owned()));
        }
        let (release_version, version_status, build_count) = parse_version(version)?;
        Ok(Self {
            namespace,
            rm_publisher: (*publisher).to_owned(),
            rm_package: (*package).to_owned(),
            rm_class: (*class).to_owned(),
            concept_id: concept_id.to_owned(),
            release_version,
            version_status,
            build_count,
        })
    }
}

/// The byte offset of the `.v` that opens the version segment of `s`, the LAST
/// `.v` followed by a digit.
///
/// A concept id may itself start with `v` and a digit
/// (`openEHR-EHR-OBSERVATION.v1_score.v1`), while the version tail never holds
/// a `v` (ADL2 `master07.05-adl_identification.adoc` §Version Identifiers), so
/// the last marker is the version.
#[must_use]
pub fn version_marker(s: &str) -> Option<usize> {
    s.as_bytes()
        .windows(3)
        .rposition(|w| matches!(w, [b'.', b'v', third] if third.is_ascii_digit()))
}

/// The HRID version modifiers and the `VERSION_STATUS` each names: the
/// `version_status` row of `org.openehr.am.aom2.archetype_hrid.adoc` gives
/// `"rc"` for release_candidate, `"alpha"`, `"beta"`, and the empty string for
/// released (`master04-versioning.adoc` §Version Numbering).
const MODIFIERS: [(&str, VersionStatus); 3] = [
    ("rc", VersionStatus::ReleaseCandidate),
    ("alpha", VersionStatus::Alpha),
    ("beta", VersionStatus::Beta),
];

/// The `VERSION_STATUS` an HRID version modifier names, the empty modifier
/// naming `released`; `None` for a token the HRID grammar does not define.
#[must_use]
pub fn version_status_of_modifier(modifier: &str) -> Option<VersionStatus> {
    if modifier.is_empty() {
        return Some(VersionStatus::Released);
    }
    MODIFIERS
        .iter()
        .find(|(token, _)| *token == modifier)
        .map(|(_, status)| status.clone())
}

/// The version tail as `(release_version, status, build_count)`.
fn parse_version(version: &str) -> Result<(String, VersionStatus, String), HridError> {
    let (status, numeric, build) = MODIFIERS
        .iter()
        .find_map(|(modifier, status)| {
            split_status(version, &format!("-{modifier}"))
                .map(|(numeric, build)| (status.clone(), numeric, build))
        })
        .unwrap_or((VersionStatus::Released, version, ""));
    let mut parts = numeric.split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    let patch = parts.next().unwrap_or("0");
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return Err(HridError::InvalidVersion(version.to_owned()));
    }
    let or_zero = |part: &str| if part.is_empty() { "0" } else { part }.to_owned();
    let release_version = format!("{major}.{}.{}", or_zero(minor), or_zero(patch));
    Ok((release_version, status, build.to_owned()))
}

/// Splits `version` at the `marker` pre-release suffix into `(numeric, build)`.
fn split_status<'a>(version: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let (numeric, after) = version.split_once(marker)?;
    let build = after.strip_prefix('.').unwrap_or("");
    Some((numeric, build))
}

impl ArchetypeHrid {
    /// Returns the major version, extracted from `release_version`.
    ///
    /// `major_version` (`org.openehr.am.aom2.archetype_hrid.adoc` §Functions)
    /// over `release_version = major_version '.' minor_version '.'
    /// patch_version` (`master04-versioning.adoc` §Version Numbering). A
    /// `release_version` with fewer parts than the grammar requires yields the
    /// empty string for the missing ones.
    #[must_use]
    pub fn major_version(&self) -> &str {
        self.release_version.split('.').next().unwrap_or("")
    }

    /// Returns the minor version, extracted from `release_version`.
    ///
    /// `minor_version` (`org.openehr.am.aom2.archetype_hrid.adoc` §Functions),
    /// the second part of the 3-part `release_version`.
    #[must_use]
    pub fn minor_version(&self) -> &str {
        self.release_version.split('.').nth(1).unwrap_or("")
    }

    /// Returns the patch version, extracted from `release_version`.
    ///
    /// `patch_version` (`org.openehr.am.aom2.archetype_hrid.adoc` §Functions),
    /// the third part of the 3-part `release_version`.
    #[must_use]
    pub fn patch_version(&self) -> &str {
        self.release_version.split('.').nth(2).unwrap_or("")
    }

    /// Returns the HRID version modifier of `version_status`: `rc`, `alpha`,
    /// `beta`, or the empty string for released.
    ///
    /// The `version_status` row of `org.openehr.am.aom2.archetype_hrid.adoc`.
    /// `build` has no modifier in the HRID grammar (`master04-versioning.adoc`
    /// §Version Numbering) and writes the empty one; an out-of-set status
    /// writes its own token.
    #[must_use]
    pub fn version_modifier(&self) -> &str {
        match &self.version_status {
            VersionStatus::ReleaseCandidate => "rc",
            VersionStatus::Alpha => "alpha",
            VersionStatus::Beta => "beta",
            VersionStatus::Released | VersionStatus::Build => "",
            VersionStatus::Other(token) => token.as_str(),
        }
    }

    /// Returns the full version identifier string.
    ///
    /// `version_id` (`org.openehr.am.aom2.archetype_hrid.adoc` §Functions):
    /// "based on `release_version`, `version_status`, and `build_count` e.g.
    /// `"1.8.2-rc.4"`". `master04-versioning.adoc` §Version Numbering gives the
    /// grammar `version_id = release_version [ extension ]`, `extension =
    /// version_modifier '.' issue_number`, `version_modifier = '-rc' |
    /// '-alpha'`; the class page adds `beta` and spells `released` as the empty
    /// modifier.
    #[must_use]
    pub fn version_id(&self) -> String {
        let modifier = self.version_modifier();
        if modifier.is_empty() {
            self.release_version.clone()
        } else {
            format!("{}-{modifier}.{}", self.release_version, self.build_count)
        }
    }

    /// Returns the interface form of this identifier, down to the major version.
    ///
    /// `semantic_id` (`org.openehr.am.aom2.archetype_hrid.adoc` §Functions):
    /// "The 'interface' form of the HRID, i.e. down to the major version",
    /// built over the `master03-artefact_source_id.adoc` §Formal Model grammar
    /// `namespaced_hrid = namespace '::' local_hrid`, `local_hrid = hrid_root
    /// '.v' version_id`, `hrid_root = rm_publisher '-' rm_closure '-' rm_class
    /// '.' concept_id`.
    #[must_use]
    pub fn semantic_id(&self) -> String {
        self.qualified(self.major_version())
    }

    /// Returns the physical form of this identifier, with full version
    /// information.
    ///
    /// `physical_id` (`org.openehr.am.aom2.archetype_hrid.adoc` §Functions):
    /// "The 'physical' form of the HRID, i.e. with complete version information
    /// specified by `version_id()`", over the same grammar as
    /// [`ArchetypeHrid::semantic_id`].
    #[must_use]
    pub fn physical_id(&self) -> String {
        self.qualified(&self.version_id())
    }

    /// The `hrid_root '.v' <version>` form, namespace-prefixed when the
    /// artefact is managed.
    fn qualified(&self, version: &str) -> String {
        let root = format!(
            "{}-{}-{}.{}.v{version}",
            self.rm_publisher, self.rm_package, self.rm_class, self.concept_id
        );
        match self.namespace.as_deref() {
            Some(ns) if !ns.is_empty() => format!("{ns}::{root}"),
            _ => root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hrid(namespace: Option<&str>, status: VersionStatus, build_count: &str) -> ArchetypeHrid {
        ArchetypeHrid {
            namespace: namespace.map(str::to_owned),
            rm_publisher: "openEHR".to_owned(),
            rm_package: "EHR".to_owned(),
            rm_class: "OBSERVATION".to_owned(),
            concept_id: "blood_pressure".to_owned(),
            release_version: "1.8.2".to_owned(),
            version_status: status,
            build_count: build_count.to_owned(),
        }
    }

    #[test]
    fn the_three_version_parts_come_from_the_release_version() {
        let h = hrid(None, VersionStatus::Released, "0");
        assert_eq!(h.major_version(), "1");
        assert_eq!(h.minor_version(), "8");
        assert_eq!(h.patch_version(), "2");
    }

    #[test]
    fn a_released_version_id_carries_no_extension() {
        assert_eq!(
            hrid(None, VersionStatus::Released, "4").version_id(),
            "1.8.2"
        );
    }

    #[test]
    fn a_release_candidate_version_id_carries_the_build_count() {
        assert_eq!(
            hrid(None, VersionStatus::ReleaseCandidate, "4").version_id(),
            "1.8.2-rc.4"
        );
        assert_eq!(
            hrid(None, VersionStatus::Alpha, "1").version_id(),
            "1.8.2-alpha.1"
        );
    }

    #[test]
    fn the_semantic_id_stops_at_the_major_version() {
        assert_eq!(
            hrid(None, VersionStatus::ReleaseCandidate, "4").semantic_id(),
            "openEHR-EHR-OBSERVATION.blood_pressure.v1"
        );
    }

    #[test]
    fn the_physical_id_carries_the_whole_version_id() {
        assert_eq!(
            hrid(None, VersionStatus::ReleaseCandidate, "4").physical_id(),
            "openEHR-EHR-OBSERVATION.blood_pressure.v1.8.2-rc.4"
        );
    }

    #[test]
    fn a_namespace_makes_the_identifier_managed() {
        assert_eq!(
            hrid(Some("org.openehr"), VersionStatus::Released, "0").physical_id(),
            "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v1.8.2"
        );
        assert_eq!(
            hrid(Some(""), VersionStatus::Released, "0").semantic_id(),
            "openEHR-EHR-OBSERVATION.blood_pressure.v1"
        );
    }

    #[test]
    fn a_physical_id_parses_back_to_its_hrid() {
        let id = "org.openehr::openEHR-EHR-OBSERVATION.blood_pressure.v1.8.2-rc.4";
        let parsed: ArchetypeHrid = id.parse().expect("parses");
        assert_eq!(parsed.namespace.as_deref(), Some("org.openehr"));
        assert_eq!(parsed.concept_id, "blood_pressure");
        assert_eq!(parsed.release_version, "1.8.2");
        assert_eq!(parsed.build_count, "4");
        assert_eq!(parsed.physical_id(), id);
    }

    #[test]
    fn a_partial_version_is_normalised_to_three_parts() {
        let parsed: ArchetypeHrid = "openEHR-EHR-OBSERVATION.v1_score.v1"
            .parse()
            .expect("parses");
        assert_eq!(parsed.concept_id, "v1_score");
        assert_eq!(parsed.release_version, "1.0.0");
        assert_eq!(parsed.version_id(), "1.0.0");
        assert_eq!(parsed.namespace, None);
    }

    #[test]
    fn a_malformed_hrid_names_its_defect() {
        assert_eq!(
            "openEHR-EHR-OBSERVATION.bp".parse::<ArchetypeHrid>(),
            Err(HridError::NoVersion(
                "openEHR-EHR-OBSERVATION.bp".to_owned()
            ))
        );
        assert!(matches!(
            "openEHR-OBSERVATION.bp.v1".parse::<ArchetypeHrid>(),
            Err(HridError::ModelPart { .. })
        ));
        assert!(matches!(
            "openEHR-EHR-OBSERVATION..v1".parse::<ArchetypeHrid>(),
            Err(HridError::EmptySegment(_))
        ));
    }

    #[test]
    fn a_short_release_version_yields_empty_missing_parts() {
        let mut h = hrid(None, VersionStatus::Released, "0");
        h.release_version = "2".to_owned();
        assert_eq!(h.major_version(), "2");
        assert_eq!(h.minor_version(), "");
        assert_eq!(h.patch_version(), "");
    }
}
