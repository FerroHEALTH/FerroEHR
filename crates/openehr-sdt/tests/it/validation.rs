// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! End-to-end composition-validation tests.
//!
//! Oracle: the already-vendored, **Apache-2.0** `openEHR_SDK` corpus — canonical
//! JSON compositions in `openehr-its/tests/vendor/openehr_sdk/…` paired with the
//! OPT 1.4 operational templates in `openehr-its/tests/fixtures/…` by
//! `template_id`. Each valid composition is validated against the `WebTemplate`
//! built from its template and must be clean; invalid variants (one real vendored
//! `*_invalid` pair plus mutations of valid corpus compositions) must be rejected
//! with a sensible path + kind.
//!
//! NOTE: the CNF `test_data_sets` are not vendored here — the
//! `specifications-CNF` repository is CC BY-SA 3.0, incompatible with this
//! fixture tree's Apache-2.0 vendoring — so the Apache-2.0 `openEHR_SDK`
//! corpus (`openehr-its/tests/vendor/` + `tests/fixtures/sdk/`) serves as the
//! same class of oracle, including a real valid/invalid pair.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use openehr_its::opt14;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::{
    ValidationKind, ValidationMessage, validate_composition, validate_rm_and_terminology,
};
use serde_json::{Value, json};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn opt_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            opt_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "opt") {
            out.push(p);
        }
    }
}

/// `template_id` → `WebTemplate` for every parseable vendored OPT.
///
/// Several vendored fixtures share one template id (`Demo Vitals` has three
/// variants), and `read_dir` order is OS-dependent — first-wins over an
/// unspecified order made the winning variant differ between macOS and Linux
/// (a CI-only corpus failure: Linux's order left a variant whose constraints
/// flag the corpus node that the curation baseline accepts). Deterministic
/// resolution: sorted paths, first wins — the ordering the `CLEAN_COMPOSITIONS`
/// curation was verified against, identical on every OS.
fn web_templates() -> HashMap<String, WebTemplate> {
    let mut opts = Vec::new();
    opt_files(&manifest_dir().join("tests/fixtures"), &mut opts);
    opt_files(
        &manifest_dir().join("../openehr-its/tests/fixtures"),
        &mut opts,
    );
    opts.sort();
    let mut wts = HashMap::new();
    for p in &opts {
        let Ok(xml) = std::fs::read_to_string(p) else {
            continue;
        };
        let Ok(opt) = opt14::from_xml(&xml) else {
            continue;
        };
        if let Ok(wt) = build_web_template(&opt) {
            wts.entry(opt.template_id.value.clone()).or_insert(wt);
        }
    }
    wts
}

fn composition(name: &str) -> Value {
    let p = manifest_dir()
        .join("../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json")
        .join(name);
    let txt = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&txt).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

