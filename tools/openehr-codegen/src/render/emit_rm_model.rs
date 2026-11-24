// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `emit-rm-model`: the static RM attribute/type model in `openehr-rm`
//! (`src/model/`), the AQL planner's oracle: `mod.rs` holds the API,
//! `data.rs` the `static CLASSES` table.

use crate::analyze::Model;
use crate::load::bmm::{BmmClass, BmmEnumValue, BmmPackage, BmmPropKind, BmmSchema, BmmType};
use crate::load::bmm_function::BmmFunction;
use crate::plan::drafts::draft_ref_target;
use crate::plan::overrides::{class_binding, ref_target};
use crate::render::emit::GenFile;
use crate::render::emit_rm_model_api::emit_mod;
use crate::render::name_index::emit_index_fn;
use std::collections::{BTreeMap, BTreeSet};

/// The classes the node codec splits into their own `node` row, mirrored from
/// `ferroehr::storage::codec::STRUCTURE_TYPES`; both change together.
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
    /// The generic type-argument tree of the declared value type (`[DV_QUANTITY]`
    /// for `DV_INTERVAL<DV_QUANTITY>`).
    type_params: Vec<TypeRefModel>,
    /// The formal generic parameter the attribute's value type is written as
    /// (`T` for `DV_INTERVAL.lower`), `None` when it names a class.
    formal: Option<String>,
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
    /// The formal generic parameter this reference is written as, resolved to
    /// `name` (its bound).
    formal: Option<String>,
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

/// Emits the `model/` files for `openehr-rm`; `schemas` carry each class's
/// package.
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
            body: emit_data(&classes, &enums)?,
        },
    ])
}

