// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 artefact that is authored as its own flat form.
//!
//! ADL 1.4 has no differential source form: "Up until ADL 1.4, archetypes
//! were expressed in 'flat' form ADL … These are now treated as legacy flat
//! format files" (`ADL2/master07.02-adl_conventions.adoc` §File-naming
//! Convention), and AOM2 names "ADL 1.4 legacy archetypes (always in flat
//! form)" (`AOM2/master03-archetype_package.adoc` §Structural Variants). The
//! AOM 1.4 `ARCHETYPE` (`AOM1.4/master03-archetype_package.adoc`) carries no
//! differential flag at all, so a 1.4 specialised archetype restates its
//! parent whole and only names it as the parent it narrows
//! (`AOM1.4/master02-model_overview.adoc` §Archetype Specialisation).

use crate::artefact::ArchetypeView;

/// The first ADL release whose specialised source is a differential: ADL 1.5
/// introduced the differential representation (`ADL2/master01-preface.adoc`
/// §ADL 1.5).
const FIRST_DIFFERENTIAL: (u64, u64) = (1, 5);

/// The specialisation parent the artefact `v` views must be flattened over to
/// reach its flat form, or `None` when its authored form already is one.
///
/// A top-level archetype is its own flat form (`ADL2/master09.02` §Differential
/// and Flat Forms), and so is an ADL 1.4 specialised archetype, which is
/// authored whole ([`is_authored_flat`]): overlaying it on its parent would
/// append every object that carries no node id a second time. Its
/// `parent_archetype_id` stays on the flat form, since it still names the
/// lineage it narrows.
pub(crate) fn flattening_parent<'a>(v: &ArchetypeView<'a>) -> Option<&'a str> {
    v.parent_archetype_id
        .filter(|_| !is_authored_flat(v.adl_version))
}

/// True if an artefact whose header declares `adl_version` is authored in the
/// ADL 1.4 flat form, so flattening it over its parent would restate the
/// parent's structure a second time.
///
/// The header's `adl_version` names "the version of the archetype formalism"
/// the artefact is expressed in (`AOM2/master03-archetype_package.adoc` §ADL
/// Version), which is how a 1.4 artefact parsed into the AOM2 model is told
/// apart from an ADL 2 one. An absent or unreadable version is read as ADL 2,
/// the form every artefact without the 1.4 header takes.
fn is_authored_flat(adl_version: Option<&str>) -> bool {
    let Some(version) = adl_version else {
        return false;
    };
    let mut parts = version.trim().split('.');
    let major = parts.next().and_then(|p| p.parse::<u64>().ok());
    let minor = parts.next().map_or(Some(0), |p| p.parse::<u64>().ok());
    match (major, minor) {
        (Some(major), Some(minor)) => (major, minor) < FIRST_DIFFERENTIAL,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_authored_flat;

    #[test]
    fn adl_14_and_earlier_are_authored_flat() {
        for version in ["1.4", "1.4.1", "1.0", "1", " 1.4 "] {
            assert!(is_authored_flat(Some(version)), "{version}");
        }
    }

    #[test]
    fn adl_15_onward_and_unknown_versions_are_differential() {
        for version in ["1.5", "1.5.1", "2.0.5", "2.4.0", "", "x.4", "1.x"] {
            assert!(!is_authored_flat(Some(version)), "{version}");
        }
        assert!(!is_authored_flat(None));
    }
}
