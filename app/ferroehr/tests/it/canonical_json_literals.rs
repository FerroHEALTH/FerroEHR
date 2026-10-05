// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Regression gate: canonical RM shapes are BUILT from the generated
//! `openehr-*` types, never hand-written as `json!` literals.
//!
//! A `json!` literal carrying a `"_type"` key is, by definition, a hand-rolled
//! canonical openEHR fragment — the scanner mechanics live in
//! [`testkit::json_literals`] (shared by every crate's gate); this file owns
//! ONLY this crate's adjudications. To add a site, classify it and put it in
//! [`ALLOWLIST`] with a one-line reason — every entry must name why that
//! file's literals are NOT a synthesized canonical shape. A stale entry
//! (allowlisted file with no remaining literals) fails too, so the list
//! cannot rot (issue #1686).

/// Files whose `_type`-carrying `json!` literals are classified as something
/// other than a synthesized canonical shape, each with the reason.
const ALLOWLIST: &[(&str, &str)] = &[
    (
        "service/message/export.rs",
        "EXTRACT envelope composed over ALREADY-CANONICAL opaque fragments \
         (versioned-object bodies, version envelopes, revision histories) — \
         see the TODO(#1695) at the composition site",
    ),
    (
        "service/demographic/contribution.rs",
        "CONTRIBUTION envelope over the opaque canonical AUDIT_DETAILS \
         fragment; the synthesized parts are built from their generated types",
    ),
    (
        "versioning/contribution.rs",
        "CONTRIBUTION envelope whose `versions` hold either OBJECT_REFs or \
         whole resolved VERSION envelopes, which `Contribution.versions: \
         Vec<ObjectRef>` cannot express",
    ),
    (
        "versioning/wire.rs",
        "ORIGINAL_VERSION / IMPORTED_VERSION envelopes over VERBATIM stored \
         fragments — this serialization is what gets digitally signed (RM \
         common master06 §Digital Signature), so re-encoding it is unsafe",
    ),
];

/// No production `json!` literal outside the classified allowlist synthesizes a
/// canonical openEHR shape.
#[test]
fn canonical_shapes_are_built_from_the_generated_types() {
    let sites = testkit::json_literals::offending_sites(env!("CARGO_MANIFEST_DIR"))
        .expect("the crate's src/ tree should be readable");
    let unlisted = testkit::json_literals::unlisted(&sites, ALLOWLIST);

    assert!(
        unlisted.is_empty(),
        "issue #1686: {} `json!` literal(s) in ferroehr/src synthesize a \
         canonical openEHR shape (they carry a `\"_type\"` key). Build the \
         generated `openehr-rm`/`openehr-base` type and serialize it with \
         `openehr_its::json::to_canonical_value` instead — that is what keeps \
         attribute order and mandatory attributes correct by construction. If \
         a site is genuinely a verbatim pass-through or an internal \
         (non-canonical) shape, add its file to the ALLOWLIST in this test \
         with a one-line reason.\nSites:\n{}",
        unlisted.len(),
        unlisted
            .iter()
            .map(|(file, line)| format!("  app/ferroehr/src/{file}:{line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Every allowlist entry still describes a real site — a stale entry is a
/// silent licence to reintroduce hand-written canonical JSON.
#[test]
fn the_allowlist_carries_no_stale_entries() {
    let sites = testkit::json_literals::offending_sites(env!("CARGO_MANIFEST_DIR"))
        .expect("the crate's src/ tree should be readable");
    let stale = testkit::json_literals::stale_entries(&sites, ALLOWLIST);

    assert!(
        stale.is_empty(),
        "issue #1686: these ALLOWLIST entries no longer match any `json!` \
         literal carrying a `\"_type\"` key — the sites were converted, so \
         drop the entries: {stale:?}",
    );
}
