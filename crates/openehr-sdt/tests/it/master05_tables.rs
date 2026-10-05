// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The master05 table-by-table battery: one semantic assertion per mapping-table row.
//!
//! Oracle: `docs/specs/openehr/ITS-REST/docs/simplified_formats/master05-rm_mapping.adoc`
//! (STABLE) — 45 sections, 43 of which carry a five-column mapping table
//! (`Flat Path` | `Flat type` | `RM Path` | `Required` | `Note`), 236 rows in
//! total. `master04-basic_concepts.adoc` supplies the surrounding syntax
//! (field identifiers, `_`-prefixed RM attributes, `|other`, level removal)
//! and `master06-context_information.adoc` the `ctx/` vocabulary.
//!
//! **What this file adds over `spec_vectors.rs`.** That file checks the spec's
//! example blocks for *syntax* and FLAT⇄STRUCTURED stability: it never builds
//! an RM value, so it cannot see whether a row's `RM Path` is actually wired
//! to its `Flat Path`, nor whether the emitted datum has the row's `Flat type`.
//! This battery does exactly that, per row: it constructs a minimal
//! canonical-JSON RM value carrying the row's RM attribute, runs the real
//! template-driven flattener through the public
//! [`openehr_sdt::flat::convert::composition_to_flat`] seam, and asserts the
//! row's Flat Path appears carrying a datum of the row's Flat type.
//!
//! Two divergences it was written to catch (both would fail against the
//! pre-fix code, and neither is visible to a syntax-only gate):
//!
//! * master05 §INSTRUCTION_DETAILS — the three flat suffixes are `|path`,
//!   `|composition_uid` and `|activity_id` **on the `_instruction_details`
//!   node itself**. The emitter previously produced a nested `instruction_id`
//!   child carrying OBJECT_REF suffixes, so `|composition_uid` never existed
//!   and `|path` sat a level too deep.
//! * master05 §INTERVAL_EVENT — `|sample_count` (INTEGER) was neither emitted
//!   nor consumed.
//!
//! **Honesty rules for this file** (a row is never quietly dropped):
//!
//! * [`Check::At`] — the row is emitted at its table path; assert key + type.
//! * [`Check::Elsewhere`] — the datum is emitted at a *different* FLAT key for
//!   a spec-cited reason (the master06 `ctx/` shortcut, or a spelling the
//!   section's own example block fixes). Assert the table path is absent and
//!   the real key present.
//! * [`Check::Absent`] — the datum has no FLAT surface at all: a spec-declared
//!   hole, an editorial defect naming an RM attribute that does not exist, or
//!   an implementation gap carrying a `TODO` at its use site. Assert the
//!   absence, so the record fails loudly the day the datum starts emitting.
//! * Every row records its `Required` column verbatim; a required row MUST
//!   also say where the requirement is enforced (the flat build rejects, an
//!   RM-mandatory fill supplies it, or nothing can enforce it). That is
//!   asserted structurally in [`check`] — a new required row cannot be added
//!   without stating its enforcement. Where the flat build genuinely rejects,
//!   the section test carries the negative assertion as well.
#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![expect(
    clippy::doc_markdown,
    reason = "the module docs quote openEHR spec prose and Simplified-Formats key names as text, not as Rust code references"
)]
#![expect(
    clippy::needless_pass_by_value,
    reason = "fixture builders take `Value` by value so call sites read as the JSON they build (`element(json!({…}))`); `json!` interpolation borrows"
)]
#![expect(
    clippy::assigning_clones,
    reason = "fixture mutation reads clearer as a plain assignment of a freshly-built value than as `clone_from`"
)]

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::webtemplate::model::{
    WebTemplate, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
};

use Check::{Absent, At, Elsewhere};
use FlatType::{Bool, Int, Real, Str, Sub, SubNum};

// ═══════════════════════════════════════════════════════════════════════════
// the row harness
// ═══════════════════════════════════════════════════════════════════════════

