// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A template whose constituent archetype the repository does not hold.
//!
//! Oracle: AOM2 VARXR (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules: `C_ARCHETYPE_ROOT`: "the archetype reference must refer to
//! an artefact that can be found in the current repository") and OPT2
//! `master03-opt_raw.adoc` §Archetype References. The template is the
//! vendored `t_encounter_opt_test`, which fills `content` with
//! `openEHR-EHR-OBSERVATION.apgar.v1` beside its parent `encounter.v1`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::{OptError, create_opt, unresolved_constituents};
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;

const APGAR: &str = "openEHR-EHR-OBSERVATION.apgar.v1";

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/archetypes/adl2/ckm-2013-12-09")
}

fn read(relative: &str) -> String {
    let path = pack().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse: {e:?}"))
}

fn template() -> String {
    read("composition/openEHR-EHR-COMPOSITION.t_encounter_opt_test.v1.0.0.adls")
}

/// The repository with the template's parent, and with the Apgar filler when
/// `with_apgar`.
fn repository(with_apgar: bool) -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse(&read(
        "composition/openEHR-EHR-COMPOSITION.encounter.v1.0.0.adls",
    )));
    if with_apgar {
        repo.insert(parse(&read(
            "entry/observation/openEHR-EHR-OBSERVATION.apgar.v1.0.0.adls",
        )));
    }
    repo
}

fn varxr(issues: &[openehr_adl::validate::ValidationIssue]) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error && i.code == ValidationCode::Varxr)
        .map(|i| i.message.clone())
        .collect()
}

#[test]
fn the_upload_schedule_refuses_a_template_whose_filler_is_missing() {
    let repo = repository(false);
    let issues = validate_source(
        &template(),
        Some(&repo),
        &ProductionRmModel,
        &NoTerminologyResolver,
    )
    .expect("the template parses");
    let refused = varxr(&issues);
    assert_eq!(
        refused.len(),
        1,
        "one VARXR for the one missing filler: {issues:?}"
    );
    assert!(
        refused.iter().all(|m| m.contains(APGAR)),
        "the VARXR names the missing archetype: {refused:?}"
    );
}

#[test]
fn the_upload_schedule_raises_no_varxr_once_the_filler_is_held() {
    let repo = repository(true);
    let issues = validate_source(
        &template(),
        Some(&repo),
        &ProductionRmModel,
        &NoTerminologyResolver,
    )
    .expect("the template parses");
    assert_eq!(varxr(&issues), Vec::<String>::new(), "no VARXR: {issues:?}");
}

#[test]
fn every_missing_constituent_is_named_where_create_opt_stops_at_one() {
    let root = parse(&template());
    let repo = repository(false);
    assert_eq!(
        create_opt(&root, &repo).expect_err("the OPT cannot be compiled"),
        OptError::UnresolvedReference(APGAR.to_owned())
    );
    assert_eq!(
        unresolved_constituents(&root, &repo),
        vec![APGAR.to_owned()]
    );
    assert_eq!(
        unresolved_constituents(&root, &repository(true)),
        Vec::<String>::new()
    );
}

#[test]
fn a_missing_specialisation_parent_is_named_too() {
    let root = parse(&template());
    let repo = ArchetypeRepository::new();
    assert_eq!(
        unresolved_constituents(&root, &repo),
        vec![
            "openEHR-EHR-COMPOSITION.encounter.v1.0.0".to_owned(),
            APGAR.to_owned()
        ]
    );
}