/// A composition's declared template id.
fn template_id(c: &Value) -> String {
    c.get("archetype_details")
        .and_then(|a| a.get("template_id"))
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn validate(name: &str, wts: &HashMap<String, WebTemplate>) -> Vec<ValidationMessage> {
    let c = composition(name);
    let tid = template_id(&c);
    let wt = wts
        .get(&tid)
        .unwrap_or_else(|| panic!("no WebTemplate for template id {tid:?} (composition {name})"));
    validate_composition(&c, wt)
}

fn kinds(msgs: &[ValidationMessage]) -> Vec<ValidationKind> {
    msgs.iter().map(|m| m.kind).collect()
}

/// The curated set of vendored valid compositions whose WebTemplate-guided
/// validation is expected to be completely clean. (Other corpus compositions
/// deliberately omit `archetype_details` on nested archetype-root entries, which
/// the RM `Is_archetype_root` invariant — surfaced from `openehr-rm` — correctly
/// flags; those are excluded here, not silently tolerated.)
///
/// The vendored corpus mixes openEHR reference data with **EHRbase-SDK** data,
/// and `EHRbase` is *lenient*: some SDK fixtures omit RM-mandatory attributes that
/// a strict validator rightly rejects. Per the project's conformance stance (openEHR spec conformance,
/// not `EHRbase` parity) we do **not** tolerate that leniency, so such fixtures are
/// excluded here with the exact spec violation named — they are not a valid
/// oracle for "validates clean". Excluded on those grounds:
/// - `rawdb_composition.json` — its `composer.external_ref` is a `PARTY_REF`
///   missing the mandatory `type` (`PARTY_REF.type [1]`, RM support) whose `id`
///   is a `GENERIC_ID` missing the mandatory `value` (`OBJECT_ID.value [1]`, RM
///   support). Strict typed validation surfaces both.
/// - `all_types_no_multimedia.json` — carries schemeless `DV_URI` values
///   (`www.iana.org`): rejected under the CNF-mandated absolute-reference
///   rule (CNF `platform_test_schedule` master17.7 `xyz | rejected | value
///   doesn't comply with RFC3986`; RM `data_types` `dv_uri` Description
///   "structurally conforms to ... RFC-3986").
/// - `minimal_action2_1.json` — carries a `DV_PROPORTION` `{type: 3
///   (pk_fraction), precision: 1}`: invalid under RM `data_types`
///   `dv_proportion` `Fraction_validity` ("(type = `pk_fraction` or type =
///   `pk_integer_fraction`) implies `is_integral`", with `is_integral()` =
///   "True ... if precision is 0"). It ALSO violates the archetype-root
///   node-id identity rule: its `/content[0]` ACTION carries an `ARCHETYPED`
///   block copied from the COMPOSITION root, so `archetype_node_id` differs
///   from `archetype_details.archetype_id.value` (RM
///   `org.openehr.rm.common.locatable.adoc` §Attributes,
///   `archetype_node_id`).
/// - `informe_amb_1_arquetip_OBS.json` — the same root-copied `ARCHETYPED`
///   defect on its `/content[0]` OBSERVATION (an
///   `openEHR-EHR-COMPOSITION.*` `archetype_id` claimed by a non-COMPOSITION
///   node), violating the same identity rule.
/// - `ehrb_adbm_op_consult_record.json` — carries the `TERMINOLOGY_ID` value
///   `"SNOMED CT"`, whose interior space is outside the `terminology_id`
///   production (BASE `base_types/master05-identification_package.adoc`
///   §Syntaxes: `terminology_id = name-str, [ '(', name-str, ')' ]`,
///   `name-str = letter, { letter | digit | '_' | '-' | '/' | '+' }`). The same
///   chapter's §Terminology Identifiers spells the terminology `"SNOMED-CT"`,
///   which the production admits. The refusal is pinned by
///   [`snomed_ct_with_a_space_is_refused`].
const CLEAN_COMPOSITIONS: &[&str] = &[
    "choice_validation_test.json",
    "compo_corona.json",
    "demo_vitals_352.json",
    "dvquantity_choice.json",
    "interval_partial_date.json",
    "ips_canonical.json",
    "minimal_with_optional_attribute.json",
    "minimal_without_optional_attribute.json",
    "my_spanish_template_v0_COMPOSITION_EXAMPLE.json",
    "nested.en.v1.json",
    "other_participations.json",
    "participation_no_content.json",
    "virology_finding_with_specimen_no_update.json",
];

#[test]
fn valid_corpus_compositions_validate_clean() {
    let wts = web_templates();
    let mut failures = Vec::new();
    for name in CLEAN_COMPOSITIONS {
        let msgs = validate(name, &wts);
        if !msgs.is_empty() {
            failures.push(format!(
                "{name}: {} unexpected violation(s): {:?}",
                msgs.len(),
                msgs.iter()
                    .map(|m| format!("[{:?}] {}", m.kind, m.path))
                    .collect::<Vec<_>>()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "expected these vendored valid compositions to validate clean:\n{}",
        failures.join("\n")
    );
}

/// Strict mandatory-attribute enforcement: removing the mandatory
/// `COMPOSITION.composer [1]` must be rejected. The node codec stores raw
/// canonical JSON with no schema enforcement at commit, so a typed-deserialize
/// failure that was once silently swallowed (committing 201) must now surface
/// the failure → 422.
#[test]
fn missing_mandatory_composer_is_rejected() {
    let wts = web_templates();
    let mut c = composition("nested.en.v1.json");
    let tid = template_id(&c);
    let wt = wts.get(&tid).expect("WebTemplate for nested.en.v1");
    assert!(
        validate_composition(&c, wt).is_empty(),
        "baseline nested.en.v1 must validate clean"
    );
    c.as_object_mut()
        .expect("composition object")
        .remove("composer");
    let msgs = validate_composition(&c, wt);
    assert!(
        !msgs.is_empty(),
        "a COMPOSITION with no composer must be rejected"
    );
    assert!(
        msgs.iter()
            .any(|m| m.message.to_lowercase().contains("composer") || m.path == "/"),
        "the violation should reference the missing mandatory composer: {msgs:?}"
    );
}

/// A vendored composition carrying `TERMINOLOGY_ID` `"SNOMED CT"` is ACCEPTED,
/// because the class constrains no value.
///
/// This assertion was inverted deliberately (#2314). It previously required
/// the interior space to be REFUSED, citing the `terminology_id` production in
/// BASE `base_types/master05-identification_package.adoc` §Syntaxes. That
/// production is prose in a chapter: `TERMINOLOGY_ID`'s own class table
/// declares no `Invariants` row and the BMM no `invariants` key — unlike its
/// sibling `VERSION_TREE_ID`, which declares seven — so enforcing it invented a
/// prohibition the model does not contain. Released QUERY 1.1.0
/// (`docs/specs/openehr/QUERY/docs/AQL/master03-syntax.adoc` §Node predicate)
/// settles it from the other side by publishing
/// `terminology_id/value='snomed_ct(3.1)'`, which that same production forbids.
#[test]
fn a_terminology_id_with_a_space_is_accepted() {
    let wts = web_templates();
    let msgs = validate("ehrb_adbm_op_consult_record.json", &wts);
    assert!(
        !msgs.iter().any(|m| m.path.ends_with("/terminology_id")),
        "no violation may be raised against a terminology_id, got {msgs:?}"
    );
}

// ── a real vendored valid / invalid pair (same template) ─────────────────────

#[test]
fn ips_valid_is_clean_invalid_is_rejected() {
    let wts = web_templates();

    // The valid IPS composition validates clean.
    assert!(
        validate("ips_canonical.json", &wts).is_empty(),
        "ips_canonical should be clean"
    );

    // Its intentionally-broken sibling is rejected with real domain violations:
    // out-of-range magnitudes/counts and coded values outside the value set.
    let bad = validate("ips_invalid.json", &wts);
    assert!(!bad.is_empty(), "ips_invalid should be rejected");
    let ks = kinds(&bad);
    assert!(
        ks.contains(&ValidationKind::RangeError),
        "expected a RangeError in ips_invalid, got {bad:?}"
    );
    assert!(
        ks.contains(&ValidationKind::CodedValue),
        "expected a CodedValue in ips_invalid, got {bad:?}"
    );
    // Every violation is keyed by a non-empty RM path.
    assert!(bad.iter().all(|m| !m.path.is_empty()));
}

// ── synthesized invalids (mutations of a valid corpus composition) ───────────

#[test]
fn dropping_mandatory_content_reports_required() {
    // Emptying the IPS content removes its mandatory sections (min >= 1).
    let wts = web_templates();
    let wt = wts.get("International Patient Summary").expect("IPS wt");
    let mut c = composition("ips_canonical.json");
    c["content"] = serde_json::json!([]);
    let msgs = validate_composition(&c, wt);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::Required),
        "expected Required for the dropped mandatory sections, got {msgs:?}"
    );
}

#[test]
fn wrong_root_type_is_rejected() {
    // Corrupt the composition's root RM type; it no longer conforms to COMPOSITION.
    let wts = web_templates();
    let wt = wts.get("Demo Vitals").expect("Demo Vitals wt");
    let mut c = composition("demo_vitals_352.json");
    c["_type"] = Value::String("OBSERVATION".to_owned());
    let msgs = validate_composition(&c, wt);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::WrongType && m.path.is_empty()),
        "expected a WrongType at the root, got {msgs:?}"
    );
}

