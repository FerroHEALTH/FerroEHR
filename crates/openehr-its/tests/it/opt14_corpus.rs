// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! OPT 1.4 corpus gate: every vendored `.opt` operational template
//! must parse into the generated `opt14::types::OperationalTemplate` model without
//! error. The corpus is the shared one at `corpus/fixtures/service`; this crate
//! reads it by a workspace-relative path.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::common;

/// The OPT corpus dir (`corpus/fixtures/service`).
fn corpus_dir() -> PathBuf {
    common::service_corpus_dir()
}

/// Recursively collect every `*.opt` file under `dir`.
fn opt_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(entries) => entries,
            Err(e) => panic!("read corpus dir {}: {e}", d.display()),
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "opt") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_opt_template_parses() {
    let files = opt_files(&corpus_dir());
    assert!(
        files.len() >= 90,
        "expected the full OPT corpus (~91 files), found {}",
        files.len()
    );

    let mut failures = Vec::new();
    let mut parsed = 0usize;
    for path in &files {
        let xml = std::fs::read_to_string(path).expect("read opt file");
        match openehr_its::opt14::from_xml(&xml) {
            Ok(_) => parsed += 1,
            Err(e) => failures.push((path.clone(), e.to_string())),
        }
    }

    if !failures.is_empty() {
        let mut msg = format!(
            "{}/{} OPT files parsed; {} failed:\n",
            parsed,
            files.len(),
            failures.len()
        );
        for (p, e) in &failures {
            let _ = writeln!(msg, "  - {}: {}", p.display(), e);
        }
        panic!("{msg}");
    }
    assert_eq!(parsed, files.len());
}

/// The official CNF Robot template corpus, VALID half
/// (`…/test_data_sets/valid_templates`): every `.opt` in it must parse —
/// the vendored corpus is fully exercised, and a template the official
/// suite uploads must never be rejected by the XML front end. (The
/// `invalid_templates` siblings are negative fixtures whose rejection is
/// the app's job — structural ones legitimately fail right here.)
#[test]
fn every_official_cnf_robot_template_parses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/valid_templates",
    );
    let files = opt_files(&dir);
    assert!(
        files.len() >= 30,
        "expected the full valid robot template corpus (~32 files), found {}",
        files.len()
    );
    // Adjudicated corpus defects: fixtures in valid_templates that are
    // XSD-INVALID first-hand, so their rejection is the parser being right
    // (the robot suite is reference material, never an oracle). Each entry
    // cites the violated schema requirement; anything else failing is a
    // genuine parser defect and fails the gate.
    let adjudicated_invalid = [
        (
            // Omits the OPERATIONAL_TEMPLATE's mandatory <language>
            // (its-xml-1.0.2-nsv1 Template.xsd: <xs:element name="language"
            // type="CODE_PHRASE"/> — no minOccurs="0").
            "minimal_action_removed_language.opt",
            "Template.xsd OPERATIONAL_TEMPLATE.language is mandatory",
        ),
        (
            // Its T_COMPLEX_OBJECT default_value DV_PROPORTION carries only
            // numerator/denominator (its-xml-1.0.2-nsv1 BaseTypes.xsd:
            // <xs:element name="type" type="PROPORTION_KIND"/> — mandatory).
            "ehrn_vital_signs.v2.opt",
            "BaseTypes.xsd DV_PROPORTION.type is mandatory",
        ),
    ];
    let mut failures = Vec::new();
    let mut adjudicated_seen = 0usize;
    for path in &files {
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        let xml = std::fs::read_to_string(path).expect("read opt file");
        let outcome = openehr_its::opt14::from_xml(&xml);
        if let Some((_, citation)) = adjudicated_invalid.iter().find(|(f, _)| *f == name) {
            assert!(
                outcome.is_err(),
                "{name} is adjudicated XSD-invalid ({citation}) but now parses — \
                 re-adjudicate or drop the entry"
            );
            adjudicated_seen += 1;
            continue;
        }
        if let Err(e) = outcome {
            failures.push((path.clone(), e.to_string()));
        }
    }
    assert_eq!(
        adjudicated_seen,
        adjudicated_invalid.len(),
        "an adjudicated fixture went missing from the corpus"
    );
    if !failures.is_empty() {
        let mut msg = format!("{} robot corpus OPT(s) failed to parse:\n", failures.len());
        for (p, e) in &failures {
            let _ = writeln!(msg, "  - {}: {}", p.display(), e);
        }
        panic!("{msg}");
    }
}

