// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A minimal spec-valid ADL2 source builder shared by the suites that need a
//! stored ADL2/OPT2 family (the `I_DEFINITION_ADL2` catalog tests and the AQL
//! archetype-lineage test).
//!
//! Helper module, not a test module (`.claude/rules/testing.md` §Where tests
//! live).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this shared \
              fixture helper; a malformed fixture argument must fail loudly"
)]

/// Build a minimal spec-valid ADL2 source: header, HRID, optional
/// `specialize`, `language`, `definition` (root node `id1`, or `id1.1` when
/// specialised — AOM2 master08 VARCN), `terminology` (ADL2 master02
/// §Structure — the registration validator enforces STCNT + the
/// terminology-side AOM2 rules).
pub(crate) fn adl2_source(keyword: &str, hrid: &str, specialize: Option<&str>) -> String {
    let rm_type = hrid
        .split('.')
        .next()
        .and_then(|q| q.rsplit_once('-').map(|(_, e)| e))
        .expect("HRID carries an RM entity");
    let root = if specialize.is_some() { "id1.1" } else { "id1" };
    let spec = specialize.map_or(String::new(), |p| format!("\nspecialize\n    {p}\n"));
    // A `description` section is mandatory (AOM2 master03 §Validity Rules VARD:
    // "A `description` section containing the main meta-data of the archetype
    // must exist").
    format!(
        "{keyword} (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n{spec}\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    {rm_type}[{root}] matches {{ *}}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"{root}\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n"
    )
}
