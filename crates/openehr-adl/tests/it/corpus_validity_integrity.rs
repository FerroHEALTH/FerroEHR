// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-1 validation corpus harness.
//!
//! Walks the vendored ADL2 conformance corpus (`tests/corpus/adl2-reference/`)
//! and asserts the phase-1 validator's behaviour against each file's
//! authoritative `regression` tag (the oracle per
//! `tests/corpus/INVENTORY.md`, never the filename):
//!
//! - a tag naming a **phase-1** code ⇒ the validator raises exactly that code;
//! - a `PASS`/untagged-features file ⇒ zero phase-1 errors (warnings allowed);
//! - a tag naming a **phase-2/3/RM or not-yet-run phase-1** code ⇒ no phase-1
//!   error false-positive (recorded as not-yet-checked);
//! - `FAIL` / `S*` (syntax) tags stay claimed by the parse gates — asserted
//!   only to reject (parse error or any typed error).
//!
//! Tag normalisations (INVENTORY §3/§10): `VDIFP1`→VDIFP, `VSONCOm`→VSONCO.
//! The spec oracle for the codes is `docs/specs/openehr/AM/docs/AOM2/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::{ValidationIssue, validate_source_integrity};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/adl2-reference");

/// The phase-1 codes the validator actively raises. A tag in this set must be
/// raised exactly; a tag outside it (but still a V/W code) is not asserted here.
const INTEGRITY_FIRING: &[&str] = &[
    "VARDT", "VARCN", "STCNT", "VACSD", "VOLT", "VARAV", "VARRV", "VOTM", "VDIFV", "VATCV", "VTSD",
    "VTLC", "VTTBK", "VTCBK", "VTVSID", "VTVSMD", "VTVSUQ", "VDSEV", "VDSIV", "VARXNC", "VARXAV",
    "VARXTV", "VATID", "VATCD", "VATDF", "VACDF", "VATDA", "VRANP", "VOKU", "VARID", "VDEOL",
    "VARD", "VASID", "VALC", "VRRLP", "VCOID", "VCOSU", "VCATU", "VDFAI", "VOBAV", "VRMVP",
    "VRMVAV", "VACMCU", "WACMCL", "VRDLA",
    "WOUC",
    // VACSO is a reference-model check (its single-valued determination needs
    // the RM `is_multiple`, not the parser's cardinality heuristic); it is
    // asserted in the reference-model corpus harness (`corpus_validity_rm.rs`).
];

