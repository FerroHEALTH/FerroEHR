// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::doc_markdown, reason = "prose with spec/crate proper nouns")]
#![expect(let_underscore_drop, reason = "test assertions/diagnostics/fixtures")]
//! Interop fidelity gate — deserialize the real EHRbase / openEHR_SDK canonical
//! JSON corpus (`tests/vendor/`) into our **generated** `openehr-rm` types.
//!
//! This is the acceptance test that our generated RM actually reads real
//! openEHR data. Each corpus file is dispatched by its top-level `_type` to the
//! matching generated type. `rawdb_*` / array / `_type`-less fragments are not
//! canonical single-RM-object roots; every one of them is named in the SINGLE
//! exclusion registry (`common::excluded`) with its adjudication, and a file
//! that is not a canonical root WITHOUT a registry entry FAILS this gate — an
//! exclusion can therefore never stop applying unnoticed.
//!
//! NOTE: the corpus is RM 1.1.0-era while the generated types are RM 1.2.0, and
//! `OpenEhrType` deserialization is lenient (ignores unknown fields), so this
//! gate proves *readability*; a stricter lossless re-serialize round-trip is a
//! follow-up once the 1.1↔1.2 field drift is characterized.

use crate::common::{corpus_files, corpus_rel, excluded};
use openehr_its::json::{
    from_canonical_json, to_canonical_json, to_canonical_value, validate_canonical,
};
use openehr_rm::prelude::{Composition, Contribution, EhrStatus, Folder, ItemTree};
use std::fs;

/// Deserialize `json` into the generated type named by `ty`, then re-serialize
/// (proving the value is well-formed on the way back out too).
fn deserialize_as(ty: &str, json: &str) -> Result<(), String> {
    macro_rules! roundtrip {
        ($T:ty) => {{
            let v: $T = from_canonical_json(json).map_err(|e| e.to_string())?;
            let _ = to_canonical_json(&v);
            Ok(())
        }};
    }
    match ty {
        "COMPOSITION" => roundtrip!(Composition),
        "FOLDER" => roundtrip!(Folder),
        "EHR_STATUS" => roundtrip!(EhrStatus),
        "CONTRIBUTION" => roundtrip!(Contribution),
        "ITEM_TREE" => roundtrip!(ItemTree),
        other => Err(format!("no dispatch for top-level _type {other:?}")),
    }
}

/// Deserialize `json` into the generated type named by `ty` and re-serialize it
/// back to a `serde_json::Value`, for the lossless round-trip comparison.
fn reserialize(ty: &str, json: &str) -> Result<serde_json::Value, String> {
    macro_rules! rt {
        ($T:ty) => {{
            let v: $T = from_canonical_json(json).map_err(|e| e.to_string())?;
            Ok(to_canonical_value(&v))
        }};
    }
    match ty {
        "COMPOSITION" => rt!(Composition),
        "FOLDER" => rt!(Folder),
        "EHR_STATUS" => rt!(EhrStatus),
        "CONTRIBUTION" => rt!(Contribution),
        "ITEM_TREE" => rt!(ItemTree),
        other => Err(format!("no dispatch for top-level _type {other:?}")),
    }
}

/// Compare re-serialized `output` against the original `input` for **semantic
/// equality** (no data loss), tolerating the three deliberate, information-
/// preserving differences between archie/EHRbase canonical JSON and ours:
///
/// 1. object key order (compared structurally, not textually);
/// 2. fields we omit that the input stated as empty (`null` / `[]`) — our
///    `OpenEhrType` serialize drops `None`/empty, which is equivalent;
/// 3. fields we materialize that the input left implicit — the always-present
///    `_type`, and the `Interval` `*_included`/`*_unbounded` flags at their
///    canonical defaults.
///
/// Anything else — an input value we dropped, or a genuine value mismatch — is a
/// real fidelity defect and is reported with its JSON path.
fn semantic_eq(
    input: &serde_json::Value,
    output: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    use serde_json::Value;
    match (input, output) {
        (Value::Object(im), Value::Object(om)) => objects_semantic_eq(im, om, path),
        (Value::Array(ia), Value::Array(oa)) => arrays_semantic_eq(ia, oa, path),
        // `5` (Integer) and `5.0` (Real) are the same magnitude on the wire.
        (Value::Number(a), Value::Number(b)) if a.as_f64() == b.as_f64() => Ok(()),
        _ if input == output => Ok(()),
        _ => Err(format!("{path}: {} vs {}", preview(input), preview(output))),
    }
}

