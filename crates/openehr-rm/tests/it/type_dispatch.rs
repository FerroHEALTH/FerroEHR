// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    let_underscore_drop,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
//! Deserialization dispatch of abstract/polymorphic openEHR slots on the
//! canonical-JSON `_type` discriminator.
//!
//! These tests pin the behaviour of the hand-rolled `Deserialize` impls the
//! `openehr-codegen` emitter now generates for abstract-slot enums (replacing
//! `#[serde(untagged)]`'s structural guessing). The governing contract is the
//! vendored ITS-JSON schema
//! (`crates/openehr-its/schemas/json/openehr_rm_1.1.0_all.json`): `_type` is
//! `required` on an abstract polymorphic slot (`DATA_VALUE`, `UID`, …) and
//! optional on a concrete polymorphic slot (`DV_TEXT`), defaulting a
//! `_type`-less value to the base concrete type.

use openehr_base::prelude::Uid;
use openehr_rm::prelude::{DataValue, DvText, PartySelf};

// ── a `_type`-less *abstract* slot value is rejected, not mis-typed ───────────

#[test]
fn abstract_slot_missing_type_is_rejected() {
    // Before the fix, `#[serde(untagged)]` structurally matched this `DV_TIME`
    // payload as the alphabetically-earlier `DvDate` — silent type corruption.
    // Now, an abstract `DATA_VALUE` slot requires `_type`, so this is rejected.
    let err = openehr_its::json::from_canonical_json::<DataValue>(r#"{"value":"12:00:00"}"#)
        .expect_err("a _type-less DATA_VALUE must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("DATA_VALUE") && msg.contains("_type"),
        "error should name the slot and the missing _type, got: {msg}"
    );
}

#[test]
fn abstract_slot_with_type_routes_to_the_named_variant() {
    // `_type` present → routed by the tag, never by structure.
    let dv: DataValue =
        openehr_its::json::from_canonical_json(r#"{"_type":"DV_TIME","value":"12:00:00"}"#)
            .expect("valid DV_TIME");
    assert!(
        matches!(dv, DataValue::DvTime(_)),
        "DV_TIME must route to DvTime, got {dv:?}"
    );

    let dv: DataValue =
        openehr_its::json::from_canonical_json(r#"{"_type":"DV_DATE","value":"2020-01-01"}"#)
            .expect("valid DV_DATE");
    assert!(
        matches!(dv, DataValue::DvDate(_)),
        "DV_DATE must route to DvDate"
    );
}

// ── wrong `_type` is rejected with an error naming it ─────────────────────────

#[test]
fn abstract_slot_unknown_type_is_rejected_naming_it() {
    let err = openehr_its::json::from_canonical_json::<DataValue>(
        r#"{"_type":"NOT_A_TYPE","value":"x"}"#,
    )
    .expect_err("an unknown _type must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("DATA_VALUE") && msg.contains("NOT_A_TYPE"),
        "error should name the slot and the offending _type, got: {msg}"
    );
}

// ── deep-descendant `_type` routes through the intermediate variant ───────────

#[test]
fn deep_descendant_type_routes_correctly_in_data_value_slot() {
    // DV_CODED_TEXT is not a *direct* variant of DATA_VALUE; it is reached via
    // the `DvText` variant (itself a polymorphic enum). The tag must route two
    // levels deep to DvCodedText.
    let dv: DataValue = openehr_its::json::from_canonical_json(
        r#"{"_type":"DV_CODED_TEXT","value":"male",
            "defining_code":{"_type":"CODE_PHRASE",
                "terminology_id":{"_type":"TERMINOLOGY_ID","value":"local"},
                "code_string":"at0001"}}"#,
    )
    .expect("valid DV_CODED_TEXT in a DATA_VALUE slot");
    match dv {
        DataValue::DvText(DvText::DvCodedText(_)) => {}
        other => panic!("DV_CODED_TEXT must route to DvText(DvCodedText), got {other:?}"),
    }
}

#[test]
fn deep_descendant_type_routes_in_uid_slot() {
    // A UID slot holding an OBJECT_VERSION_ID (a deep UID_BASED_ID descendant).
    let uid: Uid = openehr_its::json::from_canonical_json(
        r#"{"_type":"UUID","value":"550e8400-e29b-41d4-a716-446655440000"}"#,
    )
    .expect("valid UUID in a UID slot");
    assert!(matches!(uid, Uid::Uuid(_)), "UUID must route to Uid::Uuid");

    let err = openehr_its::json::from_canonical_json::<Uid>(
        r#"{"value":"550e8400-e29b-41d4-a716-446655440000"}"#,
    )
    .expect_err("a _type-less UID must be rejected");
    assert!(
        err.to_string().contains("UID"),
        "error should name the UID slot"
    );
}

// ── Concrete polymorphic slot: `_type` optional, defaults to the base type ────

#[test]
fn concrete_poly_slot_missing_type_defaults_to_base() {
    // A `name` field (DV_TEXT) frequently omits `_type` in real openEHR data;
    // the schema's `if not required _type then DV_TEXT` construction defaults it
    // to the base concrete type. This must NOT be rejected (it would break the
    // corpus round-trip).
    let t: DvText = openehr_its::json::from_canonical_json(r#"{"value":"systolic"}"#)
        .expect("a _type-less DV_TEXT must default to the base DvText");
    assert!(
        matches!(t, DvText::DvText(_)),
        "must default to DvText(DvTextData)"
    );
}

#[test]
fn concrete_poly_slot_routes_subtype_by_type() {
    let t: DvText = openehr_its::json::from_canonical_json(
        r#"{"_type":"DV_CODED_TEXT","value":"male",
            "defining_code":{"_type":"CODE_PHRASE",
                "terminology_id":{"_type":"TERMINOLOGY_ID","value":"local"},
                "code_string":"at0001"}}"#,
    )
    .expect("valid DV_CODED_TEXT");
    assert!(
        matches!(t, DvText::DvCodedText(_)),
        "DV_CODED_TEXT must route to DvCodedText"
    );
}

#[test]
fn concrete_poly_slot_unknown_type_is_rejected() {
    let err =
        openehr_its::json::from_canonical_json::<DvText>(r#"{"_type":"DV_QUANTITY","value":"x"}"#)
            .expect_err("a non-DV_TEXT _type must be rejected in a DV_TEXT slot");
    let msg = err.to_string();
    assert!(
        msg.contains("DV_TEXT") && msg.contains("DV_QUANTITY"),
        "error should name the slot and the offending _type, got: {msg}"
    );
}

// ── Monomorphic struct slot: a foreign `_type` is rejected (EHR_STATUS.subject) ─

// `EHR_STATUS.subject : PARTY_SELF` is a *monomorphic* slot — `PARTY_SELF` has no
// subtypes — so the generated `PartySelf` struct's `#[derive(OpenEhrType)]`
// `Deserialize` must reject any non-`PARTY_SELF` `_type` (a `PARTY_IDENTIFIED`
// payload in that slot is invalid canonical JSON), while tolerating an absent
// `_type` (defaults to the declared type). RM ehr master04 §EHR Status.

#[test]
fn monomorphic_struct_slot_rejects_foreign_type() {
    let err = openehr_its::json::from_canonical_json::<PartySelf>(
        r#"{"_type":"PARTY_IDENTIFIED","name":"Bob","identifiers":[]}"#,
    )
    .expect_err("a PARTY_IDENTIFIED payload must not deserialize as PARTY_SELF");
    let msg = err.to_string();
    assert!(
        msg.contains("PARTY_SELF") && msg.contains("PARTY_IDENTIFIED"),
        "error should name the expected and offending types, got: {msg}"
    );
}

#[test]
fn monomorphic_struct_slot_accepts_matching_and_absent_type() {
    // Explicit matching _type.
    openehr_its::json::from_canonical_json::<PartySelf>(r#"{"_type":"PARTY_SELF"}"#)
        .expect("explicit PARTY_SELF");
    // Absent _type defaults to the declared type (an anonymous PARTY_SELF).
    let anon: PartySelf =
        openehr_its::json::from_canonical_json(r"{}").expect("empty anonymous PARTY_SELF");
    assert!(
        anon.external_ref.is_none(),
        "an empty PARTY_SELF is an anonymous subject with no external_ref"
    );
}

// ── undeclared wire keys are refused (strict reader) ──────────────────────────

#[test]
fn unknown_keys_are_refused_on_deserialize() {
    // The strict reader is closed over the generated RM model: a key the
    // class does not declare is refused, naming the key (ITS-REST
    // Resources.md L87 — canonical JSON "SHOULD validate against" the
    // ITS-JSON schemas, which close their objects by design; the former
    // tolerant read was retired in the foundation phase). Dispatch through
    // the untagged DataValue enum must surface the refusal, not fall through
    // to "no variant matched".
    let err = openehr_its::json::from_canonical_json::<DataValue>(
        r#"{"_type":"DV_COUNT","magnitude":3,"an_unknown_extension_key":true}"#,
    )
    .expect_err("an undeclared key must be refused, not ignored");
    let msg = err.to_string();
    assert!(
        msg.contains("unknown field `an_unknown_extension_key`"),
        "the refusal names the offending key, got: {msg}"
    );
    assert!(
        !msg.contains("did not match any variant"),
        "must not be the opaque untagged-enum error, got: {msg}"
    );
    // The same document without the undeclared key stays accepted (the valid
    // twin of this refusal).
    let dv: DataValue =
        openehr_its::json::from_canonical_json(r#"{"_type":"DV_COUNT","magnitude":3}"#)
            .expect("the declared-keys-only twin parses");
    assert!(matches!(dv, DataValue::DvCount(_)));
}

// ── the inner variant's precise error survives (not "no variant") ─────────────

#[test]
fn malformed_variant_surfaces_the_real_inner_error() {
    // A DV_QUANTITY missing its mandatory `units`: the error must name the real
    // missing field, not the opaque untagged-enum "did not match any variant".
    let err = openehr_its::json::from_canonical_json::<DataValue>(
        r#"{"_type":"DV_QUANTITY","magnitude":1.0}"#,
    )
    .expect_err("a DV_QUANTITY without units must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("units"),
        "error should name the missing `units` field, got: {msg}"
    );
    assert!(
        !msg.contains("did not match any variant"),
        "must not be the opaque untagged-enum error, got: {msg}"
    );
}
