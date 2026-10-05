// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `v2_4` (ADL2 / OPT2) → Web Template front end
//! ([`openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4`]).
//!
//! Every case compiles a **real** ADL2 corpus source to its operational
//! template (`openehr_adl::opt::create_opt`) and drives it through the same
//! WebTemplate model + shared shaping the ADL 1.4 front end uses — the
//! dialect-neutral seam of `ITS-REST simplified_formats master04 §"Web Template
//! Metadata"`. The corpus lives in the sibling `openehr-adl` crate
//! (`corpus/adl2-reference`); `openehr-adl` is a dev-only dependency here
//! (the production `v2_4` front end takes an already-created OPT as input).

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![expect(
    clippy::doc_markdown,
    reason = "the module docs quote openEHR spec prose and Simplified-Formats key names as text, not as Rust code references"
)]
#![expect(
    clippy::items_after_statements,
    reason = "fixture helper fns are declared next to the assertions that use them, which keeps each case self-contained"
)]

use std::path::Path;

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_sdt::flat::example::{DetailLevel, ExampleType, example_composition};
use openehr_sdt::flat::validation::validate_archetype_conformance;
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateCardinality, WebTemplateNode};
use openehr_sdt::rm_instance::{ValidationKind, validate_rm_and_terminology};

const OBS_UPGRADE: &str =
    "upgrade/upgrade_from_14/openEHR-EHR-OBSERVATION.upgrade_add_use_nodes.v1.0.0.adls";
const OBS_APGAR: &str = "features/terminology/term_bindings/openEHR-EHR-OBSERVATION.term_bindings_paths_use_refs.v1.0.0.adls";
const COMP_ANNOTATIONS: &str =
    "features/description/annotations/openEHR-EHR-COMPOSITION.annotations_rm_path.v1.0.0.adls";

fn corpus_dir() -> String {
    format!(
        "{}/../openehr-adl/tests/corpus/adl2-reference",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn read(rel: &str) -> String {
    let p = format!("{}/{rel}", corpus_dir());
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

/// A repository over the whole `adl2-reference` tree so specialisation parents
/// and `use_archetype` fillers resolve during `create_opt`.
fn corpus_repo() -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    fn walk(dir: &Path, repo: &mut ArchetypeRepository) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, repo);
            } else if p.extension().is_some_and(|x| x == "adls")
                && let Ok(src) = std::fs::read_to_string(&p)
                && let Ok(a) = parse_artefact(&src, Dialect::Adl2)
            {
                repo.insert(a);
            }
        }
    }
    walk(Path::new(&corpus_dir()), &mut repo);
    repo
}

/// Compile a corpus source to its WebTemplate through the `v2_4` front end.
fn web_template(rel: &str) -> WebTemplate {
    let archetype = parse_artefact(&read(rel), Dialect::Adl2).expect("parse ADL2");
    let opt = create_opt(&archetype, &corpus_repo()).expect("create_opt");
    build_web_template_v2_4(&opt).expect("build v2_4 web template")
}

fn rm_types(node: &WebTemplateNode) -> Vec<String> {
    let mut out = vec![node.rm_type.clone()];
    for c in &node.children {
        out.extend(rm_types(c));
    }
    out
}

/// Every node (depth-first) whose RM type equals `rm_type`.
fn collect_typed<'a>(node: &'a WebTemplateNode, rm_type: &str, out: &mut Vec<&'a WebTemplateNode>) {
    if node.rm_type == rm_type {
        out.push(node);
    }
    for c in &node.children {
        collect_typed(c, rm_type, out);
    }
}

/// The first node (depth-first) whose json `id` equals `id`.
fn find<'a>(node: &'a WebTemplateNode, id: &str) -> Option<&'a WebTemplateNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, id))
}

fn unit_list(node: &WebTemplateNode) -> Vec<String> {
    node.inputs
        .iter()
        .filter(|i| i.suffix.as_deref() == Some("unit"))
        .flat_map(|i| i.list.iter().map(|c| c.value.clone()))
        .collect()
}

