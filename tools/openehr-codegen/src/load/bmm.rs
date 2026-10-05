// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! BMM object model + loader (openEHR **LANG 1.0.0**, `P_BMM` persisted form).
//!
//! The generator's own internal BMM reader, not an openEHR spec artifact. It
//! models more of the meta-model than the emitter consumes, hence the
//! module-wide `dead_code` allowance.
#![allow(
    dead_code,
    reason = "the LOAD stage models the vendored P_BMM persisted form in full \
              (its own discipline: parse the input verbatim, decide nothing); \
              meta-model members no emitter reads yet are part of that fidelity, \
              not leftovers, so this is a module-wide allowance rather than a \
              per-item expectation"
)]
//!
//! Loads a vendored `*.bmm.json` schema (the canonical JSON serialization of
//! the openEHR BMM meta-model) into a typed [`BmmSchema`]: packages (the module
//! tree), classes (with ancestors, abstractness, generic parameters), and each
//! class's ordered properties (single / generic / container, with cardinality
//! and mandatory-ness).
//!
//! The JSON form is read rather than the ODIN one: it serializes the identical
//! meta-model with real arrays, structured `cardinality` and explicit `_type`
//! tags. BMM captures structure; function bodies and invariant logic stay
//! hand-written, though the invariant EL text and function signatures are
//! present in the JSON.

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM/OAS bundles, emitter reports) — not the \
              application (#1694)"
)]
use serde_json::Value;
use std::collections::BTreeMap;

/// A loaded BMM schema (one vendored `*.bmm.json` file).
#[derive(Debug, Clone)]
pub(crate) struct BmmSchema {
    /// `schema_name`, e.g. `"rm"`.
    pub schema_name: String,
    /// `rm_release`, e.g. `"1.1.0"`.
    pub rm_release: String,
    /// `bmm_version`, e.g. `"2.4"`.
    pub bmm_version: String,
    /// Ids of included schemas (e.g. `"openehr_base_1.2.0"`).
    pub includes: Vec<String>,
    /// Top-level packages (a tree; see [`BmmPackage`]).
    pub packages: Vec<BmmPackage>,
    /// All class definitions, keyed by class name.
    pub classes: BTreeMap<String, BmmClass>,
}

/// A BMM package node in the package tree.
#[derive(Debug, Clone)]
pub(crate) struct BmmPackage {
    /// Dotted package name, e.g. `"org.openehr.rm.common"` or a leaf `"archetyped"`.
    pub name: String,
    /// Class names declared directly in this package.
    pub classes: Vec<String>,
    /// Sub-packages.
    pub packages: Vec<BmmPackage>,
}

/// A BMM class definition.
#[derive(Debug, Clone)]
pub(crate) struct BmmClass {
    /// Class name, e.g. `"DV_QUANTITY"`.
    pub name: String,
    /// Class documentation (verbatim), if any.
    pub documentation: Option<String>,
    /// Immediate ancestors (may include foundation classes like `"Interval"`).
    pub ancestors: Vec<String>,
    /// Whether the class is abstract (`is_abstract = true`).
    pub is_abstract: bool,
    /// Generic parameter definitions (`generic_parameter_defs`), if generic.
    pub generic_params: Vec<BmmGenericParam>,
    /// Properties in declaration order.
    pub properties: Vec<BmmProperty>,
    /// Enumeration definition when the class is a `P_BMM_ENUMERATION_*`
    /// (`BMM_ENUMERATION`: an underlying basic type + a set of named constants).
    pub enumeration: Option<BmmEnumeration>,
    /// Class constants (`BMM_CLASS.constants`: named literal values, e.g. the
    /// openEHR terminology-group / code-set identifier strings), in declaration
    /// order. Verbatim — the render stage decodes the literal `value`.
    pub constants: Vec<BmmConstant>,
    /// Declared function names (`BMM_CLASS.functions`), in file order. The BMM
    /// carries functions by NAME + result type only (no body), so the loader
    /// keeps just the names: they are what a hand-written behaviour sibling must
    /// realize, per generation.
    pub functions: Vec<String>,
    /// Class invariants (`BMM_CLASS.invariants`): a map from invariant name to
    /// its assertion-expression text (the Eiffel/UML assertion dialect), verbatim.
    /// The `analyze` stage parses + classifies these; the loader keeps them raw.
    pub invariants: BTreeMap<String, String>,
}

