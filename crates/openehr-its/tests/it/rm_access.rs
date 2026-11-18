// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The reflective RM accessor (`openehr_rm::v1_2::access`) rebuilds every
//! composition of the corpus attribute by attribute, and `finish` gives back
//! the value it read, byte for byte in canonical JSON (#6002).

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "fixture plumbing outside #[test] fns: a refused rebuild is a test failure"
)]

use openehr_rm::prelude::Composition;
use openehr_rm::v1_2::access::{
    AccessError, RmObject, RmRef, RmValue, as_rm_object, construct, finish, schema,
};

/// Copy `node` through the reflective API only: construct its class, then
/// write every attribute the source reports.
fn rebuild(node: &dyn RmObject) -> Box<dyn RmObject> {
    let rm_type = node.rm_type();
    let Some(names) = schema::attribute_names(rm_type) else {
        // A class built through its constructor travels whole.
        return node.clone_box();
    };
    let mut out = construct(rm_type).expect("a concrete class constructs");
    for &name in names {
        let info = schema::attribute(rm_type, name);
        if info.is_some_and(|i| i.multiple) {
            if let Some(items) = node.items(name) {
                for item in items {
                    out.push(name, rebuild(item)).expect("a member pushes");
                }
            }
            continue;
        }
        let value = match node.attribute(name) {
            None => continue,
            Some(RmRef::Object(o)) => RmValue::Object(rebuild(o)),
            Some(RmRef::String(s)) => RmValue::String(s.to_owned()),
            Some(RmRef::Integer(i)) => RmValue::Integer(i),
            Some(RmRef::Real(r)) => RmValue::Real(r),
            Some(RmRef::Boolean(b)) => RmValue::Boolean(b),
            Some(RmRef::Character(c)) => RmValue::Character(c),
        };
        match out.set(name, value) {
            Ok(()) | Err(AccessError::Unsupported { .. }) => {}
            Err(e) => panic!("{rm_type}.{name}: {e}"),
        }
    }
    out
}

fn corpus() -> Vec<(String, Composition)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = [
        "corpus",
        "crates/openehr-its/tests",
        "docs/specs/openehr/CNF",
    ]
    .iter()
    .map(|d| root.join(d))
    .collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("corpus dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "json")
                && let Ok(text) = std::fs::read_to_string(&path)
                && text.contains("\"COMPOSITION\"")
                && let Ok(c) = openehr_its::json::from_canonical_json::<Composition>(&text)
            {
                out.push((path.display().to_string(), c));
            }
        }
    }
    out
}

#[test]
fn every_corpus_composition_rebuilds_by_attribute_name() {
    let docs = corpus();
    assert!(docs.len() > 150, "found {} compositions", docs.len());
    for (path, comp) in &docs {
        let rebuilt: Composition = finish(rebuild(as_rm_object(comp))).expect("finishes");
        // Compared in canonical JSON: a present-but-empty optional list reads
        // as unset, and both serialize to nothing.
        let a = openehr_its::json::to_canonical_json(comp);
        let b = openehr_its::json::to_canonical_json(&rebuilt);
        if a != b {
            let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            let from = at.saturating_sub(120);
            panic!(
                "{path}: differs at byte {at}\n  read:    {}\n  rebuilt: {}",
                a.get(from..(at + 80).min(a.len())).unwrap_or(""),
                b.get(from..(at + 80).min(b.len())).unwrap_or("")
            );
        }
    }
}

#[test]
fn finish_names_the_first_unset_mandatory_attribute() {
    let mut comp = construct("COMPOSITION").expect("constructs");
    assert!(!comp.is_set("name"));
    comp.set(
        "archetype_node_id",
        RmValue::String("openEHR-EHR-COMPOSITION.x.v1".into()),
    )
    .expect("sets");
    let err = finish::<Composition>(comp).expect_err("name is unset");
    assert_eq!(
        err,
        AccessError::Unset {
            rm_type: "COMPOSITION",
            attribute: "name"
        }
    );
}

#[test]
fn set_refuses_a_value_of_another_type() {
    let mut comp = construct("COMPOSITION").expect("constructs");
    let text = construct("DV_TEXT").expect("constructs");
    assert!(matches!(
        comp.set("language", RmValue::Object(text)),
        Err(AccessError::NotConforming { .. })
    ));
    let coded = construct("DV_CODED_TEXT").expect("constructs");
    assert!(comp.set("name", RmValue::Object(coded)).is_ok());
    assert!(matches!(
        construct("CONTENT_ITEM"),
        Err(AccessError::Abstract(_))
    ));
}

#[test]
fn children_come_in_declaration_order() {
    let docs = corpus();
    let (_, comp) = docs.first().expect("a composition");
    let names: Vec<&str> = as_rm_object(comp).children().map(|(n, _, _)| n).collect();
    let declared = schema::attribute_names("COMPOSITION").expect("concrete");
    let mut last = 0;
    for n in names {
        let at = declared.iter().position(|d| *d == n).expect("declared");
        assert!(at >= last, "{n} out of order");
        last = at;
    }
}
