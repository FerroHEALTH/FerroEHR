// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Model-derived instances drive the validation walk over every concrete RM
//! class ([`openehr_rm::v1_2::model`]).
//!
//! A minimal and a rich instance of each class raise no structural violation,
//! and dropping any mandatory attribute raises one, apart from the register of
//! deliberate codec tolerances.

use std::collections::BTreeSet;

use openehr_its::wire_validate::validate_rm_value;
use serde_json::{Map, Value, json};

/// Deterministic scalar for a primitive/foundation type name the static model
/// does not carry as a class. `None` for an unknown name (recorded by the
/// caller — never silently defaulted).
fn primitive(name: &str) -> Option<Value> {
    Some(match name {
        "String" | "Character" | "Terminology_code" | "Any" => json!("x"),
        "Boolean" => json!(true),
        "Integer" | "Integer64" | "Ordered" | "Ordered_numeric" | "Numeric" => json!(1),
        "Real" | "Double" => json!(1.5),
        "Octet" => json!(0),
        "Uri" | "URI" => json!("local://x"),
        // BASE 1.3.0 `Hrid` (Foundation Types): a String obeying the HRID grammar.
        "Hrid" => json!("openEHR-EHR-OBSERVATION.x.v1"),
        // ISO 8601 value strings: several RM classes type their `value`
        // attribute directly as the ISO string form.
        "Iso8601_date" => json!("2020-01-01"),
        "Iso8601_time" => json!("10:00:00"),
        "Iso8601_date_time" => json!("2020-01-01T10:00:00"),
        "Iso8601_duration" => json!("PT1H"),
        "Hash" => Value::Object(Map::new()),
        // The one spec-declared OPEN extension point: any tagged object is a
        // legal scheme instance (`ehr_access.adoc` §settings; #1935).
        "ACCESS_CONTROL_SETTINGS" => json!({ "_type": "ACCESS_CONTROL_SETTINGS" }),
        _ => return None,
    })
}

/// A deterministic value for an attribute whose class constrains it more than
/// its BMM primitive type: each identifier class's EBNF (BASE
/// `master05-identification_package.adoc` §Syntaxes).
fn constrained_attribute(class: &str, attr: &str) -> Option<Value> {
    // A UUID in the canonical 8-4-4-4-12 hyphenated form the `uuid` production
    // names, reused wherever a `uid` is required.
    const UID: &str = "0191a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    match (class, attr) {
        // `HIER_OBJECT_ID` without an extension is a bare `uid`.
        ("UUID" | "HIER_OBJECT_ID", "value") => Some(json!(UID)),
        // `iso_oid = number, { '.', number }`.
        ("ISO_OID", "value") => Some(json!("1.2.840.113554")),
        // `internet_id = subdomain`, `subdomain = label | subdomain, '.', label`.
        ("INTERNET_ID", "value") => Some(json!("openehr.org")),
        // `object_version_id = object_id, '::', creating_system_id, '::',
        // version_tree_id`, all three parts required.
        ("OBJECT_VERSION_ID", "value") => Some(json!(format!("{UID}::openehr.org::1"))),
        // `version_tree_id = trunk_version, [ '.', branch_number, '.',
        // branch_version ]`, every part starting at 1.
        ("VERSION_TREE_ID", "value") => Some(json!("1")),
        _ => None,
    }
}

/// A generated instance of concrete class `class`: every mandatory attribute,
/// optionals when `rich`, the first concrete descendant for an abstract type,
/// and type arguments substituted. `unknown` collects unmodelled type names.
fn generate(
    class: &str,
    args: &[&openehr_rm::v1_2::model::RmTypeRef],
    rich: bool,
    depth: usize,
    unknown: &mut BTreeSet<String>,
) -> Value {
    let mut obj = Map::new();
    obj.insert("_type".into(), json!(class));
    if depth > 12 {
        return Value::Object(obj);
    }
    for attr in openehr_rm::v1_2::model::attributes(class) {
        if let Some(v) = attribute_value(class, attr, args, rich, depth, unknown) {
            obj.insert(attr.name.to_owned(), v);
        }
    }
    Value::Object(obj)
}