fn code_list(node: &WebTemplateNode) -> Vec<String> {
    node.inputs
        .iter()
        .filter(|i| i.suffix.as_deref() == Some("code"))
        .flat_map(|i| i.list.iter().map(|c| c.value.clone()))
        .collect()
}

// ── template identity + root shape ───────────────────────────────────────────

#[test]
fn carries_template_id_semver_and_interface_root_id() {
    let wt = web_template(COMP_ANNOTATIONS);
    assert_eq!(
        wt.template_id,
        "openEHR-EHR-COMPOSITION.annotations_rm_path.v1.0.0"
    );
    // OPT2 carries a semantic version (master07.05); the WT `version` is the
    // format version, `semVer` the template release version.
    assert_eq!(wt.sem_ver.as_deref(), Some("1.0.0"));
    assert_eq!(wt.version, "2.3");
    assert_eq!(wt.tree.rm_type, "COMPOSITION");
    // master04 §"Web Template Metadata": the root nodeId is the archetype id in
    // interface (major-version) form, not the internal concept code.
    assert_eq!(
        wt.tree.node_id.as_deref(),
        Some("openEHR-EHR-COMPOSITION.annotations_rm_path.v1")
    );
    assert_eq!(wt.tree.min, Some(1));
    assert_eq!(wt.tree.max, 1);
}

#[test]
fn synthesizes_the_composition_in_context_children() {
    // master04 §"Web Template Metadata": the RM-mandatory context children an OPT
    // leaves unconstrained are synthesized (shared with the OPT-1.4 front end).
    let wt = web_template(COMP_ANNOTATIONS);
    let ids: Vec<&str> = wt.tree.children.iter().map(|c| c.id.as_str()).collect();
    for expected in ["category", "language", "territory", "composer"] {
        assert!(
            ids.contains(&expected),
            "missing in-context child {expected}; got {ids:?}"
        );
    }
    let ctx = wt
        .tree
        .children
        .iter()
        .find(|c| c.rm_type == "EVENT_CONTEXT");
    assert!(ctx.is_some(), "the context wrapper is synthesized");
}

// ── generation + the validation bar (RM invariants + terminology) ────────────

#[test]
fn example_generation_validates_at_every_level() {
    // The validation bar for v2_4 examples: RM class invariants + RM-mandated
    // terminology, template-independent (module NOTE — no v2_4 structural
    // conformance walk). Every detail level must be clean.
    let wt = web_template(COMP_ANNOTATIONS);
    for level in [
        DetailLevel::Required,
        DetailLevel::Medium,
        DetailLevel::Complete,
    ] {
        let comp = example_composition(&wt, level);
        let msgs = validate_rm_and_terminology(&comp);
        assert!(
            msgs.is_empty(),
            "example at {level:?} has RM/terminology violations: {msgs:?}"
        );
        assert_eq!(
            comp.get("_type").and_then(|v| v.as_str()),
            Some("COMPOSITION")
        );
    }
}

#[test]
fn example_generation_is_deterministic() {
    let wt = web_template(COMP_ANNOTATIONS);
    let a = example_composition(&wt, DetailLevel::Medium);
    let b = example_composition(&wt, DetailLevel::Medium);
    assert_eq!(a, b, "two calls produce identical JSON");
    // The output form populates a deterministic uid.
    let mut out = a.clone();
    openehr_sdt::flat::example::apply_output_uid(&mut out, &wt.template_id);
    let mut out2 = b;
    openehr_sdt::flat::example::apply_output_uid(&mut out2, &wt.template_id);
    assert_eq!(out, out2);
    assert_eq!(
        ExampleType::from_query(Some("output")),
        Ok(ExampleType::Output)
    );
}

// ── the v2_4 inputs mapping (C_ATTRIBUTE_TUPLE / C_TERMINOLOGY_CODE) ──────────

