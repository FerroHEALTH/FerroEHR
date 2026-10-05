// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
//! VETDF external-terminology resolver-seam tests
//! (AM ADL2 `master03-archetype_package.adoc` §Validity Rules).
//!
//! `openehr-adl` stays network-free: the VETDF check consults an injected
//! [`TerminologyResolver`] rather than holding a live terminology client. These
//! tests drive the public `validate_source` entry point with a stub resolver
//! and assert the raise contract — `Some(false)` → VETDF; `Some(true)` / `None`
//! / [`NoTerminologyResolver`] → no VETDF — and that the archetype-internal
//! (`local`/`openehr`) bindings are never consulted (their keys are covered by
//! VTTBK/VTCBK, `master07` §Validity Rules).

use std::sync::Mutex;

use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::{
    NoTerminologyResolver, TerminologyResolver, external_term_bindings,
};
use openehr_adl::validate::catalogue::ValidationCode;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;

/// The external terminology id + target the fixture binds its root node to.
const SNOMED_ID: &str = "SNOMED-CT";
const SNOMED_TARGET: &str = "http://snomedct.info/id/271649006";

/// A stub resolver: answers `answer` for every query, and records the
/// `(terminology_id, code)` pairs it was consulted about.
struct StubResolver {
    answer: Option<bool>,
    asked: Mutex<Vec<(String, String)>>,
}

impl StubResolver {
    fn new(answer: Option<bool>) -> Self {
        Self {
            answer,
            asked: Mutex::new(Vec::new()),
        }
    }
}

impl TerminologyResolver for StubResolver {
    fn code_exists(&self, terminology_id: &str, code: &str) -> Option<bool> {
        self.asked
            .lock()
            .unwrap()
            .push((terminology_id.to_owned(), code.to_owned()));
        self.answer
    }
}

/// A minimal, parseable ADL2 archetype binding its root node `id1` to both an
/// external SNOMED CT target and an archetype-internal `openehr` target.
fn fixture() -> String {
    "\
archetype (adl_version=2.0.6; rm_release=1.1.0)
    openEHR-EHR-OBSERVATION.vetdf_seam_test.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
        >
    >

definition
    OBSERVATION[id1] matches { *}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>
        >
    >
    term_bindings = <
        [\"SNOMED-CT\"] = <
            [\"id1\"] = <http://snomedct.info/id/271649006>
        >
        [\"openehr\"] = <
            [\"id1\"] = <http://openehr.org/internal>
        >
    >
"
    .to_owned()
}

/// True if `validate_source` (with `resolver`) raises VETDF on the fixture.
fn has_vetdf(resolver: &dyn TerminologyResolver) -> bool {
    let issues =
        validate_source(&fixture(), None, &ProductionRmModel, resolver).expect("fixture parses");
    issues.iter().any(|i| i.code == ValidationCode::Vetdf)
}

#[test]
fn absent_external_code_raises_vetdf() {
    // Some(false) — the code is definitely absent from the terminology → VETDF.
    assert!(has_vetdf(&StubResolver::new(Some(false))));
}

#[test]
fn present_external_code_does_not_raise_vetdf() {
    // Some(true) — the code exists → no VETDF.
    assert!(!has_vetdf(&StubResolver::new(Some(true))));
}

#[test]
fn unverifiable_external_code_does_not_raise_vetdf() {
    // None — the resolver could not answer (tool inaccessible) → no VETDF
    // (`master03` §Validity Rules "no verification was possible").
    assert!(!has_vetdf(&StubResolver::new(None)));
}

#[test]
fn default_resolver_never_raises_vetdf() {
    // The default always answers None, so behaviour is unchanged from before
    // the seam existed.
    assert!(!has_vetdf(&NoTerminologyResolver));
}

#[test]
fn only_external_bindings_are_consulted() {
    let stub = StubResolver::new(Some(true));
    let _issues =
        validate_source(&fixture(), None, &ProductionRmModel, &stub).expect("fixture parses");
    let asked = stub.asked.lock().unwrap();
    // The external SNOMED CT binding is consulted exactly as authored.
    assert!(
        asked.contains(&(SNOMED_ID.to_owned(), SNOMED_TARGET.to_owned())),
        "external binding must reach the resolver: {asked:?}"
    );
    // The archetype-internal `openehr`/`local` bindings are never consulted.
    assert!(
        asked
            .iter()
            .all(|(tid, _)| tid != "openehr" && tid != "local"),
        "internal bindings must not reach the resolver: {asked:?}"
    );
}

#[test]
fn external_term_bindings_excludes_internal_terminologies() {
    let archetype = parse_artefact(&fixture(), Dialect::Adl2).expect("fixture parses");
    let bindings = external_term_bindings(&archetype);
    assert_eq!(bindings.len(), 1, "only the SNOMED CT binding is external");
    assert_eq!(bindings[0].terminology_id, SNOMED_ID);
    assert_eq!(bindings[0].target, SNOMED_TARGET);
    assert_eq!(bindings[0].key, "id1");
}
