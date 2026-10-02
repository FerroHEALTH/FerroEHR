// SPDX-FileCopyrightText: Vernum Projecten B.V.
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
//!   `class`/`attribute`/`attributes`/`descendants`/`ancestors`/`is_a`/
//!   `is_primitive`/`conforms_to_ordered`/`is_structure_root` functions + a
//!   `LazyLock` name index).
//! - `data.rs` — the generated `static CLASSES: &[RmClass]` table.

use crate::analyze::Model;
use crate::load::bmm::{BmmClass, BmmEnumValue, BmmPackage, BmmPropKind, BmmSchema, BmmType};
use crate::plan::overrides::{class_binding, ref_target};
use crate::render::emit::GenFile;
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
        out.push(ClassModel {
            name: name.clone(),
            package: package.clone(),
            generated_type: !Model::is_mapped(name),
            is_abstract: class.is_abstract,
            ancestors: ancestors_of(model, name),
            descendants: descendants_of(model, name, &concrete),
            attributes: attributes_of(model, class)?,
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
            let (declared_type, container, type_params, cardinality) = match &rp.prop.kind {
                BmmPropKind::Single(t) => (
                    declared_type(model, &class.name, t),
                    "None",
                    type_ref_params(model, &class.name, t),
                    None,
                ),
                BmmPropKind::Container {
                    container_type,
                    item,
                    cardinality,
                } => (
                    declared_type(model, &class.name, item),
                    container_kind(container_type),
                    type_ref_params(model, &class.name, item),
                    cardinality.as_ref().map(|c| CardinalityModel {
                        lower: c.lower,
                        upper: c.upper,
                    }),
                ),
            };
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

/// The hand-authored-shaped, generated `model/mod.rs`: the public API over the
/// generated `data::CLASSES` table.
fn emit_mod() -> String {
    // Note: the body is a literal — no per-class data — so it is a constant
    // string. rustfmt runs over the written file afterwards.
    r#"// @generated by openehr-codegen (emit-rm-model) — DO NOT EDIT.
//! Static RM attribute/type model — the AQL planner's spec-pinned oracle,
//! generated from the BASE + RM BMM meta-model (the same input as the `emit`
//! target). No reflection, no hand-maintained tables.
//!
//! Covers every class of the BASE + RM BMM, the BASE foundation types
//! (primitives, the `Ordered` and `Numeric` markers, containers) included, each
//! with the BMM package it is declared in. For each class it records
//! the flattened (own + inherited) attributes with their declared spec type,
//! generic type-argument tree, container kind + cardinality, and mandatory flag;
//! the abstract flag; the class's own formal generic parameters; the transitive
//! ancestor set; the transitive **concrete** descendant set; and a structure-node
//! flag. Enumeration classes (`BMM_ENUMERATION`) additionally carry their named
//! constants + values in a separate table (see [`enumeration`]).
//!
//! A reference-typed attribute (`OBJECT_REF`, `PARTY_REF`, `LOCATABLE_REF`)
//! names the class it points to where the RM text states one
//! ([`RmAttribute::ref_target`]); the BMM types the attribute by the
//! reference class alone.
//!
//! # `is_structure_root`
//!
//! `is_structure_root(class)` is true for exactly the RM types the node codec
//! splits into their own `node` row (`ferroehr::storage::codec`). That set is
//! **not** derivable from the BMM alone — it is a hand-picked list mirrored from
//! `codec::STRUCTURE_TYPES`: the LOCATABLE *content* subtypes used inside
//! COMPOSITION / EHR_STATUS / FOLDER trees, **plus** `EVENT_CONTEXT` and
//! `FEEDER_AUDIT` (which AQL addresses although the RM does not make them
//! LOCATABLE), and **without** the demographic LOCATABLE hierarchy (`PARTY`,
//! `ROLE`, `ACTOR`, …), which is never composition content. The emitter keeps
//! this in lockstep with `codec.rs`.

use std::collections::HashMap;
use std::sync::LazyLock;

mod data;

/// An RM class in the static model.
#[derive(Debug)]
pub struct RmClass {
    /// The spec class name, verbatim (e.g. `"OBSERVATION"`).
    pub name: &'static str,
    /// The qualified BMM package the class is declared in (e.g.
    /// `"org.openehr.rm.composition.content.entry"`).
    pub package: &'static str,
    /// Whether the generated `openehr-*` crates carry a type of their own for
    /// this class. False for a primitive (a Rust scalar), a container (`Vec`,
    /// map, set) and a foundation class that holds no data (the `Ordered` and
    /// `Numeric` markers, the functional types, the built-in services, the
    /// constant holders): none of them is ever a canonical-JSON object.
    pub generated_type: bool,
    /// Whether the class is abstract (never instantiated directly).
    pub is_abstract: bool,
    /// Transitive ancestor spec names, as the BMM declares them (`DV_ORDERED`
    /// carries `Ordered`, `Integer` carries `Ordered_Numeric` and `Ordered`).
    pub ancestors: &'static [&'static str],
    /// Transitive **concrete** descendant spec names, including this class when
    /// it is itself concrete.
    pub descendants: &'static [&'static str],
    /// The flattened (own + inherited) attributes, ancestor-first.
    pub attributes: &'static [RmAttribute],
    /// Whether the node codec splits this type into its own `node` row (see the
    /// module docs).
    pub is_structure_root: bool,
    /// The class's own formal generic parameters, in declaration order (empty for
    /// a non-generic class), e.g. `[T conforms_to ITEM_STRUCTURE]` on `HISTORY`.
    pub generic_params: &'static [RmGenericParam],
}

/// One formal generic parameter of an [`RmClass`] (`BMM generic_parameter_defs`).
#[derive(Debug)]
pub struct RmGenericParam {
    /// The parameter name, verbatim (e.g. `"T"`).
    pub name: &'static str,
    /// The `conforms_to` bound spec name, if the BMM declares one (e.g.
    /// `Some("ITEM_STRUCTURE")`).
    pub conforms_to: Option<&'static str>,
}

/// One attribute of an [`RmClass`].
#[derive(Debug)]
pub struct RmAttribute {
    /// The attribute name, verbatim (e.g. `"content"`).
    pub name: &'static str,
    /// The declared value spec-type name (e.g. `"HISTORY"`, `"DV_TEXT"`,
    /// `"CONTENT_ITEM"`; a generic parameter is resolved to its bound). This is
    /// the root/base name; [`Self::type_params`] carries any generic arguments.
    pub declared_type: &'static str,
    /// The attribute's container shape.
    pub container: Container,
    /// Whether the attribute is mandatory (existence ≥ 1).
    pub is_mandatory: bool,
    /// The generic type-argument tree of the declared value type, empty when it
    /// is not a generic instantiation. Together with [`Self::declared_type`] this
    /// gives the full declared type: `declared_type = "DV_INTERVAL"` +
    /// `type_params = [DV_QUANTITY]` reads `DV_INTERVAL<DV_QUANTITY>`;
    /// `declared_type = "EVENT"` + `type_params = [ITEM_STRUCTURE]` inside a
    /// `List` container reads `List<EVENT<ITEM_STRUCTURE>>`. Bare generic
    /// parameters are resolved to their bound, consistent with `declared_type`.
    pub type_params: &'static [RmTypeRef],
    /// The BMM-declared container cardinality (`None` for a single-valued
    /// attribute, or a container attribute the BMM leaves unconstrained).
    pub cardinality: Option<Cardinality>,
    /// An optional container carrying a present-implies-non-empty invariant:
    /// emitted `Option<NonEmptyVec<T>>`, so `[]` refuses at parse.
    pub nonempty: bool,
    /// The class a reference-typed attribute points to (`EHR.ehr_status` →
    /// `EHR_STATUS`), where the RM text names one; `None` for every other
    /// attribute and for a reference whose target the text leaves open
    /// (`VERSIONED_OBJECT.owner_id`, "e.g. the id of the containing EHR").
    pub ref_target: Option<&'static str>,
}

/// A resolved type reference: a root spec name plus its own generic arguments.
#[derive(Debug)]
pub struct RmTypeRef {
    /// The root/base spec-type name (a bare generic parameter resolved to bound).
    pub name: &'static str,
    /// This type's own generic arguments (empty when it is not generic).
    pub params: &'static [RmTypeRef],
}

/// A container cardinality interval (`BMM cardinality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cardinality {
    /// The lower bound (minimum number of members).
    pub lower: u32,
    /// The upper bound, or `None` when the container is unbounded.
    pub upper: Option<u32>,
}