/// Compares two objects member by member, in both directions.
fn objects_semantic_eq(
    input: &serde_json::Map<String, serde_json::Value>,
    output: &serde_json::Map<String, serde_json::Value>,
    path: &str,
) -> Result<(), String> {
    for (k, iv) in input {
        match output.get(k) {
            Some(ov) => semantic_eq(iv, ov, &format!("{path}.{k}"))?,
            None if is_omittable(iv) => {}
            None => {
                return Err(format!(
                    "{path}.{k}: present in input, dropped from output ({})",
                    preview(iv)
                ));
            }
        }
    }
    for (k, ov) in output {
        if !input.contains_key(k) && !is_default_materialization(k, ov) {
            return Err(format!(
                "{path}.{k}: emitted in output but absent from input ({})",
                preview(ov)
            ));
        }
    }
    Ok(())
}

/// Compares two arrays element by element, lengths first.
fn arrays_semantic_eq(
    input: &[serde_json::Value],
    output: &[serde_json::Value],
    path: &str,
) -> Result<(), String> {
    if input.len() != output.len() {
        return Err(format!(
            "{path}: array length {} vs {}",
            input.len(),
            output.len()
        ));
    }
    for (i, (a, b)) in input.iter().zip(output).enumerate() {
        semantic_eq(a, b, &format!("{path}[{i}]"))?;
    }
    Ok(())
}

/// An input field we may legitimately drop: `OpenEhrType` omits `None` (`null`)
/// and empty containers (`[]`), which carry no information.
fn is_omittable(v: &serde_json::Value) -> bool {
    v.is_null() || v.as_array().is_some_and(Vec::is_empty)
}

/// An output field the input left implicit: the always-emitted `_type`, and the
/// `Interval` inclusivity/boundedness flags at their canonical defaults (a
/// bounded limit is included; an unstated limit is bounded).
fn is_default_materialization(key: &str, v: &serde_json::Value) -> bool {
    key == "_type"
        || (matches!(key, "lower_included" | "upper_included") && v.as_bool() == Some(true))
        || (matches!(key, "lower_unbounded" | "upper_unbounded") && v.as_bool() == Some(false))
}

fn preview(v: &serde_json::Value) -> String {
    let s = v.to_string();
    match s.get(..80) {
        Some(head) => format!("{head}…"),
        None => s,
    }
}

// The corpus walker + the documented exclusion list live in `common` (shared
// with the canonical-output contract gate).

#[test]
fn generated_rm_reads_the_openehr_sdk_corpus() {
    let mut ok = 0;
    let mut excluded_count = 0;
    let mut failures: Vec<(String, String)> = Vec::new();

    for path in corpus_files() {
        let txt = fs::read_to_string(&path).unwrap();
        let name = corpus_rel(&path);
        if let Some(reason) = excluded(&name) {
            println!("excluded {name}: {reason}");
            excluded_count += 1;
            continue;
        }
        // Only canonical single-RM-object roots (a top-level `_type`). A file
        // that is NOT one is a registry decision, never a silent shape skip:
        // absorbing it here is how an exclusion stops applying with no gate
        // noticing (the drift this gate is unified against).
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&txt)
        else {
            failures.push((
                name,
                "non-object JSON root with no `excluded()` entry — adjudicate it in \
                 tests/it/common.rs or fix the fixture"
                    .to_owned(),
            ));
            continue;
        };
        // A `_type`-less root is LEGAL when the declared slot is concrete; the
        // class then comes from the corpus context, derived by
        // `declared_root_type`. Neither present nor derivable = a gap to
        // adjudicate, never a silent skip.
        let Some(ty) = map
            .get("_type")
            .and_then(|v| v.as_str())
            .or_else(|| crate::common::declared_root_type(&path))
        else {
            failures.push((
                name,
                "no top-level `_type`, no derivable declared root type, and no \
                 `excluded()` entry — adjudicate it in tests/it/common.rs or fix \
                 the fixture"
                    .to_owned(),
            ));
            continue;
        };
        match deserialize_as(ty, &txt) {
            Ok(()) => ok += 1,
            Err(e) => failures.push((name, e)),
        }
    }

    println!(
        "openEHR_SDK corpus: {ok} read OK, \
         {excluded_count} excluded (documented), {} failed",
        failures.len()
    );
    for (f, e) in failures.iter().take(30) {
        println!("\n--- FAILED: {f}\n  {e}");
    }
    assert!(ok > 0, "no corpus files were read");
    assert!(
        failures.is_empty(),
        "{} canonical corpus file(s) failed to deserialize into the generated RM types",
        failures.len()
    );
}

/// Excluded from the *round-trip* gate only (they read fine, but their input is
/// not itself faithfully re-emittable): a malformed field the parser correctly
/// drops. Kept separate from [`excluded`] so the readability gate still covers
/// them.
fn roundtrip_only_excluded(name: &str) -> Option<&'static str> {
    let n = name.replace('\\', "/");
    let n = n.strip_prefix("openehr_sdk/").unwrap_or(&n);
    match n {
        // `feeder_system_audit` is placed directly on an INSTRUCTION; it is a
        // field of FEEDER_AUDIT and belongs under `feeder_audit`. We correctly
        // ignore the misplaced key, so it cannot round-trip.
        "composition/canonical_json/all_types_systematic_tests_feeder_audit.json" => Some(
            "malformed fixture: `feeder_system_audit` misplaced on LOCATABLE (belongs under `feeder_audit`)",
        ),
        _ => None,
    }
}