/// Model-level losslessness gate: parse → `ToXml` → re-parse must be
/// structurally stable over the whole corpus. This is what actually exercises
/// the generated `ToXml` impls (storage/GET serve verbatim XML, so nothing else
/// does) and asserts the typed model loses nothing it captured on the way back
/// out — `xsi:type` dispatch, interval defaults, dictionary groups, and the
/// `T_CONSTRAINT` overlay all round-trip.
#[test]
fn every_opt_template_round_trips() {
    let files = opt_files(&corpus_dir());
    assert!(files.len() >= 90, "expected the full OPT corpus");

    let mut failures = Vec::new();
    for path in &files {
        let xml = std::fs::read_to_string(path).expect("read opt file");
        let opt = match openehr_its::opt14::from_xml(&xml) {
            Ok(o) => o,
            Err(e) => {
                failures.push((path.clone(), format!("first parse: {e}")));
                continue;
            }
        };
        let re_xml = match openehr_its::opt14::to_xml(&opt) {
            Ok(x) => x,
            Err(e) => {
                failures.push((path.clone(), format!("to_xml: {e}")));
                continue;
            }
        };
        match openehr_its::opt14::from_xml(&re_xml) {
            Ok(re_opt) => {
                if re_opt != opt {
                    failures.push((path.clone(), "re-parse != first parse".to_string()));
                }
            }
            Err(e) => failures.push((path.clone(), format!("re-parse: {e}"))),
        }
    }

    if !failures.is_empty() {
        let mut msg = format!("{} OPT files failed the round-trip gate:\n", failures.len());
        for (p, e) in &failures {
            let _ = writeln!(msg, "  - {}: {}", p.display(), e);
        }
        panic!("{msg}");
    }
}

/// The child objects of one OPT constraint attribute.
fn opt_attribute_children(
    a: &openehr_its::opt14::types::CAttribute,
) -> &[openehr_its::opt14::types::CObject] {
    match a {
        openehr_its::opt14::types::CAttribute::CSingleAttribute(s) => &s.children,
        openehr_its::opt14::types::CAttribute::CMultipleAttribute(m) => &m.children,
    }
}

/// Collects every inline `C_ARCHETYPE_ROOT` term definition in the tree.
fn collect_opt_terms<'a>(
    obj: &'a openehr_its::opt14::types::CObject,
    out: &mut Vec<&'a openehr_its::opt14::types::ArchetypeTerm>,
) {
    use openehr_its::opt14::types::{CAttribute, CObject};
    let attrs: &[CAttribute] = match obj {
        CObject::CArchetypeRoot(r) => {
            out.extend(&r.term_definitions);
            &r.attributes
        }
        CObject::CComplexObject(c) => &c.attributes,
        CObject::TComplexObject(t) => &t.attributes,
        _ => return,
    };
    for a in attrs {
        for c in opt_attribute_children(a) {
            collect_opt_terms(c, out);
        }
    }
}

/// Every term whose items are in the document order `text` then `description`,
/// paired with that key order.
fn text_first_orders(
    opt: &openehr_its::opt14::types::OperationalTemplate,
) -> Vec<(String, Vec<String>)> {
    let mut terms: Vec<&openehr_its::opt14::types::ArchetypeTerm> =
        opt.definition.term_definitions.iter().collect();
    for a in &opt.definition.attributes {
        for c in opt_attribute_children(a) {
            collect_opt_terms(c, &mut terms);
        }
    }
    terms
        .iter()
        .filter(|t| t.items.keys().collect::<Vec<_>>() == ["text", "description"])
        .map(|t| (t.code.clone(), t.items.keys().cloned().collect()))
        .collect()
}

/// `StringDictionaryItem` groups are XSD ordered sequences; the model must
/// preserve document order (`IndexMap`). `IndexMap`'s `PartialEq` is
/// order-insensitive, so the round-trip gate above cannot see reordering —
/// this asserts key *order* survives serialize → re-parse explicitly, on a
/// fixture whose document order is **non-alphabetical** (`text` before
/// `description`), which an alphabetically-sorted container would invert.
#[test]
fn string_dictionary_order_preserved() {
    let dir = corpus_dir();
    let path = dir.join("knowledge/operational_templates/Generic Laboratory Test Report.v0.opt");
    let xml = std::fs::read_to_string(&path).expect("read opt");
    let opt = openehr_its::opt14::from_xml(&xml).expect("parse");

    // The fixture carries terms whose document order is `text` then
    // `description`; a sorted container would yield none of these.
    let orders = text_first_orders(&opt);
    assert!(
        !orders.is_empty(),
        "expected at least one term in document order (text, description) — \
         a sorted map container would have inverted them all"
    );

    let re_xml = openehr_its::opt14::to_xml(&opt).expect("to_xml");
    let re_opt = openehr_its::opt14::from_xml(&re_xml).expect("re-parse");
    assert_eq!(
        text_first_orders(&re_opt),
        orders,
        "dictionary key order changed across the round-trip"
    );
}

