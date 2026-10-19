// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party build (`flat::build::build_party`) and the caller-chosen
//! `ARCHETYPED.rm_version` of the builders.
//!
//! A party template (a `PERSON`, `ORGANISATION` or `ROLE` root, RM demographic
//! `master02-demographic_package.adoc`) builds a party that decodes as its root
//! class, with `identities` as the `List<PARTY_IDENTITY>` the RM declares, no
//! composition attribute, a compacted structure named by its term definition,
//! and a refusal for a `ctx/` key. The templates are synthetic, generated here.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::json::from_canonical_value;
use openehr_its::opt14;
use openehr_rm::v1_2::demographic::organisation::Organisation;
use openehr_rm::v1_2::demographic::person::Person;
use openehr_rm::v1_2::demographic::role::Role;
use openehr_sdt::flat::build::{BuildOptions, RM_VERSION, build_composition_with};
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat, party_from_flat};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::sim::flat::parse_flat;
use openehr_sdt::flat::tdd::{from_tdd, from_tdd_with};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use serde_json::{Map, Value, json};

const CORPUS: &str = "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets";
const NOW: &str = "2026-01-01T00:00:00Z";

/// An OPT 1.4 interval element body.
fn interval(lower: u32, upper: Option<u32>) -> String {
    let upper_part = upper.map_or_else(
        || "<upper_unbounded>true</upper_unbounded>".to_owned(),
        |u| format!("<upper_unbounded>false</upper_unbounded><upper>{u}</upper>"),
    );
    format!(
        "<lower_included>true</lower_included><upper_included>{}</upper_included>\
         <lower_unbounded>false</lower_unbounded>{upper_part}<lower>{lower}</lower>",
        upper.is_some()
    )
}

/// A `C_COMPLEX_OBJECT` of `rm_type` with `node_id`, occurring at least once
/// and at most `upper` times (unbounded for `None`), with the given attribute
/// bodies.
fn object(kind: &str, rm_type: &str, node_id: &str, upper: Option<u32>, attrs: &str) -> String {
    format!(
        "<children xsi:type=\"{kind}\"><rm_type_name>{rm_type}</rm_type_name>\
         <occurrences>{}</occurrences><node_id>{node_id}</node_id>{attrs}</children>",
        interval(1, upper)
    )
}

/// A single attribute holding `children`.
fn single(name: &str, lower: u32, children: &str) -> String {
    format!(
        "<attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>{name}</rm_attribute_name>\
         <existence>{}</existence>{children}</attributes>",
        interval(lower, Some(1))
    )
}

/// A multiple attribute holding `children`, at least `lower` of them.
fn multiple(name: &str, lower: u32, children: &str) -> String {
    format!(
        "<attributes xsi:type=\"C_MULTIPLE_ATTRIBUTE\"><rm_attribute_name>{name}</rm_attribute_name>\
         <existence>{}</existence>{children}<cardinality><is_ordered>false</is_ordered>\
         <is_unique>false</is_unique><interval>{}</interval></cardinality></attributes>",
        interval(lower.min(1), Some(1)),
        interval(lower, None)
    )
}

/// An `ELEMENT` at `node_id` whose value is a `DV_TEXT`.
fn text_element(node_id: &str) -> String {
    object(
        "C_COMPLEX_OBJECT",
        "ELEMENT",
        node_id,
        Some(1),
        &single(
            "value",
            1,
            &object("C_COMPLEX_OBJECT", "DV_TEXT", "", Some(1), ""),
        ),
    )
}

