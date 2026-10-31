// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `emit-rm-model`: emit a **static RM attribute/type model**
//! into `openehr-rm` (`src/model/`), generated from the same BASE + RM BMM the
//! `emit` target consumes. This is the AQL planner's oracle — attribute typing,
//! multiplicity, abstract→concrete descendant sets, and structure classification
//! — compiled once at build time, with no reflection and no hand-maintained
//! tables.
//!
//! Two generated files under `openehr-rm/src/model/`:
//! - `mod.rs` — the public API (`RmClass`, `RmAttribute`, `Container`, and the
//!   `class`/`attribute`/`computed_attribute`/`attributes`/`descendants`/`ancestors`/`is_a`/
//!   `is_primitive`/`conforms_to_ordered`/`is_structure_root` functions + a
//!   `LazyLock` name index).
//! - `data.rs` — the generated `static CLASSES: &[RmClass]` table.

use crate::analyze::Model;
use crate::load::bmm::{BmmClass, BmmEnumValue, BmmPackage, BmmPropKind, BmmSchema, BmmType};
use crate::load::bmm_function::BmmFunction;
use crate::plan::overrides::{class_binding, ref_target};
use crate::render::emit::GenFile;
use crate::render::emit_rm_model_api::emit_mod;
use std::collections::{BTreeMap, BTreeSet};

/// Classes the node codec splits into their own `node` row — mirrored **verbatim**
/// from `ferroehr::storage::codec::STRUCTURE_TYPES` (the codec decompose rule).
///
/// This is a hand-picked set, **not** a pure BMM-derivable predicate. It is the
/// set of LOCATABLE *content* subtypes that occur inside COMPOSITION / `EHR_STATUS`
/// / FOLDER trees, **plus** `EVENT_CONTEXT` and `FEEDER_AUDIT` — which AQL
/// addresses although the RM does not make them LOCATABLE. Concretely it differs
/// from "concrete LOCATABLE descendants" two ways:
/// - **added** (not LOCATABLE): `EVENT_CONTEXT` (a PATHABLE) and `FEEDER_AUDIT`.
/// - **excluded** LOCATABLE descendants: the demographic `PARTY` hierarchy
///   (`PARTY`, `ROLE`, `ACTOR`, `PERSON`, …) and `EHR`/`FOLDER`-external types,
///   which are never composition content.
///
/// Kept in lockstep with `codec.rs`; if either changes, both must change together
/// (the storage spike owns the canonical list).
const STRUCTURE_ROOTS: &[&str] = &[
    "COMPOSITION",
    "EHR_STATUS",
    "EHR_ACCESS",
    "FOLDER",
    "EVENT_CONTEXT",
    "SECTION",
    "GENERIC_ENTRY",
    "ADMIN_ENTRY",
    "OBSERVATION",
    "EVALUATION",
    "INSTRUCTION",
    "ACTION",
    "ACTIVITY",
    "HISTORY",
    "POINT_EVENT",
    "INTERVAL_EVENT",
    "ITEM_TREE",
    "ITEM_LIST",
    "ITEM_SINGLE",
    "ITEM_TABLE",
    "CLUSTER",
    "ELEMENT",
    "FEEDER_AUDIT",
];

/// One RM class row for the generated table.
struct ClassModel {
    name: String,
    /// The qualified BMM package the class is declared in.
    package: String,
    /// Whether `emit` generates a type for the class (not a mapped primitive,
    /// container or data-less foundation class).
    generated_type: bool,
    is_abstract: bool,
    /// Transitive ancestors (spec names), sorted.
    ancestors: Vec<String>,
    /// Transitive **concrete** descendants (incl. self if concrete), sorted.
    descendants: Vec<String>,
    attributes: Vec<AttrModel>,
    /// The flattened no-parameter functions, typed by their result.
    computed: Vec<AttrModel>,
    is_structure_root: bool,
    /// The class's own formal generic parameters (`BMM generic_parameter_defs`),
    /// in declaration order, with their `conforms_to` bound where declared.
    generic_params: Vec<GenericParamModel>,
}

/// One formal generic parameter of a class (`T conforms_to ITEM_STRUCTURE`).
struct GenericParamModel {
    name: String,
    conforms_to: Option<String>,
}