#[test]
fn quantity_units_come_from_the_magnitude_units_tuple() {
    // AOM2 `DV_QUANTITY` constrains `[magnitude, units]` as a co-varying tuple
    // (and a plain `magnitude`/`units` form); each unit surfaces as a coded
    // `unit` value.
    let wt = web_template(OBS_UPGRADE);
    let mut quantities: Vec<&WebTemplateNode> = Vec::new();
    collect_typed(&wt.tree, "DV_QUANTITY", &mut quantities);
    let all_units: Vec<String> = quantities.iter().flat_map(|n| unit_list(n)).collect();
    // The `[magnitude, units]` tuple form (blood_glucose: mmol/l, mg/dl) …
    assert!(
        all_units.contains(&"mmol/l".to_owned()) && all_units.contains(&"mg/dl".to_owned()),
        "expected the tuple units, got {all_units:?}"
    );
    // … and the plain `units` C_STRING form (dose: gm, U).
    assert!(
        all_units.contains(&"gm".to_owned()) && all_units.contains(&"U".to_owned()),
        "expected the plain-form units, got {all_units:?}"
    );
}

#[test]
fn coded_text_expands_at_codes_and_ac_code_value_sets() {
    let wt = web_template(OBS_UPGRADE);
    // A `defining_code` constrained to an explicit at-code list.
    let status = find(&wt.tree, "test_status").expect("test_status DV_CODED_TEXT");
    assert!(
        code_list(status).contains(&"at38".to_owned()),
        "at-code list: {:?}",
        code_list(status)
    );
    // A `defining_code` constrained to an ac-code resolves to its archetype-local
    // value-set members (AOM2 §C_TERMINOLOGY_CODE).
    let intake = find(&wt.tree, "intake").expect("intake DV_CODED_TEXT (ac-code)");
    let codes = code_list(intake);
    assert!(
        codes.iter().all(|c| c.starts_with("at")) && codes.len() >= 2,
        "value-set members: {codes:?}"
    );
    // Rubric labels are resolved (not bare codes) where the archetype defines them.
    let labelled = intake
        .inputs
        .iter()
        .flat_map(|i| &i.list)
        .any(|cv| cv.label.as_deref().is_some_and(|l| l != cv.value));
    assert!(labelled, "coded value labels resolve to archetype rubrics");
}

#[test]
fn ordinal_values_carry_ordinal_integers() {
    // AOM2 `DV_ORDINAL` constrains `[value, symbol]`; the integer value rides on
    // each coded option's `ordinal`.
    let wt = web_template(OBS_APGAR);
    let effort = find(&wt.tree, "respiratory_effort").expect("a DV_ORDINAL leaf");
    let ordinals: Vec<i32> = effort
        .inputs
        .iter()
        .flat_map(|i| &i.list)
        .filter_map(|cv| cv.ordinal)
        .collect();
    assert_eq!(ordinals, vec![0, 1, 2], "ordinal integers");
}

// ── level removal (master04 §"Level Removal") ────────────────────────────────

#[test]
fn history_and_item_structures_are_level_removed() {
    // The always-collapsed wrapper types carry no node in the compacted tree.
    let wt = web_template(OBS_UPGRADE);
    let types = rm_types(&wt.tree);
    for collapsed in ["HISTORY", "ITEM_TREE", "ITEM_STRUCTURE", "ELEMENT"] {
        assert!(
            !types.contains(&collapsed.to_owned()),
            "{collapsed} should be level-removed, tree types: {types:?}"
        );
    }
}

#[test]
fn single_event_collapses_but_multiple_events_are_retained() {
    // Conditional EVENT collapse (master04 §"Conditionally Collapsed Wrapper
    // Types"): a lone `max=1` event is collapsed; sibling events are retained.
    let single = web_template(OBS_UPGRADE);
    assert!(
        !rm_types(&single.tree).iter().any(|t| t.ends_with("EVENT")),
        "the lone Any-event is collapsed"
    );
    let multiple = web_template(OBS_APGAR);
    let point_events = rm_types(&multiple.tree)
        .iter()
        .filter(|t| t.as_str() == "POINT_EVENT")
        .count();
    assert!(
        point_events >= 2,
        "the apgar OBSERVATION retains its multiple POINT_EVENTs, got {point_events}"
    );
}

// ── node-id generation (master04 §"Node ID Generation Rules") ─────────────────

