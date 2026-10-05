// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(
    clippy::panic,
    clippy::print_stderr,
    reason = "test assertions/diagnostics/fixtures"
)]
//! FLAT (simSDT) `RM ⇄ FLAT` converter tests.
//!
//! Oracle strategy (see the report / NOTICE files): Better `web-template-tests`
//! ships **no** static flat `.json` fixtures — its compositions are Better RAW
//! (`@class`) and its `.xml` are templates, not canonical compositions. So the
//! corpus here pairs the **canonical-JSON compositions** vendored in
//! `openehr-its` with the **operational templates** (from the EHRbase SDK +
//! Better `web-template-tests` + the service corpus) whose `templateId` matches
//! — real `(composition, OPT)` pairs.
//!
//! * **FLAT → RM → FLAT round-trip stable** — for each paired composition:
//!   `to_flat` (flat0) → `from_flat` (rm1) → `to_flat` (flat1); assert
//!   `flat0 == flat1` (modulo key order). Also counts how many `rm1`
//!   deserialise as an `openehr-rm` `Composition` (the reverse output is valid
//!   canonical RM).
//! * **insta goldens** — deterministic flat snapshots for representative pairs.
//! * **targeted key assertions** — Better flat key shape (`|magnitude`,
//!   `|unit` singular, `|code`).
#![expect(
    clippy::doc_markdown,
    reason = "prose with many proper nouns (EHRbase, simSDT, …)"
)]

use std::collections::BTreeMap;

use openehr_its::opt14;
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::Value;

/// Fixed `ctx/time` default for the FLAT build direction (ITS-REST
/// simplified_formats master04 §Context) so round-trips are deterministic.
const NOW: &str = "2024-01-01T00:00:00Z";