#[test]
fn bad_composition_category_reports_terminology() {
    // Replace the composition category code with one outside the openEHR group.
    let wts = web_templates();
    let wt = wts.get("Demo Vitals").expect("Demo Vitals wt");
    let mut c = composition("demo_vitals_352.json");
    c["category"]["defining_code"]["code_string"] = Value::String("99999".to_owned());
    let msgs = validate_composition(&c, wt);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology
                && m.message.contains("composition category")),
        "expected a Terminology violation for the bad category, got {msgs:?}"
    );
}

#[test]
fn out_of_range_magnitude_reports_range_error() {
    // Inflate a range-constrained quantity magnitude in a clean composition.
    let wts = web_templates();
    let wt = wts.get("International Patient Summary").expect("IPS wt");
    let mut c = composition("ips_canonical.json");
    let mutated = inflate_first_quantity(&mut c);
    assert!(mutated, "expected at least one DV_QUANTITY to mutate");
    let msgs = validate_composition(&c, wt);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::RangeError),
        "expected a RangeError after inflating a magnitude, got {msgs:?}"
    );
}

/// Set every `DV_QUANTITY.magnitude` in the tree to an out-of-range value.
fn inflate_first_quantity(v: &mut Value) -> bool {
    let mut mutated = false;
    inflate_rec(v, &mut mutated);
    mutated
}

fn inflate_rec(v: &mut Value, mutated: &mut bool) {
    match v {
        Value::Object(o) => {
            if o.get("_type").and_then(Value::as_str) == Some("DV_QUANTITY")
                && o.contains_key("magnitude")
            {
                o.insert("magnitude".to_owned(), Value::from(999_999.0));
                *mutated = true;
            }
            for val in o.values_mut() {
                inflate_rec(val, mutated);
            }
        }
        Value::Array(a) => {
            for val in a {
                inflate_rec(val, mutated);
            }
        }
        _ => {}
    }
}

// ── RM invariant routing through the updated dispatcher (EVENT/HISTORY/interval) ─
//
// These prove the composition validator's RM-invariant pass routes real
// composition content through `openehr_rm::v1_2::validate::validate_rm_value`, so the
// richer invariant layer (HISTORY Period_consistency, DV_INTERVAL
// Limits_consistent) fires on actual instances — not just in openehr-rm's own
// unit tests. They use `validate_rm_and_terminology` (the template-independent
// entry point), so no OPT fixture is needed.

