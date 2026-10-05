// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Build/spec provenance constants.
//!
//! The spec pins are **read from the `openehr-*` crates**, never hand-typed
//! literals, so a pin bump in the spec layer propagates here at compile time
//! and the identity surfaces cannot drift. For the generated multi-generation
//! crates the authority is the emitted `Generation` enum (the crates carry no
//! crate-level pin — a fixed one would contradict a configured non-current
//! generation); the hand-written single-spec crates keep their literal
//! `SPEC_VERSION`.

/// Returns the openEHR RM version the given profile serves.
#[must_use]
pub const fn rm_for(profile: crate::config::profile::SpecProfile) -> &'static str {
    profile.rm().spec_version()
}

/// Returns the openEHR BASE version the given profile serves.
#[must_use]
pub const fn base_for(profile: crate::config::profile::SpecProfile) -> &'static str {
    profile.base().spec_version()
}

/// Returns the openEHR LANG version the given profile serves.
#[must_use]
pub const fn lang_for(profile: crate::config::profile::SpecProfile) -> &'static str {
    profile.lang().spec_version()
}

/// The openEHR ITS-REST contract version this server implements.
///
/// The released `Release-1.1.0`, 19-Jul-2026; the vendored `-codegen` OAS at
/// `crates/openehr-its/vendor/rest-oas/PROVENANCE.md` pins that release.
/// Reported by management `/info`, the System Options manifest
/// (`restapi_specs_version` — a plain version string, per the System API OAS
/// example `restapi_specs_version: 1.1.0`), and `/status`.
pub const ITS_REST: &str = openehr_its::SPEC_VERSION;
/// The AQL (QUERY) specification version.
pub const AQL: &str = openehr_query::SPEC_VERSION;
/// The openEHR Reference Model version.
///
/// The default (current) RM generation's pin. A const cannot call
/// `Generation::default()` (trait fns are not const-callable), so this names
/// the `#[default]` variant explicitly; the test below pins the two
/// together.
pub const RM: &str = openehr_rm::Generation::V1_2.spec_version();
/// The openEHR BASE version.
///
/// The default (current) BASE generation's pin (same const/`Default` pairing
/// as [`RM`]).
pub const BASE: &str = openehr_base::Generation::V1_3.spec_version();
/// The ADL 1.4 generation of the Archetype Model.
///
/// The `openehr-am` crate ships both extant generations side by side; this
/// is the `v1_4` module's own pin.
pub const AM14: &str = openehr_am::Generation::V1_4.spec_version();
/// The ADL 2 generation of the Archetype Model.
///
/// The `openehr-am` crate's current generation, the `v2_4` module.
pub const AM24: &str = openehr_am::Generation::V2_4.spec_version();
/// The openEHR Terminology version.
///
/// The default (current) TERM generation's pin (same const/`Default` pairing
/// as [`RM`]).
pub const TERM: &str = openehr_term::Generation::V3_1.spec_version();
/// The `PostgreSQL` version this server targets. No openEHR spec governs the
/// datastore — our own design; no crate carries this pin, so it stays
/// hand-maintained here.
pub const PG_TARGET: &str = "18.6+";
/// The last machine-computed ECC conformance verdict — the highest profile
/// obtained — advertised by the System Options manifest (`OPTIONS /`
/// `conformance_profile`).
///
/// No openEHR spec governs the value; the conformance instrument computes it
/// (CNF master03 profiles). Updated at each conformance re-baseline from the
/// runner's committed machine verdict artifacts under
/// `docs/conformance/ferroehr/`. The manifest MUST NOT out-claim it.
pub const CONFORMANCE_PROFILE: &str = "STANDARD";

#[cfg(test)]
mod tests {
    use super::*;

    /// The derivation chain holds end to end: every pin is the owning
    /// crate's own authority — the `Generation` enum for the generated
    /// multi-generation crates (each variant agreeing with its generation
    /// module's `SPEC_VERSION`), the literal `SPEC_VERSION` for the
    /// hand-written ones.
    #[test]
    fn pins_are_the_crate_spec_versions() {
        assert_eq!(ITS_REST, openehr_its::SPEC_VERSION);
        assert_eq!(AQL, openehr_query::SPEC_VERSION);
        // The compile-time consts equal the DEFAULT (current) generation's
        // runtime pin — the pairing that keeps a composition-table `current`
        // flip from silently diverging the served identity surfaces.
        assert_eq!(RM, openehr_rm::Generation::default().spec_version());
        assert_eq!(BASE, openehr_base::Generation::default().spec_version());
        assert_eq!(TERM, openehr_term::Generation::default().spec_version());
        assert_eq!(AM14, openehr_am::Generation::V1_4.spec_version());
        assert_eq!(AM24, openehr_am::Generation::default().spec_version());
        // The released generations carry their own pins, selectable later
        // (#1943) — pinned here so a table edit is a visible identity change.
        assert_eq!(openehr_rm::Generation::V1_1.spec_version(), "1.1.0");
        assert_eq!(openehr_base::Generation::V1_2.spec_version(), "1.2.0");
    }
}
