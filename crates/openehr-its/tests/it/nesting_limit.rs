// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The RM readers' one nesting limit (#5997), on the smallest stack an
//! RM-reading thread runs on.
//!
//! No openEHR spec bounds document nesting: our own design.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "fixture helpers outside a #[test] fn: a missing fixture or a refused write is a test failure"
)]

use openehr_base::containers::NonEmptyVec;
use openehr_base::nesting::{MAX_RM_NESTING, MIN_RM_READ_STACK_BYTES};
use openehr_its::xml::runtime::Namespace;
use openehr_rm::prelude::{Composition, ContentItem};
use serde_json::{Value, json};

/// A lab composition whose `content` is a chain of SECTIONs, so its deepest
/// RM object (the innermost section's `name`) sits `depth` objects deep.
fn nested(depth: u32) -> Value {
    let section = || {
        json!({
            "_type": "SECTION",
            "name": {"_type": "DV_TEXT", "value": "s"},
            "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1"
        })
    };
    // COMPOSITION, then `depth - 2` sections, then the innermost name.
    let mut chain = section();
    for _ in 1..depth.saturating_sub(2) {
        let mut outer = section();
        outer["items"] = Value::Array(vec![chain]);
        chain = outer;
    }
    let lab = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/templates/ckm/generic-lab-test-result.example.json"
    ))
    .expect("the lab fixture should be readable");
    let mut doc: Value = serde_json::from_str(&lab).expect("the lab fixture should be JSON");
    doc["content"] = Value::Array(vec![chain]);
    doc
}

/// Run `f` on a thread of the smallest RM-reading stack.
fn on_smallest_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(MIN_RM_READ_STACK_BYTES)
        .spawn(f)
        .expect("the reader thread should start")
        .join()
        .expect("the reader thread should not panic")
}

fn json_read(depth: u32) -> Result<Composition, String> {
    let text = serde_json::to_string(&nested(depth)).expect("serializes");
    on_smallest_stack(move || {
        openehr_its::json::from_canonical_json::<Composition>(&text).map_err(|e| e.to_string())
    })
}

fn xml_text(depth: u32) -> String {
    let doc = nested(depth);
    on_smallest_stack(move || {
        let comp = openehr_its::json::from_canonical_value::<Composition>(&doc)
            .expect("a document at the limit should read");
        openehr_its::xml::runtime::to_xml(&comp, "composition", Namespace::V2)
            .expect("the composition should write as XML")
    })
}

fn xml_read(xml: String) -> Result<Composition, String> {
    on_smallest_stack(move || {
        openehr_its::xml::from_canonical_xml::<Composition>(&xml).map_err(|e| e.to_string())
    })
}

#[test]
fn json_reads_at_the_limit_on_the_smallest_stack() {
    assert!(json_read(MAX_RM_NESTING).is_ok());
}

#[test]
fn json_refuses_one_level_past_the_limit() {
    let err = json_read(MAX_RM_NESTING + 1).expect_err("one level past the limit");
    assert!(
        err.contains(&openehr_base::nesting::NestingTooDeep.to_string()),
        "{err}"
    );
}

#[test]
fn xml_reads_at_the_limit_on_the_smallest_stack() {
    assert!(xml_read(xml_text(MAX_RM_NESTING)).is_ok());
}

#[test]
fn xml_refuses_one_level_past_the_limit() {
    // The writer has no limit, so the deeper document is written from the
    // limit document with its outer SECTION wrapped once more.
    let doc = nested(MAX_RM_NESTING);
    let deeper = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut comp = openehr_its::json::from_canonical_value::<Composition>(&doc)
                .expect("a document at the limit should read");
            let content: Vec<ContentItem> = comp.content.take().expect("content").into();
            let Some(ContentItem::Section(section)) = content.into_iter().next() else {
                panic!("the content should be a section");
            };
            let mut outer = (*section).clone();
            outer.items =
                Some(NonEmptyVec::try_from(vec![ContentItem::from(*section)]).expect("one item"));
            comp.content =
                Some(NonEmptyVec::try_from(vec![ContentItem::from(outer)]).expect("one item"));
            openehr_its::xml::runtime::to_xml(&comp, "composition", Namespace::V2)
                .expect("the composition should write as XML")
        })
        .expect("the writer thread should start")
        .join()
        .expect("the writer thread should not panic");
    let err = xml_read(deeper).expect_err("one level past the limit");
    assert!(
        err.contains(&openehr_base::nesting::NestingTooDeep.to_string()),
        "{err}"
    );
}