#[test]
fn inconsistent_history_period_surfaces_period_consistency() {
    // An OBSERVATION whose HISTORY declares a 1-hour period but carries an event
    // 30 minutes off the origin violates HISTORY.Period_consistency.
    let comp = serde_json::json!({
        "_type": "COMPOSITION", "archetype_node_id": "openEHR-EHR-COMPOSITION.t.v1",
        "content": [{
            "_type": "OBSERVATION", "archetype_node_id": "openEHR-EHR-OBSERVATION.t.v1",
            "name": {"_type": "DV_TEXT", "value": "obs"},
            "data": {
                "_type": "HISTORY", "archetype_node_id": "at0001",
                "name": {"_type": "DV_TEXT", "value": "history"},
                "origin": {"_type": "DV_DATE_TIME", "value": "2021-01-01T00:00:00"},
                "period": {"_type": "DV_DURATION", "value": "PT1H"},
                "events": [{
                    "_type": "POINT_EVENT", "archetype_node_id": "at0002",
                    "name": {"_type": "DV_TEXT", "value": "event"},
                    "time": {"_type": "DV_DATE_TIME", "value": "2021-01-01T01:30:00"},
                    "data": {"_type": "ITEM_TREE", "archetype_node_id": "at0003",
                        "name": {"_type": "DV_TEXT", "value": "tree"}}
                }]
            }
        }]
    });
    let msgs = validate_rm_and_terminology(&comp);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::Invariant
            && m.message == "Invariant Period_consistency failed on type HISTORY"),
        "expected HISTORY.Period_consistency from the dispatcher, got {msgs:?}"
    );
    // The violation is keyed by an RM instance path into the HISTORY.
    assert!(
        msgs.iter().any(
            |m| m.message.contains("Period_consistency") && m.path.contains("/content[0]/data")
        ),
        "the invariant path should point into the composition, got {msgs:?}"
    );
}

#[test]
fn inverted_dv_interval_surfaces_limits_consistent() {
    // An inverted DV_INTERVAL<DV_QUANTITY> (lower 10 kg > upper 2 kg) nested as an
    // ELEMENT value violates DV_INTERVAL.Limits_consistent — reached because the
    // dispatcher deserialises the interval element type as DV_ORDERED.
    let comp = serde_json::json!({
        "_type": "COMPOSITION", "archetype_node_id": "openEHR-EHR-COMPOSITION.t.v1",
        "content": [{
            "_type": "ELEMENT", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "range"},
            "value": {
                "_type": "DV_INTERVAL",
                "lower": {"_type": "DV_QUANTITY", "magnitude": 10.0, "units": "kg"},
                "upper": {"_type": "DV_QUANTITY", "magnitude": 2.0, "units": "kg"},
                "lower_included": true, "upper_included": true,
                "lower_unbounded": false, "upper_unbounded": false
            }
        }]
    });
    let msgs = validate_rm_and_terminology(&comp);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::Invariant
            && m.message == "Invariant Limits_consistent failed on type DV_INTERVAL"),
        "expected DV_INTERVAL.Limits_consistent from the dispatcher, got {msgs:?}"
    );
}

// ── name-differentiated same-archetype-id siblings (real OPT) ─────────────────
//
// The neurologist OPT fills `openEHR-EHR-OBSERVATION.sensorum_status.v2` twice
// under one SECTION, differentiated by `name`: one unqualified sibling (inner
// `items` closed to `at0004`) and one name-qualified (closed to
// at0013/at0086/at0041). A composition carrying BOTH is valid; each instance
// routes to its own overlay (RM common `master03-archetyped_package.adoc`
// §"The `LOCATABLE` class"; AOM 1.4 `master04` §`node_id`).