/// The generated value for one attribute, or `None` when skipped: an optional
/// outside rich mode, or optional recursion below depth 2.
fn attribute_value(
    class: &str,
    attr: &'static openehr_rm::v1_2::model::RmAttribute,
    args: &[&openehr_rm::v1_2::model::RmTypeRef],
    rich: bool,
    depth: usize,
    unknown: &mut BTreeSet<String>,
) -> Option<Value> {
    if !attr.is_mandatory && (!rich || depth > 2) {
        return None;
    }
    // Canonical JSON carries `List<Octet>` (DV_MULTIMEDIA.data etc.) as an
    // inline base64 STRING, not a JSON array (ITS-JSON) — the one container
    // shape the codec re-forms.
    if attr.declared_type == "Octet"
        && matches!(attr.container, openehr_rm::v1_2::model::Container::List)
    {
        return Some(json!("AA=="));
    }
    // An attribute the class constrains beyond its declared primitive type.
    if let Some(v) = constrained_attribute(class, attr.name) {
        return Some(v);
    }
    let (ty, ty_args) = attribute_type(class, attr, args);
    let element = value_for(ty, &ty_args, rich, depth, unknown)?;
    Some(match attr.container {
        openehr_rm::v1_2::model::Container::None => element,
        openehr_rm::v1_2::model::Container::List | openehr_rm::v1_2::model::Container::Set => {
            json!([element])
        }
        openehr_rm::v1_2::model::Container::Hash => json!({ "x": element }),
    })
}

/// The effective type and type arguments of one attribute in `class`'s scope,
/// formal parameters replaced by the caller's arguments.
fn attribute_type<'a>(
    class: &str,
    attr: &'static openehr_rm::v1_2::model::RmAttribute,
    args: &[&'a openehr_rm::v1_2::model::RmTypeRef],
) -> (&'a str, Vec<&'a openehr_rm::v1_2::model::RmTypeRef>) {
    let argument = |formal: Option<&str>| {
        let params = openehr_rm::v1_2::model::class(class)?.generic_params;
        let index = params.iter().position(|p| Some(p.name) == formal)?;
        args.get(index).copied()
    };
    if let Some(arg) = argument(attr.formal) {
        return (arg.name, arg.params.iter().collect());
    }
    let ty_args = attr
        .type_params
        .iter()
        .map(|p| argument(p.formal).unwrap_or(p))
        .collect();
    (attr.declared_type, ty_args)
}

/// A value of declared type `ty` (with its generic arguments): a class with a
/// generated type recurses (abstract → first concrete descendant), a
/// primitive comes from the table, anything else is recorded as unknown.
fn value_for(
    ty: &str,
    ty_args: &[&openehr_rm::v1_2::model::RmTypeRef],
    rich: bool,
    depth: usize,
    unknown: &mut BTreeSet<String>,
) -> Option<Value> {
    if let Some(class) = openehr_rm::v1_2::model::class(ty).filter(|c| c.generated_type) {
        let concrete = if class.is_abstract {
            cheapest_descendant(class.descendants)?
        } else {
            class.name
        };
        return Some(generate(concrete, ty_args, rich, depth + 1, unknown));
    }
    if let Some(e) = openehr_rm::v1_2::model::enumeration(ty) {
        return e.literals.first().map(|l| match l.value {
            openehr_rm::v1_2::model::EnumValue::Int(i) => json!(i),
            openehr_rm::v1_2::model::EnumValue::Str(s) => json!(s),
        });
    }
    let p = primitive(ty);
    if p.is_none() {
        unknown.insert(ty.to_owned());
    }
    p
}