#[test]
fn sibling_ids_are_unique() {
    // The upgrade OBSERVATION has repeated node names (`added_by_post-parse_
    // processor`) that must get the sibling-uniqueness suffix.
    let wt = web_template(OBS_UPGRADE);
    assert!(
        all_sibling_ids_unique(&wt.tree),
        "sibling ids must be unique per parent"
    );
    // And the dedup actually fired somewhere in this template.
    let mut ids = Vec::new();
    collect_ids(&wt.tree, &mut ids);
    assert!(
        ids.iter().any(|i| i.ends_with("_1")),
        "the duplicate-name dedup suffix (_1) is present"
    );
}

fn all_sibling_ids_unique(node: &WebTemplateNode) -> bool {
    let mut seen = std::collections::HashSet::new();
    if !node.children.iter().all(|c| seen.insert(c.id.clone())) {
        return false;
    }
    node.children.iter().all(all_sibling_ids_unique)
}

fn collect_ids(node: &WebTemplateNode, out: &mut Vec<String>) {
    out.push(node.id.clone());
    for c in &node.children {
        collect_ids(c, out);
    }
}

// ── archetype-conformance validation (#269): the v2_4 builder now populates the
//    validation-only constraint fields the walk reads, symmetric with OPT 1.4 ──

/// Every `card_all` entry in the tree (depth-first).
fn collect_card_all<'a>(node: &'a WebTemplateNode, out: &mut Vec<&'a WebTemplateCardinality>) {
    out.extend(node.card_all.iter());
    for c in &node.children {
        collect_card_all(c, out);
    }
}

/// Pad every `items` container currently holding 1..=6 members up to 8 by cloning
/// its first member — exceeding an AOM2 `items cardinality {1..6}` upper bound.
fn pad_items(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Array(a) => {
            for item in a.iter_mut() {
                pad_items(item);
            }
        }
        serde_json::Value::Object(o) => {
            let keys: Vec<String> = o.keys().cloned().collect();
            for k in keys {
                if let Some(val) = o.get_mut(&k) {
                    pad_items(val);
                }
            }
            if let Some(serde_json::Value::Array(items)) = o.get_mut("items")
                && items.len() <= 6
                && let Some(proto) = items.first().cloned()
            {
                while items.len() < 8 {
                    items.push(proto.clone());
                }
            }
        }
        _ => {}
    }
}

#[test]
fn v2_4_populates_archetype_conformance_constraints() {
    // The apgar OBSERVATION source carries `events cardinality {1..*}` and
    // `items cardinality {1..6}` (AOM2 §C_ATTRIBUTE cardinality). The v2_4 front
    // end now captures EVERY constraining cardinality into `card_all` for the
    // validation walk, exactly as the OPT-1.4 front end does — so
    // `validate_archetype_conformance` runs identically for both dialects.
    let wt = web_template(OBS_APGAR);
    let mut cards = Vec::new();
    collect_card_all(&wt.tree, &mut cards);
    assert!(
        cards.iter().any(|c| c.max == 6),
        "the `items {{1..6}}` bounded cardinality is captured, got {:?}",
        cards.iter().map(|c| (c.min, c.max)).collect::<Vec<_>>()
    );
    assert!(
        cards.iter().any(|c| c.min == Some(1) && c.max == -1),
        "the `events {{1..*}}` cardinality is captured, got {:?}",
        cards.iter().map(|c| (c.min, c.max)).collect::<Vec<_>>()
    );
}