/// A `DV_CODED_TEXT` `ELEMENT` (a leaf under an `ITEM_TREE`).
fn coded_element(node_id: &str, name: &str, code: &str) -> Value {
    serde_json::json!({
        "_type": "ELEMENT", "archetype_node_id": node_id,
        "name": {"_type": "DV_TEXT", "value": name},
        "value": {"_type": "DV_CODED_TEXT", "value": name,
                  "defining_code": {"_type": "CODE_PHRASE", "code_string": code,
                                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"}}}
    })
}

/// A `sensorum_status.v2` OBSERVATION with the given runtime `name` and the given
/// leaves under its single `at0002` event's `ITEM_TREE`.
fn sensorum(name: &str, items: &[Value]) -> Value {
    serde_json::json!({
        "_type": "OBSERVATION",
        "archetype_node_id": "openEHR-EHR-OBSERVATION.sensorum_status.v2",
        "name": {"_type": "DV_TEXT", "value": name},
        "data": {"_type": "HISTORY", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "Event Series"},
            "origin": {"_type": "DV_DATE_TIME", "value": "2024-01-01T00:00:00"},
            "events": [{"_type": "POINT_EVENT", "archetype_node_id": "at0002",
                "name": {"_type": "DV_TEXT", "value": "Произвольное событие"},
                "time": {"_type": "DV_DATE_TIME", "value": "2024-01-01T00:00:00"},
                "data": {"_type": "ITEM_TREE", "archetype_node_id": "at0003",
                    "name": {"_type": "DV_TEXT", "value": "Дерево"},
                    "items": items}}]}
    })
}

#[test]
fn neurologist_both_sensorum_siblings_validate_clean() {
    let wts = web_templates();
    let wt = wts
        .get("openEHR-EHR-COMPOSITION.t_neurologist_examination(1-17)_lanit.v1")
        .expect("neurologist WebTemplate");
    let comp = serde_json::json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.t_neurologist_examination.v1",
        "name": {"_type": "DV_TEXT", "value": "Neurologist"},
        "content": [{
            "_type": "SECTION",
            "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1",
            "name": {"_type": "DV_TEXT", "value": "Общий осмотр"},
            "items": [
                sensorum("Общее состояние", &[coded_element("at0004", "Общее состояние", "at0005")]),
                sensorum(
                    "Нервно-психический статус",
                    &[
                        coded_element("at0013", "Уровень сознания", "at0014"),
                        coded_element("at0041", "Нервно-психический статус", "at0042"),
                    ],
                ),
            ]
        }]
    });
    let unexpected: Vec<ValidationMessage> = validate_composition(&comp, wt)
        .into_iter()
        .filter(|m| m.kind == ValidationKind::Unexpected)
        .collect();
    assert!(
        unexpected.is_empty(),
        "both name-differentiated sensorum_status instances must validate against \
         their own overlay (no Unexpected), got {unexpected:?}"
    );
}

// ── a WebTemplate node builder sanity check (paths line up) ───────────────────

#[test]
fn matched_violation_paths_are_archetype_paths() {
    // A rejected composition's archetype-conformance violations carry the wt
    // node's aqlPath (the archetype path), demonstrating instance↔template match.
    let wts = web_templates();
    let bad = validate("ips_invalid.json", &wts);
    let arche: Vec<&ValidationMessage> = bad
        .iter()
        .filter(|m| {
            matches!(
                m.kind,
                ValidationKind::RangeError | ValidationKind::CodedValue | ValidationKind::WrongType
            )
        })
        .collect();
    assert!(!arche.is_empty());
    for m in arche {
        assert!(
            m.path.starts_with("/content["),
            "archetype-conformance path should be an aqlPath, got {:?}",
            m.path
        );
    }
}

// ── per-rule public-seam tests (hand-shaped WebTemplate nodes / instances) ─────
//
// These exercise the public validation entry points ([`validate_composition`],
// [`validate_rm_and_terminology`]) on minimal hand-built inputs so each rule is
// checked in isolation without an OPT fixture.

fn node(rm: &str, path: &str) -> WebTemplateNode {
    WebTemplateNode::new(rm.to_owned(), path.to_owned())
}

fn wt_of(tree: WebTemplateNode) -> WebTemplate {
    WebTemplate {
        template_id: "t".to_owned(),
        sem_ver: None,
        version: "2.3".to_owned(),
        default_language: "en".to_owned(),
        languages: vec!["en".to_owned()],
        tree,
        other_details: IndexMap::new(),
    }
}

/// A bare `CODE_PHRASE` node for an external (ISO/IANA) code-set slot.
fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code,
    })
}

fn coded_text(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": "x",
        "defining_code": code_phrase(terminology, code),
    })
}

// ── RM invariant surfacing (full pipeline) ───────────────────────────────────

#[test]
fn rm_invariant_surfaces() {
    // An ELEMENT with both value AND null_flavour violates Inv_null_flavour_indicated.
    let wt = wt_of(node("COMPOSITION", ""));
    let inst = json!({
        "_type": "COMPOSITION", "archetype_node_id": "x",
        "content": [{
            "_type": "ELEMENT", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "e"},
            "value": {"_type": "DV_TEXT", "value": "v"},
            "null_flavour": {"_type": "DV_CODED_TEXT", "value": "unknown",
                "defining_code": {"_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                    "code_string": "253"}}
        }]
    });
    let msgs = validate_composition(&inst, &wt);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::Invariant
            && m.message.contains("Inv_null_flavour_indicated")),
        "expected the ELEMENT XOR invariant, got {msgs:?}"
    );
}

// ── openEHR-terminology group (full pipeline) ────────────────────────────────

