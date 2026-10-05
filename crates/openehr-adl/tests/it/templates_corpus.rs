// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `validity/templates` corpus harness — VTPL + VARXR filler validation.
//!
//! Oracle: `docs/specs/openehr/AM/docs/AOM2/master03-archetype_package.adoc`
//! §Validity Rules (VTPL — template/filler language consistency) and
//! `master08-validation.adoc` §Phase 2 (VARXR — external reference resolution),
//! resolved against the flattened fillers by
//! [`openehr_adl::validate::slots::validate_fillers`].
//!
//! Every file under `tests/corpus/adl2-reference/validity/templates` is claimed
//! here with its expected outcome (the in-file `regression` tag is the oracle,
//! INVENTORY.md):
//!
//! * `template_fail_VTPL` (tag `VTPL`) — its parent's filler is a mono-lingual
//!   (`de`-only) archetype, so the `en` template cannot flatten a common
//!   language → VTPL fires.
//! * `template_pass_VTPL` (tag `PASS`) — its filler carries `de` + `en` → clean.
//! * `t_non_existent_ext_ref` (tag `VARXR`) — a lexically-legal reference to a
//!   non-existent archetype → VARXR fires.
//! * the support archetypes (`good_include`, `bad_include`, `de_en_lang_arch`,
//!   `de_lang_arch`) — parse-only inputs; a plain archetype is exempt from VTPL
//!   (only a template's fillers must share its language), so they raise no
//!   filler issue.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::meta::regression_tag;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::slots::validate_fillers;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;

const TEMPLATES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/corpus/adl2-reference/validity/templates"
);

fn parse(path: &Path) -> Archetype {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse_artefact(&src, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("parse {}: {e:?}", path.display()))
}

/// A repository over the whole templates directory, so fillers referenced by the
/// parents (`good_include`/`bad_include`) and by the templates resolve.
fn templates_repo() -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    for entry in std::fs::read_dir(TEMPLATES).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "adls") {
            repo.insert(parse(&p));
        }
    }
    repo
}

fn error_codes(issues: &[openehr_adl::validate::ValidationIssue]) -> Vec<ValidationCode> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect()
}

/// Walk every `.adls` file in `validity/templates`, resolve its `regression`
/// tag, and assert the filler validator produces exactly the tagged outcome.
#[test]
fn templates_corpus_filler_outcomes() {
    let repo = templates_repo();
    let mut checked = 0;
    let mut violations = Vec::new();

    for entry in std::fs::read_dir(TEMPLATES).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_none_or(|e| e != "adls") {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let art = parse(&p);
        let tag = regression_tag(&art);
        let issues = validate_fillers(&art, &repo);
        let codes = error_codes(&issues);
        checked += 1;

        match tag.as_deref() {
            Some("VTPL") => {
                if !codes.contains(&ValidationCode::Vtpl) {
                    violations.push(format!("{name}: expected VTPL, got {codes:?}"));
                }
            }
            Some("VARXR") => {
                if !codes.contains(&ValidationCode::Varxr) {
                    violations.push(format!("{name}: expected VARXR, got {codes:?}"));
                }
            }
            // PASS-tagged and untagged support archetypes must raise no filler
            // error (a plain archetype is exempt from VTPL; its fillers resolve).
            Some("PASS") | None => {
                if !codes.is_empty() {
                    violations.push(format!("{name}: expected clean, got {codes:?}"));
                }
            }
            other => violations.push(format!("{name}: unexpected tag {other:?}")),
        }
    }

    assert!(
        violations.is_empty(),
        "filler-validation violations:\n{}",
        violations.join("\n")
    );
    assert_eq!(checked, 7, "all seven validity/templates files claimed");
}

/// Spot-check the two VTPL fixtures directly (the language-consistency rule).
#[test]
fn vtpl_fires_on_monolingual_filler_only() {
    let repo = templates_repo();

    let fail = parse(&PathBuf::from(format!(
        "{TEMPLATES}/openehr-TASK_PLANNING-TASK_PLAN.template_fail_VTPL.v0.0.1.adls"
    )));
    let fail_codes = error_codes(&validate_fillers(&fail, &repo));
    assert!(
        fail_codes.contains(&ValidationCode::Vtpl),
        "the en template with a de-only filler raises VTPL: {fail_codes:?}"
    );

    let pass = parse(&PathBuf::from(format!(
        "{TEMPLATES}/openehr-TASK_PLANNING-TASK_PLAN.template_pass_VTPL.v0.0.1.adls"
    )));
    let pass_codes = error_codes(&validate_fillers(&pass, &repo));
    assert!(
        pass_codes.is_empty(),
        "the en template with a de+en filler is clean: {pass_codes:?}"
    );
}