/// A synthetic party OPT: a `class` root (`openEHR-DEMOGRAPHIC-<class>.synthetic.v1`)
/// with one `PARTY_IDENTITY` (at0003) whose `ITEM_TREE` (at0004, "Identity
/// details") holds one `DV_TEXT` element (at0005, "Name"), and a details tree
/// (at0001) with one `DV_TEXT` element (at0002, "Note").
fn party_template(class: &str) -> WebTemplate {
    let concept = format!("Synthetic {}", class.to_ascii_lowercase());
    let archetype = format!("openEHR-DEMOGRAPHIC-{class}.synthetic.v1");
    let template_id = format!("synthetic.{}.v1", class.to_ascii_lowercase());
    let identity = object(
        "C_COMPLEX_OBJECT",
        "PARTY_IDENTITY",
        "at0003",
        None,
        &single(
            "details",
            1,
            &object(
                "C_COMPLEX_OBJECT",
                "ITEM_TREE",
                "at0004",
                Some(1),
                &multiple("items", 0, &text_element("at0005")),
            ),
        ),
    );
    let details = object(
        "C_COMPLEX_OBJECT",
        "ITEM_TREE",
        "at0001",
        Some(1),
        &multiple("items", 0, &text_element("at0002")),
    );
    let term = |code: &str, text: &str| {
        format!(
            "<term_definitions code=\"{code}\"><items id=\"text\">{text}</items>\
             <items id=\"description\">{text}</items></term_definitions>"
        )
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <template xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <language><terminology_id><value>ISO_639-1</value></terminology_id>\
         <code_string>en</code_string></language>\
         <template_id><value>{template_id}</value></template_id><concept>{concept}</concept>\
         <definition xsi:type=\"C_ARCHETYPE_ROOT\"><rm_type_name>{class}</rm_type_name>\
         <occurrences>{occ}</occurrences><node_id>at0000</node_id>{identities}{details}\
         <archetype_id><value>{archetype}</value></archetype_id>\
         <template_id><value>{template_id}</value></template_id>\
         {t0}{t1}{t2}{t3}{t4}{t5}</definition></template>",
        occ = interval(1, Some(1)),
        identities = multiple("identities", 1, &identity),
        details = single("details", 0, &details),
        t0 = term("at0000", &concept),
        t1 = term("at0001", "Details"),
        t2 = term("at0002", "Note"),
        t3 = term("at0003", "Identity"),
        t4 = term("at0004", "Identity details"),
        t5 = term("at0005", "Name"),
    );
    build_web_template(&opt14::from_xml(&xml).expect("parse the synthetic party OPT"))
        .expect("build the party WebTemplate")
}

/// The FLAT path of the web-template node carrying `node_id`, every repeating
/// step indexed `:0`.
fn flat_path(wt: &WebTemplate, node_id: &str) -> String {
    fn walk(node: &WebTemplateNode, node_id: &str, prefix: &str) -> Option<String> {
        let step = if node.max == 1 {
            node.id.clone()
        } else {
            format!("{}:0", node.id)
        };
        let here = if prefix.is_empty() {
            node.id.clone()
        } else {
            format!("{prefix}/{step}")
        };
        if node.node_id.as_deref() == Some(node_id) && !node.inputs.is_empty() {
            return Some(here);
        }
        node.children
            .iter()
            .find_map(|child| walk(child, node_id, &here))
    }
    walk(&wt.tree, node_id, "")
        .unwrap_or_else(|| panic!("no input node {node_id} in {}", wt.template_id))
}

/// The FLAT document naming the identity's name and the details note.
fn party_document(wt: &WebTemplate) -> Map<String, Value> {
    let mut doc = Map::new();
    doc.insert(flat_path(wt, "at0005"), json!("Synthetic Name"));
    doc.insert(flat_path(wt, "at0002"), json!("Synthetic note"));
    doc
}

/// Builds `doc` against `wt` with the default options.
fn build(doc: &Map<String, Value>, wt: &WebTemplate) -> Result<Value, FlatError> {
    party_from_flat(doc, wt, &BuildOptions::default())
}