/// The container shape of an [`RmAttribute`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Single-valued (`T` / `Option<T>`).
    None,
    /// Ordered list (`List`/`Array`).
    List,
    /// Unordered set (`Set`).
    Set,
    /// Keyed map (`Hash`).
    Hash,
}

/// An RM enumeration class (`BMM_ENUMERATION`): an underlying basic type plus a
/// set of named constants.
#[derive(Debug)]
pub struct RmEnumeration {
    /// The enumeration class name, verbatim (e.g. `"PROPORTION_KIND"`).
    pub name: &'static str,
    /// The underlying basic type: `"INTEGER"` or `"STRING"`.
    pub underlying_type: &'static str,
    /// The named constants, in declaration order.
    pub literals: &'static [RmEnumLiteral],
}

/// One constant of an [`RmEnumeration`].
#[derive(Debug)]
pub struct RmEnumLiteral {
    /// The constant name, verbatim (e.g. `"pk_percent"`, `"mandatory"`).
    pub name: &'static str,
    /// The constant value.
    pub value: EnumValue,
}

/// The value of an [`RmEnumLiteral`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumValue {
    /// An integer constant.
    Int(i64),
    /// A string constant.
    Str(&'static str),
}

/// Name → class index, built once from the generated table.
static INDEX: LazyLock<HashMap<&'static str, &'static RmClass>> =
    LazyLock::new(|| data::CLASSES.iter().map(|c| (c.name, c)).collect());