/// One attribute row (own or inherited, flattened ancestor-first).
struct AttrModel {
    name: String,
    declared_type: String,
    container: &'static str,
    is_mandatory: bool,
    /// The generic type-argument tree of the attribute's declared value type
    /// (empty when the type is not a generic instantiation), e.g. `[DV_QUANTITY]`
    /// for `DV_INTERVAL<DV_QUANTITY>` or `[EVENT<ITEM_STRUCTURE>]` for
    /// `List<EVENT<ITEM_STRUCTURE>>`. Bare parameters are resolved to their bound,
    /// consistent with `declared_type`.
    type_params: Vec<TypeRefModel>,
    /// The BMM-declared container cardinality (container attributes only).
    cardinality: Option<CardinalityModel>,
    /// Optional container carrying a present-implies-non-empty invariant
    /// (`Option<NonEmptyVec<T>>` emission).
    nonempty: bool,
    /// The class a reference-typed attribute points to, where the RM text
    /// names one ([`crate::plan::overrides::REF_TARGETS`]).
    ref_target: Option<String>,
}

/// A resolved type reference: a root spec name plus its own generic arguments.
struct TypeRefModel {
    name: String,
    params: Vec<TypeRefModel>,
}

/// A BMM-declared container cardinality interval (`upper = None` ⇒ unbounded).
struct CardinalityModel {
    lower: u32,
    upper: Option<u32>,
}

/// One enumeration class row (`BMM_ENUMERATION`): the underlying basic type and
/// its named constants with their (defaulted) values.
struct EnumModel {
    name: String,
    /// `"INTEGER"` or `"STRING"`.
    underlying_type: String,
    literals: Vec<EnumLiteralModel>,
}

/// One enumeration constant (name + value).
struct EnumLiteralModel {
    name: String,
    value: EnumValueModel,
}

/// The value of an enumeration constant.
enum EnumValueModel {
    Int(i64),
    Str(String),
}

/// Emit the `model/` files for `openehr-rm` from the merged BASE + RM model;
/// `schemas` are the generation's verbatim BMM files, which carry the package
/// each class is declared in.
///
/// # Errors
/// Returns an error naming the attribute when a reference-typed attribute
/// has no [`crate::plan::overrides::REF_TARGETS`] entry, or a class has no
/// package in `schemas`.
pub(crate) fn emit_files(model: &Model, schemas: &[&BmmSchema]) -> Result<Vec<GenFile>, String> {
    let packages = package_index(schemas);
    let classes = build(model, &packages)?;
    let enums = build_enums(model);
    Ok(vec![
        GenFile {
            path: "model/mod.rs".to_string(),
            body: emit_mod(),
        },
        GenFile {
            path: "model/data.rs".to_string(),
            body: emit_data(&classes, &enums),
        },
    ])
}

/// Class name → qualified BMM package (`org.openehr.base.foundation_types.
/// primitive_types`), over `schemas` in order; the first declaration wins.
fn package_index(schemas: &[&BmmSchema]) -> BTreeMap<String, String> {
    fn walk(p: &BmmPackage, prefix: &str, out: &mut BTreeMap<String, String>) {
        let qualified = if prefix.is_empty() {
            p.name.clone()
        } else {
            format!("{prefix}.{}", p.name)
        };
        for c in &p.classes {
            out.entry(c.clone()).or_insert_with(|| qualified.clone());
        }
        for sub in &p.packages {
            walk(sub, &qualified, out);
        }
    }
    let mut out = BTreeMap::new();
    for schema in schemas {
        for p in &schema.packages {
            walk(p, "", &mut out);
        }
    }
    out
}

/// Build the class table: every class of the merged BASE + RM model, the
/// foundation types included, in name order.
fn build(model: &Model, packages: &BTreeMap<String, String>) -> Result<Vec<ClassModel>, String> {
    // Concrete classes — the descendant-set universe.
    let concrete: Vec<&str> = model
        .class_iter()
        .filter(|(_, c)| !c.is_abstract)
        .map(|(n, _)| n.as_str())
        .collect();

    let mut out = Vec::new();
    for (name, class) in model.class_iter() {
        let package = packages
            .get(name)
            .ok_or_else(|| format!("class {name} is declared in no package of the generation"))?;
        let attributes = attributes_of(model, class)?;
        let computed = computed_attributes_of(model, class, &attributes);
        out.push(ClassModel {
            name: name.clone(),
            package: package.clone(),
            generated_type: !Model::is_mapped(name),
            is_abstract: class.is_abstract,
            ancestors: ancestors_of(model, name),
            descendants: descendants_of(model, name, &concrete),
            attributes,
            computed,
            is_structure_root: STRUCTURE_ROOTS.contains(&name.as_str()),
            generic_params: class
                .generic_params
                .iter()
                .map(|g| GenericParamModel {
                    name: g.name.clone(),
                    conforms_to: g.conforms_to.clone(),
                })
                .collect(),
        });
    }
    Ok(out)
}

