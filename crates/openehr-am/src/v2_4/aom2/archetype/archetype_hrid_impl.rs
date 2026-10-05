// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `ARCHETYPE_HRID` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.archetype_hrid.adoc` §Functions,
//! `AM/docs/Identification/master03-artefact_source_id.adoc` §Formal Model (the
//! `artefact_hrid` / `local_hrid` / `hrid_root` grammar), and
//! `AM/docs/Identification/master04-versioning.adoc` §Version Numbering (the
//! `version_id` grammar).

use crate::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_base::v1_3::base_types::definitions::version_status::VersionStatus;

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
        let modifier = match self.version_status {
            VersionStatus::ReleaseCandidate => "rc",
            VersionStatus::Alpha => "alpha",
            VersionStatus::Beta => "beta",
            VersionStatus::Released | VersionStatus::Build => "",
            VersionStatus::Other(ref s) => s.as_str(),
        };
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
    fn a_short_release_version_yields_empty_missing_parts() {
        let mut h = hrid(None, VersionStatus::Released, "0");
        h.release_version = "2".to_owned();
        assert_eq!(h.major_version(), "2");
        assert_eq!(h.minor_version(), "");
        assert_eq!(h.patch_version(), "");
    }
}