/// `T_CONSTRAINT` (`<constraints>`) carries node `default_value` overlays
///; assert they are parsed into the typed model, not skipped, for a
/// corpus file known to carry one.
#[test]
fn t_constraint_default_values_parsed() {
    let dir = corpus_dir();
    let path = dir.join("knowledge/IDCR - Adverse Reaction List.v1.opt");
    let xml = std::fs::read_to_string(&path).expect("read opt");
    let opt = openehr_its::opt14::from_xml(&xml).expect("parse");

    let tc = opt
        .constraints
        .as_ref()
        .expect("template has a <constraints> (T_CONSTRAINT) block");
    assert!(!tc.attributes.is_empty(), "T_CONSTRAINT.attributes empty");
    let attr = &tc.attributes[0];
    assert_eq!(attr.rm_attribute_name, "value");
    assert!(
        attr.differential_path
            .starts_with("[openEHR-EHR-COMPOSITION.adverse_reaction_list.v1]"),
        "differential_path not parsed: {:?}",
        attr.differential_path
    );
    let child = attr.children.first().expect("T_ATTRIBUTE.children");
    assert_eq!(child.rm_type_name, "DV_TEXT");
    let dv = child.default_value.as_ref().expect("default_value parsed");
    // The overlay's DV_TEXT default must carry its value through
    // (`DV_TEXT` is a polymorphic slot: DataValue::DvText(DvText::DvText(data))).
    let openehr_rm::prelude::DataValue::DvText(openehr_rm::prelude::DvText::DvText(text)) = dv
    else {
        panic!("default_value should be a plain DV_TEXT, got {dv:?}");
    };
    assert_eq!(text.value, "No history of allergies / adverse reactions");
}

/// Spot-check that key envelope fields are actually populated (not merely that
/// parsing returns `Ok`), on a representative subset of the corpus.
#[test]
fn key_fields_populated() {
    let dir = corpus_dir();
    for rel in [
        "knowledge/IDCR Allergies List.v0.opt",
        "knowledge/non_unique_aql_paths.opt", // the ns2:-prefixed export
    ] {
        let path = dir.join(rel);
        let xml = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {rel}"));
        let opt = openehr_its::opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse {rel}: {e}"));
        assert!(
            !opt.template_id.value.is_empty(),
            "{rel}: template_id.value empty"
        );
        assert!(!opt.concept.is_empty(), "{rel}: concept empty");
        assert_eq!(
            opt.definition.rm_type_name, "COMPOSITION",
            "{rel}: definition.rm_type_name"
        );
        assert!(
            !opt.definition.attributes.is_empty(),
            "{rel}: definition has no attributes"
        );
    }
}

/// #1648 — the OPT 1.4 template upload is a real ingestion path for a
/// client-supplied `REVISION_HISTORY` (Template.xsd's optional
/// `revision_history` on `OPERATIONAL_TEMPLATE`), and
/// `REVISION_HISTORY_ITEM.Audit_valid` (`not audits.is_empty` — RM
/// `org.openehr.rm.common.revision_history_item.adoc` §Invariants) is
/// enforced there BY CONSTRUCTION: the generated XML reader builds `audits`
/// through `NonEmptyVec::new`, so an item with no audit refuses at parse.
/// Both twins pinned.
#[test]
fn opt_revision_history_item_without_audits_is_refused_at_parse() {
    let base = std::fs::read_to_string(corpus_dir().join("knowledge/opt/minimal_observation.opt"))
        .expect("read the minimal corpus OPT");
    let audit = r#"<audits><system_id>test.system</system_id><committer xsi:type="PARTY_IDENTIFIED"><name>author</name></committer><time_committed><value>2026-01-01T00:00:00Z</value></time_committed><change_type><value>creation</value><defining_code><terminology_id><value>openehr</value></terminology_id><code_string>249</code_string></defining_code></change_type></audits>"#;
    let item = |audits: &str| {
        format!(
            "<revision_history><items><version_id><value>test::sys::1</value></version_id>{audits}</items></revision_history>"
        )
    };
    // The optional element slots in after <uid> per the XSD sequence; the
    // corpus minimal OPT starts `<template xmlns…><language>`, so insert the
    // element right after the opening tag's first child boundary — the reader
    // is order-tolerant on member elements.
    let inject = |fragment: &str| {
        let (head, tail) = base
            .split_once("<language>")
            .expect("minimal OPT carries <language>");
        format!("{head}{fragment}<language>{tail}")
    };

    let valid = inject(&item(audit));
    assert!(
        openehr_its::opt14::from_xml(&valid).is_ok(),
        "the audited revision_history twin parses"
    );

    let invalid = inject(&item(""));
    let err = openehr_its::opt14::from_xml(&invalid)
        .expect_err("an item with no audits must refuse at parse (NonEmptyVec)");
    assert!(
        err.to_string().contains("audits"),
        "the refusal names the empty container: {err}"
    );
}

