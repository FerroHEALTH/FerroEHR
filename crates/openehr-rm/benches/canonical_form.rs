// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The measurement behind the fast paths of #5682: what the RFC 8785 writer
//! costs per document now and as it stood before them
//! (`tests/it/canonical_reference.rs`).
//!
//! A second set of rows times a typed composition both ways #5849 compares:
//! `canonical_json(&serde_json::to_value(x))` and the serializer that writes
//! the form straight from `x`. A third set times the serializer over objects
//! nested ever deeper, whose cost per byte holds flat with depth (#6444). No
//! specification governs this measurement: our own design.

#![expect(
    clippy::expect_used,
    reason = "bench fixture setup: a fixture that does not read must abort the bench loudly \
              (there is no caller to return an error to)"
)]
#![expect(
    clippy::print_stdout,
    reason = "the bench is a dev binary whose output is its rows"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the canonical form is computed over raw canonical JSON, the writer's input"
)]

#[path = "../tests/it/canonical_reference.rs"]
mod canonical_reference;

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_to_string;
use openehr_rm::v1_2::composition::composition::Composition;
use serde::Serialize;
use serde_json::{Value, json};

/// The timed runs per writer and document, of which the median is reported.
const SAMPLES: usize = 31;

/// The index of the median of [`SAMPLES`] sorted runs.
const MEDIAN: usize = 15;

/// The least time one sample spends, so a short document is repeated enough.
const SAMPLE_TIME: Duration = Duration::from_millis(40);

/// A record shaped like an access-log entry: identifiers, codes, a time and a
/// digest, every string ASCII and none escaped.
fn access_record() -> Value {
    json!({
        "_type": "ACCESS_LOG_RECORD",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0193a4f2-6b1e-7c3d-9a8b-1f2e3d4c5b6a" },
        "time": { "_type": "DV_DATE_TIME", "value": "2026-11-15T09:41:27.318204+01:00" },
        "subject": {
            "_type": "PARTY_REF",
            "namespace": "ehr",
            "type": "EHR",
            "id": { "_type": "HIER_OBJECT_ID", "value": "7d0c9a1e-3f4b-4e2a-8c6d-5b4a3f2e1d0c" }
        },
        "actor": {
            "_type": "PARTY_IDENTIFIED",
            "name": "Dr. A. Jansen",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "id": "900012345", "issuer": "CIBG", "type": "UZI" }]
        },
        "operation": { "_type": "CODE_PHRASE", "terminology_id": { "value": "ferroehr-operations" }, "code_string": "read" },
        "purpose": { "_type": "CODE_PHRASE", "terminology_id": { "value": "HL7_ActReason" }, "code_string": "TREAT" },
        "outcome": { "_type": "CODE_PHRASE", "terminology_id": { "value": "ferroehr-outcome" }, "code_string": "permitted" },
        "resources": [
            "openEHR-EHR-COMPOSITION.encounter.v1",
            "openEHR-EHR-OBSERVATION.blood_pressure.v2",
            "openEHR-EHR-EVALUATION.problem_diagnosis.v1"
        ],
        "request_id": "01JCKZ4T9W7Q2M3N5P6R8S0V1X",
        "previous_digest": "sha256:4f1c2a9e8b7d6c5f4e3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e",
        "sequence": 1_048_576,
        "elapsed_micros": 1843
    })
}

/// A vendored composition, as the CDR stores one.
fn composition() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
    );
    let text = std::fs::read_to_string(path).expect("the vendored composition reads");
    serde_json::from_str(&text).expect("the vendored composition parses")
}

/// A document of non-ASCII prose with quotes, newlines and tabs to escape.
fn prose() -> Value {
    let line =
        "Patiënt meldt \"druk op de borst\" sinds één week;\n\tgeen koorts, Ø 38,2 °C → normaal. ";
    let items: Vec<Value> = (0..64)
        .map(|index| json!({ "index": index, "narrative": line.repeat(4), "naïef": true }))
        .collect();
    json!({ "_type": "SECTION", "items": items })
}

/// An object `depth` levels deep whose members are out of name order at
/// every level, as an RM value's fields are (#6444).
#[derive(Serialize)]
struct Nested {
    value: String,
    inner: Option<Box<Nested>>,
    archetype_node_id: &'static str,
}

fn nested(depth: usize) -> Nested {
    Nested {
        value: format!("level {depth}"),
        inner: (depth > 0).then(|| Box::new(nested(depth - 1))),
        archetype_node_id: "at0001",
    }
}

/// The median nanoseconds one call of `write` takes over `value`.
fn median_nanos<T>(value: &T, write: impl Fn(&T) -> String) -> f64 {
    let mut samples: Vec<f64> = (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            let mut calls = 0_u32;
            while start.elapsed() < SAMPLE_TIME {
                black_box(write(black_box(value)));
                calls += 1;
            }
            start.elapsed().as_secs_f64() * 1e9 / f64::from(calls)
        })
        .collect();
    samples.sort_unstable_by(f64::total_cmp);
    samples.get(MEDIAN).copied().expect("a median sample")
}

fn main() {
    for (name, value) in [
        ("access_record", access_record()),
        ("composition", composition()),
        ("prose", prose()),
    ] {
        let current = canonical_json(&value).expect("the document canonicalises");
        assert_eq!(
            current,
            canonical_reference::canonical(&value),
            "the writers agree on {name}"
        );
        let before = median_nanos(&value, canonical_reference::canonical);
        let after = median_nanos(&value, |value| {
            canonical_json(value).expect("the document canonicalises")
        });
        let saved = 100.0 - 100.0 * after / before;
        println!(
            "{}",
            json!({
                "document": name,
                "bytes": current.len(),
                "before_ns": before.round(),
                "after_ns": after.round(),
                "saved_percent": (saved * 10.0).round() / 10.0,
            })
        );
    }
    let typed: Composition =
        serde_json::from_value(composition()).expect("the vendored composition is a COMPOSITION");
    let through_value = |value: &Composition| {
        canonical_json(&serde_json::to_value(value).expect("a composition serialises"))
            .expect("the composition canonicalises")
    };
    let straight =
        |value: &Composition| canonical_to_string(value).expect("the composition canonicalises");
    let form = straight(&typed);
    assert_eq!(
        form,
        through_value(&typed),
        "the two paths agree on the composition"
    );
    let before = median_nanos(&typed, through_value);
    let after = median_nanos(&typed, straight);
    println!(
        "{}",
        json!({
            "document": "typed_composition",
            "bytes": form.len(),
            "through_value_ns": before.round(),
            "serializer_ns": after.round(),
            "saved_percent": ((100.0 - 100.0 * after / before) * 10.0).round() / 10.0,
        })
    );
    for depth in [4, 16, 64, 256] {
        let value = nested(depth);
        let form = canonical_to_string(&value).expect("the document canonicalises");
        let nanos = median_nanos(&value, |value| {
            canonical_to_string(value).expect("the document canonicalises")
        });
        let bytes = f64::from(u32::try_from(form.len()).expect("a bench document under 4 GiB"));
        println!(
            "{}",
            json!({
                "document": "nested",
                "depth": depth,
                "bytes": form.len(),
                "serializer_ns": nanos.round(),
                "ns_per_byte": (nanos / bytes * 100.0).round() / 100.0,
            })
        );
    }
}