/// Build the enumeration table: every `BMM_ENUMERATION` class (integer- or
/// string-based) in the merged model, in name order, with its constants resolved
/// 1:1 to values.
fn build_enums(model: &Model) -> Vec<EnumModel> {
    let mut out = Vec::new();
    for (name, class) in model.class_iter() {
        let Some(e) = &class.enumeration else {
            continue;
        };
        let literals = e
            .item_names
            .iter()
            .enumerate()
            .map(|(i, n)| EnumLiteralModel {
                name: n.clone(),
                value: enum_value(&e.underlying_type, e.item_values.as_deref(), i, n),
            })
            .collect();
        out.push(EnumModel {
            name: name.clone(),
            underlying_type: e.underlying_type.clone(),
            literals,
        });
    }
    out
}

/// The value of the `i`-th enumeration constant. Explicit `item_values` win;
/// where the BMM supplies none, `BMM_ENUMERATION` states "the integer values
/// 0, 1, 2, ... are assumed" — applied for an INTEGER underlying type, while a
/// STRING-underlying named constant takes its own name as its value.
fn enum_value(
    underlying: &str,
    item_values: Option<&[BmmEnumValue]>,
    i: usize,
    name: &str,
) -> EnumValueModel {
    match item_values.and_then(|v| v.get(i)) {
        Some(BmmEnumValue::Int(v)) => EnumValueModel::Int(*v),
        Some(BmmEnumValue::Str(s)) => EnumValueModel::Str(s.clone()),
        None if underlying == "STRING" => EnumValueModel::Str(name.to_string()),
        None => EnumValueModel::Int(i64::try_from(i).unwrap_or_default()),
    }
}

/// Transitive ancestors of `name`, as the BMM declares them, sorted.
fn ancestors_of(model: &Model, name: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    collect_ancestors(model, name, &mut set);
    set.into_iter().collect()
}

fn collect_ancestors(model: &Model, name: &str, out: &mut BTreeSet<String>) {
    if let Some(c) = model.get(name) {
        for a in &c.ancestors {
            out.insert(a.clone());
            collect_ancestors(model, a, out);
        }
    }
}

/// Transitive concrete descendants of `name` (incl. `name` itself when concrete),
/// sorted. `concrete` is the precomputed concrete-class universe.
fn descendants_of(model: &Model, name: &str, concrete: &[&str]) -> Vec<String> {
    concrete
        .iter()
        .filter(|d| **d == name || model.inherits(d, name))
        .map(|d| (*d).to_string())
        .collect()
}

/// The flattened (own + inherited) attributes of `class`, ancestor-first.
fn attributes_of(model: &Model, class: &BmmClass) -> Result<Vec<AttrModel>, String> {
    model
        .flattened_props(class)
        .iter()
        .map(|rp| {
            let (declared_type, container, type_params, cardinality) =
                value_shape(model, &class.name, &rp.prop.kind);
            let ref_target =
                if declared_type == "OBJECT_REF" || model.inherits(&declared_type, "OBJECT_REF") {
                    ref_target(&rp.owner, &rp.prop.name)
                        .ok_or_else(|| {
                            format!(
                                "{}.{} is typed {declared_type} but has no REF_TARGETS entry",
                                rp.owner, rp.prop.name
                            )
                        })?
                        .target
                        .map(str::to_string)
                } else {
                    None
                };
            Ok(AttrModel {
                name: rp.prop.name.clone(),
                declared_type,
                container,
                is_mandatory: rp.prop.is_mandatory,
                type_params,
                cardinality,
                nonempty: !rp.prop.is_mandatory
                    && crate::analyze::nonempty_optional_lists_cached(model)
                        .iter()
                        .any(|(decl, attr)| {
                            attr == &rp.prop.name
                                && (decl == &rp.owner || model.inherits(&rp.owner, decl))
                        }),
                ref_target,
            })
        })
        .collect()
}

/// The declared type, container kind, generic type-argument tree and
/// cardinality of a property-shaped value `kind`, in `concrete`'s scope.
fn value_shape(
    model: &Model,
    concrete: &str,
    kind: &BmmPropKind,
) -> (
    String,
    &'static str,
    Vec<TypeRefModel>,
    Option<CardinalityModel>,
) {
    match kind {
        BmmPropKind::Single(t) => (
            declared_type(model, concrete, t),
            "None",
            type_ref_params(model, concrete, t),
            None,
        ),
        BmmPropKind::Container {
            container_type,
            item,
            cardinality,
        } => (
            declared_type(model, concrete, item),
            container_kind(container_type),
            type_ref_params(model, concrete, item),
            cardinality.as_ref().map(|c| CardinalityModel {
                lower: c.lower,
                upper: c.upper,
            }),
        ),
    }
}