/// Documented adjudications — files skipped with a spec-cited reason (never a
/// silent exclusion). The `regression` tag names a phase-1 code whose check is
/// genuinely not checked in phase 1.
fn adjudicated_skip(name: &str) -> Option<&'static str> {
    if name.ends_with("VATID_id_code_in_node_not_in_terminology.v1.0.0.adls") {
        // The per-node id-code definedness half of VATID depends on the RM
        // multiplicity of the owning attribute (master07 §Overview: a term
        // definition is optional for children of single-valued attributes),
        // which needs the reference model; phase 1 checks only the root concept
        // code. This interior-node case is asserted in the reference-model
        // corpus harness (`corpus_validity_rm.rs`).
        Some("VATID interior-node definedness is a reference-model check (corpus_validity_rm.rs)")
    } else if name.ends_with("ENTRY_WRONG.rm_type_wrong.v1.0.0.adls") {
        // Definition root type `ENTRY` != identifier RM class `ENTRY_WRONG`, so
        // VARDT (master03 §Validity Rules L238) fires; the corpus tags it PASS —
        // a documented tag/spec inconsistency (INVENTORY §3). Adjudicated in
        // both harnesses rather than weakening VARDT.
        Some("VARDT fires (ENTRY != ENTRY_WRONG, master03 L238); corpus PASS tag inconsistent")
    } else if name.ends_with("VOKU_ac_code_duplicated_in_terminology.v1.0.0.adls")
        || name.ends_with("VOKU_at_code_duplicated_in_terminology.v1.0.0.adls")
    {
        // Duplicate sibling container keys are invalid ODIN — rule VDOBU
        // (`LANG/docs/odin/master05-content` §Container Objects), enforced at
        // the ODIN parse since #1376 — so the terminology section is refused
        // (SDINV) before the AOM-level VOKU check these tags name can run.
        // Through ADL2 TEXT a terminology key duplicate is therefore
        // structurally unreachable past the parser; the VOKU source-level
        // check stays for programmatically-built sources and its own unit
        // coverage.
        Some(
            "duplicate ODIN container keys refused at parse (VDOBU, master05) before VOKU is reachable",
        )
    } else if name.ends_with("FAIL_dadl_spurious_delimiter.v1.0.0.adls") {
        // The file carries a spurious ODIN delimiter that the ADL2 lexer/parser
        // rejects at parse time (SDINV, `ADL2/master04.6`), before the VOTM
        // semantic check the tag names can run. The stricter-parse vs
        // lenient-parse gap is a parser concern, not phase-1 validation.
        Some("spurious ODIN delimiter rejected at parse (SDINV) before VOTM is reachable")
    } else if name.ends_with("SOME_TYPE.code_phrase.v1.0.0.adls") {
        // A legacy ADL 1.4 source (validity/legacy_adl_1.4) that reuses id-code
        // `id2` for the CODE_PHRASE node under several non-sibling
        // `defining_code` attributes. In AOM 1.4 node ids are only
        // *sibling*-unique (AOM1.4 master04 §Node_id and Paths), so the reuse is
        // 1.4-legal; parsed as ADL2 the stricter archetype-wide VCOSU
        // (master04.5 §C_OBJECT) flags it. The ADL2 rule is NOT weakened — this
        // is a spec-cited tolerance for a 1.4-origin fixture.
        Some(
            "legacy 1.4 sibling-unique node-id reuse (AOM1.4 master04); ADL2 archetype-wide VCOSU not weakened to pass it",
        )
    } else {
        None
    }
}

fn normalise_tag(tag: &str) -> String {
    match tag {
        "VDIFP1" => "VDIFP".to_owned(),
        "VSONCOm" => "VSONCO".to_owned(),
        other => other.to_owned(),
    }
}

fn is_syntax_tag(tag: &str) -> bool {
    // S-codes: an `S` followed by uppercase letters/digits (the syntax
    // catalogue, ADL2 master04.6). Distinguished from `STCNT` which, despite
    // the `S` prefix, is a phase-1 semantic code in our catalogue.
    tag != "STCNT" && tag.starts_with('S') && tag.len() >= 4 && tag != "PASS"
}

/// Read the `regression` tag from raw source (fallback when the file does not
/// parse, so a tag is still available).
fn read_tag_raw(src: &str) -> Option<String> {
    let (_, rest) = src.split_once("regression")?;
    let (_, after) = rest.split_once("<\"")?;
    let (value, _) = after.split_once('"')?;
    Some(value.to_owned())
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

/// Build a repository over every parseable `.adls` in the corpus (for parent
/// resolution — VACSD/VASID/VALC).
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
    deferred: usize,
    syntax_or_fail: usize,
}

/// What one corpus file's `regression` tag turned out to mean.
///
/// A `Violation` carries the message tail; the caller prefixes the file name.
enum TagOutcome {
    ExactCode,
    PassClean,
    Deferred(String),
    SyntaxOrFail,
    Violation(String),
}

/// A `PASS`/untagged file: clean iff phase 1 raised no error (warnings allowed).
fn judge_clean(error_codes: &[String]) -> TagOutcome {
    if error_codes.is_empty() {
        TagOutcome::PassClean
    } else {
        TagOutcome::Violation(format!("PASS/untagged but raised {error_codes:?}"))
    }
}

/// A `FAIL`-tagged file that nonetheless parses: expect at least one typed error.
fn judge_fail(error_codes: &[String]) -> TagOutcome {
    if error_codes.is_empty() {
        TagOutcome::Violation("FAIL-tagged but no phase-1 error raised".to_owned())
    } else {
        TagOutcome::SyntaxOrFail
    }
}