/// One `BMM_CLASS.constants` entry: a named literal value on the class. The
/// `value` is kept as the raw BMM JSON scalar (a JSON number, or a JSON string
/// carrying a quoted `"..."` / `'...'` literal or a bareword cross-reference to
/// another constant); the render stage decodes it to a Rust literal.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BmmConstant {
    /// The verbatim BMM constant name (`Terminology_id_openehr`).
    pub name: String,
    /// Constant documentation (verbatim), if any.
    pub documentation: Option<String>,
    /// The BMM `type` (`String`, `Integer`, `Real`, `Character`, `Boolean`).
    pub type_name: String,
    /// The raw BMM `value` scalar (decoded by the render stage).
    pub value: Value,
}

/// A BMM enumeration definition (`BMM_ENUMERATION` / persisted
/// `P_BMM_ENUMERATION_INTEGER` | `P_BMM_ENUMERATION_STRING`): an underlying
/// basic type plus a 1:1 `item_names` / `item_values` pair of named constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BmmEnumeration {
    /// `underlying_type_name` — `"INTEGER"` for `P_BMM_ENUMERATION_INTEGER`,
    /// `"STRING"` for `P_BMM_ENUMERATION_STRING` (`BMM_ENUMERATION_*` redefine it).
    pub underlying_type: String,
    /// The constant names, in declaration order (`BMM_ENUMERATION.item_names`).
    pub item_names: Vec<String>,
    /// The explicit constant values, 1:1 with `item_names`, when the BMM supplies
    /// them (`BMM_ENUMERATION.item_values`, optional). Absence is preserved as
    /// `None`; the default-value rule is applied by the consumer, not here.
    pub item_values: Option<Vec<BmmEnumValue>>,
}

/// One enumeration constant value (`BMM_ENUMERATION.item_values` is `List<Any>`;
/// the concrete subtype fixes it to `Integer` or `String`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BmmEnumValue {
    /// An integer constant (`P_BMM_ENUMERATION_INTEGER`).
    Int(i64),
    /// A string constant (`P_BMM_ENUMERATION_STRING`).
    Str(String),
}

/// A generic parameter of a class, e.g. `T conforms_to DV_ORDERED`.
#[derive(Debug, Clone)]
pub(crate) struct BmmGenericParam {
    /// Parameter name, e.g. `"T"`.
    pub name: String,
    /// The `conforms_to_type` bound, if any.
    pub conforms_to: Option<String>,
}

/// A BMM property (attribute) of a class.
#[derive(Debug, Clone)]
pub(crate) struct BmmProperty {
    /// Property name, e.g. `"magnitude"`.
    pub name: String,
    /// Property documentation (verbatim), if any.
    pub documentation: Option<String>,
    /// `is_mandatory = true` → an obligatory attribute (else optional/`Option`).
    pub is_mandatory: bool,
    /// The vendored `default` facet, verbatim, if the schema carries one.
    ///
    /// The value is the schema's own text for the attribute's initial value —
    /// ODIN `default = <False>` / `<"Boolean">` arrives here as `"False"` /
    /// `"\"Boolean\""`. It is kept unparsed because the facet is NOT part of the
    /// released persistence model: LANG
    /// `docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_property.adoc`
    /// §Attributes lists `name`, `is_mandatory`, `is_computed`,
    /// `is_im_infrastructure`, `is_im_runtime`, `type_def` and `bmm_property`
    /// and no `default`, so every vendored occurrence is an undeclared extension
    /// whose meaning is adjudicated in `plan::overrides`, not assumed here. The
    /// loader's job is only to stop dropping it.
    pub default: Option<String>,
    /// The property's shape and type.
    pub kind: BmmPropKind,
}

/// The shape of a [`BmmProperty`].
#[derive(Debug, Clone)]
pub(crate) enum BmmPropKind {
    /// A single-valued property (`P_BMM_SINGLE_PROPERTY`,
    /// `P_BMM_SINGLE_PROPERTY_OPEN`, or `P_BMM_GENERIC_PROPERTY`).
    Single(BmmType),
    /// A container property (`P_BMM_CONTAINER_PROPERTY`), e.g. `List<LINK>`.
    Container {
        /// `List`, `Set`, `Array`, `Hash`, …
        container_type: String,
        /// The element type.
        item: BmmType,
        /// Structured cardinality, if present.
        cardinality: Option<BmmCardinality>,
    },
}

