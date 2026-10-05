// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whole-corpus coverage gate.
//!
//! Every ADL source file under `tests/corpus/**` (both `.adls` and `.adl`) must
//! be claimed by exactly one harness category — 100% coverage, no dead fixtures
//! (a HARD REQUIREMENT: every vendored corpus file is exercised). This gate is the
//! accounting layer: it walks the whole tree, categorises each file by its path,
//! and fails if any file is unclaimed (a new or moved fixture breaks CI). It
//! keys on the FULL PATH (two duplicate basenames exist — `tests/corpus/INVENTORY.md`
//! §1) and includes a `.adl` walker (the 31 ADL-1.4 files the `.adls`-only lexer/
//! parser harnesses do not reach).
//!
//! Each category names the harness that exercises the file (the oracle is the
//! in-file `regression` tag, never the filename — INVENTORY §2). The categories
//! mirror INVENTORY §10 "Harness-category assignment":
//!
//! | Category | Directory selector | Owning harness |
//! |---|---|---|
//! | `FlattenerSpecExamples` | `flattener/specexamples/**` | `flattener_spec.rs` |
//! | `FlattenerSiblingOrder` | `flattener/siblingorder/**` | `flattener_spec.rs` |
//! | `ValidityTemplates` | `validity/templates/*.adls` | `templates_corpus.rs` |
//! | `ValidityLegacy14Adls` | `validity/legacy_adl_1.4/*.adls` | `legacy14_corpus.rs` + basic integrity |
//! | `ValidityLegacy14Adl` | `validity/legacy_adl_1.4/*.adl` | `legacy14_corpus.rs` (1.4 tolerance) |
//! | `Validity` | `validity/**/*.adls` (rest) | `corpus_validity_{integrity,parent_conformance,rm}.rs` |
//! | `Robustness` | `robustness/**` | `corpus_validity_integrity.rs` (never-panic floor) |
//! | `Upgrade14Source` | `upgrade/upgrade_from_14/*.adl` | `adl14_conversion.rs` (convert) |
//! | `Upgrade14Target` | `upgrade/upgrade_from_14/*.adls` | `adl14_conversion.rs` (compare target) |
//! | `Upgrade15` | `upgrade/upgrade_from_15/*.adls` | `legacy14_corpus.rs` (parse+validate) |
//! | `FeaturesAdls` | `features/**/*.adls` | `corpus_{outer_parse,definition_parse,roundtrip}.rs` |
//! | `FeaturesAdl` | `features/**/*.adl` | `legacy14_corpus.rs` (1.4-tolerant parse) |
//! | `Adl14Dadl` | `adl14-dadl/*.adl` | `adl14_dadl_breadth.rs` (accept/refuse per fixture) |
//! | `Adl14Cadl` | `adl14-cadl/*.adl` | `adl14_cadl_gates.rs` (accept/refuse/invalid per fixture) |
//!
//! `adl14-dadl/` and `adl14-cadl/` are the HAND-WRITTEN trees here
//! (`tests/corpus/PROVENANCE.md`); they are counted separately so the
//! vendored-corpus size ratchet below stays a statement about the vendored trees
//! alone.

use std::path::{Path, PathBuf};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");

/// The harness category a corpus source file is claimed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Category {
    FlattenerSpecExamples,
    FlattenerSiblingOrder,
    ValidityTemplates,
    ValidityLegacy14Adls,
    ValidityLegacy14Adl,
    Validity,
    Robustness,
    Upgrade14Source,
    Upgrade14Target,
    Upgrade15,
    FeaturesAdls,
    FeaturesAdl,
    Adl14Dadl,
    Adl14Cadl,
}

/// Classify a corpus source file (relative to `tests/corpus/`) into exactly one
/// harness category, or `None` if it falls outside every known selector (an
/// unclaimed file — a gate failure).
fn classify(rel: &str, is_adl: bool) -> Option<Category> {
    // Flattener fixtures.
    if let Some(r) = rel.strip_prefix("flattener/") {
        if r.starts_with("specexamples/") {
            return Some(Category::FlattenerSpecExamples);
        }
        if r.starts_with("siblingorder/") {
            return Some(Category::FlattenerSiblingOrder);
        }
        return None;
    }
    // The hand-written ADL 1.4 trees.
    if rel.starts_with("adl14-dadl/") {
        return Some(Category::Adl14Dadl);
    }
    if rel.starts_with("adl14-cadl/") {
        return Some(Category::Adl14Cadl);
    }
    // adl2-reference fixtures.
    classify_reference(rel.strip_prefix("adl2-reference/")?, is_adl)
}

