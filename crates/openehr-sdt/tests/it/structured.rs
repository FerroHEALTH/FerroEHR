// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! STRUCTURED (structSDT) `RM ⇄ STRUCTURED` converter tests.
//!
//! STRUCTURED composes the pure nesting transform
//! ([`openehr_sdt::flat::convert::flat_to_structured`] /
//! [`openehr_sdt::flat::convert::structured_to_flat`]) with the FLAT converter, so
//! the same `(composition, OPT)` corpus pairs used
//! by `flat.rs` drive these gates:
//!
//! * **STRUCTURED → RM → STRUCTURED stable** — `to_structured` (s0) →
//!   `from_structured` (rm1) → `to_structured` (s1); assert `s0 == s1`, and
//!   count how many `rm1` deserialise as an `openehr-rm` `Composition`.
//! * **flat ⇄ structured exact inverses** — `flat_to_structured` and
//!   `structured_to_flat` round-trip the vendored flat maps exactly at the
//!   structured level.
//! * **cross-format consistency** — `to_structured` and `to_flat` on the same
//!   composition carry identical leaf values (index-normalised).
//! * **insta goldens** — deterministic structured snapshots.
#![expect(
    clippy::doc_markdown,
    reason = "the module docs quote openEHR spec prose and Simplified-Formats key names as text, not as Rust code references"
)]

use std::collections::BTreeMap;

use openehr_sdt::flat::convert::{
    composition_from_structured, composition_to_flat, composition_to_structured,
    flat_to_structured, structured_to_flat,
};
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::Value;

/// Fixed `ctx/time` default for the STRUCTURED build direction (ITS-REST
/// simplified_formats master04 §Context) so round-trips are deterministic.
const NOW: &str = "2024-01-01T00:00:00Z";

/// Drop every `:index` from a flat key, leaving path + `|suffix`.
fn strip_indices(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut skipping = false;
    for c in key.chars() {
        match c {
            ':' => skipping = true,
            '/' | '|' => {
                skipping = false;
                out.push(c);
            }
            c if skipping && c.is_ascii_digit() => {}
            c => {
                skipping = false;
                out.push(c);
            }
        }
    }
    out
}

/// A flat map keyed by index-stripped key (a value-set comparable across index
/// conventions). Collisions (a genuinely repeating leaf) keep the last value —
/// acceptable for the cross-format value check.
fn index_normalised(map: &serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    map.iter()
        .map(|(k, v)| (strip_indices(k), v.clone()))
        .collect()
}

// ── STRUCTURED round-trip + RM validation ─────────────────────────────────────

/// One pair's STRUCTURED round-trip: `to_structured` → `from_structured` →
/// `to_structured` again.
///
/// Returns the rebuilt RM value and whether the two STRUCTURED renderings agree,
/// or the conversion failure to record.
fn structured_round_trip(
    name: &str,
    comp: &Value,
    wt: &WebTemplate,
) -> Result<(Value, bool), String> {
    let s0 =
        composition_to_structured(comp, wt).map_err(|e| format!("{name}: to_structured: {e}"))?;
    let rm1 = composition_from_structured(&s0, wt, NOW)
        .map_err(|e| format!("{name}: from_structured: {e}"))?;
    let s1 = composition_to_structured(&rm1, wt)
        .map_err(|e| format!("{name}: to_structured(rm1): {e}"))?;
    let is_stable = s0 == s1;
    Ok((rm1, is_stable))
}