/// The class table of a draft package's own classes, in the RM table's shape,
/// so the archetype validator can extend the RM with the draft
/// (`AM/docs/AOM2/master11-rm_adaptation.adoc`).
///
/// # Errors
/// As [`emit_files`].
pub(crate) fn draft_model_file(
    model: &Model,
    schemas: &[&BmmSchema],
    own: &BmmSchema,
    rm_model: &str,
) -> Result<String, String> {
    let packages = package_index(schemas);
    let classes: Vec<ClassModel> = build(model, &packages)?
        .into_iter()
        .filter(|c| own.classes.contains_key(&c.name))
        .collect();
    let rows: String = classes.iter().map(class_lit).collect();
    let used: Vec<&str> = [
        "Cardinality",
        "Container",
        "RmAttribute",
        "RmClass",
        "RmGenericParam",
        "RmTypeRef",
    ]
    .into_iter()
    .filter(|name| {
        *name == "RmClass"
            || rows.contains(&format!("{name} "))
            || rows.contains(&format!("{name}::"))
    })
    .collect();
    Ok(format!(
        "// @generated by openehr-codegen (emit-drafts) — DO NOT EDIT.\n\
         //! The draft's own classes in the shape of the RM class table, which extend\n\
         //! the RM for the archetype validator.\n\n\
         use {rm_model}::{{{}}};\n\n\
         /// The draft's own classes, in name order.\n\
         pub static CLASSES: &[RmClass] = &[\n{rows}];\n",
        used.join(", ")
    ))
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

/// Builds the enumeration table, in name order.
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

/// The value of the `i`-th enumeration constant: explicit `item_values` win,
/// else "the integer values 0, 1, 2, ... are assumed" (`BMM_ENUMERATION`) for
/// an INTEGER type and the constant's name for a STRING type.
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
            let formal = formal_param(model, &class.name, value_type(&rp.prop.kind));
            let ref_target =
                if declared_type == "OBJECT_REF" || model.inherits(&declared_type, "OBJECT_REF") {
                    ref_target(&rp.owner, &rp.prop.name)
                        .or_else(|| draft_ref_target(&rp.owner, &rp.prop.name))
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
                formal,
                cardinality,
                nonempty: !rp.prop.is_mandatory
                    && (crate::analyze::nonempty_optional_lists_cached(model)
                        .iter()
                        .any(|(decl, attr)| {
                            attr == &rp.prop.name
                                && (decl == &rp.owner || model.inherits(&rp.owner, decl))
                        })
                        || crate::plan::its_json_bounds::refuses_empty(&rp.owner, &rp.prop.name)),
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

/// The flattened computed attributes of `class`, ancestor first, a
/// redefinition replacing the inherited one (AOM2 `master04.2` §Attribute
/// Nodes); a name `stored` carries is left out.
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
            let result = f.result.as_ref()?;
            let (declared_type, container, type_params, _) =
                value_shape(model, &class.name, result);
            Some(AttrModel {
                name: f.name.clone(),
                declared_type,
                container,
                is_mandatory: !f.is_nullable,
                type_params,
                formal: formal_param(model, &class.name, value_type(result)),
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
        formal: formal_param(model, concrete, t),
        params: type_ref_params(model, concrete, t),
    }
}

/// The declared spec-type name of an attribute value: a bare parameter
/// resolved to its bound or to its [`class_binding`], an instantiation reduced
/// to its root (`DV_INTERVAL<T>` → `DV_INTERVAL`).
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

/// The value type of a property-shaped `kind`: the type itself, or a
/// container's item type.
fn value_type(kind: &BmmPropKind) -> &BmmType {
    match kind {
        BmmPropKind::Single(t) | BmmPropKind::Container { item: t, .. } => t,
    }
}

/// The formal generic parameter `t` is written as in `concrete`'s scope, `None`
/// when `t` names a class or a bound parameter.
fn formal_param(model: &Model, concrete: &str, t: &BmmType) -> Option<String> {
    let root = t.root_name();
    (model.is_generic_param(concrete, root) && !class_binding(concrete).contains_key(root))
        .then(|| root.to_string())
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
fn emit_data(classes: &[ClassModel], enums: &[EnumModel]) -> Result<String, String> {
    let mut b = String::from(
        "// @generated by openehr-codegen (emit-rm-model) — DO NOT EDIT.\n\
         //! Static RM class/attribute table backing the `model` API.\n\n\
         #![allow(\n    \
         clippy::too_many_lines,\n    \
         reason = \"one arm per name in the lookups, decided at generation time\"\n\
         )]\n\n\
         use super::{\n    \
         Cardinality, Container, EnumValue, RmAttribute, RmClass, RmEnumLiteral, RmEnumeration,\n    \
         RmGenericParam, RmTypeRef,\n};\n\n\
         pub(super) static CLASSES: &[RmClass] = &[\n",
    );
    for c in classes {
        b.push_str(&class_lit(c));
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
    b.push_str("];\n\n");
    let class_names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
    b.push_str(&emit_index_fn(
        "class_index",
        "The position in [`CLASSES`] of the class named `name`, if any.",
        &class_names,
    )?);
    b.push('\n');
    let enum_names: Vec<&str> = enums.iter().map(|e| e.name.as_str()).collect();
    b.push_str(&emit_index_fn(
        "enumeration_index",
        "The position in [`ENUMERATIONS`] of the enumeration named `name`, if any.",
        &enum_names,
    )?);
    Ok(b)
}

/// Render one `RmClass { … },` row of a class table.
fn class_lit(c: &ClassModel) -> String {
    let mut b = String::new();
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
    b
}

/// Render one `RmAttribute { … },` row of an attribute table.
fn attribute_lit(a: &AttrModel) -> String {
    format!(
        "            RmAttribute {{ name: {:?}, declared_type: {:?}, formal: {}, container: Container::{}, is_mandatory: {}, type_params: &{}, cardinality: {}, nonempty: {}, ref_target: {} }},\n",
        a.name,
        a.declared_type,
        optional_str(a.formal.as_deref()),
        a.container,
        a.is_mandatory,
        type_refs(&a.type_params),
        cardinality(a.cardinality.as_ref()),
        a.nonempty,
        optional_str(a.ref_target.as_deref()),
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
            "RmTypeRef {{ name: {:?}, formal: {}, params: &{} }}",
            t.name,
            optional_str(t.formal.as_deref()),
            type_refs(&t.params)
        ));
    }
    b.push(']');
    b
}

/// Render an `Option<&'static str>` literal.
fn optional_str(value: Option<&str>) -> String {
    value.map_or_else(|| "None".to_string(), |v| format!("Some({v:?})"))
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