/// The `adl2-reference` sub-tree prefixes and the category each claims.
///
/// Order is load-bearing: the first matching prefix wins, so a narrower tree
/// (`validity/templates/`) precedes the tree that contains it (`validity/`). The
/// two categories are the ADL 1.4-source and ADL 2-source spellings; a tree that
/// does not distinguish them repeats one category.
const REFERENCE_CATEGORIES: &[(&str, Category, Category)] = &[
    (
        "validity/templates/",
        Category::ValidityTemplates,
        Category::ValidityTemplates,
    ),
    (
        "validity/legacy_adl_1.4/",
        Category::ValidityLegacy14Adl,
        Category::ValidityLegacy14Adls,
    ),
    ("validity/", Category::Validity, Category::Validity),
    ("robustness/", Category::Robustness, Category::Robustness),
    (
        "upgrade/upgrade_from_14/",
        Category::Upgrade14Source,
        Category::Upgrade14Target,
    ),
    (
        "upgrade/upgrade_from_15/",
        Category::Upgrade15,
        Category::Upgrade15,
    ),
    ("features/", Category::FeaturesAdl, Category::FeaturesAdls),
];

/// The category an `adl2-reference` path (prefix already stripped) is claimed by.
fn classify_reference(r: &str, is_adl: bool) -> Option<Category> {
    REFERENCE_CATEGORIES
        .iter()
        .find(|(prefix, _, _)| r.starts_with(prefix))
        .map(|(_, adl, adls)| if is_adl { *adl } else { *adls })
}

/// Every ADL source file (`.adls` / `.adl`) under `dir`, keyed on full path.
fn adl_sources(dir: &Path) -> Vec<PathBuf> {
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
            } else if p.extension().is_some_and(|e| e == "adls" || e == "adl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_corpus_source_is_claimed_by_exactly_one_harness() {
    // The ADL fixture trees (the `rm/` subtree is the vendored BMM reference-model
    // corpus consumed wholesale by `corpus_validity_rm.rs`, not per-file ADL
    // fixtures, so it is not walked here).
    let vendored_roots = [
        PathBuf::from(format!("{CORPUS}/adl2-reference")),
        PathBuf::from(format!("{CORPUS}/flattener")),
    ];
    let hand_written_roots = [
        PathBuf::from(format!("{CORPUS}/adl14-dadl")),
        PathBuf::from(format!("{CORPUS}/adl14-cadl")),
    ];

    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut unclaimed: Vec<String> = Vec::new();
    let mut total = 0usize;

    let mut hand_written = 0usize;
    for (root, vendored) in vendored_roots
        .iter()
        .map(|r| (r, true))
        .chain(hand_written_roots.iter().map(|r| (r, false)))
    {
        for path in adl_sources(root) {
            if vendored {
                total += 1;
            } else {
                hand_written += 1;
            }
            let rel = path
                .strip_prefix(CORPUS)
                .unwrap_or(&path)
                .to_string_lossy()
                .trim_start_matches('/')
                .to_string();
            let is_adl = path.extension().is_some_and(|e| e == "adl");
            match classify(&rel, is_adl) {
                Some(cat) => *counts.entry(format!("{cat:?}")).or_default() += 1,
                None => unclaimed.push(rel),
            }
        }
    }

    eprintln!(
        "corpus coverage: {total} vendored + {hand_written} hand-written ADL sources across {} categories",
        counts.len()
    );
    for (cat, n) in &counts {
        eprintln!("  {cat}: {n}");
    }

    assert!(
        unclaimed.is_empty(),
        "{} corpus source file(s) are not claimed by any harness category (INVENTORY §10):\n{}",
        unclaimed.len(),
        unclaimed.join("\n")
    );

    // Ratchet on the vendored corpus size (INVENTORY §1: 302 adl2-reference + 38
    // flattener ADL sources). A change here means the corpus was re-vendored —
    // re-derive INVENTORY.md and update this expected total in the same change.
    assert_eq!(
        total, 340,
        "expected 340 vendored ADL source files (302 adl2-reference + 38 flattener); found {total}"
    );
    // The hand-written tree only ratchets up: a fixture is added, never
    // removed to go green (`.claude/rules/testing.md`).
    assert!(
        hand_written >= 20,
        "expected at least the 20 hand-written adl14-dadl + adl14-cadl fixtures; found {hand_written}"
    );
}