#[test]
fn bad_composition_category_reported() {
    let wt = wt_of(node("COMPOSITION", ""));
    let inst = json!({
        "_type": "COMPOSITION", "archetype_node_id": "x",
        "archetype_details": {"_type": "ARCHETYPED",
            "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.x.v1"},
            "rm_version": "1.0.2"},
        "category": {"_type": "DV_CODED_TEXT", "value": "bogus",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                "code_string": "99999"}}
    });
    let msgs = validate_composition(&inst, &wt);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology
                && m.message.contains("composition category")),
        "expected a bad-category Terminology violation, got {msgs:?}"
    );
}

#[test]
fn valid_composition_category_is_clean_of_terminology() {
    let wt = wt_of(node("COMPOSITION", ""));
    let inst = json!({
        "_type": "COMPOSITION", "archetype_node_id": "x",
        "category": {"_type": "DV_CODED_TEXT", "value": "event",
            "defining_code": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                "code_string": "433"}}
    });
    let msgs = validate_composition(&inst, &wt);
    assert!(
        !kinds(&msgs).contains(&ValidationKind::Terminology),
        "valid category 433 should not trip terminology, got {msgs:?}"
    );
}

// ── terminology: code-set slots (ISO / IANA) ─────────────────────────────────

#[test]
fn dv_text_bad_language_and_encoding_reported() {
    // DV_TEXT.language (ISO 639-1) and .encoding (IANA character sets).
    let bad_lang = json!({
        "_type": "DV_TEXT", "value": "hi",
        "language": code_phrase("ISO_639-1", "zz"),
    });
    let msgs = validate_rm_and_terminology(&bad_lang);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.message.contains("language")),
        "expected a bad-language Terminology violation, got {msgs:?}"
    );

    let bad_enc = json!({
        "_type": "DV_TEXT", "value": "hi",
        "encoding": code_phrase("IANA_character-sets", "NOT-A-CHARSET"),
    });
    let msgs = validate_rm_and_terminology(&bad_enc);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.message.contains("character set")),
        "expected a bad-encoding Terminology violation, got {msgs:?}"
    );

    // Valid language + encoding: clean.
    let good = json!({
        "_type": "DV_TEXT", "value": "hi",
        "language": code_phrase("ISO_639-1", "en"),
        "encoding": code_phrase("IANA_character-sets", "UTF-8"),
    });
    assert!(
        !kinds(&validate_rm_and_terminology(&good)).contains(&ValidationKind::Terminology),
        "valid en/UTF-8 should be clean"
    );
}

#[test]
fn composition_bad_territory_reported() {
    // COMPOSITION.territory (ISO 3166-1 countries).
    let inst = json!({
        "_type": "COMPOSITION", "archetype_node_id": "x",
        "language": code_phrase("ISO_639-1", "en"),
        "territory": code_phrase("ISO_3166-1", "ZZ"),
    });
    let msgs = validate_rm_and_terminology(&inst);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.message.contains("country")),
        "expected a bad-territory Terminology violation, got {msgs:?}"
    );
}

#[test]
fn entry_bad_encoding_reported() {
    // ENTRY (OBSERVATION) encoding (IANA character sets).
    let inst = json!({
        "_type": "OBSERVATION", "archetype_node_id": "a",
        "language": code_phrase("ISO_639-1", "en"),
        "encoding": code_phrase("IANA_character-sets", "BOGUS-CHARSET"),
    });
    let msgs = validate_rm_and_terminology(&inst);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.message.contains("character set")),
        "expected a bad-encoding Terminology violation on the ENTRY, got {msgs:?}"
    );
}

// ── terminology: openEHR-group slots ─────────────────────────────────────────

#[test]
fn ism_transition_bad_transition_reported() {
    // ISM_TRANSITION.transition (instruction_transitions group).
    let inst = json!({
        "_type": "ISM_TRANSITION",
        "current_state": coded_text("openehr", "245"), // active (valid)
        "transition": coded_text("openehr", "99999"),  // invalid
    });
    let msgs = validate_rm_and_terminology(&inst);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::Terminology
            && m.message.contains("instruction transition")),
        "expected a bad-transition Terminology violation, got {msgs:?}"
    );
}

#[test]
fn term_mapping_bad_purpose_reported() {
    // TERM_MAPPING.purpose (term_mapping_purpose group), reached via a
    // DV_TEXT.mappings[].
    let inst = json!({
        "_type": "DV_TEXT", "value": "hi",
        "mappings": [{
            "_type": "TERM_MAPPING", "match": "=",
            "purpose": coded_text("openehr", "99999"),
            "target": code_phrase("SNOMED-CT", "123"),
        }]
    });
    let msgs = validate_rm_and_terminology(&inst);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology
                && m.message.contains("term mapping purpose")),
        "expected a bad-purpose Terminology violation, got {msgs:?}"
    );
}

