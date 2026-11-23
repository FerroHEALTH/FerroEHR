// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 artefact authored as its own flat form: 1.4 archetypes are
//! "always in flat form" (`AOM2/master03-archetype_package.adoc` §Structural
//! Variants; `ADL2/master07.02` §File-naming Convention).

use crate::artefact::ArchetypeView;

/// The first ADL release whose specialised source is a differential: ADL 1.5
/// introduced the differential representation (`ADL2/master01-preface.adoc`
/// §ADL 1.5).
const FIRST_DIFFERENTIAL: (u64, u64) = (1, 5);

/// The parent `v` must be flattened over to reach its flat form, or `None` when
/// it already is one: a top-level archetype (`ADL2/master09.02`) or an ADL 1.4
/// specialised archetype, authored whole ([`is_authored_flat`]).
pub(crate) fn flattening_parent<'a>(v: &ArchetypeView<'a>) -> Option<&'a str> {
    v.parent_archetype_id
        .filter(|_| !is_authored_flat(v.adl_version))
}

/// Whether an artefact declaring `adl_version` is authored in the ADL 1.4 flat
/// form (`AOM2/master03-archetype_package.adoc` §ADL Version); an absent or
/// unreadable version reads as ADL 2.
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