fn find(name: &str) -> Option<&'static RmClass> {
    INDEX.get(name).copied()
}

/// The class named `name`, if present in the model.
#[must_use]
pub fn class(name: &str) -> Option<&'static RmClass> {
    find(name)
}

/// Resolve attribute `attr` on `class`, through inheritance (the stored attribute
/// list is already flattened, so an inherited attribute like `LOCATABLE.name`
/// resolves for `OBSERVATION`).
#[must_use]
pub fn attribute(class: &str, attr: &str) -> Option<&'static RmAttribute> {
    find(class)?.attributes.iter().find(|a| a.name == attr)
}

/// Every attribute of `class`, own and inherited (empty for an unknown class).
pub fn attributes(class: &str) -> impl Iterator<Item = &'static RmAttribute> {
    const EMPTY: &[RmAttribute] = &[];
    find(class).map_or(EMPTY, |c| c.attributes).iter()
}

/// The declared RM type of `field` on `parent_type` when that type is
/// concrete, else `None`.
///
/// This is the effective-type rule for an UNTAGGED canonical-JSON node: the
/// ITS-JSON schema requires `_type` only on polymorphic slots, so a node under
/// a concretely-declared attribute (`COMPOSITION.context` -> `EVENT_CONTEXT`,
/// `EVENT_CONTEXT.participations` -> `PARTICIPATION`, ...) may legally omit it
/// — and a validation walk that dispatches on the wire tag alone would skip
/// every RM invariant on such a node. An abstract declared type yields `None`
/// (there the wire MUST tag, and an untagged node is unreadable rather than
/// silently valid).
#[must_use]
pub fn declared_concrete_type(parent_type: &str, field: &str) -> Option<&'static str> {
    let attr = attribute(parent_type, field)?;
    let class = class(attr.declared_type)?;
    (!class.is_abstract).then_some(class.name)
}