/// The strong gate: every canonical file **round-trips losslessly** — deserialize
/// into the generated RM, re-serialize, and confirm the output is semantically
/// equal to the input (see [`semantic_eq`] for the three tolerated, information-
/// preserving differences). This is what proves nothing is silently dropped.
#[test]
fn generated_rm_round_trips_the_openehr_sdk_corpus() {
    let mut ok = 0;
    let mut excluded_count = 0;
    let mut failures: Vec<(String, String)> = Vec::new();

    for path in corpus_files() {
        let txt = fs::read_to_string(&path).unwrap();
        let name = corpus_rel(&path);
        if excluded(&name).is_some() || roundtrip_only_excluded(&name).is_some() {
            excluded_count += 1;
            continue;
        }
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&txt)
        else {
            continue; // non-canonical root (handled by the readability gate)
        };
        // `_type`-less roots are legal on a concrete declared slot; the class
        // comes from the corpus context (see `common::declared_root_type`), so
        // they join this gate instead of being silently skipped.
        let Some(ty) = map
            .get("_type")
            .and_then(|v| v.as_str())
            .or_else(|| crate::common::declared_root_type(&path))
        else {
            continue;
        };
        let input: serde_json::Value = serde_json::from_str(&txt).unwrap();
        match reserialize(ty, &txt) {
            Ok(output) => match semantic_eq(&input, &output, "") {
                Ok(()) => ok += 1,
                Err(e) => failures.push((name, e)),
            },
            Err(e) => failures.push((name, e)),
        }
    }

    println!(
        "openEHR_SDK round-trip: {ok} lossless, {excluded_count} excluded (documented), {} failed",
        failures.len()
    );
    for (f, e) in failures.iter().take(30) {
        println!("\n--- LOSSY: {f}\n  {e}");
    }
    assert!(ok > 0, "no corpus files were round-tripped");
    assert!(
        failures.is_empty(),
        "{} canonical corpus file(s) did not round-trip losslessly",
        failures.len()
    );
}

/// Schema-validation gate: every canonical file, re-serialized through the
/// generated RM types, **validates against the vendored ITS-JSON schema**
/// (`openehr_rm_1.1.0_all.json`). This proves our output conforms to the openEHR
/// JSON contract, not merely that it round-trips. All 53 canonical files
/// conform with no schema-specific exclusions (none of the current corpus
/// carries an RM 1.2.0-only attribute); the CLOSED schema's exact
/// 1.1.0↔1.2.0 attribute delta — what WOULD fail here and why — is
/// machine-pinned by `its_json_delta.rs` (#1697). Only the shared
/// readability/round-trip exclusions apply.
#[test]
fn generated_rm_output_validates_against_its_json_schema() {
    let mut ok = 0;
    let mut excluded_count = 0;
    let mut failures: Vec<(String, String)> = Vec::new();

    for path in corpus_files() {
        let txt = fs::read_to_string(&path).unwrap();
        let name = corpus_rel(&path);
        if excluded(&name).is_some() || roundtrip_only_excluded(&name).is_some() {
            excluded_count += 1;
            continue;
        }
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&txt)
        else {
            continue;
        };
        // `_type`-less roots are legal on a concrete declared slot; the class
        // comes from the corpus context (see `common::declared_root_type`), so
        // they join this gate instead of being silently skipped.
        let Some(ty) = map
            .get("_type")
            .and_then(|v| v.as_str())
            .or_else(|| crate::common::declared_root_type(&path))
        else {
            continue;
        };
        match reserialize(ty, &txt) {
            Ok(output) => match validate_canonical(&output) {
                Ok(()) => ok += 1,
                Err(errs) => failures.push((name, errs.join("; "))),
            },
            Err(e) => failures.push((name, e)),
        }
    }

    println!(
        "ITS-JSON schema validation: {ok} conformant, {excluded_count} excluded, {} failed",
        failures.len()
    );
    for (f, e) in failures.iter().take(30) {
        println!("\n--- INVALID: {f}\n  {}", preview_str(e));
    }
    assert!(ok > 0, "no corpus files were schema-validated");
    assert!(
        failures.is_empty(),
        "{} corpus file(s) failed ITS-JSON schema validation",
        failures.len()
    );
}

fn preview_str(s: &str) -> String {
    match s.get(..300) {
        Some(head) => format!("{head}…"),
        None => s.to_string(),
    }
}