/// The concrete descendant for an abstract slot: the one with the fewest
/// mandatory class-typed attributes, so recursion terminates.
fn cheapest_descendant(descendants: &[&'static str]) -> Option<&'static str> {
    descendants
        .iter()
        .map(|d| {
            let cost = openehr_rm::v1_2::model::attributes(d)
                .filter(|a| {
                    a.is_mandatory
                        && openehr_rm::v1_2::model::class(a.declared_type)
                            .is_some_and(|c| c.generated_type)
                })
                .count();
            (*d, cost)
        })
        .min_by_key(|(_, cost)| *cost)
        .map(|(d, _)| d)
}

/// Every concrete class of the static model that has a generated type, in
/// declaration order.
fn concrete_classes() -> Vec<&'static str> {
    openehr_rm::v1_2::model::classes()
        .filter(|c| !c.is_abstract && c.generated_type)
        .map(|c| c.name)
        .collect()
}

fn structural_violations(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    validate_rm_value(v, &mut out);
    out.into_iter()
        .map(|iv| format!("{}: {}", iv.path, iv.message))
        .filter(|m| m.contains("does not conform to RM type"))
        .collect()
}

/// Property 1: what the model declares structurally valid, the walk decodes.
#[test]
fn generated_instances_are_structurally_accepted() {
    let mut unknown = BTreeSet::new();
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for class in concrete_classes() {
        for rich in [false, true] {
            let inst = generate(class, &[], rich, 0, &mut unknown);
            let bad = structural_violations(&inst);
            if !bad.is_empty() {
                failures.push(format!("{class} (rich={rich}): {bad:?}"));
            }
            checked += 1;
        }
    }
    assert!(
        checked > 200,
        "expected the full concrete class set, saw {checked}"
    );
    assert!(
        failures.is_empty(),
        "model-valid instances were structurally refused ({} classes):\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Type names outside both the model and the primitive table: pin the
    // honest register (growth = a model/table gap to close, never silence).
    assert_eq!(
        unknown,
        BTreeSet::new(),
        "attribute types neither the model nor the primitive table cover"
    );
}

/// Property 2: dropping any mandatory attribute is caught structurally —
/// classes where it is NOT are the pinned reach register.
#[test]
fn dropped_mandatory_attributes_are_refused() {
    let mut unknown = BTreeSet::new();
    let mut unreached: BTreeSet<String> = BTreeSet::new();
    let mut mutations = 0usize;
    for class in concrete_classes() {
        let base = generate(class, &[], false, 0, &mut unknown);
        for attr in openehr_rm::v1_2::model::attributes(class) {
            if !attr.is_mandatory {
                continue;
            }
            let Some(obj) = base.as_object() else {
                continue;
            };
            if !obj.contains_key(attr.name) {
                continue;
            }
            let mut mutated = base.clone();
            mutated
                .as_object_mut()
                .expect("generated instances are objects")
                .remove(attr.name);
            mutations += 1;
            let caught = structural_violations(&mutated)
                .iter()
                .any(|m| m.contains(attr.name));
            if !caught {
                unreached.insert(format!("{class}.{}", attr.name));
            }
        }
    }
    assert!(
        mutations > 300,
        "expected the full mandatory set, saw {mutations}"
    );
    // The register of mandatory attributes whose omission the codec accepts, each
    // with its reason; a newcomer fails by name.
    let expected: BTreeSet<String> = include_str!("model_walkgen_register.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        unreached, expected,
        "mandatory-drop reach register drifted (the register file states each entry's adjudicated reason)"
    );
}

/// A CLUSTER whose mandatory `items` is absent or empty is refused (RM
/// `org.openehr.rm.data_structures.cluster.adoc` §Attributes).
#[test]
fn mandatory_container_lower_bound_is_enforced() {
    use openehr_its::wire_validate::validate_rm_value;
    let mk = |items: Option<Value>| {
        let mut c = json!({
            "_type": "CLUSTER",
            "name": {"_type": "DV_TEXT", "value": "specimen"},
            "archetype_node_id": "at0001",
        });
        if let Some(i) = items {
            c["items"] = i;
        }
        c
    };
    let judge = |v: &Value| {
        let mut out = Vec::new();
        validate_rm_value(v, &mut out);
        out.iter()
            .any(|iv| iv.message.contains("mandatory container `items`"))
    };
    assert!(judge(&mk(None)), "absent items must be refused");
    assert!(judge(&mk(Some(json!([])))), "empty items must be refused");
    let element = json!([{
        "_type": "ELEMENT",
        "name": {"_type": "DV_TEXT", "value": "x"},
        "archetype_node_id": "at0002",
    }]);
    assert!(
        !judge(&mk(Some(element))),
        "populated items must not raise the container violation"
    );
}

/// The CKM lab-result example with the specimen CLUSTER's items deleted is
/// refused by the full walk.
#[test]
fn cluster_no_items_fixture_is_refused() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/fixtures/composition/lab_result.cluster_no_items.json"),
    )
    .expect("fixture exists");
    let doc: Value = serde_json::from_str(&text).expect("fixture parses");
    let violations = openehr_sdt::rm_instance::validate_rm_and_terminology(&doc, "COMPOSITION");
    assert!(
        violations
            .iter()
            .any(|m| m.message.contains("mandatory container `items`")),
        "the cluster-without-items fixture must be refused, got: {violations:?}"
    );
}