/// A structured container cardinality (`{lower, upper_unbounded, upper}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BmmCardinality {
    /// Lower bound (default 0).
    pub lower: u32,
    /// Upper bound, or `None` if unbounded.
    pub upper: Option<u32>,
}

/// A BMM type reference: a named type, or a generic instantiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BmmType {
    /// A simple named type (`Real`, `DV_TEXT`, or a generic parameter like `T`).
    Simple(String),
    /// A generic instantiation, e.g. `DV_INTERVAL<DV_QUANTITY>`.
    Generic {
        /// The root (open) type, e.g. `DV_INTERVAL`.
        root: String,
        /// The actual type arguments.
        params: Vec<BmmType>,
    },
}

impl BmmType {
    /// The underlying root/simple type name (ignoring generic args).
    #[must_use]
    pub(crate) fn root_name(&self) -> &str {
        match self {
            BmmType::Simple(s) => s,
            BmmType::Generic { root, .. } => root,
        }
    }
}

impl BmmSchema {
    /// Parse a BMM schema from its JSON serialization.
    ///
    /// # Errors
    /// Returns [`serde_json::Error`] if the JSON does not parse.
    pub(crate) fn parse_json(src: &str) -> Result<Self, serde_json::Error> {
        let doc: Value = serde_json::from_str(src)?;
        Ok(Self::from_value(&doc))
    }

