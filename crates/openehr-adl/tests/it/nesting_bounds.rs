// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The engine's nesting bound as a typed refusal at every recursive seam:
//! the cADL parser, the flattener (lineage length and composed depth) and
//! the OPT transform (inlined depth and filler cycles). Native recursion past
//! a thread's stack aborts the process, which no caller can catch; each of
//! these walks refuses instead.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxErrorCode;
use openehr_adl::flatten::{FlattenError, flat_form};
use openehr_adl::opt::{OptError, create_opt};
use openehr_adl::parse::{Dialect, parse_definition_body};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_lang::nesting::MAX_NESTING_DEPTH;
use std::fmt::Write;

/// Run `f` on a thread whose stack fits a walk at the bound: the bound is set
/// for the engine's 256 MiB thread, not the 2 MiB test thread.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

/// A `CLUSTER` chain `levels` objects deep under `root_id`, each level one
/// `items` attribute below the last, ending in `innermost` (a leaf object).
/// Node ids run `id{first_id}`, `id{first_id + 1}`, … down the chain.
fn cluster_chain(root_id: &str, levels: usize, first_id: usize, innermost: &str) -> String {
    let mut open = String::new();
    for k in 0..levels {
        let nid = if k == 0 {
            root_id.to_owned()
        } else {
            format!("id{}", first_id + k - 1)
        };
        write!(open, "CLUSTER[{nid}] matches {{ items matches {{ ").expect("a String write");
    }
    let close = " } }".repeat(levels);
    format!("{open}{innermost}{close}")
}

/// A minimal ADL 2 archetype around `definition`.
fn archetype_source(hrid: &str, specialize: Option<&str>, definition: &str) -> String {
    let spec = specialize.map_or(String::new(), |p| format!("\nspecialize\n    {p}\n"));
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n{spec}\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    {definition}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n"
    )
}

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).unwrap_or_else(|errs| panic!("fixture parses: {errs:?}"))
}

fn cluster_hrid(concept: &str) -> String {
    format!("openEHR-EHR-CLUSTER.{concept}.v1.0.0")
}

// ── the cADL parser ───────────────────────────────────────────────────────

#[test]
fn a_definition_nested_past_the_bound_is_refused_as_sunk() {
    // The parser recurses up to the bound before refusing, so the refusal too
    // needs the stack the bound is sized for.
    let errs = on_big_stack(|| {
        let body = cluster_chain(
            "id1",
            MAX_NESTING_DEPTH + 2,
            2,
            "ELEMENT[id9999] matches {*}",
        );
        parse_definition_body(&body, Dialect::Adl2)
            .expect_err("a definition nested past the bound is refused, never recursed into")
    });
    let first = errs.first().expect("one refusal");
    assert_eq!(first.code, SyntaxErrorCode::Sunk);
    assert!(
        first
            .message
            .contains(&format!("exceeds the limit of {MAX_NESTING_DEPTH} levels")),
        "the refusal names the bound: {}",
        first.message
    );
}

#[test]
fn a_definition_well_within_the_bound_parses() {
    on_big_stack(|| {
        let body = cluster_chain(
            "id1",
            MAX_NESTING_DEPTH.div_ceil(2),
            2,
            "ELEMENT[id9999] matches {*}",
        );
        parse_definition_body(&body, Dialect::Adl2).expect("a deep but bounded definition parses");
    });
}

// ── the flattener ─────────────────────────────────────────────────────────

#[test]
fn a_specialisation_lineage_longer_than_the_bound_is_refused() {
    let length = MAX_NESTING_DEPTH + 2;
    let mut repo = ArchetypeRepository::new();
    let mut leaf = None;
    for k in 0..length {
        let hrid = cluster_hrid(&format!("chain{k}"));
        let parent = (k > 0).then(|| cluster_hrid(&format!("chain{}", k - 1)));
        let root_id = format!("id1{}", ".1".repeat(k));
        let src = archetype_source(
            &hrid,
            parent.as_deref(),
            &format!("CLUSTER[{root_id}] matches {{*}}"),
        );
        let archetype = parse(&src);
        if k + 1 == length {
            leaf = Some(archetype);
        } else {
            repo.insert(archetype);
        }
    }
    let err = flat_form(&leaf.expect("the deepest child"), &repo)
        .expect_err("a lineage past the bound is refused before the walk reaches its root");
    assert!(
        matches!(err, FlattenError::LineageTooDeep { limit } if limit == MAX_NESTING_DEPTH),
        "got {err:?}"
    );
}