#[test]
fn v2_4_archetype_conformance_walk_enforces_cardinality() {
    let wt = web_template(OBS_APGAR);
    // A self-consistent example carries no cardinality violation against its own
    // ADL2 template (symmetric with the OPT-1.4 front end, whose generated
    // examples validate clean — `crates/openehr-its/tests/validation.rs`).
    let comp = example_composition(&wt, DetailLevel::Complete);
    let baseline = validate_archetype_conformance(&comp, &wt);
    assert!(
        !baseline
            .iter()
            .any(|m| m.kind == ValidationKind::Cardinality),
        "baseline example must have no cardinality violation, got {baseline:?}"
    );
    // Padding a constrained `items` container beyond its `{1..6}` upper bound is
    // rejected with the SAME typed outcome the OPT-1.4 path produces
    // (`ValidationKind::Cardinality`).
    let mut bad = comp;
    pad_items(&mut bad);
    let msgs = validate_archetype_conformance(&bad, &wt);
    assert!(
        msgs.iter().any(|m| m.kind == ValidationKind::Cardinality),
        "expected a Cardinality violation after exceeding `items {{1..6}}`, got {:?}",
        msgs.iter()
            .map(|m| (m.kind, m.path.clone()))
            .collect::<Vec<_>>()
    );
}

/// The template-with-filler seam: an ADL2 `template` whose content is a
/// `use_archetype` C_ARCHETYPE_ROOT fill of a separately-held archetype must
/// project a WebTemplate that CONTAINS the filler's flattened subtree — the
/// normal ADL2 composition case (AOM2 master06 §Templates: slot fills;
/// OPT2 master03: the created OPT inlines every filler). Regression for the
/// live defect where the projected WT carried no content nodes and every FLAT
/// path under the fill was "unknown simplified path".
const FILLER_ARCHETYPE: &str = r#"archetype (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-OBSERVATION.cnf_count_a.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"unmanaged">
    original_author = <
        ["name"] = <"openEHR CNF">
    >