/// A tag naming a code the validator actively raises: it must be raised exactly.
fn judge_firing_code(tag: &str, error_codes: &[String], issues: &[ValidationIssue]) -> TagOutcome {
    if error_codes.iter().any(|c| c == tag) || issues.iter().any(|i| i.code.mnemonic() == tag) {
        TagOutcome::ExactCode
    } else {
        TagOutcome::Violation(format!("expected {tag} but raised {error_codes:?}"))
    }
}

/// A phase-2/3/RM code, or a not-yet-run phase-1 code: assert no false positive.
fn judge_deferred_code(tag: &str, error_codes: &[String]) -> TagOutcome {
    if error_codes.is_empty() {
        TagOutcome::Deferred(tag.to_owned())
    } else {
        TagOutcome::Violation(format!(
            "deferred tag {tag} but phase-1 raised {error_codes:?}"
        ))
    }
}

/// Classifies one parsed corpus file against its authoritative `regression` tag.
fn judge_tagged_outcome(
    tag: Option<&str>,
    error_codes: &[String],
    issues: &[ValidationIssue],
) -> TagOutcome {
    match tag {
        None | Some("PASS") => judge_clean(error_codes),
        Some("FAIL") => judge_fail(error_codes),
        // A syntax-tagged file that nonetheless parsed — the syntax defect is
        // milder than a hard parse error; accept any typed error or none
        // (owned by the parse gates).
        Some(t) if is_syntax_tag(t) => TagOutcome::SyntaxOrFail,
        Some(t) if INTEGRITY_FIRING.contains(&t) => judge_firing_code(t, error_codes, issues),
        Some(t) => judge_deferred_code(t, error_codes),
    }
}

#[test]
fn corpus_integrity_outcomes() {
    let repo = build_repository();
    let mut counts = Counts::default();
    let mut violations: Vec<String> = Vec::new();
    let mut deferred_by_code: BTreeMap<String, usize> = BTreeMap::new();

    let mut roots = vec![
        PathBuf::from(format!("{CORPUS}/validity")),
        PathBuf::from(format!("{CORPUS}/robustness")),
    ];
    roots.retain(|p| p.exists());

    for root in &roots {
        for path in adls_files(root) {
            let name = path
                .strip_prefix(CORPUS)
                .unwrap_or(&path)
                .display()
                .to_string();
            if adjudicated_skip(&name).is_some() {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };

            let parsed = parse_artefact(&src, Dialect::Adl2);
            let raw_tag = read_tag_raw(&src);

            // ── files that do not parse: claimed by the parse gates ──────────
            let Ok(_archetype) = parsed else {
                match raw_tag.as_deref() {
                    Some(t) if is_syntax_tag(t) || t == "FAIL" => counts.syntax_or_fail += 1,
                    other => violations.push(format!(
                        "{name}: failed to parse but tag is {other:?} (expected a syntax/FAIL tag)"
                    )),
                }
                continue;
            };

            let tag = raw_tag.map(|t| normalise_tag(&t));
            let Ok(issues) = validate_source_integrity(&src, Dialect::Adl2, Some(&repo)) else {
                // Re-parse succeeded above; a source-parse failure here is a
                // harness inconsistency.
                violations.push(format!("{name}: validate_source_integrity parse error"));
                continue;
            };
            let error_codes: Vec<String> = issues
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .map(|i| i.code.mnemonic().to_owned())
                .collect();

            match judge_tagged_outcome(tag.as_deref(), &error_codes, &issues) {
                TagOutcome::ExactCode => counts.exact_code += 1,
                TagOutcome::PassClean => counts.pass_clean += 1,
                TagOutcome::SyntaxOrFail => counts.syntax_or_fail += 1,
                TagOutcome::Deferred(code) => {
                    counts.deferred += 1;
                    *deferred_by_code.entry(code).or_default() += 1;
                }
                TagOutcome::Violation(message) => violations.push(format!("{name}: {message}")),
            }
        }
    }

    eprintln!(
        "phase-1 corpus: exact={} pass_clean={} deferred={} syntax_or_fail={}",
        counts.exact_code, counts.pass_clean, counts.deferred, counts.syntax_or_fail
    );
    eprintln!("deferred-by-code: {deferred_by_code:?}");

    assert!(
        violations.is_empty(),
        "phase-1 corpus violations ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}