/// The flattened (own + inherited) computed attributes of `class`,
/// ancestor-first: every BMM function with no parameters and a result, a
/// descendant's redefinition replacing the inherited one (AOM2 `master04.2`
/// §Attribute Nodes). A name `stored` already carries is left out.
fn computed_attributes_of(model: &Model, class: &BmmClass, stored: &[AttrModel]) -> Vec<AttrModel> {
    fn gather<'a>(
        model: &'a Model,
        name: &str,
        order: &mut Vec<String>,
        map: &mut BTreeMap<String, &'a BmmFunction>,
    ) {
        let Some(class) = model.get(name) else {
            return;
        };
        for anc in &class.ancestors {
            gather(model, anc, order, map);
        }
        for f in &class.functions {
            if map.insert(f.name.clone(), f).is_none() {
                order.push(f.name.clone());
            }
        }
    }
    let mut order = Vec::new();
    let mut map = BTreeMap::new();
    gather(model, &class.name, &mut order, &mut map);
    order
        .into_iter()
        .filter_map(|n| map.remove(&n))
        .filter(|f| f.parameter_count == 0 && !stored.iter().any(|a| a.name == f.name))
        .filter_map(|f| {
            let (declared_type, container, type_params, _) =
                value_shape(model, &class.name, f.result.as_ref()?);
            Some(AttrModel {
                name: f.name.clone(),
                declared_type,
                container,
                is_mandatory: !f.is_nullable,
                type_params,
                cardinality: None,
                nonempty: false,
                ref_target: None,
            })
        })
        .collect()
}

/// The generic type-argument tree of `t` (empty for a non-generic type), each
/// argument resolved via [`type_ref`] in `concrete`'s scope.
fn type_ref_params(model: &Model, concrete: &str, t: &BmmType) -> Vec<TypeRefModel> {
    match t {
        BmmType::Simple(_) => Vec::new(),
        BmmType::Generic { params, .. } => params
            .iter()
            .map(|p| type_ref(model, concrete, p))
            .collect(),
    }
}

/// Resolve one type reference to its spec name (bare generic parameter → bound,
/// consistent with [`declared_type`]) plus its own generic-argument tree.
fn type_ref(model: &Model, concrete: &str, t: &BmmType) -> TypeRefModel {
    TypeRefModel {
        name: declared_type(model, concrete, t),
        params: type_ref_params(model, concrete, t),
    }
}

/// The declared spec-type name of an attribute value, resolving a bare generic
/// parameter (`T`) to its bound in the concrete class's scope (the same
/// bound-fill `emit` applies), and reducing generic instantiations to their root
/// (`DV_INTERVAL<T>` → `DV_INTERVAL`; the container column carries multiplicity).
///
/// A **monomorphizing** class (`X_VERSIONED_COMPOSITION :
/// X_VERSIONED_OBJECT<COMPOSITION>`, `Multiplicity_interval : Interval<Integer>`)
/// binds the parameter its generic ancestor leaves open; that binding
/// ([`class_binding`], each entry spec-cited) is what `emit` substitutes into the
/// struct field, so the static model records it too. Without it the model would
/// contradict the emitted type — reporting the parameter's BOUND (`Any`) where
/// the struct says `Vec<OriginalVersion<Composition>>` — and any consumer
/// deriving instances from the model would produce values the codec rightly
/// refuses.
fn declared_type(model: &Model, concrete: &str, t: &BmmType) -> String {
    let root = t.root_name();
    if model.is_generic_param(concrete, root) {
        if let Some(bound) = class_binding(concrete).get(root) {
            return bound.clone();
        }
        model
            .resolved_param_bound(concrete, root)
            .unwrap_or_else(|| "Any".to_string())
    } else {
        root.to_string()
    }
}

/// Map a BMM container kind to a `Container` variant name.
fn container_kind(container_type: &str) -> &'static str {
    match container_type {
        "Set" => "Set",
        "Hash" => "Hash",
        // `List`, `Array`, and any unknown container are ordered lists.
        _ => "List",
    }
}

/// Render a `&["A", "B"]`-style array literal body (`["A", "B"]`, or `[]`).
fn str_slice(items: &[String]) -> String {
    let mut b = String::from("[");
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            b.push_str(", ");
        }
        b.push_str(&format!("{s:?}"));
    }
    b.push(']');
    b
}