/// What every built party must satisfy: `identities` an array of one identity
/// whose compacted details tree carries its term definition as its name, the
/// template's identity on the root, and no composition attribute.
#[expect(
    clippy::indexing_slicing,
    clippy::missing_assert_message,
    reason = "a test assertion helper outside #[test] fns: a missing member panics with the \
              assertion's own diff, which is the failure report"
)]
fn assert_party_shape(party: &Value, class: &str) {
    assert_eq!(party["_type"], class);
    let identities = party["identities"]
        .as_array()
        .unwrap_or_else(|| panic!("identities must be an array: {party}"));
    assert_eq!(identities.len(), 1, "{party}");
    let identity = identities.first().expect("one identity");
    assert_eq!(identity["_type"], "PARTY_IDENTITY");
    assert_eq!(identity["details"]["_type"], "ITEM_TREE");
    assert_eq!(identity["details"]["archetype_node_id"], "at0004");
    assert_eq!(identity["details"]["name"]["value"], "Identity details");
    assert_eq!(
        identity["details"]["items"][0]["value"]["value"],
        "Synthetic Name"
    );
    assert_eq!(party["details"]["name"]["value"], "Details");
    for attr in ["category", "language", "territory", "composer", "context"] {
        assert!(party.get(attr).is_none(), "{attr} on a party: {party}");
    }
    assert_eq!(party["archetype_details"]["rm_version"], RM_VERSION);
    assert_eq!(
        party["archetype_details"]["template_id"]["value"],
        format!("synthetic.{}.v1", class.to_ascii_lowercase())
    );
}

#[test]
fn a_person_template_builds_a_person() {
    let wt = party_template("PERSON");
    let party = build(&party_document(&wt), &wt).expect("the PERSON builds");
    from_canonical_value::<Person>(&party).expect("decodes as PERSON");
    assert_party_shape(&party, "PERSON");
}

#[test]
fn an_organisation_template_builds_an_organisation() {
    let wt = party_template("ORGANISATION");
    let party = build(&party_document(&wt), &wt).expect("the ORGANISATION builds");
    from_canonical_value::<Organisation>(&party).expect("decodes as ORGANISATION");
    assert_party_shape(&party, "ORGANISATION");
}

#[test]
fn a_role_template_builds_a_role_with_its_performer() {
    let wt = party_template("ROLE");
    let mut doc = party_document(&wt);
    let root = &wt.tree.id;
    doc.insert(
        format!("{root}/performer|id"),
        json!("4c1b2f8e-1d0a-4a52-9e0b-2f3c5d6e7f80"),
    );
    doc.insert(format!("{root}/performer|namespace"), json!("demographic"));
    doc.insert(format!("{root}/performer|type"), json!("PERSON"));
    let party = build(&doc, &wt).expect("the ROLE builds");
    from_canonical_value::<Role>(&party).expect("decodes as ROLE");
    assert_party_shape(&party, "ROLE");
    assert_eq!(
        party["performer"],
        json!({
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": "PERSON",
            "id": {"_type": "HIER_OBJECT_ID", "value": "4c1b2f8e-1d0a-4a52-9e0b-2f3c5d6e7f80"},
        })
    );
}

/// RM demographic §ROLE makes `performer` mandatory, so a ROLE without one is
/// refused by the decode check instead of being handed on.
#[test]
fn a_role_without_a_performer_is_refused() {
    let wt = party_template("ROLE");
    match build(&party_document(&wt), &wt) {
        Err(FlatError::PartyDecode { class, .. }) => assert_eq!(class, "ROLE"),
        other => panic!("expected PartyDecode, got {other:?}"),
    }
}

#[test]
fn a_performer_missing_a_part_is_refused_by_name() {
    let wt = party_template("ROLE");
    let mut doc = party_document(&wt);
    let root = wt.tree.id.clone();
    doc.insert(format!("{root}/performer|id"), json!("some-id"));
    doc.insert(format!("{root}/performer|type"), json!("PERSON"));
    match build(&doc, &wt) {
        Err(FlatError::MissingRequiredSuffix { key }) => {
            assert_eq!(key, format!("{root}/performer|namespace"));
        }
        other => panic!("expected MissingRequiredSuffix, got {other:?}"),
    }
}