    /// Build a schema from an already-parsed JSON value.
    #[must_use]
    pub(crate) fn from_value(doc: &Value) -> Self {
        let s = |k: &str| {
            doc.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };

        let includes = doc
            .get("includes")
            .and_then(Value::as_object)
            .map(|m| {
                m.values()
                    .filter_map(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let packages = doc
            .get("packages")
            .and_then(Value::as_object)
            .map(|m| m.values().map(parse_package).collect())
            .unwrap_or_default();

        // Classes come from two sibling sections: `class_definitions` (the
        // model's own classes) and `primitive_types` (foundation built-ins —
        // primitives, containers, `Interval`, ISO 8601, terminology). Both are
        // needed so ancestor flattening and type resolution work; the emitter
        // decides which to actually emit.
        let mut classes = BTreeMap::new();
        for section in ["primitive_types", "class_definitions"] {
            if let Some(defs) = doc.get(section).and_then(Value::as_object) {
                for node in defs.values() {
                    let class = parse_class(node);
                    classes.insert(class.name.clone(), class);
                }
            }
        }

        BmmSchema {
            schema_name: s("schema_name"),
            rm_release: s("rm_release"),
            bmm_version: s("bmm_version"),
            includes,
            packages,
            classes,
        }
    }

    /// Fold another schema into this one to form a crate's **dependency view**:
    /// the union of every BMM generation composing the crate, resolving a
    /// class-name collision **last-wins** (the later-declared generation's
    /// definition survives).
    ///
    /// openEHR ships some components as several vendored BMM files describing
    /// **two generations of the same meta-model** — LANG publishes the stable
    /// v2.x BMM (`LANG/docs/bmm/master01-preface.adoc` §History: "This document
    /// describes the stable v2.x form of BMM … the normative, tool-implemented
    /// version") beside the v3 development line
    /// (`LANG/docs/bmm3/master01-preface.adoc` §Previous Versions) — and both
    /// generations are emitted COMPLETELY, each from its own schema at its own
    /// source-package path.
    ///
    /// This view is therefore **never an emission input**: it exists only so a
    /// downstream crate and the crate prelude see exactly one type per Rust
    /// name (see [`crate::plan::composition::Composed::generations`] for the
    /// per-generation schemas the emitter actually renders). Because every
    /// generation is emitted in full, nothing a losing definition declares is
    /// dropped — the collision is resolved for *naming*, never for *shape*.
    #[must_use]
    pub(crate) fn dependency_view(mut self, other: &BmmSchema) -> Self {
        for (name, class) in &other.classes {
            self.classes.insert(name.clone(), class.clone());
        }
        self.packages.extend(other.packages.iter().cloned());
        self
    }

    /// A map from class name to its leaf package name (the innermost package
    /// that directly lists the class), for deriving the module layout.
    #[must_use]
    pub(crate) fn class_packages(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for p in &self.packages {
            collect_class_packages(p, &mut out);
        }
        out
    }
}

fn collect_class_packages(p: &BmmPackage, out: &mut BTreeMap<String, String>) {
    for c in &p.classes {
        out.insert(c.clone(), p.name.clone());
    }
    for sub in &p.packages {
        collect_class_packages(sub, out);
    }
}

fn str_vec(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_package(node: &Value) -> BmmPackage {
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let classes = str_vec(node.get("classes"));
    let packages = node
        .get("packages")
        .and_then(Value::as_object)
        .map(|m| m.values().map(parse_package).collect())
        .unwrap_or_default();
    BmmPackage {
        name,
        classes,
        packages,
    }
}

fn parse_class(node: &Value) -> BmmClass {
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let documentation = node
        .get("documentation")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ancestors = str_vec(node.get("ancestors"));
    let is_abstract = node
        .get("is_abstract")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let generic_params = node
        .get("generic_parameter_defs")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .map(|n| BmmGenericParam {
                    name: n
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    conforms_to: n
                        .get("conforms_to_type")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();

    // Properties: preserve declaration order. serde_json with `preserve_order`
    // keeps object key order, so the map iterates in file order.
    let properties = node
        .get("properties")
        .and_then(Value::as_object)
        .map(|m| m.values().map(parse_property).collect())
        .unwrap_or_default();

    let enumeration = parse_enumeration(node);
    let constants = parse_constants(node);
    let invariants = parse_invariants(node);
    let functions = parse_function_names(node);

    BmmClass {
        name,
        documentation,
        ancestors,
        is_abstract,
        generic_params,
        properties,
        enumeration,
        constants,
        functions,
        invariants,
    }
}

/// Parse the class node's `functions` object into its declaration-ordered name
/// list (`serde_json`'s `preserve_order` keeps file order). Absent → empty.
fn parse_function_names(node: &Value) -> Vec<String> {
    node.get("functions")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// Parse the class's `constants` object (name → `{type, value, documentation}`)
/// into declaration-ordered [`BmmConstant`]s (`serde_json`'s `preserve_order`
/// keeps file order). Absent → empty.
fn parse_constants(node: &Value) -> Vec<BmmConstant> {
    node.get("constants")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .map(|c| BmmConstant {
                    name: c
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    documentation: c
                        .get("documentation")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    type_name: c
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("String")
                        .to_string(),
                    value: c.get("value").cloned().unwrap_or(Value::Null),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the class's `invariants` object (name → assertion-expression string)
/// verbatim into a deterministic map. Absent → empty.
fn parse_invariants(node: &Value) -> BTreeMap<String, String> {
    node.get("invariants")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the enumeration facet of a class node, if its `_type` marks it a
/// `P_BMM_ENUMERATION_INTEGER` / `P_BMM_ENUMERATION_STRING` (both carry
/// `item_names` + optional `item_values`; the underlying type distinguishes them).
fn parse_enumeration(node: &Value) -> Option<BmmEnumeration> {
    let ptype = node.get("_type").and_then(Value::as_str).unwrap_or("");
    let underlying_type = match ptype {
        "P_BMM_ENUMERATION_INTEGER" => "INTEGER",
        "P_BMM_ENUMERATION_STRING" => "STRING",
        _ => return None,
    }
    .to_string();
    let item_names = str_vec(node.get("item_names"));
    // `item_values` is `List<Any>`; each element is a JSON integer (INTEGER
    // enum) or JSON string (STRING enum). Missing → `None` (default applied
    // downstream per `BMM_ENUMERATION`).
    let item_values = node
        .get("item_values")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(parse_enum_value).collect());
    Some(BmmEnumeration {
        underlying_type,
        item_names,
        item_values,
    })
}

/// One `item_values` element: a JSON integer → [`BmmEnumValue::Int`], anything
/// else (a JSON string) → [`BmmEnumValue::Str`] of its string form.
fn parse_enum_value(v: &Value) -> BmmEnumValue {
    v.as_i64().map_or_else(
        || BmmEnumValue::Str(v.as_str().unwrap_or_default().to_string()),
        BmmEnumValue::Int,
    )
}

fn parse_property(node: &Value) -> BmmProperty {
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let documentation = node
        .get("documentation")
        .and_then(Value::as_str)
        .map(str::to_string);
    let is_mandatory = node
        .get("is_mandatory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // The `default` facet, verbatim. The JSON export stringifies every ODIN
    // literal (`<False>` → `"False"`, `<"Boolean">` → `"\"Boolean\""`), but a
    // schema that writes a real JSON scalar is read too, so the value is
    // normalized to its text rather than required to be a string.
    let default = node.get("default").and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(_) | Value::Number(_) => Some(v.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    });
    let ptype = node.get("_type").and_then(Value::as_str).unwrap_or("");

    let kind = if ptype == "P_BMM_CONTAINER_PROPERTY" {
        let td = node.get("type_def");
        let container_type = td
            .and_then(|t| t.get("container_type"))
            .and_then(Value::as_str)
            .unwrap_or("List")
            .to_string();
        let item = td.map_or(BmmType::Simple("Any".into()), parse_container_item);
        let cardinality = node.get("cardinality").map(parse_cardinality);
        BmmPropKind::Container {
            container_type,
            item,
            cardinality,
        }
    } else if let Some(td) = node.get("type_def") {
        // P_BMM_GENERIC_PROPERTY
        BmmPropKind::Single(parse_type_def(td))
    } else {
        // P_BMM_SINGLE_PROPERTY / P_BMM_SINGLE_PROPERTY_OPEN
        let t = node
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("Any")
            .to_string();
        BmmPropKind::Single(BmmType::Simple(t))
    };

    BmmProperty {
        name,
        documentation,
        is_mandatory,
        default,
        kind,
    }
}

fn parse_cardinality(v: &Value) -> BmmCardinality {
    let lower = v
        .get("lower")
        .and_then(Value::as_u64)
        .and_then(|u| u32::try_from(u).ok())
        .unwrap_or(0);
    let upper = if v
        .get("upper_unbounded")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        None
    } else {
        v.get("upper")
            .and_then(Value::as_u64)
            .and_then(|u| u32::try_from(u).ok())
    };
    BmmCardinality { lower, upper }
}

/// The element type inside a container's `type_def`: either a simple `type`
/// (`List<LINK>`) or a nested generic `type_def` (`List<REFERENCE_RANGE<...>>`).
fn parse_container_item(td: &Value) -> BmmType {
    if let Some(inner) = td.get("type_def") {
        parse_type_def(inner)
    } else if let Some(t) = td.get("type").and_then(Value::as_str) {
        BmmType::Simple(t.to_string())
    } else {
        BmmType::Simple("Any".to_string())
    }
}

/// A `type_def` with `root_type` → a generic type; otherwise a simple `type`.
/// The generic arguments come either as `generic_parameters` (a list, whose
/// entries may be bare type names *or* nested `type_def`s) or as
/// `generic_parameter_defs` (an ordered map of named params, used for
/// `Hash<K,V>`; `serde_json`'s `preserve_order` keeps K before V).
fn parse_type_def(td: &Value) -> BmmType {
    if let Some(root) = td.get("root_type").and_then(Value::as_str) {
        let params: Vec<BmmType> =
            if let Some(arr) = td.get("generic_parameters").and_then(Value::as_array) {
                arr.iter().map(parse_generic_param).collect()
            } else if let Some(obj) = td.get("generic_parameter_defs").and_then(Value::as_object) {
                obj.values().map(parse_type_def).collect()
            } else {
                Vec::new()
            };
        BmmType::Generic {
            root: root.to_string(),
            params,
        }
    } else if let Some(t) = td.get("type").and_then(Value::as_str) {
        BmmType::Simple(t.to_string())
    } else {
        BmmType::Simple("Any".to_string())
    }
}

/// A single `generic_parameters` entry: a bare type name (`"String"`) or a
/// nested `type_def` (`{ "root_type": "Interval", "generic_parameters": [...] }`).
fn parse_generic_param(x: &Value) -> BmmType {
    match x.as_str() {
        Some(s) => BmmType::Simple(s.to_string()),
        None => parse_type_def(x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNIPPET: &str = r#"{
        "schema_name": "rm",
        "rm_release": "1.1.0",
        "bmm_version": "2.4",
        "includes": { "openehr_base_1.2.0": { "id": "openehr_base_1.2.0" } },
        "packages": {
            "org.openehr.rm.data_types": {
                "name": "org.openehr.rm.data_types",
                "packages": {
                    "quantity": { "name": "quantity", "classes": ["DV_QUANTITY", "DV_INTERVAL"] }
                }
            }
        },
        "class_definitions": {
            "DV_INTERVAL": {
                "name": "DV_INTERVAL",
                "ancestors": ["DATA_VALUE", "Interval"],
                "generic_parameter_defs": { "T": { "name": "T", "conforms_to_type": "DV_ORDERED" } }
            },
            "DATA_VALUE": { "name": "DATA_VALUE", "is_abstract": true },
            "DV_QUANTITY": {
                "name": "DV_QUANTITY",
                "documentation": "A quantity.",
                "ancestors": ["DV_AMOUNT"],
                "properties": {
                    "magnitude": { "_type": "P_BMM_SINGLE_PROPERTY", "name": "magnitude", "is_mandatory": true, "type": "Real" },
                    "precision": { "_type": "P_BMM_SINGLE_PROPERTY", "name": "precision", "type": "Integer" },
                    "normal_range": { "_type": "P_BMM_GENERIC_PROPERTY", "name": "normal_range", "type_def": { "root_type": "DV_INTERVAL", "generic_parameters": ["DV_QUANTITY"] } },
                    "other_reference_ranges": { "_type": "P_BMM_CONTAINER_PROPERTY", "name": "other_reference_ranges", "cardinality": { "lower": 0, "upper_unbounded": true }, "type_def": { "container_type": "List", "type_def": { "_type": "P_BMM_GENERIC_TYPE", "root_type": "REFERENCE_RANGE", "generic_parameters": ["DV_QUANTITY"] } } },
                    "links": { "_type": "P_BMM_CONTAINER_PROPERTY", "name": "links", "type_def": { "container_type": "List", "type": "LINK" } }
                }
            }
        }
    }"#;

    #[test]
    fn loads_schema_header_and_includes() {
        let s = BmmSchema::parse_json(SNIPPET).unwrap();
        assert_eq!(s.schema_name, "rm");
        assert_eq!(s.rm_release, "1.1.0");
        assert_eq!(s.includes, vec!["openehr_base_1.2.0"]);
        assert_eq!(s.classes.len(), 3);
    }

    #[test]
    fn packages_map_classes() {
        let s = BmmSchema::parse_json(SNIPPET).unwrap();
        let cp = s.class_packages();
        assert_eq!(cp.get("DV_QUANTITY").map(String::as_str), Some("quantity"));
        assert_eq!(cp.get("DV_INTERVAL").map(String::as_str), Some("quantity"));
    }

    #[test]
    fn generic_class_params_and_abstract() {
        let s = BmmSchema::parse_json(SNIPPET).unwrap();
        let iv = &s.classes["DV_INTERVAL"];
        assert_eq!(iv.generic_params.len(), 1);
        assert_eq!(iv.generic_params[0].name, "T");
        assert_eq!(
            iv.generic_params[0].conforms_to.as_deref(),
            Some("DV_ORDERED")
        );
        assert!(s.classes["DATA_VALUE"].is_abstract);
        assert!(!iv.is_abstract);
    }

    #[test]
    fn property_kinds_and_types() {
        let s = BmmSchema::parse_json(SNIPPET).unwrap();
        let q = &s.classes["DV_QUANTITY"];
        assert_eq!(q.ancestors, vec!["DV_AMOUNT"]);
        assert_eq!(q.documentation.as_deref(), Some("A quantity."));

        let by = |n: &str| q.properties.iter().find(|p| p.name == n).unwrap();

        let mag = by("magnitude");
        assert!(mag.is_mandatory);
        assert!(matches!(&mag.kind, BmmPropKind::Single(BmmType::Simple(t)) if t == "Real"));

        assert!(!by("precision").is_mandatory);

        let nr = by("normal_range");
        match &nr.kind {
            BmmPropKind::Single(BmmType::Generic { root, params }) => {
                assert_eq!(root, "DV_INTERVAL");
                assert_eq!(params, &[BmmType::Simple("DV_QUANTITY".into())]);
            }
            other => panic!("expected generic single, got {other:?}"),
        }

        let orr = by("other_reference_ranges");
        match &orr.kind {
            BmmPropKind::Container {
                container_type,
                item,
                cardinality,
            } => {
                assert_eq!(container_type, "List");
                assert_eq!(
                    cardinality,
                    &Some(BmmCardinality {
                        lower: 0,
                        upper: None
                    })
                );
                assert!(matches!(item, BmmType::Generic { root, .. } if root == "REFERENCE_RANGE"));
            }
            other @ BmmPropKind::Single(_) => panic!("expected container, got {other:?}"),
        }

        match &by("links").kind {
            BmmPropKind::Container { item, .. } => {
                assert_eq!(item, &BmmType::Simple("LINK".into()));
            }
            other @ BmmPropKind::Single(_) => panic!("expected container, got {other:?}"),
        }
    }

    #[test]
    fn parses_enumeration_classes() {
        const ENUM_SNIPPET: &str = r#"{
            "schema_name": "base",
            "class_definitions": {
                "PROPORTION_KIND": {
                    "_type": "P_BMM_ENUMERATION_INTEGER",
                    "name": "PROPORTION_KIND",
                    "ancestors": ["Integer"],
                    "item_names": ["pk_ratio", "pk_unitary"],
                    "item_values": [0, 1]
                },
                "VALIDITY_KIND": {
                    "_type": "P_BMM_ENUMERATION_STRING",
                    "name": "VALIDITY_KIND",
                    "ancestors": ["String"],
                    "item_names": ["mandatory", "optional"]
                },
                "DV_QUANTITY": { "name": "DV_QUANTITY" }
            }
        }"#;
        let s = BmmSchema::parse_json(ENUM_SNIPPET).unwrap();

        let pk = s.classes["PROPORTION_KIND"].enumeration.as_ref().unwrap();
        assert_eq!(pk.underlying_type, "INTEGER");
        assert_eq!(pk.item_names, vec!["pk_ratio", "pk_unitary"]);
        assert_eq!(
            pk.item_values,
            Some(vec![BmmEnumValue::Int(0), BmmEnumValue::Int(1)])
        );

        let vk = s.classes["VALIDITY_KIND"].enumeration.as_ref().unwrap();
        assert_eq!(vk.underlying_type, "STRING");
        assert_eq!(vk.item_names, vec!["mandatory", "optional"]);
        assert_eq!(vk.item_values, None);

        // A non-enumeration class carries no enumeration facet.
        assert!(s.classes["DV_QUANTITY"].enumeration.is_none());
    }

    #[test]
    fn parses_constants_and_invariants() {
        const SNIPPET: &str = r#"{
            "schema_name": "rm",
            "class_definitions": {
                "OPENEHR_CODE_SET_IDENTIFIERS": {
                    "name": "OPENEHR_CODE_SET_IDENTIFIERS",
                    "constants": {
                        "Code_set_id_languages": { "name": "Code_set_id_languages", "type": "String", "value": "\"languages\"" },
                        "Max_days_in_year": { "name": "Max_days_in_year", "type": "Integer", "value": 366 }
                    }
                },
                "DV_IDENTIFIER": {
                    "name": "DV_IDENTIFIER",
                    "invariants": {
                        "Id_valid": "not id.is_empty",
                        "Size_valid": "size >= 0"
                    }
                }
            }
        }"#;
        let s = BmmSchema::parse_json(SNIPPET).unwrap();

        let cs = &s.classes["OPENEHR_CODE_SET_IDENTIFIERS"];
        assert_eq!(cs.constants.len(), 2);
        let lang = cs
            .constants
            .iter()
            .find(|c| c.name == "Code_set_id_languages")
            .unwrap();
        assert_eq!(lang.type_name, "String");
        assert_eq!(lang.value, Value::String("\"languages\"".into()));
        let days = cs
            .constants
            .iter()
            .find(|c| c.name == "Max_days_in_year")
            .unwrap();
        assert_eq!(days.type_name, "Integer");
        assert_eq!(days.value, Value::from(366));

        let dv = &s.classes["DV_IDENTIFIER"];
        assert_eq!(dv.invariants.len(), 2);
        assert_eq!(dv.invariants["Id_valid"], "not id.is_empty");
        assert_eq!(dv.invariants["Size_valid"], "size >= 0");

        // A class with neither section carries empty collections.
        assert!(
            s.classes["OPENEHR_CODE_SET_IDENTIFIERS"]
                .invariants
                .is_empty()
        );
        assert!(s.classes["DV_IDENTIFIER"].constants.is_empty());
    }

    #[test]
    fn declaration_order_preserved() {
        let s = BmmSchema::parse_json(SNIPPET).unwrap();
        let names: Vec<&str> = s.classes["DV_QUANTITY"]
            .properties
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "magnitude",
                "precision",
                "normal_range",
                "other_reference_ranges",
                "links"
            ]
        );
    }
}