/// The transitive concrete descendants of `class` (empty for an unknown class).
#[must_use]
pub fn descendants(class: &str) -> &'static [&'static str] {
    const EMPTY: &[&str] = &[];
    find(class).map_or(EMPTY, |c| c.descendants)
}

/// The transitive ancestors of `class` (empty for an unknown class).
#[must_use]
pub fn ancestors(class: &str) -> &'static [&'static str] {
    const EMPTY: &[&str] = &[];
    find(class).map_or(EMPTY, |c| c.ancestors)
}

/// Whether `sub` is `sup` or a (transitive) subtype of it.
#[must_use]
pub fn is_a(sub: &str, sup: &str) -> bool {
    sub == sup || find(sub).is_some_and(|c| c.ancestors.contains(&sup))
}

/// The BMM package of the primitive types (BASE foundation_types
/// master03-primitive_types.adoc §Overview).
pub const PRIMITIVE_TYPES_PACKAGE: &str = "org.openehr.base.foundation_types.primitive_types";

/// Whether `name` is a primitive type: a concrete class of the BASE
/// `primitive_types` package (`Boolean`, `Character`, `Integer`, `Integer64`,
/// `Octet`, `Real`, `Double`, `String`, `Uri`).
#[must_use]
pub fn is_primitive(name: &str) -> bool {
    find(name).is_some_and(|c| !c.is_abstract && c.package == PRIMITIVE_TYPES_PACKAGE)
}

/// The root BMM package of the BASE foundation types.
pub const FOUNDATION_TYPES_PACKAGE: &str = "org.openehr.base.foundation_types";

/// Whether `name` is a BASE foundation type: a class of the
/// `foundation_types` package or one of its sub-packages (primitives,
/// structures, intervals, time and terminology types, `Any`).
///
/// A foundation type is a value type: BASE draws the inheritance among the
/// primitives to "facilitate the type descriptions", substitutability for
/// `Any` "is not assumed" (BASE foundation_types master03-primitive_types.adoc
/// §Overview), and an enumeration such as `PROPORTION_KIND` is a descendant of
/// `Integer` only as its underlying type. A consumer expanding a declared type
/// to the RM content that may fill it therefore expands RM classes only.
#[must_use]
pub fn is_foundation_type(name: &str) -> bool {
    find(name).is_some_and(|c| {
        c.package
            .strip_prefix(FOUNDATION_TYPES_PACKAGE)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    })
}

/// Whether `name` is `Ordered` or conforms to it, so its values compare by the
/// `Ordered` operators (QUERY master03 §ORDER BY: the operators "available to
/// primitives and `Ordered` types").
#[must_use]
pub fn conforms_to_ordered(name: &str) -> bool {
    is_a(name, "Ordered")
}

/// Whether the node codec splits `class` into its own `node` row (see module docs).
#[must_use]
pub fn is_structure_root(class: &str) -> bool {
    find(class).is_some_and(|c| c.is_structure_root)
}

/// Every class of the static model, in emission order — the whole-model
/// iterator for exhaustive sweeps (generators, reach instrumentation).
pub fn classes() -> impl Iterator<Item = &'static RmClass> {
    data::CLASSES.iter()
}

/// Name → enumeration index, built once from the generated table.
static ENUM_INDEX: LazyLock<HashMap<&'static str, &'static RmEnumeration>> =
    LazyLock::new(|| data::ENUMERATIONS.iter().map(|e| (e.name, e)).collect());

/// The enumeration class named `name`, if it is one (e.g. `"PROPORTION_KIND"`).
#[must_use]
pub fn enumeration(name: &str) -> Option<&'static RmEnumeration> {
    ENUM_INDEX.get(name).copied()
}
"#
    .to_string()
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
        b.push_str("        attributes: &[\n");
        for a in &c.attributes {
            b.push_str(&format!(
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
            ));
        }
        b.push_str("        ],\n");
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