#[test]
fn a_ctx_key_on_a_party_is_refused() {
    let wt = party_template("PERSON");
    let mut doc = party_document(&wt);
    doc.insert("ctx/language".to_owned(), json!("en"));
    assert!(
        matches!(build(&doc, &wt), Err(FlatError::ContextOnParty)),
        "a ctx/ key must be refused"
    );
}

#[test]
fn a_composition_template_is_not_a_party_template() {
    let wt = minimal_composition_template();
    match build(&Map::new(), &wt) {
        Err(FlatError::NotAPartyTemplate { root }) => assert_eq!(root, "COMPOSITION"),
        other => panic!("expected NotAPartyTemplate, got {other:?}"),
    }
}

// ── the caller-chosen rm_version (#3809) ─────────────────────────────────────

fn minimal_composition_template() -> WebTemplate {
    let path = format!("{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt");
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    build_web_template(&opt14::from_xml(&xml).expect("parse OPT")).expect("build WebTemplate")
}

/// The template's FLAT instance, flattened from the CNF canonical composition.
fn minimal_flat(wt: &WebTemplate) -> Map<String, Value> {
    let path = format!("{CORPUS}/compositions/CANONICAL_JSON/persistent_minimal.en.v1__full.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let composition: Value = serde_json::from_str(&text).expect("canonical JSON");
    composition_to_flat(&composition, wt).expect("FLAT from the canonical composition")
}

/// Every `archetype_details.rm_version` in `value`.
fn rm_versions(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![value];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(obj) => {
                if obj.get("_type").and_then(Value::as_str) == Some("ARCHETYPED") {
                    out.push(
                        obj.get("rm_version")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
                stack.extend(obj.values());
            }
            Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
    out
}

#[test]
fn the_flat_builder_stamps_the_chosen_rm_version_on_every_archetype_root() {
    let wt = minimal_composition_template();
    let doc = minimal_flat(&wt);
    let tree = parse_flat(&doc).expect("parse FLAT");
    let options = BuildOptions::default().with_rm_version("1.1.0");
    let built = build_composition_with(&tree, &wt, NOW, &options).expect("FLAT build");
    let versions = rm_versions(&built);
    assert!(!versions.is_empty(), "the build stamps archetype roots");
    assert!(versions.iter().all(|v| v == "1.1.0"), "{versions:?}");
}

#[test]
fn without_the_option_the_builders_stamp_the_pinned_release() {
    let wt = minimal_composition_template();
    let built = composition_from_flat(&minimal_flat(&wt), &wt, NOW).expect("FLAT build");
    let versions = rm_versions(&built);
    assert!(!versions.is_empty());
    assert!(versions.iter().all(|v| v == RM_VERSION), "{versions:?}");

    let tdd = std::fs::read_to_string(format!(
        "{CORPUS}/compositions/TDD/persistent_minimal.en.v1__full.xml"
    ))
    .expect("read the CNF TDD");
    let versions = rm_versions(&from_tdd(&tdd, &wt).expect("TDD build"));
    assert!(!versions.is_empty());
    assert!(versions.iter().all(|v| v == RM_VERSION), "{versions:?}");
}

#[test]
fn the_tdd_builder_stamps_the_chosen_rm_version() {
    let wt = minimal_composition_template();
    let tdd = std::fs::read_to_string(format!(
        "{CORPUS}/compositions/TDD/persistent_minimal.en.v1__full.xml"
    ))
    .expect("read the CNF TDD");
    let options = BuildOptions::default().with_rm_version("1.1.0");
    let built = from_tdd_with(&tdd, &wt, &options).expect("TDD build");
    let versions = rm_versions(&built);
    assert!(!versions.is_empty());
    assert!(versions.iter().all(|v| v == "1.1.0"), "{versions:?}");
}

#[test]
fn a_party_build_stamps_the_chosen_rm_version() {
    let wt = party_template("PERSON");
    let options = BuildOptions::default().with_rm_version("1.1.0");
    let party = party_from_flat(&party_document(&wt), &wt, &options).expect("PERSON build");
    assert_eq!(party["archetype_details"]["rm_version"], "1.1.0");
}