/// Every `ARCHETYPE_SLOT` reachable from an OPT definition tree, in document
/// order.
fn slots(
    root: &openehr_its::opt14::types::CArchetypeRoot,
) -> Vec<&openehr_its::opt14::types::ArchetypeSlot> {
    use openehr_its::opt14::types::{CAttribute, CObject};

    fn walk<'a>(objs: &'a [CObject], out: &mut Vec<&'a openehr_its::opt14::types::ArchetypeSlot>) {
        for o in objs {
            let attrs: &[CAttribute] = match o {
                CObject::ArchetypeSlot(s) => {
                    out.push(s);
                    continue;
                }
                CObject::CComplexObject(c) => &c.attributes,
                CObject::CArchetypeRoot(r) => &r.attributes,
                _ => continue,
            };
            for a in attrs {
                match a {
                    CAttribute::CMultipleAttribute(m) => walk(&m.children, out),
                    CAttribute::CSingleAttribute(s) => walk(&s.children, out),
                }
            }
        }
    }

    let mut out = Vec::new();
    for a in &root.attributes {
        match a {
            CAttribute::CMultipleAttribute(m) => walk(&m.children, &mut out),
            CAttribute::CSingleAttribute(s) => walk(&s.children, &mut out),
        }
    }
    out
}

/// The two `EXPR_LEAF`s of an `archetype_id/value matches {…}` assertion.
fn matches_operands(
    a: &openehr_its::opt14::types::Assertion,
) -> (
    &openehr_its::opt14::types::ExprLeaf,
    &openehr_its::opt14::types::ExprLeaf,
) {
    use openehr_its::opt14::types::ExprItem;

    let ExprItem::ExprBinaryOperator(op) = a.expression.as_ref() else {
        panic!("a slot assertion is a binary `matches` expression");
    };
    let (ExprItem::ExprLeaf(left), ExprItem::ExprLeaf(right)) =
        (op.left_operand.as_ref(), op.right_operand.as_ref())
    else {
        panic!("both operands of a slot `matches` are leaves");
    };
    (left, right)
}

/// `EXPR_LEAF.item` is declared `xs:anyType` (`Template.xsd` ALL/Archetype.xsd
/// §`EXPR_LEAF`) over a model that types it `Any` — "a manifest constant, an
/// attribute path …, or … a constraint, often a `C_PRIMITIVE_OBJECT`"
/// (`AM aom14 §EXPR_LEAF Class`). The codec must therefore carry the element's
/// SUBTREE, not discard it: a slot's `includes` holds the constrained
/// archetype-id regex, and every OPT 1.4 slot constraint is erased if the
/// payload is dropped.
#[test]
fn expr_leaf_any_type_items_carry_their_payload() {
    let path = corpus_dir().join("knowledge/IDCR Problem List.v1.opt");
    let xml = std::fs::read_to_string(&path).expect("read the IDCR corpus OPT");
    let opt = openehr_its::opt14::from_xml(&xml).expect("the IDCR OPT parses");

    let found = slots(&opt.definition);
    let slot = found
        .iter()
        .find(|s| s.node_id == "at0002")
        .expect("the problem/diagnosis EVALUATION slot at0002");
    let assertion = slot.includes.first().expect("at0002 has an includes");
    let (left, right) = matches_operands(assertion);

    // Left operand: the attribute path, a bare text payload under an
    // XML-Schema primitive `xsi:type` (`xsd:string` — prefix stripped).
    assert_eq!(left.item.xsi_type(), Some("string"));
    assert_eq!(left.item.text(), "archetype_id/value");

    // Right operand: the C_STRING constraint, whose `<pattern>` child is the
    // whole datum the slot constrains on.
    assert_eq!(right.item.xsi_type(), Some("C_STRING"));
    assert_eq!(
        right
            .item
            .child("pattern")
            .map(openehr_its::xml::runtime::XmlAny::text),
        Some(r"openEHR-EHR-EVALUATION\.problem_diagnosis(-[a-zA-Z0-9_]+)*\.v1".to_owned()),
    );

    // Every slot in this template constrains a real archetype id.
    for s in &found {
        for a in &s.includes {
            let (_, constraint) = matches_operands(a);
            assert!(
                constraint.item.child("pattern").is_some(),
                "slot {} lost its C_STRING pattern",
                s.node_id
            );
        }
    }

    // The payload survives serialization: re-emitted attributes, text and
    // children re-parse to the same tree.
    let re_xml = openehr_its::opt14::to_xml(&opt).expect("re-serialize the OPT");
    let re_opt = openehr_its::opt14::from_xml(&re_xml).expect("the re-serialized OPT parses");
    assert_eq!(slots(&re_opt.definition), found);
}