/// The `Flat type` column of a master05 row, as it is checkable on the wire.
#[derive(Debug, Clone, Copy)]
enum FlatType {
    /// `STRING` / `String` — a JSON string datum.
    Str,
    /// `INTEGER` / `Integer` — a JSON integer datum.
    Int,
    /// `Real` — a JSON number datum.
    Real,
    /// `Boolean` / `BOOLEAN` — a JSON boolean datum.
    Bool,
    /// A composite Flat type (`<<DV_TEXT,DV_TEXT>>`, …): the row's path names a
    /// sub-tree governed by that type's own master05 table. The payload is the
    /// witness suffix / sub-path taken from that table (normally its required
    /// row), appended to this row's path; the datum found there is a string.
    Sub(&'static str),
    /// [`FlatType::Sub`] whose witness datum is a number.
    SubNum(&'static str),
}

/// The expected JSON shape of a datum.
#[derive(Debug, Clone, Copy)]
enum Shape {
    Text,
    Integer,
    Number,
    Boolean,
}

impl Shape {
    fn matches(self, v: &Value) -> bool {
        match self {
            Shape::Text => v.is_string(),
            Shape::Integer => v.is_i64() || v.is_u64(),
            Shape::Number => v.is_number(),
            Shape::Boolean => v.is_boolean(),
        }
    }
}

/// `(witness suffix appended to the row path, expected datum shape)`.
fn witness_of(ty: FlatType) -> (&'static str, Shape) {
    match ty {
        Str => ("", Shape::Text),
        Int => ("", Shape::Integer),
        Real => ("", Shape::Number),
        Bool => ("", Shape::Boolean),
        Sub(w) => (w, Shape::Text),
        SubNum(w) => (w, Shape::Number),
    }
}

/// How a row is verified against the emitted FLAT document.
#[derive(Debug, Clone, Copy)]
enum Check {
    /// Emitted at the row's own table path, with the row's Flat type.
    At(FlatType),
    /// Emitted at a different FLAT key (given absolutely) for a spec-cited
    /// reason; the table path itself carries nothing.
    Elsewhere(&'static str),
    /// Not emitted anywhere; the payload states why (a spec-declared hole, an
    /// editorial defect, or an implementation gap with a `TODO`).
    Absent(&'static str),
}

/// One master05 mapping-table row.
#[derive(Debug, Clone, Copy)]
struct Row {
    /// The row's `Flat Path` column verbatim, except that a `:i` family is
    /// spelled with the concrete index the fixture carries (`:0`), and the
    /// asciidoc pipe escape (`\|`) is written as the plain `|` it denotes.
    path: &'static str,
    /// How the row is verified.
    check: Check,
    /// The row's `Required` column, verbatim (`yes` / `Yes` / `YES` / `no` /
    /// `(yes)`).
    req: &'static str,
    /// For a required row: where the requirement is actually enforced. Empty
    /// only for rows the table marks optional or conditional.
    enforced: &'static str,
}

const fn row(path: &'static str, check: Check, req: &'static str, enforced: &'static str) -> Row {
    Row {
        path,
        check,
        req,
        enforced,
    }
}

/// Whether any emitted key addresses `path` — the key itself, a `|suffix` of
/// it, or a `/child` under it.
fn addressed(flat: &Map<String, Value>, path: &str) -> bool {
    flat.keys().any(|k| {
        k == path || k.starts_with(&format!("{path}|")) || k.starts_with(&format!("{path}/"))
    })
}

/// A FLAT document from `(key, value)` pairs — the input side of the seam.
fn flat_of(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn sorted_keys(flat: &Map<String, Value>) -> Vec<&String> {
    let mut keys: Vec<&String> = flat.keys().collect();
    keys.sort();
    keys
}

/// The `(key, value)` pairs of `flat` addressing `path` — the key itself, a
/// `|suffix` of it, or a `/child` under it — in key order. The comparison unit
/// for a round-trip assertion scoped to one node.
fn under(flat: &Map<String, Value>, path: &str) -> std::collections::BTreeMap<String, Value> {
    flat.iter()
        .filter(|(k, _)| {
            k.as_str() == path
                || k.starts_with(&format!("{path}|"))
                || k.starts_with(&format!("{path}/"))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Assert every row of one master05 table against the FLAT document produced
/// for that table's fixture. `base` is the fixture's key prefix for the class
/// under test (a row's full key is `base` + the row's `Flat Path`).
fn check(flat: &Map<String, Value>, base: &str, rows: &[Row]) {
    for row in rows {
        assert!(
            !row.req.eq_ignore_ascii_case("yes") || !row.enforced.is_empty(),
            "master05 row `{}` is Required={} — record where that requirement \
             is enforced before adding the row",
            row.path,
            row.req
        );
        let table_path = format!("{base}{}", row.path);
        match row.check {
            At(ty) => {
                let (witness, shape) = witness_of(ty);
                let key = format!("{table_path}{witness}");
                let value = flat.get(&key).unwrap_or_else(|| {
                    panic!(
                        "master05 row `{}` must emit `{key}`; emitted keys: {:?}",
                        row.path,
                        sorted_keys(flat)
                    )
                });
                assert!(
                    shape.matches(value),
                    "master05 row `{}` (`{key}`) must carry a {shape:?} datum, got {value}",
                    row.path
                );
            }
            Elsewhere(key) => {
                assert!(
                    !addressed(flat, &table_path),
                    "master05 row `{}`: nothing may be emitted at `{table_path}` — \
                     the datum belongs at `{key}`",
                    row.path
                );
                assert!(
                    flat.contains_key(key),
                    "master05 row `{}` must emit its datum at `{key}`; emitted keys: {:?}",
                    row.path,
                    sorted_keys(flat)
                );
            }
            Absent(reason) => {
                assert!(
                    !reason.is_empty(),
                    "master05 row `{}` is recorded absent — say why",
                    row.path
                );
                assert!(
                    !addressed(flat, &table_path),
                    "master05 row `{}` is recorded absent ({reason}) but `{table_path}` \
                     is emitted — the record is stale and must be revisited",
                    row.path
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// fixtures — minimal (template, composition) pairs per master05 class
// ═══════════════════════════════════════════════════════════════════════════

const TEMPLATE_ID: &str = "master05.tables.v1";
const ROOT_ARCHETYPE: &str = "openEHR-EHR-COMPOSITION.master05.v1";
/// Deterministic `now` for the build direction (`master04 §Context`).
const NOW: &str = "2024-01-15T10:30:00Z";

/// Key prefixes the fixtures below produce.
const ROOT: &str = "test";
const ENTRY: &str = "test/entry";
const LEAF: &str = "test/entry/leaf";
const EVENT: &str = "test/entry/any_event:0";
const CONTEXT: &str = "test/context";

// ── template plumbing ────────────────────────────────────────────────────────

/// A single-occurrence container template node (no inputs).
fn container(rm_type: &str, aql: &str, id: &str) -> WebTemplateNode {
    let mut n = WebTemplateNode::new(rm_type.to_owned(), aql.to_owned());
    n.id = id.to_owned();
    n.min = Some(0);
    n.max = 1;
    n
}

/// A repeating container node — its FLAT keys carry `:i` instance indices
/// (`master04 §Instance Indexing`).
fn repeating(rm_type: &str, aql: &str, id: &str) -> WebTemplateNode {
    let mut n = container(rm_type, aql, id);
    n.max = -1;
    n
}

/// A leaf (DATA_VALUE) node: a container plus one input, which is what makes
/// the walker treat it as a datum leaf (`WebTemplateNode::has_input`).
fn leaf(rm_type: &str, aql: &str, id: &str) -> WebTemplateNode {
    let mut n = container(rm_type, aql, id);
    n.inputs = vec![WebTemplateInput::new(WebTemplateInputType::Text, None)];
    n
}

/// An in-context leaf, as the Web Template shape synthesizes for
/// `language`/`territory`/`composer`/`subject`/`start_time`/`setting`
/// (`master04 §"Web Template Metadata"`).
fn in_context(rm_type: &str, aql: &str, id: &str) -> WebTemplateNode {
    let mut n = leaf(rm_type, aql, id);
    n.min = Some(1);
    n.in_context = Some(true);
    n
}

/// A `DV_CODED_TEXT` leaf whose constraint admits an open value-set
/// (`listOpen: true` — master04 §"Open Value-Sets and the `|other` Suffix").
fn open_coded_leaf(aql: &str, id: &str) -> WebTemplateNode {
    let mut n = container("DV_CODED_TEXT", aql, id);
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list_open = Some(true);
    n.inputs = vec![input];
    n
}

/// The template root: the COMPOSITION with its in-context children plus the
/// caller's data children.
fn root_node(data_children: Vec<WebTemplateNode>) -> WebTemplateNode {
    let mut context = container("EVENT_CONTEXT", "/context", "context");
    context.children = vec![
        in_context("DV_DATE_TIME", "/context/start_time", "start_time"),
        in_context("DV_CODED_TEXT", "/context/setting", "setting"),
    ];
    let mut children = vec![
        context,
        leaf("DV_CODED_TEXT", "/category", "category"),
        in_context("CODE_PHRASE", "/language", "language"),
        in_context("CODE_PHRASE", "/territory", "territory"),
        in_context("PARTY_PROXY", "/composer", "composer"),
    ];
    children.extend(data_children);
    let mut root = container("COMPOSITION", "", ROOT);
    root.node_id = Some(ROOT_ARCHETYPE.to_owned());
    root.children = children;
    root
}

fn web_template(root: WebTemplateNode) -> WebTemplate {
    WebTemplate {
        template_id: TEMPLATE_ID.to_owned(),
        sem_ver: None,
        version: "2.3".to_owned(),
        default_language: "en".to_owned(),
        languages: vec!["en".to_owned()],
        tree: root,
        other_details: IndexMap::new(),
    }
}

// ── canonical-JSON plumbing ──────────────────────────────────────────────────

fn dv_text(value: &str) -> Value {
    json!({"_type": "DV_TEXT", "value": value})
}

fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code,
    })
}

fn dv_coded(value: &str, terminology: &str, code: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT",
        "value": value,
        "defining_code": code_phrase(terminology, code),
    })
}

fn dv_date_time(value: &str) -> Value {
    json!({"_type": "DV_DATE_TIME", "value": value})
}

/// A `PARTY_IDENTIFIED` with a name and a scheme-bearing external reference —
/// the shape master05 §PARTY_IDENTIFIED maps.
fn party_identified(name: &str, id: &str) -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED",
        "name": name,
        "external_ref": {
            "_type": "PARTY_REF",
            "namespace": "HOSPITAL-NS",
            "type": "PERSON",
            "id": {"_type": "GENERIC_ID", "value": id, "scheme": "HOSPITAL-NS"},
        },
    })
}

/// An `ELEMENT` (`at0002`) wrapping `value`.
fn element(value: Value) -> Value {
    json!({
        "_type": "ELEMENT",
        "archetype_node_id": "at0002",
        "name": dv_text("Element"),
        "value": value,
    })
}

/// A `FEEDER_AUDIT` whose `originating_system_audit` carries every master05
/// §FEEDER_AUDIT_DETAILS row, plus both item-id lists.
fn feeder_audit(original_content: Value) -> Value {
    let mut fa = json!({
        "_type": "FEEDER_AUDIT",
        "originating_system_audit": {
            "_type": "FEEDER_AUDIT_DETAILS",
            "system_id": "orig",
            "version_id": "final",
            "time": dv_date_time("2021-12-21T15:19:31+01:00"),
            "subject": party_identified("Subject", "s-1"),
            "provider": party_identified("Provider", "p-1"),
            "location": party_identified("Location", "l-1"),
        },
        "feeder_system_audit": {
            "_type": "FEEDER_AUDIT_DETAILS",
            "system_id": "feeder",
        },
        "originating_system_item_ids": [
            {"_type": "DV_IDENTIFIER", "id": "oid-1", "issuer": "issuer",
             "assigner": "assigner", "type": "type"}
        ],
        "feeder_system_item_ids": [
            {"_type": "DV_IDENTIFIER", "id": "fid-1"}
        ],
    });
    merge(&mut fa, original_content);
    fa
}

/// The single `LINK` of the `_link:i` family (master05 §LINK).
fn link() -> Value {
    json!({
        "_type": "LINK",
        "type": dv_text("problem"),
        "meaning": dv_text("problem related note"),
        "target": {"_type": "DV_EHR_URI", "value": "ehr://ehr.network/347a5490"},
    })
}

/// The `OBJECT_REF` of the `_guideline_id` / `_work_flow_id` families
/// (master05 §OBJECT_REF).
fn object_ref(ref_type: &str, id: &str) -> Value {
    json!({
        "_type": "OBJECT_REF",
        "namespace": "HOSPITAL-NS",
        "type": ref_type,
        "id": {"_type": "GENERIC_ID", "value": id, "scheme": "HOSPITAL-NS"},
    })
}

/// Copy every field of `extra` onto `target` (both JSON objects).
fn merge(target: &mut Value, extra: Value) {
    let (Some(t), Value::Object(e)) = (target.as_object_mut(), extra) else {
        return;
    };
    for (k, v) in e {
        t.insert(k, v);
    }
}

/// The COMPOSITION wrapper: mandatory attributes filled, `content` holding
/// `entry`, and `extra` merged onto the root (`_uid`/`links`/`feeder_audit`).
fn composition(entry: Value, extra: Value) -> Value {
    let mut comp = json!({
        "_type": "COMPOSITION",
        "archetype_node_id": ROOT_ARCHETYPE,
        "name": dv_text("master05"),
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {"_type": "ARCHETYPE_ID", "value": ROOT_ARCHETYPE},
            "template_id": {"_type": "TEMPLATE_ID", "value": TEMPLATE_ID},
            "rm_version": "1.2.0",
        },
        "language": code_phrase("ISO_639-1", "en"),
        "territory": code_phrase("ISO_3166-1", "US"),
        "category": dv_coded("event", "openehr", "433"),
        "composer": party_identified("Silvia Blake", "c-1"),
        "context": {
            "_type": "EVENT_CONTEXT",
            "start_time": dv_date_time("2021-12-21T14:19:31+01:00"),
            "setting": dv_coded("other care", "openehr", "238"),
        },
        "content": [entry],
    });
    merge(&mut comp, extra);
    comp
}

fn flatten(root: WebTemplateNode, comp: &Value) -> Map<String, Value> {
    composition_to_flat(comp, &web_template(root)).expect("composition_to_flat")
}

fn entry_archetype(rm_type: &str) -> String {
    format!("openEHR-EHR-{rm_type}.master05.v1")
}

fn entry_aql(rm_type: &str) -> String {
    format!("/content[{}]", entry_archetype(rm_type))
}

/// Flatten a composition whose single `content` item is `entry` (RM type
/// `rm_type`), with `children` hung under the entry's template node. The
/// entry's keys are based at [`ENTRY`].
fn flat_entry(rm_type: &str, children: Vec<WebTemplateNode>, entry: Value) -> Map<String, Value> {
    flat_entry_with(rm_type, children, entry, json!({}))
}

/// [`flat_entry`] with `comp_extra` merged onto the COMPOSITION root.
fn flat_entry_with(
    rm_type: &str,
    children: Vec<WebTemplateNode>,
    entry: Value,
    comp_extra: Value,
) -> Map<String, Value> {
    let wt = entry_web_template(rm_type, children);
    composition_to_flat(&entry_composition(rm_type, entry, comp_extra), &wt)
        .expect("composition_to_flat")
}

/// The web template an ENTRY-family fixture flattens against: the root
/// COMPOSITION carrying a single `entry` child of `rm_type` with `children`.
/// Exposed separately so a test can drive the build direction against the very
/// same tree.
fn entry_web_template(rm_type: &str, children: Vec<WebTemplateNode>) -> WebTemplate {
    let mut node = container(rm_type, &entry_aql(rm_type), "entry");
    node.node_id = Some(entry_archetype(rm_type));
    node.children = children;
    web_template(root_node(vec![node]))
}

/// The composition [`flat_entry_with`] flattens: `entry` stamped with the
/// fixture's archetype identity and hung under the root's `content`.
fn entry_composition(rm_type: &str, entry: Value, comp_extra: Value) -> Value {
    let mut entry = entry;
    merge(
        &mut entry,
        json!({"archetype_node_id": entry_archetype(rm_type), "name": dv_text("Entry")}),
    );
    composition(entry, comp_extra)
}

/// The in-context ENTRY children every ENTRY-family fixture carries
/// (`master04 §"Web Template Metadata"`: `language`, `encoding`, `subject`).
fn entry_in_context(rm_type: &str) -> Vec<WebTemplateNode> {
    let base = entry_aql(rm_type);
    vec![
        in_context("CODE_PHRASE", &format!("{base}/language"), "language"),
        in_context("CODE_PHRASE", &format!("{base}/encoding"), "encoding"),
        in_context("PARTY_PROXY", &format!("{base}/subject"), "subject"),
    ]
}

/// The RM attributes every ENTRY-family fixture carries: the in-context
/// `language`/`encoding`/`subject` plus the shared `_` families
/// (`_work_flow_id`, `_link:i`, `_feeder_audit`, `_uid`).
fn entry_common() -> Value {
    json!({
        "language": code_phrase("ISO_639-1", "en"),
        "encoding": code_phrase("IANA_character-sets", "UTF-8"),
        "subject": party_identified("Subject", "s-9"),
        "workflow_id": object_ref("WORKFLOW", "335645"),
        "links": [link()],
        "feeder_audit": feeder_audit(json!({})),
        "uid": {"_type": "HIER_OBJECT_ID", "value": "9fcc1c70-9349-444d-b9cb-8fa817697f5e"},
    })
}

/// An ENTRY-family fixture: `entry_common` plus the caller's own attributes.
fn entry_of(rm_type: &str, extra: Value) -> Value {
    let mut entry = json!({"_type": rm_type});
    merge(&mut entry, entry_common());
    merge(&mut entry, extra);
    entry
}

/// The template leaf node [`flat_element`] hangs its ELEMENT value under.
fn element_leaf_node(rm_type: &str) -> WebTemplateNode {
    leaf(
        rm_type,
        &format!(
            "{}/data[at0001]/items[at0002]/value",
            entry_aql("EVALUATION")
        ),
        "leaf",
    )
}

/// Flatten an EVALUATION whose ITEM_TREE holds `element`, with the leaf value
/// typed `rm_type` in the template. Leaf keys are based at [`LEAF`].
fn flat_element(rm_type: &str, element: Value) -> Map<String, Value> {
    flat_element_node(element_leaf_node(rm_type), element)
}

/// [`flat_element`] with a caller-supplied leaf template node (so a fixture can
/// set `listOpen` or a generic slot type).
fn flat_element_node(leaf_node: WebTemplateNode, element: Value) -> Map<String, Value> {
    flat_entry("EVALUATION", vec![leaf_node], element_entry(element))
}

/// The EVALUATION an element fixture lives in: one ITEM_TREE (`at0001`)
/// holding `element` at `at0002`.
fn element_entry(element: Value) -> Value {
    json!({
        "_type": "EVALUATION",
        "data": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": dv_text("Tree"),
            "items": [element],
        },
    })
}

/// The common leaf fixture: a plain ELEMENT wrapping `value`.
fn flat_leaf(rm_type: &str, value: Value) -> Map<String, Value> {
    flat_element(rm_type, element(value))
}

/// Flatten an OBSERVATION whose HISTORY carries the single `event` (RM type
/// `event_rm_type` in the template). Event keys are based at [`EVENT`].
fn flat_event(event_rm_type: &str, event: Value) -> Map<String, Value> {
    let events_aql = format!("{}/data[at0001]/events[at0002]", entry_aql("OBSERVATION"));
    let value_aql = format!("{events_aql}/data[at0003]/items[at0004]/value");
    let mut event_node = repeating(event_rm_type, &events_aql, "any_event");
    event_node.children = vec![leaf("DV_QUANTITY", &value_aql, "dv_quantity")];
    let entry = json!({
        "_type": "OBSERVATION",
        "data": {
            "_type": "HISTORY",
            "archetype_node_id": "at0001",
            "name": dv_text("History"),
            "origin": dv_date_time("2021-12-21T16:00:00+01:00"),
            "events": [event],
        },
    });
    flat_entry("OBSERVATION", vec![event_node], entry)
}

/// The `data` of an EVENT: an ITEM_TREE (`at0003`) with one DV_QUANTITY
/// ELEMENT (`at0004`).
fn event_data() -> Value {
    json!({
        "_type": "ITEM_TREE",
        "archetype_node_id": "at0003",
        "name": dv_text("Tree"),
        "items": [{
            "_type": "ELEMENT",
            "archetype_node_id": "at0004",
            "name": dv_text("Element"),
            "value": {"_type": "DV_QUANTITY", "magnitude": 65.9, "units": "unit"},
        }],
    })
}

/// Flatten a composition whose EVENT_CONTEXT carries `extra`. Context keys are
/// based at [`CONTEXT`].
fn flat_context(extra: Value) -> Map<String, Value> {
    let entry = json!({"_type": "EVALUATION"});
    let mut comp = composition(entry, json!({}));
    if let Some(context) = comp.get_mut("context") {
        merge(context, extra);
    }
    flatten(root_node(vec![]), &comp)
}

/// The `PARTICIPATION` of the `_participation:i` family, with the `performer`
/// the caller supplies (master05 §PARTICIPATION inlines it).
fn participation(performer: Value) -> Value {
    json!({
        "_type": "PARTICIPATION",
        "function": dv_text("requester"),
        "mode": dv_coded("face-to-face communication", "openehr", "216"),
        "performer": performer,
    })
}

/// A `PARTY_IDENTIFIED` performer carrying the four `|identifiers_*:i` fields.
fn performer_with_identifiers() -> Value {
    let mut p = party_identified("Dr. Marcus Johnson", "199");
    merge(
        &mut p,
        json!({"identifiers": [{
            "_type": "DV_IDENTIFIER", "id": "122", "issuer": "issuer",
            "assigner": "assigner", "type": "type"
        }]}),
    );
    p
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 §COMPOSITION
// ═══════════════════════════════════════════════════════════════════════════

/// master05 §COMPOSITION — 8 rows.
#[test]
fn master05_composition() {
    let flat = flat_entry_with(
        "EVALUATION",
        vec![],
        json!({"_type": "EVALUATION"}),
        json!({
            "uid": {"_type": "OBJECT_VERSION_ID",
                    "value": "6e3a9506-b81c-4d74-a37f-1464fb7106b2::ehrbase.org::1"},
            "links": [link()],
            "feeder_audit": feeder_audit(json!({})),
            "context": {
                "_type": "EVENT_CONTEXT",
                "start_time": dv_date_time("2021-12-21T14:19:31+01:00"),
                "setting": dv_coded("other care", "openehr", "238"),
                "health_care_facility": party_identified("Hospital", "9091"),
            },
        }),
    );
    check(
        &flat,
        ROOT,
        &[
            row(
                "/language",
                Elsewhere("ctx/language"),
                "Yes",
                "master06 §\"Language and Territory\": the composition language is the \
                 `ctx/language` shortcut on output; the path spelling is accepted on input",
            ),
            row(
                "/territory",
                Elsewhere("ctx/territory"),
                "Yes",
                "master06 §\"Language and Territory\" — as `/language`",
            ),
            row(
                "/category",
                At(Sub("|code")),
                "yes",
                "RM ehr §COMPOSITION `category` is 1..1: the builder fills the openEHR \
                 `433|event` default rather than rejecting a document without the row",
            ),
            row(
                "/composer",
                Elsewhere("ctx/composer_name"),
                "yes",
                "master06 §Composer: the composer is the `ctx/composer_name`/`ctx/composer_id` \
                 shortcut on output; RM ehr §COMPOSITION `composer` is 1..1 and the builder \
                 fills it from the resolved context",
            ),
            row(
                // The EVENT_CONTEXT sub-tree does surface here, but its own
                // required rows relocate to `ctx/` — see the §EVENT_CONTEXT test.
                "/context",
                At(Sub("/_health_care_facility|name")),
                "yes",
                "RM ehr master05 §\"Persistent Compositions may optionally have an Event \
                 context\": the context is not mandatory for every category, and the builder \
                 materialises one only when context content was expressed",
            ),
            row("/_link:0", At(Sub("|type")), "no", ""),
            row(
                "/_feeder_audit",
                At(Sub("/originating_system_audit|system_id")),
                "no",
                "",
            ),
            row("/_uid", At(Str), "no", ""),
        ],
    );

    // The path spelling of the in-context rows is accepted on input
    // (master05 §COMPOSITION spells them as paths; master06 as `ctx/`).
    let built = composition_from_flat(
        &flat_of(&[
            ("test/language|code", json!("de")),
            ("test/territory|code", json!("DE")),
            ("test/composer|name", json!("Dr. Smith")),
        ]),
        &web_template(root_node(vec![])),
        NOW,
    )
    .expect("the master05 §COMPOSITION path spellings must build");
    assert_eq!(built["language"]["code_string"], json!("de"));
    assert_eq!(built["territory"]["code_string"], json!("DE"));
    assert_eq!(built["composer"]["name"], json!("Dr. Smith"));
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 — the ENTRY family
// ═══════════════════════════════════════════════════════════════════════════

/// The rows every master05 ENTRY table repeats: the two in-context rows, the
/// `/territory` phantom, `/subject`, and the shared `_` families.
fn entry_shared_rows() -> Vec<Row> {
    vec![
        row(
            "/language",
            Elsewhere("ctx/language"),
            "Yes",
            "master06 §\"Language and Territory\": an ENTRY's language defaults from the \
             composition language, which is the `ctx/language` shortcut in both directions. \
             RM ehr §ENTRY declares `language` 1..1 and the builder fills it from the \
             resolved context, so the row is satisfied without a per-entry key",
        ),
        row(
            "/territory",
            Absent(
                "RM ehr §ENTRY declares no `territory` attribute — only COMPOSITION has one \
                 (RM ehr §COMPOSITION); the row is an editorial defect repeated across the \
                 five ENTRY tables and names nothing that can be mapped",
            ),
            "Yes",
            "unenforceable: the row names an RM attribute that does not exist",
        ),
        // The row's Note ("will be set to PARTY_SELF if not explicitly set")
        // is what keeps the default out of the emission: only a subject that
        // is NOT the bare PARTY_SELF default is real data and emits.
        row("/subject", At(Sub("|name")), "no", ""),
        row("/_work_flow_id", At(Sub("|id")), "no", ""),
        row("/_link:0", At(Sub("|type")), "no", ""),
        row(
            "/_feeder_audit",
            At(Sub("/originating_system_audit|system_id")),
            "no",
            "",
        ),
        row("/_uid", At(Str), "no", ""),
    ]
}

/// The `/encoding` editorial hole: RM ehr §ENTRY makes `encoding` (CODE_PHRASE)
/// 1..1, and the master05 §COMPOSITION example block emits
/// `…/conformance_observation/encoding|code` — yet no ENTRY table carries an
/// `/encoding` row. Our flattener routes it the same way as `/language`
/// (master06 §"Language and Territory"), so no `…/encoding` path key is
/// emitted; the value is restored by the builder's ENTRY defaults
/// (`IANA_character-sets` `UTF-8`).
fn assert_entry_encoding_hole(flat: &Map<String, Value>) {
    assert!(
        !addressed(flat, &format!("{ENTRY}/encoding")),
        "the ENTRY `encoding` is routed through the context, not a path key"
    );
}

/// master05 §ADMIN_ENTRY — 7 rows.
#[test]
fn master05_admin_entry() {
    let flat = flat_entry(
        "ADMIN_ENTRY",
        entry_in_context("ADMIN_ENTRY"),
        entry_of("ADMIN_ENTRY", json!({})),
    );
    check(&flat, ENTRY, &entry_shared_rows());
    assert_entry_encoding_hole(&flat);
}

/// master05 §INSTRUCTION — 11 rows.
#[test]
fn master05_instruction() {
    let flat = flat_entry(
        "INSTRUCTION",
        entry_in_context("INSTRUCTION"),
        entry_of(
            "INSTRUCTION",
            json!({
                "narrative": dv_text("Human readable instruction narrative"),
                "expiry_time": dv_date_time("2022-01-31T10:33:28+01:00"),
                "wf_definition": {"_type": "DV_PARSABLE", "value": "wf_definition",
                                  "formalism": "formalism"},
                "guideline_id": object_ref("GUIDELINE", "3445"),
            }),
        ),
    );
    let mut rows = vec![
        row(
            "/narrative",
            At(Str),
            "yes",
            "RM ehr §INSTRUCTION `narrative` is 1..1: the builder fills a `<narrative>` \
             placeholder rather than rejecting a document without the row",
        ),
        row(
            "/_expiry_time",
            At(Str),
            "Yes",
            "unenforceable, and an editorial defect: RM ehr §INSTRUCTION declares \
             `expiry_time` 0..1, so nothing may require it",
        ),
        row("/_wf_definition", At(Str), "no", ""),
        row("/_guideline_id", At(Sub("|id")), "no", ""),
    ];
    rows.extend(entry_shared_rows());
    check(&flat, ENTRY, &rows);
    assert_entry_encoding_hole(&flat);
}

/// master05 §ACTION — 11 rows.
#[test]
fn master05_action() {
    let flat = flat_entry(
        "ACTION",
        entry_in_context("ACTION"),
        entry_of("ACTION", action_attributes()),
    );
    let mut rows = vec![
        row(
            "/time",
            At(Str),
            "YES",
            "RM ehr §ACTION `time` is 1..1: the builder fills a deterministic time rather \
             than rejecting a document without the row",
        ),
        row(
            "/ism_transition",
            At(Sub("/current_state|code")),
            "Yes",
            "RM ehr §ACTION `ism_transition` is 1..1: the builder fills the openEHR \
             `524|initial` state rather than rejecting",
        ),
        row(
            "/_instruction_details",
            At(Sub("|composition_uid")),
            "no",
            "",
        ),
        row("/_guideline_id", At(Sub("|id")), "no", ""),
    ];
    rows.extend(entry_shared_rows());
    check(&flat, ENTRY, &rows);
    assert_entry_encoding_hole(&flat);
}

/// The ACTION-specific RM attributes (shared with the §ISM_TRANSITION and
/// §INSTRUCTION_DETAILS fixtures).
fn action_attributes() -> Value {
    json!({
        "time": dv_date_time("2022-01-31T10:33:28+01:00"),
        "ism_transition": {
            "_type": "ISM_TRANSITION",
            "current_state": dv_coded("completed", "openehr", "532"),
            "transition": dv_coded("finish", "openehr", "548"),
            "careflow_step": dv_coded("transition", "local", "at0006"),
            "reason": [dv_text("reason 1")],
        },
        "instruction_details": {
            "_type": "INSTRUCTION_DETAILS",
            "instruction_id": {
                "_type": "LOCATABLE_REF",
                "namespace": "EHR",
                "type": "VERSIONED_COMPOSITION",
                "id": {"_type": "HIER_OBJECT_ID",
                       "value": "4cdc3017-d8c5-4cd3-9900-f3bb7171d006"},
                "path": "/content[openEHR-EHR-SECTION.conformance_section.v0]\
                         /items[openEHR-EHR-INSTRUCTION.conformance_instruction.v0]",
            },
            "activity_id": "activities[at0001]",
        },
        "guideline_id": object_ref("GUIDELINE", "3445"),
        "description": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": dv_text("Tree"), "items": [],
        },
    })
}

/// master05 §EVALUATION — 8 rows.
#[test]
fn master05_evaluation() {
    let flat = flat_entry(
        "EVALUATION",
        entry_in_context("EVALUATION"),
        entry_of(
            "EVALUATION",
            json!({"guideline_id": object_ref("GUIDELINE", "3445")}),
        ),
    );
    let mut rows = vec![row("/_guideline_id", At(Sub("|id")), "no", "")];
    rows.extend(entry_shared_rows());
    check(&flat, ENTRY, &rows);
    assert_entry_encoding_hole(&flat);
}

/// master05 §OBSERVATION — 9 rows.
#[test]
fn master05_observation() {
    let mut children = entry_in_context("OBSERVATION");
    children.push(repeating(
        "POINT_EVENT",
        &format!("{}/data[at0001]/events[at0002]", entry_aql("OBSERVATION")),
        "any_event",
    ));
    let entry = entry_of(
        "OBSERVATION",
        json!({
            "guideline_id": object_ref("GUIDELINE", "3445"),
            "data": {
                "_type": "HISTORY",
                "archetype_node_id": "at0001",
                "name": dv_text("History"),
                "origin": dv_date_time("2021-12-21T16:00:00+01:00"),
                "events": [{
                    "_type": "POINT_EVENT",
                    "archetype_node_id": "at0002",
                    "name": dv_text("Any event"),
                    "time": dv_date_time("2021-12-21T16:02:58+01:00"),
                    "data": event_data(),
                }],
            },
        }),
    );
    let flat = flat_entry("OBSERVATION", children, entry);
    let mut rows = vec![
        row("/history_origin", At(Str), "no", ""),
        row("/_guideline_id", At(Sub("|id")), "no", ""),
    ];
    rows.extend(entry_shared_rows());
    check(&flat, ENTRY, &rows);
    assert_entry_encoding_hole(&flat);
}

/// The `(template, FLAT)` pair for an EVALUATION whose ENTRY-level `subject`
/// is `party` — the fixture behind the master05 ENTRY `/subject` row (typed
/// `PARTY_PROXY`, so each of the three subtype tables reaches it).
fn entry_subject_case(party: Value) -> (WebTemplate, Map<String, Value>) {
    // One leaf datum keeps the ENTRY representable on the wire: FLAT structure
    // is rebuilt from datum paths (master04 §Building the RM composition), so
    // an ENTRY whose only content is the OFF-WIRE PARTY_SELF default would
    // have no keys at all and, correctly, never rebuild.
    let mut children = entry_in_context("EVALUATION");
    children.push(element_leaf_node("DV_TEXT"));
    let wt = entry_web_template("EVALUATION", children);
    let entry = json!({
        "_type": "EVALUATION",
        "subject": party,
        "data": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": dv_text("Tree"),
            "items": [element(dv_text("anchor"))],
        },
    });
    let comp = entry_composition("EVALUATION", entry, json!({}));
    let flat = composition_to_flat(&comp, &wt).expect("composition_to_flat");
    (wt, flat)
}

/// The rebuilt `subject` of the single ENTRY in a document built from `flat`.
fn built_subject(wt: &WebTemplate, flat: &Map<String, Value>) -> Value {
    let built =
        composition_from_flat(flat, wt, NOW).expect("the master05 ENTRY `/subject` row must build");
    built
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|e| e.get("subject"))
        .cloned()
        .expect("the built ENTRY carries a `subject`")
}

/// master05 ENTRY `/subject` (PARTY_PROXY) in both directions, once per
/// concrete subtype the section dispatches to (master05 §PARTY_PROXY: "See
/// PARTY_SELF, PARTY_IDENTIFIED and PARTY_RELATED"). Each subtype must survive
/// RM → FLAT → RM unchanged; the row's Note ("will be set to PARTY_SELF if not
/// explicitly set") governs the default, which stays off the wire.
#[test]
fn master05_entry_subject_round_trips_every_party_subtype() {
    // PARTY_IDENTIFIED — the `|name`/`|id`/`|id_scheme`/`|id_namespace` rows.
    let subject = party_identified("Susan Doe", "199");
    let (wt, flat) = entry_subject_case(subject.clone());
    assert_eq!(flat[&format!("{ENTRY}/subject|name")], json!("Susan Doe"));
    assert_eq!(flat[&format!("{ENTRY}/subject|id")], json!("199"));
    assert_eq!(
        flat[&format!("{ENTRY}/subject|id_scheme")],
        json!("HOSPITAL-NS")
    );
    assert_eq!(
        flat[&format!("{ENTRY}/subject|id_namespace")],
        json!("HOSPITAL-NS")
    );
    assert_eq!(built_subject(&wt, &flat), subject);

    // PARTY_RELATED — adds the `/relationship` DV_CODED_TEXT sub-path and the
    // `/_identifier:i` family (master05 §PARTY_RELATED).
    let mut subject = party_identified("Susan Doe", "199");
    merge(
        &mut subject,
        json!({
            "_type": "PARTY_RELATED",
            "relationship": dv_coded("mother", "openehr", "10"),
            "identifiers": [{"_type": "DV_IDENTIFIER", "id": "122", "issuer": "issuer"}],
        }),
    );
    let (wt, flat) = entry_subject_case(subject.clone());
    assert_eq!(
        flat[&format!("{ENTRY}/subject/relationship|code")],
        json!("10")
    );
    assert_eq!(
        flat[&format!("{ENTRY}/subject/_identifier:0|id")],
        json!("122")
    );
    assert_eq!(built_subject(&wt, &flat), subject);

    // PARTY_SELF with an external reference — `|_type` is the discriminator
    // (master05 §FEEDER_AUDIT_DETAILS `/subject` row Note); without it the
    // rebuild would produce a PARTY_IDENTIFIED.
    let subject = json!({
        "_type": "PARTY_SELF",
        "external_ref": {
            "_type": "PARTY_REF", "namespace": "DEMOGRAPHIC", "type": "PERSON",
            "id": {"_type": "GENERIC_ID", "value": "42", "scheme": "HOSPITAL-NS"},
        },
    });
    let (wt, flat) = entry_subject_case(subject.clone());
    assert_eq!(flat[&format!("{ENTRY}/subject|_type")], json!("PARTY_SELF"));
    assert_eq!(built_subject(&wt, &flat), subject);

    // The bare PARTY_SELF default never reaches the wire and is restored by
    // the builder (the row's Note).
    let (wt, flat) = entry_subject_case(json!({"_type": "PARTY_SELF"}));
    assert!(
        !addressed(&flat, &format!("{ENTRY}/subject")),
        "the PARTY_SELF default must not be emitted: {:?}",
        sorted_keys(&flat)
    );
    assert_eq!(built_subject(&wt, &flat), json!({"_type": "PARTY_SELF"}));
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 — structure classes
// ═══════════════════════════════════════════════════════════════════════════

/// master05 §ELEMENT — 5 rows, over an element carrying every one of them at
/// once. The `_null_flavour`/`_null_reason` rows sit here beside a `value`,
/// which RM data_structures §ELEMENT forbids (`Inv_null_flavour_indicated`) —
/// the combination isolates the five rows in one fixture; the shape the RM
/// actually admits (null flavour, no value) is asserted in both directions by
/// [`master05_element_null_flavoured_without_value`].
#[test]
fn master05_element() {
    let mut el = element(dv_text("value"));
    merge(
        &mut el,
        json!({
            "null_flavour": dv_coded("unknown", "openehr", "253"),
            "null_reason": dv_text("not asked"),
            "links": [link()],
            "feeder_audit": feeder_audit(json!({})),
            "uid": {"_type": "HIER_OBJECT_ID", "value": "9fcc1c70-9349-444d-b9cb-8fa817697f5e"},
        }),
    );
    let flat = flat_element("DV_TEXT", el);
    check(
        &flat,
        LEAF,
        &[
            row("/_null_flavour", At(Sub("|code")), "no", ""),
            row("/_null_reason", At(Str), "no", ""),
            row("/_link:0", At(Sub("|type")), "no", ""),
            row(
                "/_feeder_audit",
                At(Sub("/originating_system_audit|system_id")),
                "no",
                "",
            ),
            row("/_uid", At(Str), "no", ""),
        ],
    );
}

/// master05 §ELEMENT `/_null_flavour` + `/_null_reason` on the shape the RM
/// admits: a **value-less** ELEMENT. RM data_structures §ELEMENT
/// (`Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`) makes
/// `value` and `null_flavour` mutually exclusive, so this — not the
/// value-bearing probe above — is how a real null flavour reaches the wire.
/// The section's third example block spells exactly this document.
#[test]
fn master05_element_null_flavoured_without_value() {
    let el = json!({
        "_type": "ELEMENT",
        "archetype_node_id": "at0002",
        "name": dv_text("Element"),
        "null_flavour": dv_coded("unknown", "openehr", "253"),
        "null_reason": dv_text("sample reason"),
    });
    let flat = flat_element("DV_TEXT", el);
    check(
        &flat,
        LEAF,
        &[
            row("/_null_flavour", At(Sub("|code")), "no", ""),
            row("/_null_reason", At(Str), "no", ""),
        ],
    );
    // The element itself carries no datum: `value` is exactly what a null
    // flavour replaces.
    assert!(
        !flat.contains_key(LEAF),
        "a value-less ELEMENT emits no datum, got {:?}",
        sorted_keys(&flat)
    );

    // …and the same document rebuilds the value-less ELEMENT (the master05
    // §ELEMENT third example block, replayed as an input).
    let wt = entry_web_template("EVALUATION", vec![element_leaf_node("DV_TEXT")]);
    let built = composition_from_flat(&flat, &wt, NOW)
        .expect("a null-flavoured ELEMENT must build from its master05 §ELEMENT rows");
    let rebuilt = &built["content"][0]["data"]["items"][0];
    assert_eq!(rebuilt["_type"], json!("ELEMENT"));
    assert!(
        rebuilt.get("value").is_none(),
        "the rebuilt ELEMENT must carry no value: {rebuilt}"
    );
    assert_eq!(
        rebuilt["null_flavour"]["defining_code"]["code_string"],
        json!("253")
    );
    assert_eq!(rebuilt["null_reason"]["value"], json!("sample reason"));

    // RM → FLAT → RM → FLAT is stable: the null flavour survives the round
    // trip that used to drop it.
    let reflattened = composition_to_flat(&built, &wt).expect("composition_to_flat");
    assert_eq!(
        under(&reflattened, LEAF),
        under(&flat, LEAF),
        "the null-flavoured ELEMENT must round-trip key-for-key"
    );
}

/// master05 §CLUSTER — 3 rows.
#[test]
fn master05_cluster() {
    let cluster_aql = format!("{}/data[at0001]/items[at0003]", entry_aql("EVALUATION"));
    let mut cluster_node = container("CLUSTER", &cluster_aql, "cluster");
    cluster_node.children = vec![leaf(
        "DV_TEXT",
        &format!("{cluster_aql}/items[at0002]/value"),
        "leaf",
    )];
    let entry = json!({
        "_type": "EVALUATION",
        "data": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": dv_text("Tree"),
            "items": [{
                "_type": "CLUSTER",
                "archetype_node_id": "at0003",
                "name": dv_text("Cluster"),
                "items": [element(dv_text("value"))],
                "links": [link()],
                "feeder_audit": feeder_audit(json!({})),
                "uid": {"_type": "HIER_OBJECT_ID",
                        "value": "9fcc1c70-9349-444d-b9cb-8fa817697f5e"},
            }],
        },
    });
    let flat = flat_entry("EVALUATION", vec![cluster_node], entry);
    check(
        &flat,
        "test/entry/cluster",
        &[
            row("/_link:0", At(Sub("|type")), "no", ""),
            row(
                "/_feeder_audit",
                At(Sub("/originating_system_audit|system_id")),
                "no",
                "",
            ),
            row("/_uid", At(Str), "no", ""),
        ],
    );
}

/// master05 §LINK — 3 rows.
#[test]
fn master05_link() {
    let flat = flat_entry_with(
        "EVALUATION",
        vec![],
        json!({"_type": "EVALUATION"}),
        json!({"links": [link()]}),
    );
    let enforced = "RM common §LINK declares `meaning`/`type`/`target` 1..1; the flat build \
                    substitutes an empty DV_TEXT / DV_EHR_URI for a missing suffix rather \
                    than rejecting";
    check(
        &flat,
        "test/_link:0",
        &[
            row("|type", At(Str), "yes", enforced),
            row("|meaning", At(Str), "yes", enforced),
            row("|target", At(Str), "yes", enforced),
        ],
    );
}

/// master05 §FEEDER_AUDIT — 6 rows.
///
/// `/original_content` and `/original_content_multimedia` map the same RM
/// attribute ("one one of … can be set"), so they are probed with two
/// fixtures.
#[test]
fn master05_feeder_audit() {
    let base = format!("{ENTRY}/_feeder_audit");
    let parsable = flat_entry(
        "EVALUATION",
        vec![],
        entry_of(
            "EVALUATION",
            json!({"feeder_audit": feeder_audit(json!({
                "original_content": {"_type": "DV_PARSABLE", "value": "<x/>",
                                     "formalism": "xml"}
            }))}),
        ),
    );
    check(
        &parsable,
        &base,
        &[
            row("/originating_system_item_id:0", At(Sub("|id")), "no", ""),
            row("/feeder_system_item_id:0", At(Sub("|id")), "no", ""),
            // master05 §DV_PARSABLE's `|value` row is emitted as the bare datum
            // (the §ACTIVITY / §INSTRUCTION example blocks spell it bare).
            row("/original_content", At(Str), "no", ""),
            row(
                "/originating_system_audit",
                At(Sub("|system_id")),
                "yes",
                "RM common §FEEDER_AUDIT declares `originating_system_audit` 1..1; the flat \
                 build substitutes `system_id: \"unknown\"` rather than rejecting. (The Flat \
                 type column says PARTY_IDENTIFIED — an editorial slip; the RM attribute is a \
                 FEEDER_AUDIT_DETAILS, as the row below and the example block both show.)",
            ),
            row("/feeder_system_audit", At(Sub("|system_id")), "no", ""),
        ],
    );

    let multimedia = flat_entry(
        "EVALUATION",
        vec![],
        entry_of(
            "EVALUATION",
            json!({"feeder_audit": feeder_audit(json!({
                "original_content": {
                    "_type": "DV_MULTIMEDIA",
                    "media_type": code_phrase("IANA_media-types", "text/plain"),
                    "size": 12,
                }
            }))}),
        ),
    );
    check(
        &multimedia,
        &base,
        &[row(
            "/original_content_multimedia",
            At(Sub("|mediatype")),
            "no",
            "",
        )],
    );
}

/// master05 §FEEDER_AUDIT_DETAILS — 6 rows.
#[test]
fn master05_feeder_audit_details() {
    let flat = flat_entry("EVALUATION", vec![], entry_of("EVALUATION", json!({})));
    check(
        &flat,
        &format!("{ENTRY}/_feeder_audit/originating_system_audit"),
        &[
            row(
                "|system_id",
                At(Str),
                "yes",
                "RM common §FEEDER_AUDIT_DETAILS declares `system_id` 1..1; the flat build \
                 substitutes `\"unknown\"` rather than rejecting",
            ),
            row("|version_id", At(Str), "no", ""),
            row("|time", At(Str), "no", ""),
            row("/subject", At(Sub("|name")), "no", ""),
            row("/provider", At(Sub("|name")), "no", ""),
            row("/location", At(Sub("|name")), "no", ""),
        ],
    );

    // The row's own Note: `/subject|_type: "PARTY_SELF"` selects a PARTY_SELF.
    let built = composition_from_flat(
        &flat_of(&[(
            "test/entry/_feeder_audit/originating_system_audit/subject|_type",
            json!("PARTY_SELF"),
        )]),
        &web_template(root_node(vec![{
            let mut n = container("EVALUATION", &entry_aql("EVALUATION"), "entry");
            n.node_id = Some(entry_archetype("EVALUATION"));
            n
        }])),
        NOW,
    )
    .expect("the master05 §FEEDER_AUDIT_DETAILS `/subject|_type` note must build");
    assert_eq!(
        built["content"][0]["feeder_audit"]["originating_system_audit"]["subject"]["_type"],
        json!("PARTY_SELF")
    );
}

/// master05 §ACTIVITY — 2 rows.
#[test]
fn master05_activity() {
    let activity_aql = format!("{}/activities[at0001]", entry_aql("INSTRUCTION"));
    let entry = json!({
        "_type": "INSTRUCTION",
        "narrative": dv_text("narrative"),
        "activities": [{
            "_type": "ACTIVITY",
            "archetype_node_id": "at0001",
            "name": dv_text("Current activity"),
            "timing": {"_type": "DV_PARSABLE",
                       "value": "R4/2022-01-31T10:00:00+01:00/P3M",
                       "formalism": "timing"},
            "action_archetype_id": "/openEHR-EHR-CLUSTER.conformance_action.v0/",
            "description": {"_type": "ITEM_TREE", "archetype_node_id": "at0002",
                            "name": dv_text("Tree"), "items": []},
        }],
    });
    let flat = flat_entry(
        "INSTRUCTION",
        vec![container("ACTIVITY", &activity_aql, "activity")],
        entry,
    );
    check(
        &flat,
        "test/entry/activity",
        &[
            row("/timing", At(Str), "no", ""),
            row("/action_archetype_id", At(Str), "no", ""),
        ],
    );
    // The row's Note ("Will be set to /.*/ if not set explicit."): the
    // match-all default is re-synthesized on build, so it is never emitted.
    let built = composition_from_flat(
        &flat_of(&[("test/entry/activity/timing", json!("R1/P1D"))]),
        &web_template(root_node(vec![{
            let mut n = container("INSTRUCTION", &entry_aql("INSTRUCTION"), "entry");
            n.node_id = Some(entry_archetype("INSTRUCTION"));
            n.children = vec![container("ACTIVITY", &activity_aql, "activity")];
            n
        }])),
        NOW,
    )
    .expect("a master05 §ACTIVITY without an explicit action_archetype_id must build");
    assert_eq!(
        built["content"][0]["activities"][0]["action_archetype_id"],
        json!("/.*/")
    );
}

/// master05 §ISM_TRANSITION — 4 rows.
#[test]
fn master05_ism_transition() {
    let flat = flat_entry(
        "ACTION",
        entry_in_context("ACTION"),
        entry_of("ACTION", action_attributes()),
    );
    check(
        &flat,
        "test/entry/ism_transition",
        &[
            row(
                "/current_state",
                At(Sub("|code")),
                "yes",
                "RM ehr §ISM_TRANSITION declares `current_state` 1..1; the flat build fills \
                 the openEHR `524|initial` state rather than rejecting",
            ),
            row("/transition", At(Sub("|code")), "no", ""),
            row("/careflow_step", At(Sub("|code")), "no", ""),
            row("/_reason:0", At(Str), "no", ""),
        ],
    );
}

/// master05 §INSTRUCTION_DETAILS — 3 rows.
///
/// The table and the section's example block agree: three STRING suffixes on
/// the `_instruction_details` node itself. This test fails against an emitter
/// that nests an `instruction_id` OBJECT_REF child (no `|composition_uid`
/// exists there, and `|path` sits one level too deep).
#[test]
fn master05_instruction_details() {
    let flat = flat_entry(
        "ACTION",
        entry_in_context("ACTION"),
        entry_of("ACTION", action_attributes()),
    );
    let base = format!("{ENTRY}/_instruction_details");
    let enforced = "RM ehr §INSTRUCTION_DETAILS declares `instruction_id` 1..1 and \
                    `activity_id` 1..1 (`Activity_path_valid`); the flat build derives the \
                    LOCATABLE_REF from `|composition_uid` (plus the OBJECT_REF-mandatory \
                    `namespace`/`type`, which master05 gives no suffix for) rather than \
                    rejecting";
    check(
        &flat,
        &base,
        &[
            row("|path", At(Str), "yes", enforced),
            row("|composition_uid", At(Str), "yes", enforced),
            row("|activity_id", At(Str), "yes", enforced),
        ],
    );
    assert_eq!(
        flat[&format!("{base}|composition_uid")],
        json!("4cdc3017-d8c5-4cd3-9900-f3bb7171d006")
    );
    assert_eq!(
        flat[&format!("{base}|activity_id")],
        json!("activities[at0001]")
    );
    // master05 defines no nested `instruction_id` node and no OBJECT_REF
    // suffixes on this table.
    for undefined in ["/instruction_id", "|id", "|type", "|namespace"] {
        assert!(
            !addressed(&flat, &format!("{base}{undefined}")),
            "master05 §INSTRUCTION_DETAILS defines no `{undefined}` on `_instruction_details`"
        );
    }

    // The three suffixes rebuild the RM shape symmetrically.
    let mut entry_node = container("ACTION", &entry_aql("ACTION"), "entry");
    entry_node.node_id = Some(entry_archetype("ACTION"));
    let built = composition_from_flat(
        &flat_of(&[
            (
                "test/entry/_instruction_details|path",
                json!("/content[openEHR-EHR-INSTRUCTION.x.v1]"),
            ),
            (
                "test/entry/_instruction_details|composition_uid",
                json!("4cdc3017-d8c5-4cd3-9900-f3bb7171d006::ehrbase.org::1"),
            ),
            (
                "test/entry/_instruction_details|activity_id",
                json!("activities[at0001]"),
            ),
        ]),
        &web_template(root_node(vec![entry_node])),
        NOW,
    )
    .expect("the master05 §INSTRUCTION_DETAILS suffixes must build");
    let details = &built["content"][0]["instruction_details"];
    assert_eq!(details["activity_id"], json!("activities[at0001]"));
    assert_eq!(
        details["instruction_id"]["path"],
        json!("/content[openEHR-EHR-INSTRUCTION.x.v1]")
    );
    assert_eq!(
        details["instruction_id"]["id"]["value"],
        json!("4cdc3017-d8c5-4cd3-9900-f3bb7171d006::ehrbase.org::1")
    );
    // BASE base_types §LOCATABLE_REF / §OBJECT_REF: `namespace` and `type` are
    // 1..1 and carry no flat suffix, so the build derives them.
    assert_eq!(details["instruction_id"]["namespace"], json!("EHR"));
    assert_eq!(details["instruction_id"]["type"], json!("COMPOSITION"));
    assert_eq!(
        details["instruction_id"]["id"]["_type"],
        json!("OBJECT_VERSION_ID")
    );
}

/// master05 §EVENT_CONTEXT — 6 rows.
#[test]
fn master05_event_context() {
    let flat = flat_context(json!({
        "end_time": dv_date_time("2021-12-21T15:19:31+01:00"),
        "location": "microbiology lab 2",
        "health_care_facility": party_identified("Hospital", "9091"),
        "participations": [participation(performer_with_identifiers())],
    }));
    check(
        &flat,
        CONTEXT,
        &[
            row(
                "/start_time",
                Elsewhere("ctx/time"),
                "yes",
                "RM ehr §EVENT_CONTEXT declares `start_time` 1..1; master06 §time makes it \
                 the `ctx/time` shortcut (defaulting to `now()`), so the path form is never \
                 required on input and is not emitted — but IS accepted, asserted below",
            ),
            row(
                "/setting",
                Elsewhere("ctx/setting"),
                "yes",
                "RM ehr §EVENT_CONTEXT declares `setting` 1..1; master06 §setting makes it \
                 the `ctx/setting` shortcut, which the builder defaults — the path form is \
                 accepted too, asserted below",
            ),
            row("/_end_time", Elsewhere("ctx/end_time"), "no", ""),
            row("/_location", Elsewhere("ctx/location"), "no", ""),
            row("/_health_care_facility", At(Sub("|name")), "no", ""),
            row("/_participation:0", At(Sub("|function")), "no", ""),
        ],
    );

    // The route the two required rows actually travel on output
    // (master06 §§time, setting).
    let built = composition_from_flat(
        &flat_of(&[
            ("ctx/time", json!("2021-12-21T14:19:31+01:00")),
            ("ctx/setting", json!("238")),
        ]),
        &web_template(root_node(vec![])),
        NOW,
    )
    .expect("the master06 context shortcuts must build an EVENT_CONTEXT");
    assert_eq!(
        built["context"]["start_time"]["value"],
        json!("2021-12-21T14:19:31+01:00")
    );
    assert_eq!(
        built["context"]["setting"]["defining_code"]["code_string"],
        json!("238")
    );

    // …and the table's own PATH spellings are honoured on input, not ignored:
    // the Web-Template context node always carries synthesized
    // `start_time`/`setting` children, which used to make the builder treat
    // the path keys as "already handled" and silently drop them in favour of
    // the `ctx/` defaults — the one outcome master04 §Validation forbids.
    let built = composition_from_flat(
        &flat_of(&[
            (
                "test/context/start_time",
                json!("2021-12-21T14:19:31+01:00"),
            ),
            ("test/context/setting|code", json!("238")),
            ("test/context/setting|value", json!("other care")),
            ("test/context/setting|terminology", json!("openehr")),
        ]),
        &web_template(root_node(vec![])),
        NOW,
    )
    .expect("the master05 §EVENT_CONTEXT path spellings must build");
    assert_eq!(
        built["context"]["start_time"]["value"],
        json!("2021-12-21T14:19:31+01:00")
    );
    assert_eq!(
        built["context"]["setting"]["defining_code"]["code_string"],
        json!("238")
    );
    assert_eq!(built["context"]["setting"]["value"], json!("other care"));

    // The `/setting` row's Note binds it to a ValueSet ("openEHR `setting`
    // group"), so a bare `|code` resolves its rubric from that group exactly
    // as `ctx/setting` does (master06 §setting) rather than standing as a
    // `local` code.
    let built = composition_from_flat(
        &flat_of(&[("test/context/setting|code", json!("238"))]),
        &web_template(root_node(vec![])),
        NOW,
    )
    .expect("a bare master05 §EVENT_CONTEXT `/setting|code` must build");
    assert_eq!(
        built["context"]["setting"]["defining_code"]["terminology_id"]["value"],
        json!("openehr")
    );
    assert_eq!(built["context"]["setting"]["value"], json!("other care"));

    // A spelling master05 does NOT define on EVENT_CONTEXT is still rejected
    // loudly (master04 §Validation: field identifiers match WT metadata
    // structure) — the fix widens what is honoured, never what is ignored.
    assert!(
        matches!(
            composition_from_flat(
                &flat_of(&[("test/context/frobnicate", json!("x"))]),
                &web_template(root_node(vec![])),
                NOW,
            ),
            Err(FlatError::UnknownPath(_))
        ),
        "an undefined EVENT_CONTEXT path must be rejected, never ignored"
    );
}

/// master05 §PARTICIPATION — 10 rows, plus the section's own `time` NOTE.
#[test]
fn master05_participation() {
    let flat = flat_context(json!({
        "participations": [{
            "_type": "PARTICIPATION",
            "function": dv_text("requester"),
            "mode": dv_coded("face-to-face communication", "openehr", "216"),
            "time": {"_type": "DV_INTERVAL",
                     "lower": dv_date_time("2021-12-21T14:00:00+01:00")},
            "performer": performer_with_identifiers(),
        }],
    }));
    let base = format!("{CONTEXT}/_participation:0");
    check(
        &flat,
        &base,
        &[
            row(
                "|function",
                At(Str),
                "yes",
                "RM common §PARTICIPATION declares `function` 1..1; the flat build \
                 substitutes an empty DV_TEXT rather than rejecting",
            ),
            row("|mode", At(Str), "no", ""),
            row("|name", At(Str), "no", ""),
            row("|id", At(Str), "no", ""),
            row("|id_scheme", At(Str), "no", ""),
            row("|id_namespace", At(Str), "(yes)", ""),
            row("|identifiers_id:0", At(Str), "no", ""),
            row("|identifiers_issuer:0", At(Str), "no", ""),
            row("|identifiers_assigner:0", At(Str), "no", ""),
            row("|identifiers_type:0", At(Str), "no", ""),
        ],
    );
    // The section's closing NOTE: "PARTICIPATION's `time` … is not currently
    // emitted by FLAT mappings" — a spec-declared hole, asserted as absent.
    for spelling in ["|time", "/time"] {
        assert!(
            !addressed(&flat, &format!("{base}{spelling}")),
            "master05 §PARTICIPATION's NOTE declares `time` not emitted by FLAT mappings"
        );
    }
}

/// master05 §"PARTY_RELATED performer" — 1 row.
#[test]
fn master05_participation_party_related_performer() {
    let mut performer = party_identified("Susan Doe", "199");
    merge(
        &mut performer,
        json!({"_type": "PARTY_RELATED", "relationship": dv_coded("mother", "openehr", "10")}),
    );
    let flat = flat_context(json!({"participations": [participation(performer)]}));
    check(
        &flat,
        &format!("{CONTEXT}/_participation:0"),
        &[row("/relationship", At(Sub("|code")), "(yes)", "")],
    );
}

/// master05 §OBJECT_REF — 4 rows.
#[test]
fn master05_object_ref() {
    let flat = flat_entry(
        "INSTRUCTION",
        vec![],
        entry_of(
            "INSTRUCTION",
            json!({
                "narrative": dv_text("narrative"),
                "guideline_id": object_ref("GUIDELINE", "3445"),
            }),
        ),
    );
    let base = format!("{ENTRY}/_guideline_id");
    let enforced = "BASE base_types §OBJECT_REF declares `namespace`, `type` and `id` 1..1; \
                    the flat build defaults the absent parts (`ANY`, `EHR`, `id_scheme`) \
                    rather than rejecting";
    check(
        &flat,
        &base,
        &[
            row("|type", At(Str), "yes", enforced),
            row("|id", At(Str), "yes", enforced),
            row(
                "|scheme",
                Elsewhere("test/entry/_guideline_id|id_scheme"),
                "yes",
                "the section's own example block spells this suffix `|id_scheme` \
                 (`_guideline_id|id_scheme`), and so does every other example that carries \
                 it; the example blocks are the wire authority. Both spellings are accepted \
                 on input",
            ),
            row("|namespace", At(Str), "yes", enforced),
        ],
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 — the EVENT family
// ═══════════════════════════════════════════════════════════════════════════

/// master05 §INTERVAL_EVENT — 3 rows.
///
/// `|sample_count` is a datum suffix on the event node itself (the section's
/// second example block: `…/any_event:0|sample_count: 5`), so this test fails
/// against an implementation that does not emit it.
#[test]
fn master05_interval_event() {
    let flat = flat_event(
        "INTERVAL_EVENT",
        json!({
            "_type": "INTERVAL_EVENT",
            "archetype_node_id": "at0002",
            "name": dv_text("Any event"),
            "time": dv_date_time("2021-12-21T16:02:58+01:00"),
            "width": {"_type": "DV_DURATION", "value": "P30D"},
            "math_function": dv_coded("mean", "openehr", "146"),
            "sample_count": 5,
            "data": event_data(),
        }),
    );
    let enforced = "RM data_structures §INTERVAL_EVENT declares `width` and `math_function` \
                    1..1; the flat build fills `P0D` and the openEHR `146|mean` default \
                    rather than rejecting";
    check(
        &flat,
        EVENT,
        &[
            row("/width", At(Str), "yes", enforced),
            row("/math_function", At(Sub("|code")), "yes", enforced),
            row("|sample_count", At(Int), "no", ""),
        ],
    );
    assert_eq!(flat[&format!("{EVENT}|sample_count")], json!(5));

    // …and rebuilds symmetrically (RM `sample_count`: Integer 0..1).
    let events_aql = format!("{}/data[at0001]/events[at0002]", entry_aql("OBSERVATION"));
    let mut entry_node = container("OBSERVATION", &entry_aql("OBSERVATION"), "entry");
    entry_node.node_id = Some(entry_archetype("OBSERVATION"));
    entry_node.children = vec![repeating("INTERVAL_EVENT", &events_aql, "any_event")];
    let built = composition_from_flat(
        &flat_of(&[
            ("test/entry/any_event:0|sample_count", json!(5)),
            ("test/entry/any_event:0/width", json!("P30D")),
        ]),
        &web_template(root_node(vec![entry_node])),
        NOW,
    )
    .expect("master05 §INTERVAL_EVENT `|sample_count` must build");
    let event = &built["content"][0]["data"]["events"][0];
    assert_eq!(event["sample_count"], json!(5));
    assert_eq!(event["width"]["value"], json!("P30D"));
}

/// master05 §POINT_EVENT — 1 row.
#[test]
fn master05_point_event() {
    let flat = flat_event(
        "POINT_EVENT",
        json!({
            "_type": "POINT_EVENT",
            "archetype_node_id": "at0002",
            "name": dv_text("Any event"),
            "time": dv_date_time("2021-12-21T16:02:58+01:00"),
            "data": event_data(),
        }),
    );
    check(
        &flat,
        EVENT,
        &[row(
            "/time",
            At(Str),
            "yes",
            "RM data_structures §EVENT declares `time` 1..1; the flat build fills the \
             history origin (master06 §time) rather than rejecting",
        )],
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 — the PARTY_PROXY family
// ═══════════════════════════════════════════════════════════════════════════

/// A FEEDER_AUDIT_DETAILS `/subject` carrying `party` — master05
/// §FEEDER_AUDIT_DETAILS types that slot `PARTY_PROXY`, so it is the fixture
/// for each of the three subtype tables.
fn flat_party_subject(party: Value) -> Map<String, Value> {
    flat_entry(
        "EVALUATION",
        vec![],
        entry_of(
            "EVALUATION",
            json!({"feeder_audit": {
                "_type": "FEEDER_AUDIT",
                "originating_system_audit": {
                    "_type": "FEEDER_AUDIT_DETAILS",
                    "system_id": "orig",
                    "subject": party,
                },
            }}),
        ),
    )
}

const SUBJECT: &str = "test/entry/_feeder_audit/originating_system_audit/subject";

/// master05 §PARTY_PROXY — no table: the section only points at the three
/// concrete subtypes, so the assertion is that a `PARTY_PROXY` slot maps each
/// of them by its own table.
#[test]
fn master05_party_proxy() {
    let self_party = flat_party_subject(json!({
        "_type": "PARTY_SELF",
        "external_ref": {"_type": "PARTY_REF", "namespace": "DEMOGRAPHIC", "type": "PERSON",
                         "id": {"_type": "GENERIC_ID", "value": "42", "scheme": "HOSPITAL-NS"}},
    }));
    assert_eq!(self_party[&format!("{SUBJECT}|id")], json!("42"));

    let identified = flat_party_subject(party_identified("Dr. Marcus Johnson", "199"));
    assert_eq!(
        identified[&format!("{SUBJECT}|name")],
        json!("Dr. Marcus Johnson")
    );

    let mut related = party_identified("Susan Doe", "199");
    merge(
        &mut related,
        json!({"_type": "PARTY_RELATED", "relationship": dv_coded("mother", "openehr", "10")}),
    );
    let related = flat_party_subject(related);
    assert_eq!(
        related[&format!("{SUBJECT}/relationship|code")],
        json!("10")
    );
}

/// master05 §PARTY_SELF — 3 rows.
#[test]
fn master05_party_self() {
    let flat = flat_party_subject(json!({
        "_type": "PARTY_SELF",
        "external_ref": {"_type": "PARTY_REF", "namespace": "DEMOGRAPHIC", "type": "PERSON",
                         "id": {"_type": "GENERIC_ID", "value": "42", "scheme": "HOSPITAL-NS"}},
    }));
    check(
        &flat,
        SUBJECT,
        &[
            row("|id", At(Str), "no", ""),
            // The table's Flat type column reads `Integer`; a scheme is a
            // String in both the RM (BASE base_types §GENERIC_ID `scheme`)
            // and every example block — an editorial slip.
            row("|id_scheme", At(Str), "no", ""),
            row("|id_namespace", At(Str), "(yes)", ""),
        ],
    );
}

/// master05 §PARTY_IDENTIFIED — 5 rows.
#[test]
fn master05_party_identified() {
    let mut facility = party_identified("Hospital", "9091");
    merge(
        &mut facility,
        json!({"identifiers": [{"_type": "DV_IDENTIFIER", "id": "122", "issuer": "issuer"}]}),
    );
    let flat = flat_context(json!({"health_care_facility": facility}));
    check(
        &flat,
        &format!("{CONTEXT}/_health_care_facility"),
        &[
            row("|name", At(Str), "no", ""),
            row("|id", At(Str), "no", ""),
            row("|id_scheme", At(Str), "no", ""),
            row("|id_namespace", At(Str), "(yes)", ""),
            row("/_identifier:0", At(Sub("|id")), "no", ""),
        ],
    );
}

/// master05 §PARTY_RELATED — 6 rows.
#[test]
fn master05_party_related() {
    let mut related = party_identified("Susan Doe", "199");
    merge(
        &mut related,
        json!({
            "_type": "PARTY_RELATED",
            "relationship": dv_coded("mother", "openehr", "10"),
            "identifiers": [{"_type": "DV_IDENTIFIER", "id": "122"}],
        }),
    );
    let flat = flat_party_subject(related);
    check(
        &flat,
        SUBJECT,
        &[
            row("|name", At(Str), "no", ""),
            row("|id", At(Str), "no", ""),
            row("|id_scheme", At(Str), "no", ""),
            row("|id_namespace", At(Str), "(yes)", ""),
            row("/_identifier:0", At(Sub("|id")), "no", ""),
            // The table's Flat Path column reads `/_relationship`, but both
            // example blocks of the SAME section (and the §"PARTY_RELATED
            // performer" table + example) spell it `…/relationship|code` —
            // the example form is the emitted one. The table spelling is
            // accepted on INPUT as an alias; that direction is pinned by
            // `master05_party_related_table_spelling_is_an_input_alias`.
            row(
                "/_relationship",
                Elsewhere(
                    "test/entry/_feeder_audit/originating_system_audit/subject/relationship|code",
                ),
                "(yes)",
                "",
            ),
        ],
    );
}

/// master05 §PARTY_RELATED gives the relationship sub-path two spellings: the
/// mapping table's Flat Path column reads `/_relationship`, while both example
/// blocks of the same section write `"…/composer/relationship|code": "10"`
/// (and the §"PARTY_RELATED performer" table + example agree with the
/// examples). The emitted form is the example one — pinned by the
/// `Elsewhere` row above and re-asserted here — and the table spelling is
/// accepted on **input** as an alias, so a producer that followed the table
/// row is not rejected with an unknown-path error.
#[test]
fn master05_party_related_table_spelling_is_an_input_alias() {
    let mut subject = party_identified("Susan Doe", "199");
    merge(
        &mut subject,
        json!({
            "_type": "PARTY_RELATED",
            "relationship": dv_coded("mother", "openehr", "10"),
        }),
    );
    let (wt, flat) = entry_subject_case(subject.clone());

    // Emission is unchanged: the example spelling, never the table spelling.
    assert!(
        flat.contains_key(&format!("{ENTRY}/subject/relationship|code")),
        "the example spelling is the emitted one: {:?}",
        sorted_keys(&flat)
    );
    assert!(
        !addressed(&flat, &format!("{ENTRY}/subject/_relationship")),
        "the `/_relationship` table spelling must never be emitted: {:?}",
        sorted_keys(&flat)
    );
    assert_eq!(built_subject(&wt, &flat), subject, "round-trip unchanged");

    // The same document re-keyed to the table spelling rebuilds the identical
    // PARTY_RELATED: the alias is read as the PARTY_PROXY subtype
    // discriminator, not attached to an already-decided PARTY_IDENTIFIED.
    let aliased: Map<String, Value> = flat
        .iter()
        .map(|(k, v)| {
            (
                k.replace(
                    &format!("{ENTRY}/subject/relationship"),
                    &format!("{ENTRY}/subject/_relationship"),
                ),
                v.clone(),
            )
        })
        .collect();
    assert!(
        aliased.contains_key(&format!("{ENTRY}/subject/_relationship|code")),
        "the alias fixture carries the table spelling: {:?}",
        sorted_keys(&aliased)
    );
    assert_eq!(
        built_subject(&wt, &aliased),
        subject,
        "the `/_relationship` table spelling must build the same PARTY_RELATED \
         as the `/relationship` example spelling"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 — the text family
// ═══════════════════════════════════════════════════════════════════════════

/// The `_language`/`_encoding`/`_mapping:i` attributes the DV_TEXT-family
/// tables share.
fn text_meta() -> Value {
    json!({
        "language": code_phrase("ISO_639-1", "en"),
        "encoding": code_phrase("IANA_character-sets", "UTF-8"),
        "mappings": [{
            "_type": "TERM_MAPPING",
            "match": "=",
            "target": code_phrase("SNOMED-CT", "260360000"),
            "purpose": dv_coded("public health", "openehr", "638"),
        }],
    })
}

/// master05 §DV_TEXT — 5 rows.
#[test]
fn master05_dv_text() {
    let mut value = dv_text("DV_TEXT 45");
    merge(&mut value, json!({"formatting": "bold"}));
    merge(&mut value, text_meta());
    let flat = flat_leaf("DV_TEXT", value);
    check(
        &flat,
        LEAF,
        &[
            row(
                "|value",
                At(Str),
                "yes",
                "RM data_types §DV_TEXT declares `value` 1..1, and the flat build REJECTS a \
                 leaf carrying neither the bare datum nor `|value` (`FlatError::InvalidValue`)",
            ),
            row("|formatting", At(Str), "no", ""),
            row("/_language", At(Sub("|code")), "no", ""),
            row("/_encoding", At(Sub("|code")), "no", ""),
            row("/_mapping:0", At(Sub("|match")), "no", ""),
        ],
    );

    // The required row is genuinely enforced on build.
    let mut leaf_node = leaf(
        "DV_TEXT",
        &format!(
            "{}/data[at0001]/items[at0002]/value",
            entry_aql("EVALUATION")
        ),
        "leaf",
    );
    leaf_node.min = Some(0);
    let mut entry_node = container("EVALUATION", &entry_aql("EVALUATION"), "entry");
    entry_node.node_id = Some(entry_archetype("EVALUATION"));
    entry_node.children = vec![leaf_node];
    let err = composition_from_flat(
        &flat_of(&[("test/entry/leaf|formatting", json!("bold"))]),
        &web_template(root_node(vec![entry_node])),
        NOW,
    )
    .unwrap_err();
    assert!(matches!(err, FlatError::InvalidValue { .. }), "got {err:?}");
}

/// master05 §CODE_PHRASE — 3 rows.
///
/// A CODE_PHRASE is not a `DATA_VALUE`, so it never wraps in an ELEMENT: the
/// carrier here is the master05 §DV_TEXT `/_language` row, which types its
/// sub-node CODE_PHRASE.
#[test]
fn master05_code_phrase() {
    let mut language = code_phrase("ISO_639-1", "en");
    merge(&mut language, json!({"preferred_term": "English"}));
    let mut value = dv_text("DV_TEXT 45");
    merge(&mut value, json!({"language": language}));
    let flat = flat_leaf("DV_TEXT", value);
    let enforced = "RM data_types §CODE_PHRASE declares `terminology_id` and `code_string` \
                    1..1; the flat build REJECTS a CODE_PHRASE leaf without `|code` \
                    (`FlatError::InvalidValue`) and defaults an absent `|terminology` to \
                    the template's terminology, else `local`";
    check(
        &flat,
        &format!("{LEAF}/_language"),
        &[
            row("|code", At(Str), "yes", enforced),
            row("|terminology", At(Str), "yes", enforced),
            row("|preferred_term", At(Str), "no", ""),
        ],
    );
}

/// master05 §TERM_MAPPING — 3 rows.
#[test]
fn master05_term_mapping() {
    let mut value = dv_text("DV_TEXT 45");
    merge(&mut value, text_meta());
    let flat = flat_leaf("DV_TEXT", value);
    check(
        &flat,
        &format!("{LEAF}/_mapping:0"),
        &[
            row(
                "|match",
                At(Str),
                "yes",
                "RM data_types §TERM_MAPPING declares `match` 1..1; the flat build \
                 substitutes `=` rather than rejecting",
            ),
            row(
                "/target",
                At(Sub("|code")),
                "yes",
                "RM data_types §TERM_MAPPING declares `target` 1..1; the flat build omits it \
                 when no `/target|code` is supplied rather than rejecting",
            ),
            row("/purpose", At(Sub("|code")), "no", ""),
        ],
    );
}

/// master05 §DV_CODED_TEXT — 9 rows.
#[test]
fn master05_dv_coded_text() {
    let mut value = dv_coded("high", "SNOMED-CT", "260360000");
    merge(
        &mut value,
        json!({"formatting": "bold",
               "defining_code": {
                   "_type": "CODE_PHRASE",
                   "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "SNOMED-CT"},
                   "code_string": "260360000",
                   "preferred_term": "high"}}),
    );
    merge(&mut value, text_meta());
    let flat = flat_leaf("DV_CODED_TEXT", value);
    check(
        &flat,
        LEAF,
        &[
            row(
                "|code",
                At(Str),
                "yes",
                "RM data_types §DV_CODED_TEXT declares `defining_code` 1..1; the flat build \
                 REJECTS a coded leaf without `|code` (`FlatError::InvalidValue`)",
            ),
            row("|value", At(Str), "(yes)", ""),
            row("|terminology", At(Str), "(yes)", ""),
            row("|preferred_term", At(Str), "no", ""),
            row("|formatting", At(Str), "no", ""),
            row("/_language", At(Sub("|code")), "no", ""),
            row("/_encoding", At(Sub("|code")), "no", ""),
            row("/_mapping:0", At(Sub("|match")), "no", ""),
        ],
    );

    // `|other` — the open-value-set row, which excludes the coded suffixes.
    let value_aql = format!(
        "{}/data[at0001]/items[at0002]/value",
        entry_aql("EVALUATION")
    );
    let open = flat_element_node(
        open_coded_leaf(&value_aql, "leaf"),
        element(dv_text("free text")),
    );
    check(&open, LEAF, &[row("|other", At(Str), "no", "")]);
    for coded in ["|code", "|value", "|terminology", "|preferred_term"] {
        assert!(
            !open.contains_key(&format!("{LEAF}{coded}")),
            "master05 §DV_CODED_TEXT `|other` excludes `{coded}`"
        );
    }
}

/// master05 §"When a `DV_CODED_TEXT` becomes a `DV_TEXT`" — no table; the
/// section states three normative rules about the `|other` suffix.
#[test]
fn master05_coded_text_becomes_dv_text() {
    let value_aql = format!(
        "{}/data[at0001]/items[at0002]/value",
        entry_aql("EVALUATION")
    );
    let mut entry_node = container("EVALUATION", &entry_aql("EVALUATION"), "entry");
    entry_node.node_id = Some(entry_archetype("EVALUATION"));
    entry_node.children = vec![open_coded_leaf(&value_aql, "leaf")];
    let wt = web_template(root_node(vec![entry_node]));

    // "the canonical RM serialisation of the leaf is a DV_TEXT, not a
    // DV_CODED_TEXT with empty defining_code".
    let built = composition_from_flat(
        &flat_of(&[("test/entry/leaf|other", json!("free text"))]),
        &wt,
        NOW,
    )
    .expect("an open value-set `|other` must build");
    let leaf_value = &built["content"][0]["data"]["items"][0]["value"];
    assert_eq!(leaf_value["_type"], json!("DV_TEXT"));
    assert_eq!(leaf_value["value"], json!("free text"));
    assert!(leaf_value.get("defining_code").is_none());

    // "`|other` is mutually exclusive with `|code`, `|value`, `|terminology`
    // and `|preferred_term` on the same leaf".
    for conflicting in ["code", "value", "terminology", "preferred_term"] {
        let mut doc = Map::new();
        doc.insert("test/entry/leaf|other".to_owned(), json!("free text"));
        doc.insert(format!("test/entry/leaf|{conflicting}"), json!("x"));
        let err = composition_from_flat(&doc, &wt, NOW).unwrap_err();
        assert!(
            matches!(err, FlatError::OtherSuffixConflict(_)),
            "`|other` + `|{conflicting}` must conflict, got {err:?}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// master05 — the DV_ORDERED / scalar family
// ═══════════════════════════════════════════════════════════════════════════

/// A `DV_INTERVAL<T>` normal range plus one `REFERENCE_RANGE<T>`, for the
/// `/_normal_range` and `/_other_reference_ranges:i` rows the DV_ORDERED
/// tables share. `lower`/`upper` are values of `T`.
fn reference_ranges(lower: Value, upper: Value) -> Value {
    json!({
        "normal_range": {"_type": "DV_INTERVAL", "lower": lower.clone(), "upper": upper.clone()},
        "other_reference_ranges": [
            {"_type": "REFERENCE_RANGE",
             "meaning": dv_text("high"),
             "range": {"_type": "DV_INTERVAL", "lower": lower, "upper": upper}},
            {"_type": "REFERENCE_RANGE",
             "meaning": dv_text("very high"),
             "range": {"_type": "DV_INTERVAL", "lower_unbounded": true,
                       "upper_unbounded": true, "lower_included": false,
                       "upper_included": false}},
        ],
    })
}

/// The `DV_ORDERED` rows every scalar table repeats. `normal_range` is the
/// witness for the `/_normal_range` row — a `DV_INTERVAL<T>` whose endpoints
/// are values of the host type, so the witness differs per table.
fn ordered_rows(normal_range: FlatType) -> Vec<Row> {
    vec![
        row("|normal_status", At(Str), "no", ""),
        row("/_normal_range", At(normal_range), "no", ""),
        row("/_other_reference_ranges:0", At(Sub("/meaning")), "no", ""),
    ]
}

fn quantity(magnitude: f64) -> Value {
    json!({"_type": "DV_QUANTITY", "magnitude": magnitude, "units": "unit"})
}

/// master05 §DV_ORDINAL — 5 rows.
#[test]
fn master05_dv_ordinal() {
    let ordinal = |value: i64, code: &str, symbol: &str| {
        json!({"_type": "DV_ORDINAL", "value": value,
               "symbol": dv_coded(symbol, "local", code)})
    };
    let mut value = ordinal(1, "at0002", "Mild");
    merge(
        &mut value,
        reference_ranges(ordinal(0, "at0001", "None"), ordinal(3, "at0004", "Severe")),
    );
    let flat = flat_leaf("DV_ORDINAL", value);
    check(
        &flat,
        LEAF,
        &[
            row(
                "|code",
                At(Str),
                "Yes",
                "RM data_types §DV_ORDINAL declares `symbol` 1..1; the flat build REJECTS an \
                 ordinal leaf without `|code` (`FlatError::InvalidValue`)",
            ),
            row("|value", At(Str), "(Yes)", ""),
            row("|ordinal", At(Int), "(Yes)", ""),
            row("/_normal_range", At(Sub("/lower|code")), "no", ""),
            row("/_other_reference_ranges:0", At(Sub("/meaning")), "no", ""),
        ],
    );
}

/// master05 §DV_BOOLEAN — 1 row (the bare datum).
#[test]
fn master05_dv_boolean() {
    let flat = flat_leaf("DV_BOOLEAN", json!({"_type": "DV_BOOLEAN", "value": true}));
    check(
        &flat,
        LEAF,
        &[row(
            "",
            At(Bool),
            "Yes",
            "RM data_types §DV_BOOLEAN declares `value` 1..1; the flat build REJECTS a \
             boolean leaf without the bare datum (`FlatError::InvalidValue`)",
        )],
    );
}

/// master05 §DV_URI — 1 row (the bare datum).
#[test]
fn master05_dv_uri() {
    let flat = flat_leaf(
        "DV_URI",
        json!({"_type": "DV_URI", "value": "https://example.org/x"}),
    );
    check(
        &flat,
        LEAF,
        &[row(
            "",
            At(Str),
            "Yes",
            "RM data_types §DV_URI declares `value` 1..1; the flat build REJECTS a URI leaf \
             without the bare datum (`FlatError::InvalidValue`)",
        )],
    );
}

/// master05 §DV_EHR_URI — 1 row (the bare datum).
#[test]
fn master05_dv_ehr_uri() {
    let flat = flat_leaf(
        "DV_EHR_URI",
        json!({"_type": "DV_EHR_URI", "value": "ehr://ehr.network/347a5490"}),
    );
    check(
        &flat,
        LEAF,
        &[row(
            "",
            At(Str),
            "Yes",
            "RM data_types §DV_EHR_URI declares `value` 1..1 (inherited from DV_URI); the \
             flat build REJECTS the leaf without the bare datum",
        )],
    );
}

/// master05 §DV_IDENTIFIER — 4 rows.
#[test]
fn master05_dv_identifier() {
    let flat = flat_leaf(
        "DV_IDENTIFIER",
        json!({"_type": "DV_IDENTIFIER", "id": "id-1", "issuer": "issuer",
               "assigner": "assigner", "type": "type"}),
    );
    check(
        &flat,
        LEAF,
        &[
            row(
                "|id",
                At(Str),
                "Yes",
                "the row's own Note says the input may omit `|id`, so nothing rejects: RM \
                 data_types §DV_IDENTIFIER declares `id` 1..1 and the flat build substitutes \
                 an empty string",
            ),
            row("|issuer", At(Str), "no", ""),
            row("|assigner", At(Str), "no", ""),
            row("|type", At(Str), "no", ""),
        ],
    );
}

/// master05 §DV_QUANTITY — 8 rows.
#[test]
fn master05_dv_quantity() {
    let mut value = json!({
        "_type": "DV_QUANTITY",
        "magnitude": 65.9,
        "units": "unit",
        "magnitude_status": "=",
        "normal_status": code_phrase("openehr", "N"),
        "accuracy": 0.5,
        "accuracy_is_percent": false,
    });
    merge(&mut value, reference_ranges(quantity(70.5), quantity(77.6)));
    let flat = flat_leaf("DV_QUANTITY", value);
    let enforced = "RM data_types §DV_QUANTITY declares `magnitude` and `units` 1..1; the \
                    flat build REJECTS a quantity leaf without `|magnitude` \
                    (`FlatError::InvalidValue`)";
    let mut rows = vec![
        // The table's Flat type column swaps the two: it types `|magnitude` as
        // String and `|unit` as Real. The section's example block
        // (`|magnitude: 65.9`, `|unit: "unit"`) is the wire authority.
        row("|magnitude", At(Real), "yes", enforced),
        row("|unit", At(Str), "yes", enforced),
        row("|magnitude_status", At(Str), "no", ""),
        row("|accuracy", At(Real), "no", ""),
        row("|accuracy_is_percent", At(Bool), "no", ""),
    ];
    rows.extend(ordered_rows(SubNum("/lower|magnitude")));
    check(&flat, LEAF, &rows);
}

/// master05 §DV_PROPORTION — 11 rows.
#[test]
fn master05_dv_proportion() {
    let proportion = |numerator: f64, denominator: f64| {
        json!({"_type": "DV_PROPORTION", "numerator": numerator,
               "denominator": denominator, "type": 0})
    };
    let mut value = proportion(1.0, 4.0);
    merge(
        &mut value,
        json!({
            "precision": 2,
            "magnitude_status": "=",
            "normal_status": code_phrase("openehr", "N"),
            "accuracy": 0.5,
            "accuracy_is_percent": false,
        }),
    );
    merge(
        &mut value,
        reference_ranges(proportion(0.0, 4.0), proportion(4.0, 4.0)),
    );
    let flat = flat_leaf("DV_PROPORTION", value);
    let enforced = "RM data_types §DV_PROPORTION declares `numerator`, `denominator` and \
                    `type` 1..1; the flat build REJECTS a proportion leaf without \
                    `|numerator` (`FlatError::InvalidValue`) and omits an absent \
                    `|denominator`/`|type`";
    let mut rows = vec![
        row("|numerator", At(Real), "yes", enforced),
        row("|denominator", At(Real), "yes", enforced),
        row("|type", At(Int), "yes", enforced),
        row("|precision", At(Int), "no", ""),
        // The unnamed row: "magnitude … calculated on output".
        row("", At(Real), "no", ""),
        row("|magnitude_status", At(Str), "no", ""),
        row("|accuracy", At(Real), "no", ""),
        row("|accuracy_is_percent", At(Bool), "no", ""),
    ];
    rows.extend(ordered_rows(SubNum("/lower|numerator")));
    check(&flat, LEAF, &rows);
}

/// master05 §DV_COUNT — 7 rows.
#[test]
fn master05_dv_count() {
    let count = |magnitude: i64| json!({"_type": "DV_COUNT", "magnitude": magnitude});
    let mut value = count(5);
    merge(
        &mut value,
        json!({
            "magnitude_status": "=",
            "normal_status": code_phrase("openehr", "N"),
            "accuracy": 0.5,
            "accuracy_is_percent": false,
        }),
    );
    merge(&mut value, reference_ranges(count(1), count(9)));
    let flat = flat_leaf("DV_COUNT", value);
    let mut rows = vec![
        row(
            "",
            At(Int),
            "Yes",
            "RM data_types §DV_COUNT declares `magnitude` 1..1; the flat build REJECTS a \
             count leaf without the bare datum (`FlatError::InvalidValue`)",
        ),
        row("|magnitude_status", At(Str), "no", ""),
        row("|accuracy", At(Real), "no", ""),
        row("|accuracy_is_percent", At(Bool), "no", ""),
    ];
    rows.extend(ordered_rows(SubNum("/lower")));
    check(&flat, LEAF, &rows);
}

/// The rows the three temporal tables (DV_DATE / DV_DATE_TIME / DV_TIME)
/// share: the bare ISO-8601 value, the `/_accuracy` DV_DURATION sub-path, and
/// the DV_ORDERED family.
fn temporal_rows(rm_type: &'static str) -> Vec<Row> {
    let enforced: &'static str = match rm_type {
        "DV_DATE" => {
            "RM data_types §DV_DATE declares `value` 1..1; the flat build REJECTS the \
                      leaf without the bare datum (`FlatError::InvalidValue`)"
        }
        "DV_TIME" => {
            "RM data_types §DV_TIME declares `value` 1..1; the flat build REJECTS the \
                      leaf without the bare datum (`FlatError::InvalidValue`)"
        }
        _ => {
            "RM data_types §DV_DATE_TIME declares `value` 1..1; the flat build REJECTS the \
              leaf without the bare datum (`FlatError::InvalidValue`)"
        }
    };
    let mut rows = vec![
        row("", At(Str), "Yes", enforced),
        row("/_accuracy", At(Str), "no", ""),
        row("|magnitude_status", At(Str), "no", ""),
    ];
    rows.extend(ordered_rows(Sub("/lower")));
    rows
}

/// A temporal leaf value with every shared row populated.
fn temporal_value(rm_type: &str, value: &str, lower: &str, upper: &str) -> Value {
    let point = |v: &str| json!({"_type": rm_type, "value": v});
    let mut dv = point(value);
    merge(
        &mut dv,
        json!({
            "accuracy": {"_type": "DV_DURATION", "value": "PT1H"},
            "magnitude_status": "=",
            "normal_status": code_phrase("openehr", "N"),
        }),
    );
    merge(&mut dv, reference_ranges(point(lower), point(upper)));
    dv
}

/// master05 §DV_DATE — 6 rows.
#[test]
fn master05_dv_date() {
    let flat = flat_leaf(
        "DV_DATE",
        temporal_value("DV_DATE", "2021-12-21", "2021-12-01", "2021-12-31"),
    );
    check(&flat, LEAF, &temporal_rows("DV_DATE"));
}

/// master05 §DV_DATE_TIME — 6 rows.
#[test]
fn master05_dv_date_time() {
    let flat = flat_leaf(
        "DV_DATE_TIME",
        temporal_value(
            "DV_DATE_TIME",
            "2021-12-21T16:02:58+01:00",
            "2021-12-01T00:00:00+01:00",
            "2021-12-31T00:00:00+01:00",
        ),
    );
    check(&flat, LEAF, &temporal_rows("DV_DATE_TIME"));
}

/// master05 §DV_TIME — 6 rows.
#[test]
fn master05_dv_time() {
    let flat = flat_leaf(
        "DV_TIME",
        temporal_value("DV_TIME", "16:02:58", "08:00:00", "18:00:00"),
    );
    check(&flat, LEAF, &temporal_rows("DV_TIME"));
}

/// master05 §DV_DURATION — 7 rows.
#[test]
fn master05_dv_duration() {
    let duration = |v: &str| json!({"_type": "DV_DURATION", "value": v});
    let mut value = duration("P30D");
    merge(
        &mut value,
        json!({
            "accuracy": 0.5,
            "accuracy_is_percent": false,
            "magnitude_status": "=",
            "normal_status": code_phrase("openehr", "N"),
        }),
    );
    merge(
        &mut value,
        reference_ranges(duration("P1D"), duration("P60D")),
    );
    let flat = flat_leaf("DV_DURATION", value);
    let mut rows = vec![
        row(
            "",
            At(Str),
            "Yes",
            "RM data_types §DV_DURATION declares `value` 1..1; the flat build REJECTS the \
             leaf without the bare datum (`FlatError::InvalidValue`)",
        ),
        row("|accuracy", At(Real), "no", ""),
        row("|accuracy_is_percent", At(Bool), "no", ""),
        row("|magnitude_status", At(Str), "no", ""),
    ];
    rows.extend(ordered_rows(Sub("/lower")));
    check(&flat, LEAF, &rows);
}

/// master05 §REFERENCE_RANGE — 7 rows.
///
/// The four boundary flags are "only in output if" they differ from their
/// default (master05 §DV_INTERVAL), so the bounded range (`:0`) carries the
/// bounds + meaning and the unbounded one (`:1`) carries the flags.
#[test]
fn master05_reference_range() {
    let mut value = quantity(65.9);
    merge(&mut value, reference_ranges(quantity(70.5), quantity(77.6)));
    let flat = flat_leaf("DV_QUANTITY", value);
    check(
        &flat,
        &format!("{LEAF}/_other_reference_ranges:0"),
        &[
            row("/lower", At(SubNum("|magnitude")), "no", ""),
            row("/upper", At(SubNum("|magnitude")), "no", ""),
            // The table spells this row `\meaning`; the section's example
            // block spells it `/meaning|value`, and a plain DV_TEXT meaning
            // emits as the bare datum (master05 §DV_TEXT).
            row(
                "/meaning",
                At(Str),
                "yes",
                "RM data_types §REFERENCE_RANGE declares `meaning` 1..1; the flat build omits \
                 an absent meaning rather than rejecting",
            ),
        ],
    );
    check(
        &flat,
        &format!("{LEAF}/_other_reference_ranges:1"),
        &[
            row("|lower_unbounded", At(Bool), "no", ""),
            row("|upper_unbounded", At(Bool), "no", ""),
            row("|lower_included", At(Bool), "no", ""),
            row("|upper_included", At(Bool), "no", ""),
        ],
    );
}

/// master05 §DV_PARSABLE — 4 rows.
#[test]
fn master05_dv_parsable() {
    let flat = flat_leaf(
        "DV_PARSABLE",
        json!({
            "_type": "DV_PARSABLE",
            "value": "R4/2022-01-31T10:00:00+01:00/P3M",
            "formalism": "timing",
            "charset": code_phrase("IANA_character-sets", "UTF-8"),
            "language": code_phrase("ISO_639-1", "en"),
        }),
    );
    check(
        &flat,
        LEAF,
        &[
            row(
                "|value",
                Elsewhere(LEAF),
                "Yes",
                "the datum is emitted bare, as every example block that carries a DV_PARSABLE \
                 spells it (master05 §§ACTIVITY, INSTRUCTION: `…/timing` + `…/timing|formalism`); \
                 the `|value` spelling is accepted on input. RM data_types §DV_PARSABLE \
                 declares `value` 1..1 and the flat build REJECTS a leaf carrying neither",
            ),
            row(
                "|formalism",
                At(Str),
                "Yes",
                "RM data_types §DV_PARSABLE declares `formalism` 1..1; the flat build \
                 substitutes an empty string rather than rejecting",
            ),
            row("/_charset", At(Sub("|code")), "no", ""),
            row("/_language", At(Sub("|code")), "no", ""),
        ],
    );
}

/// master05 §DV_MULTIMEDIA — 11 rows.
#[test]
fn master05_dv_multimedia() {
    let multimedia = json!({
        "_type": "DV_MULTIMEDIA",
        "uri": {"_type": "DV_URI", "value": "https://example.org/x.png"},
        "media_type": code_phrase("IANA_media-types", "image/png"),
        "size": 1024,
        "alternate_text": "an image",
        "compression_algorithm": code_phrase("openehr_compression_algorithms", "other"),
        "integrity_check": "3q2+7w==",
        "integrity_check_algorithm": code_phrase("openehr_integrity_check_algorithms", "SHA-256"),
        "data": "3q2+7w==",
        "charset": code_phrase("IANA_character-sets", "UTF-8"),
        "language": code_phrase("ISO_639-1", "en"),
    });
    let mut value = multimedia.clone();
    merge(&mut value, json!({"thumbnail": multimedia}));
    let flat = flat_leaf("DV_MULTIMEDIA", value);
    let enforced = "RM data_types §DV_MULTIMEDIA declares `media_type` 1..1 and `size` 1..1; \
                    the flat build omits an absent `|mediatype`/`|size` rather than rejecting";
    check(
        &flat,
        LEAF,
        &[
            // The unnamed row: `uri.value` as the bare datum.
            row("", At(Str), "no", ""),
            row("|mediatype", At(Str), "Yes", enforced),
            row("|size", At(Int), "Yes", enforced),
            row("|alternatetext", At(Str), "no", ""),
            row("|compression_algorithm", At(Str), "no", ""),
            row("|integrity_check_algorithm", At(Str), "no", ""),
            row("|integrity_check", At(Str), "no", ""),
            // The row's RM Path column reads `dta` — a typo for `data`
            // (RM data_types §DV_MULTIMEDIA `data`: List<Byte>).
            row("|data", At(Str), "no", ""),
            row("/_thumbnail", At(Sub("|mediatype")), "no", ""),
            row("/_charset", At(Sub("|code")), "no", ""),
            row("/_language", At(Sub("|code")), "no", ""),
        ],
    );
}

/// master05 §DV_INTERVAL — 6 rows.
///
/// As with §REFERENCE_RANGE, the boundary flags are emitted only when they
/// differ from their defaults, so two fixtures are needed.
#[test]
fn master05_dv_interval() {
    let value_aql = format!(
        "{}/data[at0001]/items[at0002]/value",
        entry_aql("EVALUATION")
    );
    let interval_leaf = || leaf("DV_INTERVAL<DV_QUANTITY>", &value_aql, "leaf");

    let bounded = flat_element_node(
        interval_leaf(),
        element(json!({"_type": "DV_INTERVAL", "lower": quantity(72.83),
                       "upper": quantity(80.83)})),
    );
    check(
        &bounded,
        LEAF,
        &[
            row("/lower", At(SubNum("|magnitude")), "no", ""),
            row("/upper", At(SubNum("|magnitude")), "no", ""),
        ],
    );

    let open = flat_element_node(
        interval_leaf(),
        element(json!({"_type": "DV_INTERVAL", "lower_unbounded": true,
                       "upper_unbounded": true, "lower_included": false,
                       "upper_included": false})),
    );
    check(
        &open,
        LEAF,
        &[
            row("|lower_unbounded", At(Bool), "no", ""),
            row("|upper_unbounded", At(Bool), "no", ""),
            row("|lower_included", At(Bool), "no", ""),
            row("|upper_included", At(Bool), "no", ""),
        ],
    );
}
