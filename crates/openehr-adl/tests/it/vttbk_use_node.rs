// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and fixture plumbing"
)]
//! VTTBK over term-binding paths that run through a `use_node` (#5457).
//!
//! Oracle: AOM2 `master07-terminology_package.adoc` §Validity Rules, VTTBK
//! ("a path that is valid in the flat archetype"); AOM2
//! `master03-archetype_package.adoc` §Structural Variants (flattening replaces
//! each `use_node` by a copy of the subtree it points to); ADL2
//! `master04.3-cadl_complex_types.adoc` §Paths and Proxy Objects (a path
//! through a proxy that is not a sibling of its target uses the target's node
//! id). ADL2 `master07.13-adl_terminology.adoc` §Term bindings quotes the
//! reference archetype `term_bindings_paths_use_refs` read here.

use std::path::{Path, PathBuf};

use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::{ValidationIssue, validate_source};

fn corpus(relative: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The ADL 2 reference archetype whose purpose is "bindings to paths that
/// traverse `use_node` references".
fn use_refs() -> String {
    corpus(
        "tests/corpus/adl2-reference/features/terminology/term_bindings/\
         openEHR-EHR-OBSERVATION.term_bindings_paths_use_refs.v1.0.0.adls",
    )
}

fn errors(src: &str) -> Vec<ValidationIssue> {
    validate_source(src, None, &ProductionRmModel, &NoTerminologyResolver)
        .expect("the source parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect()
}

fn vttbk(src: &str) -> Vec<String> {
    errors(src)
        .into_iter()
        .filter(|i| i.code == ValidationCode::Vttbk)
        .map(|i| i.message)
        .collect()
}

/// `src` with the binding key `from` written as `to`.
fn rebind(src: &str, from: &str, to: &str) -> String {
    let from = format!("[\"{from}\"]");
    assert_eq!(src.matches(&from).count(), 1, "one binding keyed {from}");
    src.replacen(&from, &format!("[\"{to}\"]"), 1)
}

#[test]
fn the_reference_archetype_binds_through_its_use_nodes_and_validates() {
    let issues = errors(&use_refs());
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn the_2013_ckm_apgar_raises_no_vttbk() {
    let src = corpus(
        "../../corpus/archetypes/adl2/ckm-2013-12-09/entry/observation/\
         openEHR-EHR-OBSERVATION.apgar.v1.0.0.adls",
    );
    assert_eq!(vttbk(&src), Vec::<String>::new());
}

/// The proxy `id41` is not a sibling of its target `id2`, so a path through it
/// names the target's id; the proxy's own id names no node of the flat form.
#[test]
fn a_path_through_a_proxy_by_its_own_id_stays_refused() {
    let src = rebind(
        &use_refs(),
        "/data[id3]/events[id27]/data[id2]/items[id26]",
        "/data[id3]/events[id27]/data[id41]/items[id26]",
    );
    let raised = vttbk(&src);
    assert_eq!(raised.len(), 1, "{raised:?}");
    assert!(
        raised.iter().all(|m| m.contains("data[id41]")),
        "{raised:?}"
    );
}

#[test]
fn a_path_through_a_proxy_to_a_node_its_target_lacks_stays_refused() {
    let src = rebind(
        &use_refs(),
        "/data[id3]/events[id27]/data[id2]/items[id26]",
        "/data[id3]/events[id27]/data[id2]/items[id99]",
    );
    let raised = vttbk(&src);
    assert_eq!(raised.len(), 1, "{raised:?}");
    assert!(
        raised.iter().all(|m| m.contains("items[id99]")),
        "{raised:?}"
    );
}