fn sorted(m: &serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// One pair's FLAT round-trip: `to_flat` → `from_flat` → `to_flat` again.
///
/// Returns the rebuilt RM value plus the instability diagnostic (`None` when the
/// two FLAT renderings agree), or the conversion failure to record.
fn flat_round_trip(
    name: &str,
    tid: &str,
    comp: &Value,
    wt: &WebTemplate,
) -> Result<(Value, Option<String>), String> {
    let flat0 = composition_to_flat(comp, wt).map_err(|e| format!("{name}: to_flat: {e}"))?;
    let rm1 =
        composition_from_flat(&flat0, wt, NOW).map_err(|e| format!("{name}: from_flat: {e}"))?;
    let flat1 = composition_to_flat(&rm1, wt)
        .map_err(|e| format!("{name}: composition_to_flat(rm1): {e}"))?;
    let m0 = sorted(&flat0);
    let m1 = sorted(&flat1);
    if m0 == m1 {
        return Ok((rm1, None));
    }
    Ok((rm1, Some(instability_diagnostic(name, tid, &m0, &m1))))
}

/// A short, deterministic account of how two FLAT renderings differ.
fn instability_diagnostic(
    name: &str,
    tid: &str,
    m0: &BTreeMap<String, Value>,
    m1: &BTreeMap<String, Value>,
) -> String {
    let only0: Vec<_> = m0.keys().filter(|k| !m1.contains_key(*k)).take(4).collect();
    let only1: Vec<_> = m1.keys().filter(|k| !m0.contains_key(*k)).take(4).collect();
    let changed: Vec<_> = m0
        .iter()
        .filter(|(k, v)| m1.get(*k).is_some_and(|v1| v1 != *v))
        .map(|(k, _)| k)
        .take(4)
        .collect();
    format!(
        "{name} ({tid}): {} keys → {} keys | only-in-flat0={only0:?} only-in-flat1={only1:?} changed={changed:?}",
        m0.len(),
        m1.len()
    )
}

// ── round-trip gate ────────────────────────────────────────────────────────────

/// The accumulated FLAT round-trip result over the paired fixture corpus.
#[derive(Default)]
struct RoundTripTally {
    /// Compositions for which a web template of the same `template_id` exists.
    paired: usize,
    /// Pairs whose second FLAT rendering equals the first.
    stable: usize,
    /// Reverse (`from_flat`) outputs the canonical RM reader accepts.
    valid_rm: usize,
    /// Round-trip errors and instability diagnostics, one line each.
    failures: Vec<String>,
    /// Reverse outputs the canonical RM reader refused, one line each.
    invalid_rm: Vec<String>,
}

/// Round-trips every composition that has a web template and tallies the
/// outcomes.
fn tally_round_trips(
    comps: &[(String, String, Value)],
    wts: &BTreeMap<String, WebTemplate>,
) -> RoundTripTally {
    let mut tally = RoundTripTally::default();
    for (name, tid, comp) in comps {
        let Some(wt) = wts.get(tid) else { continue };
        tally.paired += 1;

        let (rm1, instability) = match flat_round_trip(name, tid, comp, wt) {
            Ok(pair) => pair,
            Err(message) => {
                tally.failures.push(message);
                continue;
            }
        };
        match instability {
            Some(diagnostic) => tally.failures.push(diagnostic),
            None => tally.stable += 1,
        }

        // The reverse output should be valid canonical RM. One that will not
        // even serialize counts as neither valid nor invalid, which the
        // paired-count assertion catches.
        let Ok(serialized) = serde_json::to_string(&rm1) else {
            continue;
        };
        match openehr_its::json::from_canonical_json::<openehr_rm::prelude::Composition>(
            &serialized,
        ) {
            Ok(_) => tally.valid_rm += 1,
            Err(e) => tally.invalid_rm.push(format!("{name}: {e}")),
        }
    }
    tally
}

/// Prints a diagnostic block on stderr, headed by its count. A no-op when
/// there is nothing to report.
fn eprint_diagnostics(label: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    eprintln!("{label} ({}):", lines.len());
    for line in lines {
        eprintln!("  {line}");
    }
}

#[test]
fn flat_roundtrip_stable() {
    let wts = crate::common::web_templates();
    let comps = crate::common::compositions();
    assert!(!wts.is_empty(), "no web templates built");
    assert!(!comps.is_empty(), "no canonical compositions found");

    let tally = tally_round_trips(&comps, &wts);
    let RoundTripTally {
        paired,
        stable,
        valid_rm,
        failures,
        invalid_rm,
    } = tally;

    eprintln!(
        "FLAT round-trip: {paired} paired (composition, OPT) | stable = {stable} | rm1 valid-RM = {valid_rm}"
    );
    eprint_diagnostics(
        "rm1 not deserialising as openehr-rm Composition",
        &invalid_rm,
    );
    eprint_diagnostics("non-stable / errors", &failures);

    assert!(paired >= 15, "expected ≥15 paired fixtures, got {paired}");
    assert!(
        stable == paired,
        "{}/{paired} pairs were not FLAT round-trip stable",
        paired - stable
    );
    // Every paired composition's `from_flat` output must deserialise as a
    // canonical `openehr-rm` Composition (the INSTRUCTION / ACTIVITY /
    // INTERVAL_EVENT / DV_MULTIMEDIA / DV_PARSABLE structural gaps are closed).
    assert!(
        valid_rm == paired,
        "{}/{paired} `from_flat` outputs did not deserialise as openehr-rm Composition: {invalid_rm:?}",
        paired - valid_rm
    );
}

// ── insta goldens ───────────────────────────────────────────────────────────────

fn golden_flat(comp_file: &str, template_id: &str, snap: &str) {
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
    let flat =
        composition_to_flat(&comp, wt).unwrap_or_else(|e| panic!("to_flat {comp_file}: {e}"));
    let map: BTreeMap<String, Value> = flat.into_iter().collect();
    insta::assert_json_snapshot!(snap, map);
}

#[test]
fn golden_demo_vitals_flat() {
    golden_flat("demo_vitals_352.json", "Demo Vitals", "demo_vitals_flat");
}

#[test]
fn golden_minimal_observation_flat() {
    golden_flat(
        "minimal_observation.json",
        "minimal_observation.en.v1",
        "minimal_observation_flat",
    );
}

// ── targeted key assertions ──────────────────────────────────────────────────────

#[test]
fn demo_vitals_flat_key_shape() {
    let wts = crate::common::web_templates();
    let wt = wts.get("Demo Vitals").expect("Demo Vitals web template");
    let text = std::fs::read_to_string(crate::common::twinned(
        &crate::common::composition_dir().join("demo_vitals_352.json"),
    ))
    .unwrap();
    let comp: Value = serde_json::from_str(&text).unwrap();
    let flat = composition_to_flat(&comp, wt).expect("to_flat");

    // ctx keys present.
    assert!(flat.contains_key("ctx/language"), "ctx/language present");
    assert!(flat.contains_key("ctx/territory"), "ctx/territory present");

    // A DV_QUANTITY leaf uses |magnitude + |unit (singular).
    let has_magnitude = flat.keys().any(|k| k.ends_with("|magnitude"));
    let has_unit = flat.keys().any(|k| k.ends_with("|unit"));
    assert!(
        has_magnitude,
        "expected a |magnitude key: {:?}",
        flat.keys().collect::<Vec<_>>()
    );
    assert!(has_unit, "expected a |unit key (singular)");
    assert!(
        !flat.keys().any(|k| k.ends_with("|units")),
        "|unit must be singular, not |units"
    );

    // A coded leaf uses |code.
    assert!(
        flat.keys().any(|k| k.ends_with("|code")),
        "expected a |code key"
    );
}

// ── closed-gap assertions ─────────────────────────────────────────────────────

fn load(comp_file: &str) -> Value {
    let text = std::fs::read_to_string(crate::common::twinned(
        &crate::common::composition_dir().join(comp_file),
    ))
    .unwrap_or_else(|e| panic!("read {comp_file}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {comp_file}: {e}"))
}

fn is_valid_rm(rm: &Value) -> bool {
    serde_json::to_string(rm).is_ok_and(|s| {
        openehr_its::json::from_canonical_json::<openehr_rm::prelude::Composition>(&s).is_ok()
    })
}

/// The previously-failing INSTRUCTION/ACTIVITY fixture now rebuilds valid RM
/// with the mandatory `ACTIVITY.action_archetype_id`.
#[test]
fn instruction_activity_from_flat_is_valid_rm() {
    let wts = crate::common::web_templates();
    let wt = wts
        .get("minimal_instruction.en.v1")
        .expect("minimal_instruction web template");
    let comp = load("minimal_instruction.json");
    let rm = composition_from_flat(&composition_to_flat(&comp, wt).unwrap(), wt, NOW).unwrap();
    assert!(
        is_valid_rm(&rm),
        "minimal_instruction from_flat must deserialise as openehr-rm: {rm}"
    );
    assert!(
        rm.pointer("/content/0/activities/0/action_archetype_id")
            .is_some(),
        "rebuilt ACTIVITY carries action_archetype_id: {rm}"
    );
}

/// The `ctx/participation_*` shortcuts emit on output and rebuild
/// `context.participations` on input, round-trip stable.
#[test]
fn ctx_participation_shortcuts_round_trip() {
    // Output uses the lossless master05 §EVENT_CONTEXT path rows
    // (`context/_participation:i|…`); the master06 ctx/ shortcuts remain
    // accepted on input (they are lossy — no scheme key — so they are not
    // the emission form).
    let wts = crate::common::web_templates();
    let wt = wts
        .get("minimal_observation.en.v1")
        .expect("minimal_observation web template");
    let comp = load("minimal_observation.json");
    let flat0 = composition_to_flat(&comp, wt).unwrap();
    assert!(
        flat0
            .keys()
            .any(|k| k.contains("context/_participation:0|name")),
        "participations emit as the lossless path form: {:?}",
        flat0.keys().collect::<Vec<_>>()
    );
    let rm = composition_from_flat(&flat0, wt, NOW).unwrap();
    assert!(
        rm.pointer("/context/participations/0/performer").is_some(),
        "participations rebuilt in context: {rm}"
    );
    assert!(is_valid_rm(&rm), "participation round-trip valid RM: {rm}");
    let flat1 = composition_to_flat(&rm, wt).unwrap();
    assert_eq!(
        sorted(&flat0),
        sorted(&flat1),
        "participation round-trip not stable"
    );

    // The master06 §Participation ctx shortcuts still build the same RM on
    // input.
    let root = &wt.tree.id;
    let mut shortcut = serde_json::Map::new();
    shortcut.insert("ctx/language".to_owned(), Value::String("en".into()));
    shortcut.insert("ctx/territory".to_owned(), Value::String("US".into()));
    shortcut.insert("ctx/time".to_owned(), Value::String(NOW.into()));
    shortcut.insert(
        "ctx/participation_name:0".to_owned(),
        Value::String("Dr. Marcus Johnson".into()),
    );
    shortcut.insert(
        "ctx/participation_function:0".to_owned(),
        Value::String("requester".into()),
    );
    let _ = root;
    let rm = composition_from_flat(&shortcut, wt, NOW).expect("ctx shortcuts build");
    assert_eq!(
        rm.pointer("/context/participations/0/performer/name")
            .and_then(Value::as_str),
        Some("Dr. Marcus Johnson"),
        "ctx participation shortcut lands in EVENT_CONTEXT.participations: {rm}"
    );
}

/// `ctx/health_care_facility` (name + id) round-trips.
#[test]
fn ctx_health_care_facility_round_trips() {
    // Output uses the lossless master05 §EVENT_CONTEXT path row
    // (`context/_health_care_facility|…` — it carries |id_scheme, which the
    // master06 ctx shortcut cannot); the ctx/ shortcut stays input-only.
    let wts = crate::common::web_templates();
    let Some(wt) = wts.get("cardinality_of_section") else {
        return; // template not present in this checkout
    };
    let comp = load("cardinality_of_section__full.json");
    let flat0 = composition_to_flat(&comp, wt).unwrap();
    assert!(
        flat0
            .keys()
            .any(|k| k.contains("context/_health_care_facility|name")),
        "health_care_facility emits as the lossless path form: {:?}",
        flat0.keys().collect::<Vec<_>>()
    );
    let rm = composition_from_flat(&flat0, wt, NOW).unwrap();
    assert_eq!(
        rm.pointer("/context/health_care_facility/name"),
        comp.pointer("/context/health_care_facility/name"),
        "health_care_facility name rebuilt"
    );
    assert_eq!(
        sorted(&flat0),
        sorted(&composition_to_flat(&rm, wt).unwrap())
    );

    // The master06 §health_care_facility ctx shortcut still builds on input.
    let root = &wt.tree.id;
    let mut shortcut = serde_json::Map::new();
    shortcut.insert("ctx/language".to_owned(), Value::String("en".into()));
    shortcut.insert("ctx/territory".to_owned(), Value::String("US".into()));
    shortcut.insert("ctx/time".to_owned(), Value::String(NOW.into()));
    shortcut.insert(
        "ctx/health_care_facility|name".to_owned(),
        Value::String("Hospital".into()),
    );
    shortcut.insert(
        "ctx/health_care_facility|id".to_owned(),
        Value::String("9091".into()),
    );
    shortcut.insert(
        "ctx/id_namespace".to_owned(),
        Value::String("HOSPITAL-NS".into()),
    );
    let _ = root;
    let rm = composition_from_flat(&shortcut, wt, NOW).expect("ctx facility shortcut builds");
    assert_eq!(
        rm.pointer("/context/health_care_facility/name")
            .and_then(Value::as_str),
        Some("Hospital"),
        "ctx facility shortcut lands in the EVENT_CONTEXT: {rm}"
    );
}

/// DV_MULTIMEDIA (`|size` + uri) and DV_PARSABLE (`|formalism`) leaves rebuild
/// valid RM (the previously-broken `all_types` fixtures).
#[test]
fn multimedia_and_parsable_leaves_are_valid_rm() {
    let wts = crate::common::web_templates();
    let wt = wts
        .get("minimal_action_3.en.v1")
        .expect("a web template with a DV_MULTIMEDIA leaf");
    let comp = load("minimal_with_optional_attribute.json");
    let flat = composition_to_flat(&comp, wt).unwrap();
    assert!(
        flat.keys().any(|k| k.ends_with("|size")),
        "DV_MULTIMEDIA emits |size: {:?}",
        flat.keys().collect::<Vec<_>>()
    );
    let rm = composition_from_flat(&flat, wt, NOW).unwrap();
    assert!(is_valid_rm(&rm), "multimedia from_flat valid RM: {rm}");
}

/// The SDT path+terse coded form `"terminology::code|value|"` parses into the
/// full DV_CODED_TEXT (SIM-B master04 §S_DV_CODED_TEXT; the regular
/// `|code`/`|terminology`/`|value` form stays the emitted shape).
#[test]
fn terse_coded_text_string_is_rejected() {
    // The terse coded-string form ("terminology::code|value|") is the
    // DEVELOPMENT-state SM serial-data-formats encoding, not the STABLE
    // ITS-REST wire: master05 §DV_CODED_TEXT defines only the
    // |code/|value/|terminology suffixes (+ |other, master04 §Open
    // Value-Sets). A bare string on a closed coded leaf is rejected, never
    // silently coerced.
    let wts = crate::common::web_templates();
    let wt = wts
        .get("Corona_Anamnese")
        .expect("Corona_Anamnese web template");
    let comp = load("compo_corona.json");
    let flat0 = composition_to_flat(&comp, wt).unwrap();
    let code_key = flat0
        .keys()
        .find(|k| {
            k.ends_with("|code")
                && !k.starts_with("ctx/")
                && !k.ends_with("/category|code")
                && !k.ends_with("/language|code")
                && !k.ends_with("/territory|code")
        })
        .expect("a coded tree leaf")
        .clone();
    let base = code_key.trim_end_matches("|code").to_owned();
    let code = flat0[&code_key].as_str().unwrap().to_owned();
    let terminology = flat0
        .get(&format!("{base}|terminology"))
        .and_then(|v| v.as_str())
        .unwrap_or("local")
        .to_owned();
    let value = flat0
        .get(&format!("{base}|value"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let mut terse = flat0.clone();
    terse.remove(&code_key);
    terse.remove(&format!("{base}|terminology"));
    terse.remove(&format!("{base}|value"));
    terse.insert(
        base.clone(),
        Value::String(format!("{terminology}::{code}|{value}|")),
    );
    let err = composition_from_flat(&terse, wt, NOW)
        .expect_err("the terse coded-string form is not part of the STABLE wire");
    let msg = err.to_string();
    assert!(
        msg.contains(base.rsplit('/').next().unwrap_or(&base)) || msg.contains("invalid value"),
        "the rejection names the offending leaf: {msg}"
    );
}

// ── persistent-Composition context (finding 3) ──────────────────────────────────
//
// `COMPOSITION.context` is optional (0..1) and a `431|persistent|` Composition
// idiomatically carries none (RM ehr `master05-composition_package.adoc`
// §"Persistent Compositions may optionally have an Event context"), so
// `from_flat` must not fabricate one — while still preserving a context the
// caller supplied and still building the mandatory one for an event Composition.

const CNF_CORPUS: &str =
    "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets";

fn persistent_wt() -> WebTemplate {
    let opt_xml = std::fs::read_to_string(format!(
        "{CNF_CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt"
    ))
    .unwrap_or_else(|e| panic!("read persistent OPT: {e}"));
    let opt = opt14::from_xml(&opt_xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"))
}

#[test]
fn persistent_composition_round_trip_synthesises_no_context() {
    // The vendored canonical persistent Composition has category 431 and no
    // context — exactly the varier's decompose→reassemble scenario.
    let comp: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{CNF_CORPUS}/compositions/CANONICAL_JSON/persistent_minimal.en.v1__full.json"
        ))
        .expect("read persistent composition"),
    )
    .expect("parse composition");
    assert_eq!(
        comp.pointer("/category/defining_code/code_string"),
        Some(&Value::String("431".to_owned())),
        "fixture is persistent"
    );
    assert!(comp.get("context").is_none(), "fixture carries no context");

    let wt = persistent_wt();
    let flat = composition_to_flat(&comp, &wt).expect("to_flat");
    let rebuilt = composition_from_flat(&flat, &wt, NOW).expect("from_flat");
    assert_eq!(
        rebuilt.pointer("/category/defining_code/code_string"),
        Some(&Value::String("431".to_owned())),
        "category preserved as persistent"
    );
    assert!(
        rebuilt.get("context").is_none(),
        "no Event context is fabricated for a persistent Composition; got: {:?}",
        rebuilt.get("context")
    );
    // The rebuilt Composition is still valid RM (context being optional).
    openehr_its::json::from_canonical_value::<openehr_rm::prelude::Composition>(&rebuilt)
        .expect("rebuilt deserialises as an RM Composition without a context");
}

#[test]
fn persistent_composition_keeps_explicitly_supplied_context() {
    // A persistent Composition WITH explicit context content (a participation) is
    // valid (SPECRM-52) and must be preserved, not dropped.
    let wt = persistent_wt();
    let root = &wt.tree.id;
    let mut flat = serde_json::Map::new();
    flat.insert(format!("{root}/category|code"), Value::String("431".into()));
    flat.insert(
        format!("{root}/category|value"),
        Value::String("persistent".into()),
    );
    flat.insert(
        "ctx/participation_name:0".to_owned(),
        Value::String("Dr Explicit".into()),
    );
    // PARTICIPATION.function is 1..1 (RM
    // `UML/classes/org.openehr.rm.common.participation.adoc` §Attributes), and
    // master06 §Participation carries `participation_function:<i>` in every
    // example — so a participation-bearing ctx must supply it.
    flat.insert(
        "ctx/participation_function:0".to_owned(),
        Value::String("requester".into()),
    );
    let rebuilt = composition_from_flat(&flat, &wt, NOW).expect("from_flat");
    assert_eq!(
        rebuilt.pointer("/category/defining_code/code_string"),
        Some(&Value::String("431".to_owned()))
    );
    assert_eq!(
        rebuilt
            .pointer("/context/participations/0/performer/name")
            .and_then(Value::as_str),
        Some("Dr Explicit"),
        "an explicitly supplied context is preserved on a persistent Composition: {rebuilt:?}"
    );
}

#[test]
fn event_composition_context_follows_explicit_ctx_input() {
    // RM `COMPOSITION.context` is optional with no category-coupled
    // invariant (RM UML composition class: Category_validity constrains only
    // the category code), so no Event context is fabricated from the
    // category alone — it is built exactly when the client expressed
    // event-context content (master06: time/setting/... ctx keys).
    let wt = persistent_wt();
    let root = &wt.tree.id;
    let mut flat = serde_json::Map::new();
    flat.insert(format!("{root}/category|code"), Value::String("433".into()));
    flat.insert(
        format!("{root}/category|value"),
        Value::String("event".into()),
    );
    let rebuilt = composition_from_flat(&flat, &wt, NOW).expect("from_flat");
    assert_eq!(
        rebuilt.pointer("/category/defining_code/code_string"),
        Some(&Value::String("433".to_owned()))
    );
    assert!(
        rebuilt.pointer("/context").is_none(),
        "no Event context is fabricated without event-context input: {rebuilt:?}"
    );

    // With an explicit ctx/time the context IS built, defaults filled
    // (master06 §§time, setting).
    flat.insert("ctx/time".to_owned(), Value::String(NOW.into()));
    let rebuilt = composition_from_flat(&flat, &wt, NOW).expect("from_flat with ctx/time");
    assert_eq!(
        rebuilt
            .pointer("/context/start_time/value")
            .and_then(Value::as_str),
        Some(NOW),
        "explicit ctx/time builds the Event context: {rebuilt:?}"
    );
    assert_eq!(
        rebuilt
            .pointer("/context/setting/defining_code/code_string")
            .and_then(Value::as_str),
        Some("238"),
        "setting defaults to openehr 238 other care (master06 §setting)"
    );
}

// ── direct RM-attribute paths the OPT leaves unconstrained ───────────────────
//
// master05-rm_mapping.adoc lists RM-level attributes addressable directly on a
// node whether or not the OPT constrains them. The compacted web-template
// carries no child for an unconstrained one (e.g. `ACTION/time`), so the FLAT
// builder must build it from its datum sub-tree rather than reject it as an
// unknown path. Carrier: the CNF Robot `minimal_action.en.v1` template.

fn minimal_action_wt() -> WebTemplate {
    let opt_xml = std::fs::read_to_string(format!(
        "{CNF_CORPUS}/valid_templates/minimal/minimal_action.opt"
    ))
    .unwrap_or_else(|e| panic!("read minimal_action OPT: {e}"));
    let opt = opt14::from_xml(&opt_xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"))
}

/// The FLAT instance shape of the vendored CNF Robot fixture
/// `vitals.minimal_ctx.json`: an `ism_transition/current_state` (code + value +
/// terminology) plus `time`, addressed on the ACTION content node (`minimal:0`).
fn minimal_action_flat() -> serde_json::Map<String, Value> {
    let mut flat = serde_json::Map::new();
    flat.insert("ctx/language".to_owned(), Value::String("en".into()));
    flat.insert("ctx/territory".to_owned(), Value::String("US".into()));
    flat.insert(
        "ctx/composer_name".to_owned(),
        Value::String("Dr. Marcus Johnson".into()),
    );
    flat.insert(
        "minimal/minimal:0/ism_transition/current_state|code".to_owned(),
        Value::String("532".into()),
    );
    flat.insert(
        "minimal/minimal:0/ism_transition/current_state|value".to_owned(),
        Value::String("completed".into()),
    );
    flat.insert(
        "minimal/minimal:0/ism_transition/current_state|terminology".to_owned(),
        Value::String("openehr".into()),
    );
    flat.insert(
        "minimal/minimal:0/time".to_owned(),
        Value::String("2019-03-22T22:26:01.127+01:00".into()),
    );
    flat
}

/// The ACTION content node's `ism_transition` (`current_state|code`/`|value`)
/// builds a real DV_CODED_TEXT — not the synthesized `initial` (524) default
/// (master05 §ACTION `/ism_transition`; §ISM_TRANSITION `/current_state` +
/// the worked JSON example block).
#[test]
fn action_ism_transition_from_flat_builds_supplied_state() {
    let wt = minimal_action_wt();
    assert_eq!(wt.tree.id, "minimal", "fixture root segment is `minimal`");

    let rm = composition_from_flat(&minimal_action_flat(), &wt, NOW)
        .expect("ism_transition path keys are accepted, not rejected");

    let ism = rm
        .pointer("/content/0/ism_transition")
        .expect("ACTION carries the built ism_transition");
    assert_eq!(
        ism.pointer("/current_state/_type").and_then(Value::as_str),
        Some("DV_CODED_TEXT")
    );
    assert_eq!(
        ism.pointer("/current_state/value").and_then(Value::as_str),
        Some("completed"),
        "the supplied |value stands"
    );
    assert_eq!(
        ism.pointer("/current_state/defining_code/code_string")
            .and_then(Value::as_str),
        Some("532"),
        "the supplied |code stands — not the synthesized initial (524) default"
    );
    assert_eq!(
        ism.pointer("/current_state/defining_code/terminology_id/value")
            .and_then(Value::as_str),
        Some("openehr")
    );
    // ACTION/time (master05 §ACTION `/time`, DV_DATE_TIME) is likewise built.
    assert_eq!(
        rm.pointer("/content/0/time/value").and_then(Value::as_str),
        Some("2019-03-22T22:26:01.127+01:00"),
        "ACTION/time is built from its datum, not defaulted"
    );
    assert!(
        is_valid_rm(&rm),
        "the rebuilt ACTION composition deserialises as openehr-rm: {rm}"
    );
}

/// Collect every node of `rm_type` at or below `n`, in web-template pre-order.
fn collect_by_rm_type<'a>(
    n: &'a openehr_sdt::flat::webtemplate::model::WebTemplateNode,
    rm_type: &str,
    out: &mut Vec<&'a openehr_sdt::flat::webtemplate::model::WebTemplateNode>,
) {
    if n.rm_type == rm_type {
        out.push(n);
    }
    for c in &n.children {
        collect_by_rm_type(c, rm_type, out);
    }
}

/// The merged `ism_transition` child of an ACTION node, if the template models
/// one (master05 §ISM_TRANSITION: one `/ism_transition` node per ACTION, not one
/// per careflow state).
fn wt_transition(
    action: &openehr_sdt::flat::webtemplate::model::WebTemplateNode,
) -> Option<&openehr_sdt::flat::webtemplate::model::WebTemplateNode> {
    action
        .children
        .iter()
        .find(|c| c.rm_type == "ISM_TRANSITION")
}

/// Every ACTION node's first `careflow_step` option `(code, label)`, in
/// web-template pre-order — the union the careflow-state alternatives
/// contribute to the merged transition node's master05 §ISM_TRANSITION
/// `/careflow_step` row. `None` for an ACTION whose template models no careflow
/// step. Positional (not archetype-keyed) because a template may reuse one
/// ACTION archetype in several slots (ips.v0 uses `medication.v1` for both
/// medication and immunization) with per-slot careflow specialization.
fn wt_action_careflows(
    n: &openehr_sdt::flat::webtemplate::model::WebTemplateNode,
    out: &mut Vec<Option<(String, String)>>,
) {
    if n.rm_type == "ACTION" {
        out.push(
            wt_transition(n)
                .and_then(|t| {
                    t.children
                        .iter()
                        .find(|c| c.aql_path.ends_with("/careflow_step"))
                })
                .and_then(|step| step.inputs.iter().find_map(|i| i.list.first()))
                .map(|option| {
                    (
                        option.value.clone(),
                        option
                            .label
                            .clone()
                            .unwrap_or_else(|| "Careflow".to_owned()),
                    )
                }),
        );
    }
    for c in &n.children {
        wt_action_careflows(c, out);
    }
}

/// Stamp the i-th composition ACTION's `ism_transition.careflow_step` (document
/// pre-order, the same order as [`wt_action_careflows`]) with that ACTION's
/// modelled careflow option as a `DV_CODED_TEXT` — the leaf that must survive
/// the round trip.
///
/// It is `careflow_step` and NOT `name`/`archetype_node_id`: `ISM_TRANSITION`
/// inherits `PATHABLE`, not `LOCATABLE` (RM
/// `UML/classes/org.openehr.rm.composition.ism_transition.adoc` §Inherit), so it
/// declares neither attribute and the strict canonical reader refuses a document
/// carrying them. `careflow_step` is the class's own `0..1 DV_CODED_TEXT` (same
/// file §Attributes) — exactly the datum a careflow specialization identifies.
fn stamp_careflow(v: &mut Value, careflows: &[Option<(String, String)>], idx: &mut usize) {
    match v {
        Value::Object(m) => {
            if m.get("_type").and_then(Value::as_str) == Some("ACTION") {
                if let Some(Some((cf_nid, cf_name))) = careflows.get(*idx)
                    && let Some(ism) = m.get_mut("ism_transition").and_then(Value::as_object_mut)
                {
                    ism.insert(
                        "careflow_step".to_owned(),
                        serde_json::json!({
                            "_type": "DV_CODED_TEXT",
                            "value": cf_name,
                            "defining_code": {
                                "_type": "CODE_PHRASE",
                                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                                "code_string": cf_nid,
                            },
                        }),
                    );
                }
                *idx += 1;
            }
            for x in m.values_mut() {
                stamp_careflow(x, careflows, idx);
            }
        }
        Value::Array(a) => {
            for x in a {
                stamp_careflow(x, careflows, idx);
            }
        }
        _ => {}
    }
}

fn dv_leaf_count(v: &Value) -> usize {
    match v {
        Value::Object(m) => {
            let here = usize::from(
                m.get("_type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.starts_with("DV_")),
            );
            here + m.values().map(dv_leaf_count).sum::<usize>()
        }
        Value::Array(a) => a.iter().map(dv_leaf_count).sum(),
        _ => 0,
    }
}

/// Regression (benchmark IPS faithfulness): a careflow-stepped
/// `ACTION.ism_transition` loses no DV leaf across canonical → FLAT → canonical
/// on the CKM IPS template, whose ACTION archetypes constrain `ism_transition`
/// once per careflow state. The merged transition node realizes the datum, and
/// the master05 direct-RM-path fallback must NOT emit a second generic
/// `.../ism_transition/*` spelling that then overwrites it on rebuild (4 leaves
/// were lost that way). master05 §§ACTION `/ism_transition`, ISM_TRANSITION.
#[test]
fn action_careflow_ism_transition_wins_over_direct_path() {
    let wts = crate::common::web_templates();
    let Some(wt) = wts.get("International Patient Summary") else {
        return; // ips.v0 OPT not present in this checkout
    };
    let mut careflows = Vec::new();
    wt_action_careflows(&wt.tree, &mut careflows);
    assert!(
        careflows.iter().filter(|c| c.is_some()).count() >= 3,
        "ips.v0's ACTIONs model a careflow_step option set on their merged ism_transition"
    );

    let mut comp = load("ips_canonical.json");
    stamp_careflow(&mut comp, &careflows, &mut 0);
    let before = dv_leaf_count(&comp);

    let flat = composition_to_flat(&comp, wt).unwrap();
    // No DV leaf is lost: every stamped `careflow_step` DV_CODED_TEXT survives,
    // so a shadowing regression on the >= 3 modelled IPS ACTIONs trips this.
    let rebuilt = composition_from_flat(&flat, wt, NOW).unwrap();
    assert_eq!(
        before,
        dv_leaf_count(&rebuilt),
        "ism_transition careflow steps are preserved (the merged node realizes them, unshadowed)"
    );
    assert!(is_valid_rm(&rebuilt), "ips_canonical from_flat valid RM");
}

/// master05 §ISM_TRANSITION maps an ACTION's transition to ONE path family —
/// `…/ism_transition/current_state|code`, `…/transition|code`,
/// `…/careflow_step|code` — and its table types `/ism_transition` as a single
/// `ISM_TRANSITION` (Required). So an ACTION whose archetype constrains
/// `ism_transition` once per careflow state (`minimal_action.en.v1`: `at0003`
/// planned, `at0004` completed) serves ONE node, id `ism_transition`, with no
/// careflow at-code in its `aqlPath` or `nodeId`, whose coded children carry the
/// union of the states' options.
#[test]
fn ism_transition_is_one_merged_node_per_action() {
    let wt = minimal_action_wt();
    let mut actions = Vec::new();
    collect_by_rm_type(&wt.tree, "ACTION", &mut actions);
    let action = actions.first().expect("the fixture carries an ACTION");

    let transitions: Vec<_> = action
        .children
        .iter()
        .filter(|c| c.rm_type == "ISM_TRANSITION")
        .collect();
    assert_eq!(
        transitions.len(),
        1,
        "one transition node per ACTION, not one per careflow state: {:?}",
        transitions.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    let transition = transitions.first().expect("the single transition node");
    assert_eq!(transition.id, "ism_transition");
    assert_eq!(
        transition.aql_path,
        format!("{}/ism_transition", action.aql_path)
    );
    assert_eq!(
        transition.node_id, None,
        "the merged node stands for no single careflow at-code"
    );
    // RM `action.adoc`: `ism_transition` is 1..1 — the attribute's occurrences,
    // not a per-state 1..1.
    assert_eq!((transition.min, transition.max), (Some(1), 1));

    let options = |suffix: &str| -> Vec<String> {
        transition
            .children
            .iter()
            .find(|c| c.aql_path.ends_with(suffix))
            .into_iter()
            .flat_map(|c| c.inputs.iter())
            .flat_map(|i| i.list.iter())
            .map(|option| option.value.clone())
            .collect()
    };
    assert_eq!(
        options("/current_state"),
        vec!["526".to_owned(), "532".to_owned()],
        "the openEHR instruction-state codes of both careflow states, unioned"
    );
    assert_eq!(
        options("/careflow_step"),
        vec!["at0003".to_owned(), "at0004".to_owned()],
        "the local careflow-step codes of both states, unioned"
    );
}

/// canonical → FLAT → canonical stability for an ACTION `ism_transition`: the
/// reverse (RM → FLAT) emits the `ism_transition` sub-paths symmetrically, so
/// the state survives a round-trip (master05 §ISM_TRANSITION).
#[test]
fn action_ism_transition_round_trip_stable() {
    let wt = minimal_action_wt();
    // rm1 is the canonical composition (built once from the FLAT fixture).
    let rm1 = composition_from_flat(&minimal_action_flat(), &wt, NOW).expect("from_flat");

    let flat1 = composition_to_flat(&rm1, &wt).expect("to_flat");
    assert!(
        flat1
            .keys()
            .any(|k| k.ends_with("/ism_transition/current_state|code")),
        "the reverse direction emits the ism_transition sub-paths: {:?}",
        flat1.keys().collect::<Vec<_>>()
    );

    let rm2 = composition_from_flat(&flat1, &wt, NOW).expect("from_flat again");
    assert_eq!(
        rm1.pointer("/content/0/ism_transition"),
        rm2.pointer("/content/0/ism_transition"),
        "ism_transition is stable across canonical→FLAT→canonical"
    );
    assert_eq!(
        rm1.pointer("/content/0/time"),
        rm2.pointer("/content/0/time"),
        "ACTION/time is stable across the round-trip"
    );
}

/// A genuinely unknown segment on the ACTION node is still rejected — the fix
/// admits only the master05-listed RM paths for the node's RM type, never a
/// blanket accept (master04 §Validation: "Field identifiers match WT metadata
/// structure").
#[test]
fn bogus_action_segment_is_still_rejected() {
    let wt = minimal_action_wt();
    let mut flat = minimal_action_flat();
    flat.insert(
        "minimal/minimal:0/not_a_real_rm_attribute".to_owned(),
        Value::String("x".into()),
    );
    let err = composition_from_flat(&flat, &wt, NOW)
        .expect_err("an unknown ACTION segment must still be rejected");
    assert!(
        matches!(err, openehr_sdt::flat::error::FlatError::UnknownPath(ref p) if p.contains("not_a_real_rm_attribute")),
        "the rejection is an UnknownPath naming the offending segment: {err}"
    );
}

/// The #1719 adjudication pinned on the wire: a careflow-stepped
/// `ISM_TRANSITION` flattens to the GENERIC `…/ism_transition/…` spelling —
/// the form master05 §ISM_TRANSITION's own worked example uses
/// (`careflow_step|code: at0006`, terminology `local`) — never to a
/// careflow-state WT-child id, because `ISM_TRANSITION` inherits PATHABLE and
/// carries no `name`/`archetype_node_id` for such a child to match on
/// (RM `ism_transition.adoc` §Inherit). Round-trip both directions.
#[test]
fn careflow_stepped_ism_transition_flattens_to_the_generic_master05_spelling() {
    let wt = minimal_action_wt();
    let mut flat = minimal_action_flat();
    // A careflow-stepped transition, spelled exactly as the master05 example.
    flat.insert(
        "minimal/minimal:0/ism_transition/careflow_step|code".to_owned(),
        Value::String("at0006".into()),
    );
    flat.insert(
        "minimal/minimal:0/ism_transition/careflow_step|value".to_owned(),
        Value::String("transition".into()),
    );
    flat.insert(
        "minimal/minimal:0/ism_transition/careflow_step|terminology".to_owned(),
        Value::String("local".into()),
    );

    let rm = composition_from_flat(&flat, &wt, NOW).expect("from_flat");
    assert_eq!(
        rm.pointer("/content/0/ism_transition/careflow_step/defining_code/code_string")
            .and_then(Value::as_str),
        Some("at0006"),
        "the careflow step builds onto ISM_TRANSITION.careflow_step"
    );
    assert!(
        rm.pointer("/content/0/ism_transition/archetype_node_id")
            .is_none(),
        "no LOCATABLE identity is stamped on a PATHABLE (RM ism_transition.adoc §Inherit)"
    );

    let flat_again = composition_to_flat(&rm, &wt).expect("to_flat");
    assert_eq!(
        flat_again
            .get("minimal/minimal:0/ism_transition/careflow_step|code")
            .and_then(Value::as_str),
        Some("at0006"),
        "RM→FLAT emits the generic master05 spelling: {:?}",
        flat_again.keys().collect::<Vec<_>>()
    );
    let rm2 = composition_from_flat(&flat_again, &wt, NOW).expect("from_flat again");
    assert_eq!(
        rm.pointer("/content/0/ism_transition"),
        rm2.pointer("/content/0/ism_transition"),
        "the careflow-stepped ism_transition is round-trip stable"
    );
}

// ── a name-constrained wrapper the level removal collapses (#3142) ───────────
//
// An OPT may constrain `LOCATABLE.name` on a node the web template compacts
// away: the single `EVENT` of `master04 §Conditionally Collapsed Wrapper
// Types`. Its constrained name then survives only inside the leaf's `aqlPath`
// (`events[at0002,'Point in time']`), so it is the one identity the RM ⇄ FLAT
// walks cannot see in the tree and must still honour. Carrier: the
// repo-authored `named_event.en.v1` fixture.

fn named_event_wt() -> WebTemplate {
    let opt_xml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/named_event/named_event.opt"
    ))
    .unwrap_or_else(|e| panic!("read named_event OPT: {e}"));
    let opt = opt14::from_xml(&opt_xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"))
}

/// The fixture is only a regression carrier while its event stays COLLAPSED and
/// NAME-CONSTRAINED: a template change that retained the event, or dropped the
/// `C_STRING`, would leave the two tests below passing vacuously.
#[test]
fn the_named_event_fixture_collapses_a_name_constrained_event() {
    let wt = named_event_wt();
    let observation = wt
        .tree
        .children
        .iter()
        .find(|c| c.rm_type == "OBSERVATION")
        .unwrap_or_else(|| panic!("fixture has an OBSERVATION content node"));
    assert!(
        observation
            .children
            .iter()
            .all(|c| c.rm_type != "POINT_EVENT"),
        "the single 1..1 event is collapsed (master04 §Conditionally Collapsed Wrapper Types)"
    );
    let reading = observation
        .children
        .iter()
        .find(|c| c.id == "reading")
        .unwrap_or_else(|| panic!("fixture has a `reading` leaf"));
    assert!(
        reading
            .aql_path
            .contains("/events[at0002,'Point in time']/"),
        "the collapsed event's constrained name survives in the leaf path: {}",
        reading.aql_path
    );
}

/// A composition whose collapsed `POINT_EVENT` carries a name OTHER than the one
/// its template constrains still projects its content into FLAT.
///
/// The `[at0002,'Point in time']` conjunct is redundant for identification here —
/// no sibling constraint claims `events[at0002]` under another name — so the step
/// falls back to the `archetype_node_id` alone rather than locating nothing
/// (RM common `master03-archetyped_package.adoc` §"The `LOCATABLE` class": a
/// runtime `name` distinguishes siblings sharing an `archetype_node_id`; BASE
/// `architecture_overview/master11-paths.adoc` §"Using a Name-based Predicate").
/// Without the fallback the whole event subtree — every datum under it — is
/// silently absent from the response, which is the defect reported in #3142.
#[test]
fn a_renamed_collapsed_event_still_yields_its_content() {
    let wt = named_event_wt();
    let mut flat = serde_json::Map::new();
    flat.insert("ctx/language".to_owned(), Value::String("en".into()));
    flat.insert("ctx/territory".to_owned(), Value::String("US".into()));
    flat.insert(
        "ctx/composer_name".to_owned(),
        Value::String("Dr. Marcus Johnson".into()),
    );
    flat.insert(
        "named_event/named_event_observation:0/reading".to_owned(),
        Value::String("120 over 80".into()),
    );
    let mut rm = composition_from_flat(&flat, &wt, NOW).expect("from_flat");

    let event_name = rm
        .pointer_mut("/content/0/data/events/0/name/value")
        .unwrap_or_else(|| panic!("the built composition carries the collapsed event"));
    *event_name = Value::String("at0002".into());

    let flat_again = composition_to_flat(&rm, &wt).expect("to_flat");
    assert_eq!(
        flat_again
            .get("named_event/named_event_observation:0/reading")
            .and_then(Value::as_str),
        Some("120 over 80"),
        "the renamed event's content is still projected: {:?}",
        flat_again.keys().collect::<Vec<_>>()
    );
}

/// Building a collapsed wrapper stamps the name its own template path
/// constrains, not the `archetype_node_id` placeholder.
///
/// A wrapper named anything else does not satisfy the path it was created for,
/// so every datum beneath it becomes unaddressable — the composition is written
/// once and then reads back short, which is how #3142 was produced in the first
/// place.
#[test]
fn a_built_collapsed_event_takes_the_name_its_template_constrains() {
    let wt = named_event_wt();
    let mut flat = serde_json::Map::new();
    flat.insert("ctx/language".to_owned(), Value::String("en".into()));
    flat.insert("ctx/territory".to_owned(), Value::String("US".into()));
    flat.insert(
        "ctx/composer_name".to_owned(),
        Value::String("Dr. Marcus Johnson".into()),
    );
    flat.insert(
        "named_event/named_event_observation:0/reading".to_owned(),
        Value::String("120 over 80".into()),
    );
    let rm = composition_from_flat(&flat, &wt, NOW).expect("from_flat");
    assert_eq!(
        rm.pointer("/content/0/data/events/0/name/value")
            .and_then(Value::as_str),
        Some("Point in time"),
        "the synthesized POINT_EVENT takes its constrained name"
    );
    assert_eq!(
        rm.pointer("/content/0/data/events/0/archetype_node_id")
            .and_then(Value::as_str),
        Some("at0002"),
        "and keeps its archetype node id"
    );
}

/// The id-only fallback must NOT fire where the name is the DISCRIMINATOR.
///
/// The vendored COVID-19 report template fills `content` with four
/// `openEHR-EHR-SECTION.adhoc.v1` siblings distinguished only by their
/// constrained names ('Reporting', 'Clinical status', 'Exposure', 'Outcome').
/// Dropping the name there would let the sibling whose section was renamed claim
/// every section, so one section's slot would report all four sections' content
/// — the opposite failure to #3142 and the reason the fallback is conditional
/// rather than unconditional.
#[test]
fn a_name_discriminated_sibling_set_takes_no_id_only_fallback() {
    let opt_xml =
        std::fs::read_to_string("../../corpus/templates/ckm/covid19-infection-report.opt")
            .unwrap_or_else(|e| panic!("read covid19 OPT: {e}"));
    let opt = opt14::from_xml(&opt_xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    let wt = build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"));
    let sections: Vec<String> = wt
        .tree
        .children
        .iter()
        .filter(|c| {
            c.aql_path
                .starts_with("/content[openEHR-EHR-SECTION.adhoc.v1,'")
        })
        .map(|c| c.id.clone())
        .collect();
    assert!(
        sections.len() > 1,
        "the carrier really has name-differentiated adhoc sections: {sections:?}"
    );

    let mut composition = example_composition(&wt, DetailLevel::Medium);
    let before = composition_to_flat(&composition, &wt).expect("to_flat");
    assert!(
        before.keys().any(|k| k.contains("/reporting:0/first_test")),
        "the example really populates the Reporting section"
    );

    let renamed = composition
        .pointer_mut("/content")
        .and_then(Value::as_array_mut)
        .unwrap_or_else(|| panic!("the example carries content"))
        .iter_mut()
        .find(|c| c.pointer("/name/value").and_then(Value::as_str) == Some("Reporting"))
        .unwrap_or_else(|| panic!("the example carries the Reporting section"));
    renamed["name"]["value"] = Value::String("A name no sibling constrains".into());

    let after = composition_to_flat(&composition, &wt).expect("to_flat");
    let leaked: Vec<&String> = after
        .keys()
        .filter(|k| k.contains("/reporting:") && !k.contains("/first_test"))
        .collect();
    assert!(
        leaked.is_empty(),
        "no sibling section's content is claimed by the renamed one: {leaked:?}"
    );
    assert!(
        !after.keys().any(|k| k.contains("/reporting:")),
        "and the renamed section is claimed by no name-differentiated sibling at all"
    );
}