#[test]
fn party_related_bad_relationship_reported() {
    // PARTY_RELATED.relationship (subject_relationship group).
    let inst = json!({
        "_type": "PARTY_RELATED",
        "relationship": coded_text("openehr", "99999"),
    });
    let msgs = validate_rm_and_terminology(&inst);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology
                && m.message.contains("subject relationship")),
        "expected a bad-relationship Terminology violation, got {msgs:?}"
    );
}

#[test]
fn dv_ordered_bad_normal_status_reported() {
    // DV_ORDERED.normal_status (normal_statuses code set) — checked on
    // any node carrying `normal_status`.
    let inst = json!({
        "_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg",
        "normal_status": code_phrase("openehr", "X"),
    });
    let msgs = validate_rm_and_terminology(&inst);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.message.contains("normal status")),
        "expected a bad-normal-status Terminology violation, got {msgs:?}"
    );
    // A valid normal status ("N") is clean.
    let good = json!({
        "_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg",
        "normal_status": code_phrase("openehr", "N"),
    });
    assert!(!kinds(&validate_rm_and_terminology(&good)).contains(&ValidationKind::Terminology));
}

// ── LOCATABLE.Archetyped_valid ───────────────────────────────────────────────

/// A non-root node — `archetype_node_id` an `at`/`id` term code — must not
/// carry `archetype_details` (`locatable.adoc` §`Archetyped_valid`); an
/// archetype-HRID node carrying them, and a nested archetype root *without*
/// them (the CNF-corpus-sanctioned shape), both stay valid.
#[test]
fn non_root_node_with_archetype_details_rejected() {
    let details = json!({"_type": "ARCHETYPED",
        "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-CLUSTER.x.v1"},
        "rm_version": "1.0.2"});
    for bad_id in ["at0001", "id42", "at0001.1"] {
        let inst = json!({
            "_type": "CLUSTER", "archetype_node_id": bad_id,
            "name": {"_type": "DV_TEXT", "value": "c"},
            "archetype_details": details,
            "items": []
        });
        let msgs = validate_rm_and_terminology(&inst);
        assert!(
            msgs.iter()
                .any(|m| m.kind == ValidationKind::Invariant
                    && m.message.contains("Archetyped_valid")),
            "expected an Archetyped_valid violation for {bad_id}, got {msgs:?}"
        );
    }
    // Root shapes stay valid: HRID with details, and HRID without details
    // (nested archetype root as the CNF corpus ships it).
    for (id, with_details) in [
        ("openEHR-EHR-CLUSTER.x.v1", true),
        ("openEHR-EHR-CLUSTER.x.v1", false),
    ] {
        let mut inst = json!({
            "_type": "CLUSTER", "archetype_node_id": id,
            "name": {"_type": "DV_TEXT", "value": "c"},
            "items": []
        });
        if with_details {
            inst.as_object_mut()
                .unwrap()
                .insert("archetype_details".into(), details.clone());
        }
        let msgs = validate_rm_and_terminology(&inst);
        assert!(
            !msgs.iter().any(|m| m.message.contains("Archetyped_valid")),
            "root shape (details={with_details}) must not violate Archetyped_valid: {msgs:?}"
        );
    }
}

// ── "present implies non-empty" list invariants ──────────────────────────────

/// `COMPOSITION.Content_valid` / `SECTION.Items_valid` /
/// `INSTRUCTION.Activities_valid` etc.: a PRESENT empty list violates the
/// invariant; an absent list does not (`composition.adoc` §Invariants and
/// siblings).
#[test]
fn present_empty_lists_are_rejected() {
    let base = json!({
        "_type": "COMPOSITION", "archetype_node_id": "openEHR-EHR-COMPOSITION.x.v1",
        "name": {"_type": "DV_TEXT", "value": "c"}
    });
    // Absent content: no Content_valid violation.
    assert!(
        !validate_rm_and_terminology(&base)
            .iter()
            .any(|m| m.message.contains("Content_valid")),
        "absent content must not violate Content_valid"
    );
    // Present-empty content: violation.
    let mut empty = base.clone();
    empty
        .as_object_mut()
        .unwrap()
        .insert("content".into(), json!([]));
    let msgs = validate_rm_and_terminology(&empty);
    assert!(
        msgs.iter()
            .any(|m| m.message.contains("content") && m.message.contains("at least one member")),
        "present-empty content must refuse at the typed decode (#1730), got {msgs:?}"
    );
    // A nested SECTION with empty items: Items_valid violation at its path.
    let mut nested = base;
    nested.as_object_mut().unwrap().insert(
        "content".into(),
        json!([{
            "_type": "SECTION", "archetype_node_id": "at0001",
            "name": {"_type": "DV_TEXT", "value": "s"},
            "items": []
        }]),
    );
    let msgs = validate_rm_and_terminology(&nested);
    assert!(
        msgs.iter().any(|m| m.message.contains("items")
            && m.message.contains("at least one member")
            && m.path.contains("content")),
        "present-empty SECTION.items must violate Items_valid, got {msgs:?}"
    );
}