#[test]
fn a_flat_form_composed_past_the_bound_is_refused() {
    // Each artefact parses within the bound, but the child's differential
    // path hangs its own chain under the parent's deepest node, so the flat
    // form is deeper than either.
    let half = MAX_NESTING_DEPTH.div_ceil(2) + 8;
    let parent_hrid = cluster_hrid("deep_parent");
    let parent_src = archetype_source(
        &parent_hrid,
        None,
        &cluster_chain("id1", half, 2, "ELEMENT[id9999] matches {*}"),
    );
    // The differential path to the parent's deepest `items` attribute:
    // `/items[id2]/items[id3]/…/items[id{half}]/items`.
    let mut path = String::new();
    for k in 2..=half {
        write!(path, "/items[id{k}]").expect("a String write");
    }
    path.push_str("/items");
    let child_chain = cluster_chain("id0.1", half, 20_000, "ELEMENT[id0.9999] matches {*}");
    let child_src = archetype_source(
        &cluster_hrid("deep_child"),
        Some(&parent_hrid),
        &format!("CLUSTER[id1.1] matches {{ {path} matches {{ {child_chain} }} }}"),
    );
    on_big_stack(move || {
        let mut repo = ArchetypeRepository::new();
        repo.insert(parse(&parent_src));
        let child = parse(&child_src);
        let err =
            flat_form(&child, &repo).expect_err("a flat form composed past the bound is refused");
        assert!(
            matches!(err, FlattenError::NestingTooDeep { limit } if limit == MAX_NESTING_DEPTH),
            "got {err:?}"
        );
    });
}

// ── the OPT transform ─────────────────────────────────────────────────────

#[test]
fn fillers_referencing_each_other_are_a_typed_refusal() {
    let a = cluster_hrid("filler_a");
    let b = cluster_hrid("filler_b");
    let a_src = archetype_source(
        &a,
        None,
        &format!("CLUSTER[id1] matches {{ items matches {{ use_archetype CLUSTER[id2, {b}] }} }}"),
    );
    let b_src = archetype_source(
        &b,
        None,
        &format!("CLUSTER[id1] matches {{ items matches {{ use_archetype CLUSTER[id2, {a}] }} }}"),
    );
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse(&b_src));
    let root = parse(&a_src);
    repo.insert(root.clone());
    let err =
        create_opt(&root, &repo).expect_err("a filler cycle can never be inlined to a finite OPT");
    match err {
        OptError::CyclicReference(chain) => {
            assert_eq!(
                chain,
                vec![a.clone(), b, a],
                "the chain names the cycle outermost-first"
            );
        }
        other => panic!("expected a cyclic-reference refusal, got {other:?}"),
    }
}

#[test]
fn fillers_inlined_past_the_bound_are_a_typed_refusal() {
    // Two artefacts each within the bound whose inlining composes past it.
    let half = MAX_NESTING_DEPTH.div_ceil(2) + 8;
    let filler_hrid = cluster_hrid("deep_filler");
    let filler_src = archetype_source(
        &filler_hrid,
        None,
        &cluster_chain("id1", half, 2, "ELEMENT[id9999] matches {*}"),
    );
    let root_src = archetype_source(
        &cluster_hrid("deep_root"),
        None,
        &cluster_chain(
            "id1",
            half,
            2,
            &format!("use_archetype CLUSTER[id9998, {filler_hrid}]"),
        ),
    );
    on_big_stack(move || {
        let mut repo = ArchetypeRepository::new();
        repo.insert(parse(&filler_src));
        let root = parse(&root_src);
        repo.insert(root.clone());
        let err = create_opt(&root, &repo).expect_err("an OPT composed past the bound is refused");
        assert!(
            matches!(err, OptError::NestingTooDeep { limit } if limit == MAX_NESTING_DEPTH),
            "got {err:?}"
        );
    });
}
