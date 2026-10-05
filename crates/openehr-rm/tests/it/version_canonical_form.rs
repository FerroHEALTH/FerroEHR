// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

//! `VERSION.canonical_form()` over a real corpus COMPOSITION, on BOTH emitted
//! generations of the function.
//!
//! Spec authority: RM common §"Digital Signature"
//! (`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`)
//! and `VERSION.canonical_form()`
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.version.adoc`)
//! — "created by serialising all attributes except signature".
//!
//! These live here rather than beside `version_impl.rs` because the fixture is
//! the `openehr-its` vendored corpus, which sits outside this crate's published
//! package (`include = ["src/**", …]`): a packaged `src/` test reading it
//! compiles for us and fails with "couldn't read file" for anyone running
//! `cargo test` inside the published `.crate`.

use serde_json::Value;

/// A representative `ORIGINAL_VERSION` JSON whose `data` is a real corpus
/// COMPOSITION (`openehr-its` vendored `minimal_persistent.json`), with fixed
/// version metadata so the canonical form is deterministic.
const COMPOSITION: &str = include_str!(
    "../../../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/minimal_persistent.json"
);

/// Returns the canonical form under both emitted generations of the function.
///
/// The two are generation twins, so they must agree byte for byte; every
/// property below is therefore asserted on both at once.
fn canonical_form(value: &Value) -> String {
    let stable =
        openehr_rm::v1_1::common::change_control::version_impl::canonical_form_of_json(value)
            .expect("RFC 8785 canonicalisation of a well-formed value should succeed");
    let development =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json(value)
            .expect("RFC 8785 canonicalisation of a well-formed value should succeed");
    assert_eq!(
        stable, development,
        "the v1_1 and v1_2 canonical forms are generation twins and must agree"
    );
    development
}

/// Builds the `ORIGINAL_VERSION` canonical-JSON `Value` around the corpus
/// composition, with the given `signature` value (so tests can compare None
/// vs Some).
///
/// The signed input is assembled as a `Value` — the shape the versioning
/// service holds — and canonicalised via [`canonical_form`].
fn original_version(signature: Option<&str>) -> Value {
    let data: Value = serde_json::from_str(COMPOSITION).unwrap();
    let mut ov = serde_json::json!({
        "_type": "ORIGINAL_VERSION",
        "contribution": {
            "_type": "OBJECT_REF",
            "namespace": "local",
            "type": "CONTRIBUTION",
            "id": {
                "_type": "HIER_OBJECT_ID",
                "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000abc"
            }
        },
        "commit_audit": {
            "_type": "AUDIT_DETAILS",
            "system_id": "ferroehr.local",
            "time_committed": {
                "_type": "DV_DATE_TIME",
                "value": "2026-07-07T10:11:12.5Z"
            },
            "change_type": {
                "_type": "DV_CODED_TEXT",
                "value": "creation",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "249"
                }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "FerroEHR" }
        },
        "uid": {
            "_type": "OBJECT_VERSION_ID",
            "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000001::ferroehr.local::1"
        },
        "lifecycle_state": {
            "_type": "DV_CODED_TEXT",
            "value": "complete",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "532"
            }
        },
        "data": data
    });
    if let Some(sig) = signature {
        ov.as_object_mut()
            .unwrap()
            .insert("signature".to_owned(), Value::String(sig.to_owned()));
    }
    ov
}

#[test]
fn canonical_form_is_byte_stable_across_repeated_calls() {
    let ov = original_version(None);
    let a = canonical_form(&ov);
    let b = canonical_form(&ov);
    assert_eq!(a, b, "canonical_form must be deterministic");
}

#[test]
fn canonical_form_independent_of_signature_value() {
    // RM common §"Digital Signature": the signature attribute is Void during
    // serialisation — so its stored value must not affect the canonical form.
    let none = canonical_form(&original_version(None));
    let some = canonical_form(&original_version(Some("sha256:tampered")));
    let other = canonical_form(&original_version(Some(
        "-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----",
    )));
    assert_eq!(none, some);
    assert_eq!(none, other);
}

#[test]
fn canonical_form_golden() {
    // Golden vector: pins the RFC 8785 canonical form of the corpus
    // ORIGINAL_VERSION so any accidental change to the algorithm is caught.
    let canonical = canonical_form(&original_version(None));
    insta::assert_snapshot!("original_version_canonical_form", canonical);
}
