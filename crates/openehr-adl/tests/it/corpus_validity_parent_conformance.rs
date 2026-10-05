// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-2 specialisation validation corpus harness.
//!
//! Walks the `validity/specialisation/**` corpus (plus the four slot-
//! specialisation fixtures under `validity/slots/`) and asserts the phase-2
//! validator's behaviour against each file's authoritative `regression` tag
//! (the oracle per `tests/corpus/INVENTORY.md`, never the filename).
//!
//! The full gated pipeline (`validate_source`: phase 1 → RM → phase-2
//! specialisation) runs with a repository built over the whole corpus so the
//! flat parent and external references resolve. A level-0 parent is its own flat
//! form (`ADL2/master09.02` §Differential and Flat Forms); a specialised parent
//! needs the flattener and is adjudicated ([`FlatParent::NeedsFlattener`]).
//!
//! Spec oracle for the codes:
//! `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules + `master08-validation.adoc` §Phase 2.

// A test harness: vendored-fixture reads/parses are asserted to succeed.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::artefact::{ArchetypeRepository, FlatParent, resolve_flat_parent};
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/adl2-reference");

/// The phase-2 (and gated phase-1) codes this harness asserts a tag names is
/// raised on its fixture.
const PARENT_CONFORMANCE_FIRING: &[&str] = &[
    "VSANCE", "VSANCC", "VSONIN", "VSSM", "VDIFP", "VCORMT", "VARXID", "VARXS", "VARXR", "VDSSID",
    "VSONCO", "VPOV", // gated phase-1 codes reached through the same pipeline:
    "VACSD", "VTSD",
];

/// The extra slot-specialisation fixtures claimed from `validity/slots/`.
const SLOT_FIXTURES: &[&str] = &[
    "validity/slots/openEHR-EHR-SECTION.VARXID_filler_id_not_valid.v1.0.0.adls",
    "validity/slots/openEHR-EHR-SECTION.VARXS_slot_id_mismatch.v1.0.0.adls",
    "validity/slots/openEHR-EHR-SECTION.VARXR_slot_id_match_but_not_found.v1.0.0.adls",
    "validity/slots/openEHR-EHR-SECTION.VDSSID_slot_redefine_bad_id.v1.0.0.adls",
];

fn normalise_tag(tag: &str) -> String {
    match tag {
        "VDIFP1" => "VDIFP".to_owned(),
        "VSONCOm" => "VSONCO".to_owned(),
        other => other.to_owned(),
    }
}

fn read_tag_raw(src: &str) -> Option<String> {
    let idx = src.find("regression")?;
    let rest = src.get(idx..)?;
    let open = rest.find("<\"")? + 2;
    let after = rest.get(open..)?;
    let end = after.find('"')?;
    after.get(..end).map(str::to_owned)
}

fn adls_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "adls") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn build_repository() -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    for path in adls_files(Path::new(CORPUS)) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(art) = parse_artefact(&src, Dialect::Adl2) {
            repo.insert(art);
        }
    }
    repo
}

#[derive(Default)]
struct Counts {
    exact_code: usize,
    pass_clean: usize,
    parent_not_found: usize,
    non_specialised: usize,
}

/// The upstream tuple-narrowing pair under `features/specialisation` is
/// PASS-tagged, so the second-order tuple conformance functions (`master04.5`
/// §`C_SECOND_ORDER`) must accept both: dropping a tuple row and narrowing the
/// surviving rows are the sanctioned redefinitions (`ADL2/master09.05` §Tuple
/// Redefinition). The corpus carries no VTPNC/VTPIN fixture of its own
/// (`tests/corpus/INVENTORY.md`); the refusals are pinned by the unit twins in
/// `validate::specialisation`.
#[test]
fn corpus_tuple_narrowing_raises_no_tuple_code() {
    let repo = build_repository();
    for name in [
        "features/specialisation/openEHR-EHR-OBSERVATION.tuple_redefine_to_single.v1.0.0.adls",
        "features/specialisation/openEHR-EHR-OBSERVATION.tuple_redefine_to_narrower.v1.0.0.adls",
    ] {
        let src = std::fs::read_to_string(format!("{CORPUS}/{name}")).expect("fixture is readable");
        let issues = validate_source(
            &src,
            Some(&repo),
            &ProductionRmModel,
            &NoTerminologyResolver,
        )
        .expect("fixture parses");
        let raised: Vec<&str> = issues
            .iter()
            .map(|i| i.code.mnemonic())
            .filter(|c| *c == "VTPNC" || *c == "VTPIN")
            .collect();
        assert!(raised.is_empty(), "{name}: raised {raised:?}");
    }
    // Non-vacuity: widening the units member of the narrowed row to one no
    // parent row admits must be refused.
    let widened = std::fs::read_to_string(format!(
        "{CORPUS}/features/specialisation/openEHR-EHR-OBSERVATION.tuple_redefine_to_single.v1.0.0.adls"
    ))
    .expect("fixture is readable")
    .replace("{\"cm[H20]\"}", "{\"kPa\"}");
    let issues = validate_source(
        &widened,
        Some(&repo),
        &ProductionRmModel,
        &NoTerminologyResolver,
    )
    .expect("fixture parses");
    let raised: Vec<&str> = issues.iter().map(|i| i.code.mnemonic()).collect();
    assert!(raised.contains(&"VTPNC"), "widened row: raised {raised:?}");
}

