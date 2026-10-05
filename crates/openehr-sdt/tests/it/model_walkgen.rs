// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Model-derived instance generation drives the validation walk over EVERY
//! concrete RM class.
//!
//! The generators are derived from the BMM-generated static RM model
//! (`openehr_rm::v1_2::model`) — never hand-written shapes — and the enumeration is
//! deterministic and EXHAUSTIVE over the finite class set (strictly more
//! systematic than random sampling for the same domain; there is nothing for
//! a fuzzer to discover that full enumeration does not visit):
//!
//! 1. **Structural acceptance** — for every concrete class, a minimal
//!    (mandatory-only) and a rich (optionals populated) generated instance
//!    must produce NO structural `does not conform to RM type` violation:
//!    what the model declares structurally valid, the codec-backed walk must
//!    decode. Semantic violations (invariants, terminology) are legitimate on
//!    synthetic values and are not the property.
//! 2. **Mutation refusal** — for every concrete class and every mandatory
//!    attribute, dropping that attribute from the minimal instance must
//!    surface a structural violation naming the missing field. The dispatcher
//!    reaches every emitted class (the generated structural dispatch), so an
//!    uncaught drop is now only ever a deliberate codec tolerance; the honest
//!    register of those pins each one with its reason, and can only shrink.

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

/// A deterministic value for an attribute whose *class* constrains the value
/// space more tightly than the BMM primitive type it is declared with.
///
/// Every openEHR **identifier** class is such a case: BASE
/// `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// §Syntaxes gives each one an EBNF lexical form over its `value` string, so an
/// arbitrary string is NOT an instance of the class. Construction runs that
/// grammar (the construction-door scheme), and the codec rightly refuses
/// anything else — the generator must therefore emit a grammar-conformant
/// example, exactly as it already does for `UUID` (whose generated field is a
/// real `uuid::Uuid`).
fn constrained_attribute(class: &str, attr: &str) -> Option<Value> {
    // A UUID in the canonical 8-4-4-4-12 hyphenated form the `uuid` production
    // names, reused wherever a `uid` is required.
    const UID: &str = "0191a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    match (class, attr) {
        // `uuid = hex-number, '-', … (five groups)`; and
        // `hier_object_id = uid_based_id`, `uid_based_id = root, [ '::',
        // extension ]`, `root = uid` — whose extension-less form is a bare
        // `uid`, so the same value serves both productions.
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

/// A generated instance of concrete class `class`, derived from the model:
/// `_type`-tagged, every mandatory attribute present; optionals included when
/// `rich`. Abstract attribute types instantiate their first concrete
/// descendant (deterministic); when the class is generic and the caller
/// supplied type arguments, an attribute declared as the bare parameter
/// (which the static model resolves to its BOUND) substitutes the argument —
/// `DV_INTERVAL<DV_QUANTITY>.lower` generates a `DV_QUANTITY`, never an
/// arbitrary `Ordered` descendant. `unknown` collects type names neither the
/// model nor the primitive table covers.
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

/// The generated value for one attribute, or `None` when it is skipped.
///
/// Skipped: an optional attribute outside rich mode, and (in rich mode too)
/// optional RECURSIVE containment below depth 2 — folders in folders, links,
/// feeder audit — which keeps instances finite and small.
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

/// The effective type (and type arguments) of one attribute in `class`'s scope.
///
/// An attribute whose declared type equals a generic parameter's bound takes the
/// caller's argument. A BARE reference to a generic class (the BMM drops the
/// argument: `IMPORTED_VERSION.item: ORIGINAL_VERSION`) is monomorphized by the
/// emitter with the enclosing scope's type argument (`item: OriginalVersion<T>`),
/// so the caller's argument is threaded the same way — the emitted type is what
/// the codec enforces, and an unthreaded element is not an instance of it.
fn attribute_type<'a>(
    class: &str,
    attr: &'static openehr_rm::v1_2::model::RmAttribute,
    args: &[&'a openehr_rm::v1_2::model::RmTypeRef],
) -> (&'static str, Vec<&'a openehr_rm::v1_2::model::RmTypeRef>) {
    let generic_params =
        openehr_rm::v1_2::model::class(class).map_or(&[][..], |c| c.generic_params);
    let substituted = generic_params
        .iter()
        .zip(args.iter())
        .find(|(p, _)| p.conforms_to.unwrap_or("Any") == attr.declared_type)
        .map(|(_, a)| (a.name, a.params.iter().collect()));
    substituted.unwrap_or_else(|| {
        let mut ty_args: Vec<&openehr_rm::v1_2::model::RmTypeRef> =
            attr.type_params.iter().collect();
        if ty_args.is_empty()
            && !args.is_empty()
            && openehr_rm::v1_2::model::class(attr.declared_type)
                .is_some_and(|c| !c.generic_params.is_empty())
        {
            ty_args = args.to_vec();
        }
        (attr.declared_type, ty_args)
    })
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

/// The concrete descendant to instantiate for an abstract slot.
///
/// Deterministic non-recursive preference: the descendant with the FEWEST
/// mandatory class-typed attributes (ELEMENT over CLUSTER for an ITEM slot), so
/// mandatory recursion terminates.
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
    // The honest register: every entry is a mandatory attribute whose omission
    // the dispatcher does not surface structurally, each with its adjudicated
    // reason in the file itself. Type-reach entries are gone — the generated
    // structural dispatch decodes every emitted class — so what remains is only
    // where the codec deliberately ACCEPTS the omission (an absent 1..*
    // container reads as an empty `Vec`; an `Interval` boundary flag reads from
    // its literal default). It may only SHRINK (a fix) or grow with an
    // adjudicated reason — never grow silently (the assert names each newcomer).
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

/// The model-driven mandatory-container lower bound (#1461): a CLUSTER whose
/// RM-mandatory 1..* `items` is absent or empty is refused; a populated one is
/// clean of that violation (RM `data_structures`
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

/// The wire-reachable representative fixture of the family: the CKM
/// lab-result example with the specimen CLUSTER's items deleted (the CNF
/// `create_composition-cluster_no_items` case's payload) is refused by the
/// full walk.
#[test]
fn cluster_no_items_fixture_is_refused() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/fixtures/composition/lab_result.cluster_no_items.json"),
    )
    .expect("fixture exists");
    let doc: Value = serde_json::from_str(&text).expect("fixture parses");
    let violations = openehr_sdt::rm_instance::validate_rm_and_terminology(&doc);
    assert!(
        violations
            .iter()
            .any(|m| m.message.contains("mandatory container `items`")),
        "the cluster-without-items fixture must be refused, got: {violations:?}"
    );
}