definition
    OBSERVATION[id1] matches {    -- Observation one
        data matches {
            HISTORY[id2] matches {
                events cardinality matches {1..*; unordered} matches {
                    POINT_EVENT[id3] occurrences matches {0..*} matches {
                        data matches {
                            ITEM_TREE[id4] matches {
                                items cardinality matches {1..1; unordered} matches {
                                    ELEMENT[id5] occurrences matches {1..1} matches {
                                        value matches {
                                            DV_COUNT[id6]
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = <text = <"Observation one">; description = <"A constrained observation">>
            ["id2"] = <text = <"History">; description = <"Event history">>
            ["id3"] = <text = <"Any event">; description = <"A point event">>
            ["id4"] = <text = <"Tree">; description = <"Item tree">>
            ["id5"] = <text = <"Count item">; description = <"A count element">>
            ["id6"] = <text = <"Count value">; description = <"The count value">>
        >
    >
"#;

const FILLER_TEMPLATE: &str = r#"template (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.cnf_adl2_flat_a.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"unmanaged">
    original_author = <
        ["name"] = <"openEHR CNF">
    >

definition
    COMPOSITION[id1] matches {    -- Adl2 flat
        content cardinality matches {1..*; unordered} matches {
            use_archetype OBSERVATION[id2, openEHR-EHR-OBSERVATION.cnf_count_a.v1.0.0]
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = <text = <"Adl2 flat">; description = <"ADL2 FLAT-parity encounter">>
            ["id2"] = <text = <"Observation one">; description = <"The filled observation">>
        >
    >
"#;

#[test]
fn v2_4_template_filler_subtree_reaches_the_web_template() {
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse_artefact(FILLER_ARCHETYPE, Dialect::Adl2).expect("parse filler archetype"));
    let template = parse_artefact(FILLER_TEMPLATE, Dialect::Adl2).expect("parse template");
    let opt = create_opt(&template, &repo).expect("create_opt inlines the filler");
    let wt = build_web_template_v2_4(&opt).expect("build v2_4 web template");

    let mut observations = Vec::new();
    collect_typed(&wt.tree, "OBSERVATION", &mut observations);
    assert!(
        !observations.is_empty(),
        "the filled OBSERVATION must appear in the projected WebTemplate; tree rm types: {:?}",
        rm_types(&wt.tree)
    );
    let count = find(&wt.tree, "count_item").unwrap_or_else(|| {
        panic!(
            "count_item leaf missing; tree rm types: {:?}",
            rm_types(&wt.tree)
        )
    });
    // The shared shape pass compacts an ELEMENT to its value type (the Better
    // WebTemplate convention, identical to the OPT 1.4 front end).
    assert_eq!(count.rm_type, "DV_COUNT");
    // The generated Complete example must realize the mandatory leaf, and it
    // must satisfy its own template (the FLAT commit path depends on both).
    let comp = example_composition(&wt, DetailLevel::Complete);
    assert!(
        comp.pointer("/content/0/archetype_details").is_some(),
        "the filled ENTRY must be an archetype root (RM composition entry.adoc §Invariants Is_archetype_root)"
    );
    let msgs = validate_archetype_conformance(&comp, &wt);
    assert!(
        msgs.is_empty(),
        "the Complete example must satisfy its own template, got {msgs:?}"
    );
}

/// The slot-pattern probe reads the assertion's EXPRESSION TREE, not its string
/// form: `ASSERTION.expression` is the "Root of expression tree" and
/// `string_expression` only its "String form of expression"
/// (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package).
///
/// The fixture's slot carries three includes — a `∈`-spelled delimited regex, a
/// space-padded `matches` one, and a literal-value constraint carrying no regex
/// at all (`ADL2/master04.3` §Slots based on Lexical Archetype Identifiers +
/// `AOM2/master04.5` §`C_STRING`) — so a source-text scan for `"matches {/"`
/// would see one of three.
const SLOT_ARCHETYPE: &str = r#"archetype (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.wt_slot_probe.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"unmanaged">

definition
    COMPOSITION[id1] matches {    -- Slot probe
        content cardinality matches {0..*; unordered} matches {
            allow_archetype OBSERVATION[id2] occurrences matches {0..*} ∈ {
                include
                    archetype_id/value ∈ {/openEHR-EHR-OBSERVATION\.alpha\.v1/}
                    archetype_id/value    matches    {/openEHR-EHR-OBSERVATION\.beta\.v1/}
                    archetype_id/value matches {"openEHR-EHR-OBSERVATION.gamma.v1"}
            }
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = <text = <"Slot probe">; description = <"A composition with a slot">>
            ["id2"] = <text = <"Slot">; description = <"The probed slot">>
        >
    >
"#;

#[test]
fn v2_4_slot_patterns_come_from_the_assertion_tree() {
    let archetype = parse_artefact(SLOT_ARCHETYPE, Dialect::Adl2).expect("parse ADL2");
    let mut opt = create_opt(&archetype, &ArchetypeRepository::new()).expect("create_opt");
    // Strip every `string_expression`: the attribute is optional in the model
    // and only a serialisation of the tree, so a probe reading the tree is
    // unaffected while a string scan goes blind.
    strip_string_expressions(&mut opt.definition);
    let wt = build_web_template_v2_4(&opt).expect("build v2_4 web template");

    let mut includes: Vec<String> = wt
        .tree
        .closed_attributes
        .iter()
        .flat_map(|a| a.slots.iter())
        .flat_map(|s| s.includes.iter().cloned())
        .collect();
    includes.sort();
    assert_eq!(
        includes,
        [
            r"openEHR-EHR-OBSERVATION\.alpha\.v1",
            r"openEHR-EHR-OBSERVATION\.beta\.v1"
        ],
        "both regex includes are read from the tree, whatever their source spelling; \
         the literal-value include carries no regex and is not invented into one"
    );
}

/// Clear `ASSERTION.string_expression` on every `ARCHETYPE_SLOT` in the tree.
fn strip_string_expressions(cco: &mut CComplexObject) {
    let attributes = match cco {
        CComplexObject::CComplexObject(d) => d.attributes.as_mut(),
        CComplexObject::CArchetypeRoot(r) => r.attributes.as_mut(),
    };
    for attr in attributes.into_iter().flatten() {
        for child in attr.children.iter_mut().flatten() {
            match child {
                CObject::ArchetypeSlot(slot) => {
                    for a in slot
                        .includes
                        .iter_mut()
                        .flatten()
                        .chain(slot.excludes.iter_mut().flatten())
                    {
                        a.string_expression = None;
                    }
                }
                CObject::CComplexObject(inner) => strip_string_expressions(inner),
                _ => {}
            }
        }
    }
}
