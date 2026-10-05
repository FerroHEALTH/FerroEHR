// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    reason = "test fixtures/diagnostics — a malformed fixture should fail loudly"
)]
//! Pins for the `x /= Void implies not x.is_empty` invariant family: since
//! #1730 an OPTIONAL container carrying the invariant emits
//! `Option<NonEmptyVec<T>>`, so a present-but-empty list is UNREPRESENTABLE
//! and the strict reader refuses `[]` at parse — these pins retarget the
//! #1623 behaviour pins from the retired `NONEMPTY_LIST_RULES` evaluator onto
//! the construction door (the issue's "refusal twins retargeted to
//! construction/parse" criterion).

use serde_json::json;

/// A representative flipped field per re-adjudicated #1623 row:
/// `PARTY_IDENTIFIED.identifiers` (RM
/// `org.openehr.rm.common.party_identified.adoc` §Invariants,
/// `Identifiers_valid`).
#[test]
fn present_but_empty_refuses_at_parse_absent_and_populated_pass() {
    use openehr_rm::v1_2::common::generic::party_identified::PartyIdentifiedData;

    let empty = json!({ "name": "x", "identifiers": [] });
    let err = openehr_its_free_decode::<PartyIdentifiedData>(&empty)
        .expect_err("a present-but-empty identifiers list must refuse at parse");
    assert!(
        err.contains("identifiers"),
        "the refusal names the container: {err}"
    );

    let absent = json!({ "name": "x" });
    openehr_its_free_decode::<PartyIdentifiedData>(&absent).expect("absent is legal (0..1)");

    let populated = json!({ "name": "x", "identifiers": [{ "id": "i1" }] });
    openehr_its_free_decode::<PartyIdentifiedData>(&populated).expect("populated passes");
}

/// The typed model itself: `Option<NonEmptyVec<..>>` construction has no
/// empty-present state (the `EXTRACT_UPDATE_SPEC.trigger_events` /
/// `ORIGINAL_VERSION.attestations` rows share the shape by generation).
#[test]
fn the_flipped_shape_is_option_nonemptyvec() {
    let one = openehr_base::containers::present_nonempty(vec![1_i32]);
    assert_eq!(one.as_deref().map(<[i32]>::len), Some(1));
    assert!(
        openehr_base::containers::present_nonempty::<i32>(Vec::new()).is_none(),
        "empty input is the ABSENT state, never a present-empty value"
    );
}

/// Decode through the crate's own emitted manual serde impls (the same door
/// `openehr_its::json::from_canonical_value` wraps — openehr-rm is upstream
/// of the codec crate, so the wrapper is not importable here).
fn openehr_its_free_decode<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<T, String> {
    serde_path_to_error::deserialize(value).map_err(|e| e.to_string())
}