#[test]
fn structured_roundtrip_and_rm_validation() {
    let wts = crate::common::web_templates();
    let comps = crate::common::compositions();
    assert!(!wts.is_empty(), "no web templates built");
    assert!(!comps.is_empty(), "no canonical compositions found");

    let mut paired = 0usize;
    let mut stable = 0usize;
    let mut valid_rm = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut invalid_rm: Vec<String> = Vec::new();

    for (name, tid, comp) in &comps {
        let Some(wt) = wts.get(tid) else { continue };
        paired += 1;

        let (rm1, is_stable) = match structured_round_trip(name, comp, wt) {
            Ok(pair) => pair,
            Err(message) => {
                failures.push(message);
                continue;
            }
        };
        if is_stable {
            stable += 1;
        } else {
            failures.push(format!("{name} ({tid}): STRUCTURED not stable"));
        }

        if let Ok(s) = serde_json::to_string(&rm1) {
            match openehr_its::json::from_canonical_json::<openehr_rm::prelude::Composition>(&s) {
                Ok(_) => valid_rm += 1,
                Err(e) => invalid_rm.push(format!("{name}: {e}")),
            }
        }
    }

    eprintln!(
        "STRUCTURED round-trip: {paired} paired | stable = {stable} | rm valid-RM = {valid_rm}"
    );
    for f in &invalid_rm {
        eprintln!("  invalid-RM {f}");
    }
    for f in &failures {
        eprintln!("  {f}");
    }

    assert!(paired >= 15, "expected ≥15 paired fixtures, got {paired}");
    assert!(
        stable == paired,
        "{}/{paired} STRUCTURED pairs were not round-trip stable",
        paired - stable
    );
    assert!(
        valid_rm == paired,
        "{}/{paired} STRUCTURED `from_structured` outputs did not deserialise as openehr-rm: {invalid_rm:?}",
        paired - valid_rm
    );
}

// ── flat ⇄ structured exact inverses ──────────────────────────────────────────

#[test]
fn flat_structured_exact_inverses() {
    let wts = crate::common::web_templates();
    let comps = crate::common::compositions();
    let mut paired = 0usize;
    let mut structured_exact = 0usize;
    let mut flat_exact = 0usize;

    for (_name, tid, comp) in &comps {
        let Some(wt) = wts.get(tid) else { continue };
        let Ok(flat) = composition_to_flat(comp, wt) else {
            continue;
        };
        paired += 1;
        let f = flat.clone();

        let s = flat_to_structured(&f).expect("flat_to_structured");
        // structured → flat → structured is exact by construction.
        let s2 = flat_to_structured(&structured_to_flat(&s).expect("structured_to_flat"))
            .expect("flat_to_structured");
        if s == s2 {
            structured_exact += 1;
        }
        // flat → structured → flat is exact up to single-occurrence `:0` drop.
        if structured_to_flat(&s).expect("structured_to_flat") == f {
            flat_exact += 1;
        }
    }

    eprintln!(
        "flat⇄structured: {paired} paired | structured-exact = {structured_exact} | flat-exact = {flat_exact}"
    );
    assert!(paired >= 15);
    assert!(
        structured_exact == paired,
        "structured round-trip (structured→flat→structured) not exact for {}/{paired}",
        paired - structured_exact
    );
}

// ── cross-format value consistency ────────────────────────────────────────────

#[test]
fn structured_and_flat_carry_identical_leaves() {
    let wts = crate::common::web_templates();
    let comps = crate::common::compositions();
    let mut checked = 0usize;

    for (name, tid, comp) in &comps {
        let Some(wt) = wts.get(tid) else { continue };
        let Ok(flat) = composition_to_flat(comp, wt) else {
            continue;
        };
        let Ok(structured) = composition_to_structured(comp, wt) else {
            continue;
        };
        let via_structured = structured_to_flat(&structured).expect("structured_to_flat");
        let a = index_normalised(&flat);
        let b = index_normalised(&via_structured);
        assert_eq!(
            a, b,
            "{name} ({tid}): to_flat and to_structured carry different leaf values"
        );
        checked += 1;
    }
    assert!(
        checked >= 15,
        "expected ≥15 checked fixtures, got {checked}"
    );
}

// ── insta goldens ─────────────────────────────────────────────────────────────

fn golden_structured(comp_file: &str, template_id: &str, snap: &str) {
    let wts = crate::common::web_templates();
    let wt = wts
        .get(template_id)
        .unwrap_or_else(|| panic!("no web template for {template_id:?}"));
    let text = std::fs::read_to_string(crate::common::twinned(
        &crate::common::composition_dir().join(comp_file),
    ))
    .unwrap_or_else(|e| panic!("read {comp_file}: {e}"));
    let comp: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {comp_file}: {e}"));
    let structured =
        composition_to_structured(&comp, wt).unwrap_or_else(|e| panic!("to_structured: {e}"));
    insta::assert_json_snapshot!(snap, structured);
}

#[test]
fn golden_demo_vitals_structured() {
    golden_structured(
        "demo_vitals_352.json",
        "Demo Vitals",
        "demo_vitals_structured",
    );
}

#[test]
fn golden_minimal_observation_structured() {
    golden_structured(
        "minimal_observation.json",
        "minimal_observation.en.v1",
        "minimal_observation_structured",
    );
}