// ── data-structure shape duties ──────────────────────────────────────────────

/// `CLUSTER.items` is 1..1 (`cluster.adoc`; the ITS-JSON CLUSTER schema
/// requires it) and one `HISTORY`'s events must all carry the same
/// `ITEM_STRUCTURE` subtype in `data` (RM `data_structures` master06 §History).
#[test]
fn data_structure_shapes_are_enforced() {
    // CLUSTER without items → violation.
    let cluster = json!({
        "_type": "CLUSTER", "archetype_node_id": "at0001",
        "name": {"_type": "DV_TEXT", "value": "c"}
    });
    let msgs = validate_rm_and_terminology(&cluster);
    assert!(
        msgs.iter()
            .any(|m| m.message.contains("CLUSTER.items is mandatory")),
        "items-less CLUSTER must be rejected, got {msgs:?}"
    );

    // HISTORY mixing ITEM_TREE and ITEM_LIST event data → violation.
    let history = json!({
        "_type": "HISTORY", "archetype_node_id": "at0002",
        "name": {"_type": "DV_TEXT", "value": "h"},
        "origin": {"_type": "DV_DATE_TIME", "value": "2026-01-01T00:00:00Z"},
        "events": [
            { "_type": "POINT_EVENT", "archetype_node_id": "at0003",
              "name": {"_type": "DV_TEXT", "value": "e1"},
              "time": {"_type": "DV_DATE_TIME", "value": "2026-01-01T00:00:00Z"},
              "data": {"_type": "ITEM_TREE", "archetype_node_id": "at0004",
                       "name": {"_type": "DV_TEXT", "value": "d"}, "items": []} },
            { "_type": "POINT_EVENT", "archetype_node_id": "at0003",
              "name": {"_type": "DV_TEXT", "value": "e2"},
              "time": {"_type": "DV_DATE_TIME", "value": "2026-01-01T01:00:00Z"},
              "data": {"_type": "ITEM_LIST", "archetype_node_id": "at0004",
                       "name": {"_type": "DV_TEXT", "value": "d"}, "items": []} }
        ]
    });
    let msgs = validate_rm_and_terminology(&history);
    assert!(
        msgs.iter()
            .any(|m| m.message.contains("same ITEM_STRUCTURE") && m.path.contains("events[1]")),
        "a HISTORY mixing event data types must be rejected, got {msgs:?}"
    );
}

// ── ARCHETYPE_SLOT filler admission against a real OPT ───────────────────────

/// The `IDCR Problem List.v1.opt` corpus template, whose slot assertions carry
/// no `string_expression`.
fn idcr_problem_list() -> Result<WebTemplate, String> {
    let path = crate::common::service_corpus_dir().join("knowledge/IDCR Problem List.v1.opt");
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
    let opt = opt14::from_xml(&xml).map_err(|e| format!("opt14 parse: {e}"))?;
    build_web_template(&opt).map_err(|e| format!("build: {e}"))
}

/// A COMPOSITION whose single `content` child is an EVALUATION bearing
/// `archetype_id`.
fn problem_list_with_content(archetype_id: &str) -> Value {
    json!({
        "_type": "COMPOSITION", "archetype_node_id": "openEHR-EHR-COMPOSITION.problem_list.v1",
        "name": {"_type": "DV_TEXT", "value": "Problem list"},
        "content": [{
            "_type": "EVALUATION", "archetype_node_id": archetype_id,
            "name": {"_type": "DV_TEXT", "value": "Problem"}
        }]
    })
}

/// A slot admits exactly the archetypes its `include` assertions name: an
/// archetype outside the regex is not a legal filler (ADL 1.4
/// `master05-cadl.adoc` §Archetype Slots — "two lists of assertions statements
/// defining which archetypes are allowed and/or which are excluded from filling
/// that slot").
///
/// The commit seam is [`validate_composition`], which every write path (canonical
/// JSON, FLAT, STRUCTURED, TDD) converges on after conversion to RM.
#[test]
fn slot_filler_outside_the_includes_regex_is_refused() {
    let wt = match idcr_problem_list() {
        Ok(wt) => wt,
        Err(e) => panic!("the IDCR corpus template must build: {e}"),
    };

    let admitted = validate_composition(
        &problem_list_with_content("openEHR-EHR-EVALUATION.problem_diagnosis.v1"),
        &wt,
    );
    assert!(
        !admitted
            .iter()
            .any(|m| m.kind == ValidationKind::Unexpected),
        "the archetype the slot includes must be admitted, got {admitted:?}"
    );

    let refused = validate_composition(
        &problem_list_with_content("openEHR-EHR-EVALUATION.medication_summary.v1"),
        &wt,
    );
    assert!(
        refused.iter().any(|m| m.kind == ValidationKind::Unexpected),
        "an archetype no slot includes must be refused, got {refused:?}"
    );
}