/// The generated `model/data.rs`: the `static CLASSES` + `static ENUMERATIONS`
/// tables.
fn emit_data(classes: &[ClassModel], enums: &[EnumModel]) -> String {
    let mut b = String::from(
        "// @generated by openehr-codegen (emit-rm-model) — DO NOT EDIT.\n\
         //! Static RM class/attribute table backing the `model` API.\n\n\
         use super::{\n    \
         Cardinality, Container, EnumValue, RmAttribute, RmClass, RmEnumLiteral, RmEnumeration,\n    \
         RmGenericParam, RmTypeRef,\n};\n\n\
         pub(super) static CLASSES: &[RmClass] = &[\n",
    );
    for c in classes {
        b.push_str(&format!(
            "    RmClass {{\n        name: {:?},\n        package: {:?},\n        generated_type: {},\n        is_abstract: {},\n",
            c.name, c.package, c.generated_type, c.is_abstract
        ));
        b.push_str(&format!(
            "        ancestors: &{},\n",
            str_slice(&c.ancestors)
        ));
        b.push_str(&format!(
            "        descendants: &{},\n",
            str_slice(&c.descendants)
        ));
        for (field, rows) in [
            ("attributes", &c.attributes),
            ("computed_attributes", &c.computed),
        ] {
            b.push_str(&format!("        {field}: &[\n"));
            for a in rows {
                b.push_str(&attribute_lit(a));
            }
            b.push_str("        ],\n");
        }
        b.push_str(&format!(
            "        is_structure_root: {},\n",
            c.is_structure_root
        ));
        b.push_str(&format!(
            "        generic_params: &{},\n    }},\n",
            generic_params(&c.generic_params)
        ));
    }
    b.push_str("];\n\n");

    b.push_str("pub(super) static ENUMERATIONS: &[RmEnumeration] = &[\n");
    for e in enums {
        b.push_str(&format!(
            "    RmEnumeration {{\n        name: {:?},\n        underlying_type: {:?},\n        literals: &[\n",
            e.name, e.underlying_type
        ));
        for lit in &e.literals {
            b.push_str(&format!(
                "            RmEnumLiteral {{ name: {:?}, value: {} }},\n",
                lit.name,
                enum_value_lit(&lit.value)
            ));
        }
        b.push_str("        ],\n    },\n");
    }
    b.push_str("];\n");
    b
}

/// Render one `RmAttribute { … },` row of an attribute table.
fn attribute_lit(a: &AttrModel) -> String {
    format!(
        "            RmAttribute {{ name: {:?}, declared_type: {:?}, container: Container::{}, is_mandatory: {}, type_params: &{}, cardinality: {}, nonempty: {}, ref_target: {} }},\n",
        a.name,
        a.declared_type,
        a.container,
        a.is_mandatory,
        type_refs(&a.type_params),
        cardinality(a.cardinality.as_ref()),
        a.nonempty,
        a.ref_target
            .as_ref()
            .map_or_else(|| "None".to_string(), |t| format!("Some({t:?})")),
    )
}

/// Render a `&[RmTypeRef { … }]`-body array literal (`[]` when empty), recursing
/// into each argument's own parameter tree.
fn type_refs(items: &[TypeRefModel]) -> String {
    let mut b = String::from("[");
    for (i, t) in items.iter().enumerate() {
        if i > 0 {
            b.push_str(", ");
        }
        b.push_str(&format!(
            "RmTypeRef {{ name: {:?}, params: &{} }}",
            t.name,
            type_refs(&t.params)
        ));
    }
    b.push(']');
    b
}

/// Render an `Option<Cardinality>` literal.
fn cardinality(card: Option<&CardinalityModel>) -> String {
    card.map_or_else(
        || "None".to_string(),
        |c| {
            let upper = c
                .upper
                .map_or_else(|| "None".to_string(), |u| format!("Some({u})"));
            format!(
                "Some(Cardinality {{ lower: {}, upper: {} }})",
                c.lower, upper
            )
        },
    )
}

/// Render a `&[RmGenericParam { … }]`-body array literal (`[]` when empty).
fn generic_params(params: &[GenericParamModel]) -> String {
    let mut b = String::from("[");
    for (i, g) in params.iter().enumerate() {
        if i > 0 {
            b.push_str(", ");
        }
        let bound = g
            .conforms_to
            .as_ref()
            .map_or_else(|| "None".to_string(), |c| format!("Some({c:?})"));
        b.push_str(&format!(
            "RmGenericParam {{ name: {:?}, conforms_to: {} }}",
            g.name, bound
        ));
    }
    b.push(']');
    b
}

/// Render an `EnumValue` literal.
fn enum_value_lit(value: &EnumValueModel) -> String {
    match value {
        EnumValueModel::Int(v) => format!("EnumValue::Int({v})"),
        EnumValueModel::Str(s) => format!("EnumValue::Str({s:?})"),
    }
}