/// The phase-2 verdict for one specialisation fixture.
///
/// A `Violation` carries the message tail; the caller prefixes the file name.
enum ParentOutcome {
    /// The fixture's tagged code was raised exactly.
    ExactCode,
    /// A `PASS`/untagged fixture raised no error.
    PassClean,
    /// The parent is deliberately absent and the typed outcome says so.
    ParentNotFound,
    /// A level-0 support archetype, not a phase-2 subject.
    NonSpecialised,
    /// The fixture's outcome contradicts its tag.
    Violation(String),
}

/// Judges one specialisation fixture against its authoritative `regression` tag.
fn judge_parent_fixture(
    src: &str,
    tag: Option<&str>,
    repo: &ArchetypeRepository,
    rm: ProductionRmModel,
) -> ParentOutcome {
    let Ok(archetype) = parse_artefact(src, Dialect::Adl2) else {
        // A non-parsing fixture is owned by the parse gates; a FAIL tag is a
        // valid outcome here.
        if tag == Some("FAIL") {
            return ParentOutcome::ParentNotFound;
        }
        return ParentOutcome::Violation(format!("failed to parse (tag {tag:?})"));
    };

    // FAIL fixtures whose parent is deliberately absent assert the typed
    // parent-not-found outcome (INVENTORY §5).
    if tag == Some("FAIL") {
        return match resolve_flat_parent(&archetype, repo) {
            FlatParent::NotFound => ParentOutcome::ParentNotFound,
            other => ParentOutcome::Violation(format!(
                "FAIL fixture expected parent-not-found, got {other:?}"
            )),
        };
    }

    // Non-specialised support archetypes (the level-0 parents that live in this
    // directory) are not phase-2 subjects — their RM/phase-1 validation is the
    // reference-model / phase-1 harnesses' concern (and some exercise an
    // emit-rm-model gap, e.g. spec_test_obs3's DV_QUANTITY attributes).
    if matches!(
        resolve_flat_parent(&archetype, repo),
        FlatParent::NotSpecialised
    ) {
        return ParentOutcome::NonSpecialised;
    }

    let Ok(issues) = validate_source(src, Some(repo), &rm, &NoTerminologyResolver) else {
        return ParentOutcome::Violation("validate_source parse error".to_owned());
    };
    let error_codes: Vec<String> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code.mnemonic().to_owned())
        .collect();
    judge_parent_codes(tag, &error_codes)
}

/// The verdict for a fixture that reached phase-2 validation.
fn judge_parent_codes(tag: Option<&str>, error_codes: &[String]) -> ParentOutcome {
    match tag {
        None | Some("PASS") => {
            if error_codes.is_empty() {
                ParentOutcome::PassClean
            } else {
                ParentOutcome::Violation(format!("PASS/untagged but raised {error_codes:?}"))
            }
        }
        Some(t) if PARENT_CONFORMANCE_FIRING.contains(&t) => {
            if error_codes.iter().any(|c| c == t) {
                ParentOutcome::ExactCode
            } else {
                ParentOutcome::Violation(format!("expected {t} but raised {error_codes:?}"))
            }
        }
        Some(t) => ParentOutcome::Violation(format!("unhandled tag {t} (raised {error_codes:?})")),
    }
}

#[test]
fn corpus_parent_conformance_outcomes() {
    let repo = build_repository();
    let rm = ProductionRmModel;
    let mut counts = Counts::default();
    let mut violations: Vec<String> = Vec::new();

    let mut paths = adls_files(&PathBuf::from(format!("{CORPUS}/validity/specialisation")));
    for extra in SLOT_FIXTURES {
        paths.push(PathBuf::from(format!("{CORPUS}/{extra}")));
    }
    paths.sort();

    for path in &paths {
        let name = path
            .strip_prefix(CORPUS)
            .unwrap_or(path)
            .display()
            .to_string();
        let Ok(src) = std::fs::read_to_string(path) else {
            violations.push(format!("{name}: unreadable"));
            continue;
        };
        let tag = read_tag_raw(&src).map(|t| normalise_tag(&t));

        match judge_parent_fixture(&src, tag.as_deref(), &repo, rm) {
            ParentOutcome::ExactCode => counts.exact_code += 1,
            ParentOutcome::PassClean => counts.pass_clean += 1,
            ParentOutcome::ParentNotFound => counts.parent_not_found += 1,
            ParentOutcome::NonSpecialised => counts.non_specialised += 1,
            ParentOutcome::Violation(message) => violations.push(format!("{name}: {message}")),
        }
    }

    eprintln!(
        "phase-2 corpus: exact={} pass_clean={} parent_not_found={} non_specialised={} ({} files)",
        counts.exact_code,
        counts.pass_clean,
        counts.parent_not_found,
        counts.non_specialised,
        paths.len(),
    );

    assert!(
        violations.is_empty(),
        "phase-2 corpus violations ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}
