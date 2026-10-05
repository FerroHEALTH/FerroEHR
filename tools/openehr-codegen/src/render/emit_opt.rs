// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The XSD-driven constraint-model emitter: generates a typed Rust model +
//! canonical-XML `ToXml`/`FromXml` impls for each vendored ARCHETYPE-family XSD
//! closure. Three targets, one pipeline (see [`ModelTarget`]):
//!
//! | target | subcommand | closure | root |
//! |---|---|---|---|
//! | `opt14` | `emit-opt` | `Template.xsd` + includes | `<template>` = `OPERATIONAL_TEMPLATE` |
//! | `aom2` | `emit-aom2` | `P_Archetype.xsd` + includes | `<archetype>` = `P_AUTHORED_ARCHETYPE` |
//! | `aom2_model` | `emit-aom2` | `Archetype.xsd` + includes | `<archetype>` = `AUTHORED_ARCHETYPE` |
//!
//! Unlike `emit-xml`, which drives off the BMM model, this emitter builds its
//! [`XmlType`]s directly from the XSD closure. A complexType that
//! `openehr-base`/`openehr-rm` already export resolves to that crate's prelude;
//! every other complexType (the AOM/OPT constraint model, the OPT envelope, the
//! `IntervalOf*` helpers) is generated.
//!
//! A named `xs:simpleType` whose restriction declares `xs:enumeration` facets is
//! a closed value space, so it emits a fieldless enum over the XSD's own
//! vocabulary and refuses text outside the set at parse. Abstract types used as
//! polymorphic slots become untagged enums dispatching on `xsi:type`, whose
//! codecs come from [`crate::render::emit_xml::emit_to_xml`] /
//! [`crate::render::emit_xml::emit_from_xml`].
//!
//! NOTE: `opt14` re-generates the AOM 1.4 `C_*` constraint tree that the
//! BMM-generated `openehr-am::v1_4` also carries, because the OPT-XML wire shape
//! (`Template.xsd` + `OpenehrProfile.xsd`) and the AOM 1.4 BMM are not
//! structurally reconcilable: different domain-type sets (`C_DV_STATE` and
//! `C_CODE_REFERENCE` have no `v1_4` counterpart), differently typed
//! `assumed_value`, the XSD `IntervalOf*` shape against
//! `openehr_base::Interval<T>`, and OPT-envelope-only types.
//!
//! The two models are generated independently, so
//! `crates/openehr-its/tests/opt14_v1_4_divergence.rs` is a compile-time
//! inventory sentinel that fails when either gains or loses a constraint type.

use crate::load::xsd::{XsdEnumFacet, XsdModel, XsdSimpleType};
use crate::plan::{XmlField, XmlType, XmlVariant};
use crate::render::emit_xml::{emit_from_xml, emit_to_xml};
use crate::render::naming;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// The Rust module path the OPT model's generated types live at.
///
/// The prelude is a MODEL PARAMETER, not a constant of this emitter: the same
/// XSD-driven pipeline emits several `openehr-its` submodules (`opt14`, `aom2`,
/// `aom2_model`), and every generated impl and defaulted literal must name the
/// module it is being emitted INTO. A hardcoded path here silently emits `aom2`
/// impls that reference `crate::opt14` types.
///
/// Each path ends at the defining `types` module rather than at the parent: the
/// parent re-exported these names with a glob until the zero-re-exports rule
/// removed it, and a generated impl must name where a type is DEFINED.
pub(crate) const OPT_PRELUDE: &str = "crate::opt14::types";

/// The Rust module path the AOM2 persistent-form model's generated types live at.
pub(crate) const AOM2_PRELUDE: &str = "crate::aom2::types";

/// The Rust module path the AOM2 model-form model's generated types live at.
pub(crate) const AOM2_MODEL_PRELUDE: &str = "crate::aom2_model::types";

/// The `opt14` emission target (OPT 1.4 operational templates).
pub(crate) static OPT_TARGET: ModelTarget = ModelTarget {
    generator: "emit-opt",
    prelude: OPT_PRELUDE,
    types_subject: "OPT 1.4 operational templates",
    impls_subject: "OPT 1.4",
    type_label: "AOM/OPT",
    field_label: "OPT",
};

/// The `aom2` emission target (the AOM2 persistent form, `P_Archetype.xsd`).
pub(crate) static AOM2_TARGET: ModelTarget = ModelTarget {
    generator: "emit-aom2",
    prelude: AOM2_PRELUDE,
    types_subject: "AOM2 persistent-form archetypes",
    impls_subject: "AOM2 persistent-form archetype",
    type_label: "AOM2 persistent-form",
    field_label: "AOM2 persistent-form",
};

/// The `aom2_model` emission target (the AOM2 model form, `Archetype.xsd`).
pub(crate) static AOM2_MODEL_TARGET: ModelTarget = ModelTarget {
    generator: "emit-aom2",
    prelude: AOM2_MODEL_PRELUDE,
    types_subject: "AOM2 model-form archetypes",
    impls_subject: "AOM2 model-form archetype",
    type_label: "AOM2 model-form",
    field_label: "AOM2 model-form",
};

/// Resource-metadata types that must be generated from the **OPT XSD** shape
/// rather than resolved to the `openehr-base`/`openehr-rm` impls: the BMM
/// (RM 1.2.0 / BASE 1.3.0) and the OPT XSD (Release 1.0.2) have diverged on
/// their optionality (e.g. `RESOURCE_DESCRIPTION.parent_resource` is mandatory
/// in the BMM but `minOccurs="0"` in the XSD — and the corpus omits it), so the
/// BMM-driven impls reject valid OPT `<description>` blocks. The OPT XSD is the
/// authority for OPT documents, so these are emitted fresh into `opt14`.
const FORCE_GENERATE: &[&str] = &[
    "AUTHORED_RESOURCE",
    "RESOURCE_DESCRIPTION",
    "RESOURCE_DESCRIPTION_ITEM",
    "TRANSLATION_DETAILS",
];

/// `StringDictionaryItem` is an XSD `simpleContent` helper (`<x id="k">v</x>`);
/// it is never generated as a struct — its repeated-element usage is emitted as
/// an order-preserving `IndexMap<String, String>` field.
///
/// NOTE: the XSD models this as an ordered `sequence`, so
/// `IndexMap` (insertion order = document order, keyed `.get()` for the
/// `WebTemplate` consumer) is used rather than the alphabetical `BTreeMap` the RM
/// `emit-xml` path uses — a `ToXml` re-serialization then preserves element
/// order. A genuinely duplicate `id` (not a conformant-OPT case) is still
/// collapsed last-wins by the map. The `OrderedDict` field target
/// (vs `emit-xml`'s `Hash`) selects this shape without affecting the RM codec.
const STRING_DICT_ITEM: &str = "StringDictionaryItem";

/// OPT-envelope sections carried as the verbatim XML subtree
/// (`crate::xml::runtime::XmlAny`) rather than a generated struct.
///
/// `T_VIEW` (the `<view>` presentation block) holds an **anonymous inline
/// complexType** (`T_VIEW.constraints` → nested `items` with an `id` attribute
/// and an `anySimpleType` value) that the XSD reader cannot flatten into a named
/// type; it carries only presentation hints (`pass_through` markers), never the
/// operational definition, so it is kept as read instead of modelled.
///
/// `T_CONSTRAINT` (the top-level `<constraints>` block) is not opaque: it is a
/// named `T_ATTRIBUTE` → `T_COMPLEX_OBJECT` tree carrying node `default_value`
/// overlays, generated like any other type. Its
/// differential children may omit `rm_type_name`/`occurrences`/`node_id`
/// (they carry only `default_value` + `differential_path`); [`lenient_default`]
/// fills those, so the corpus parses cleanly and the `default_value`s are
/// preserved on the model for FLAT default-value population to consume.
const OPAQUE_TYPES: &[&str] = &["T_VIEW"];

/// A default expression for an XSD-mandatory field that real-world OPT exports
/// (Ocean/tooling) nevertheless omit — so `from_xml` fills it instead of
/// erroring. `node_id`/`purpose` fall back to empty; `occurrences`/`existence`
/// (both `IntervalOfInteger`) to the conservative `0..1` (present, optional
/// single) so a missing multiplicity never over-constrains. The expression is
/// emitted in the `opt14` impl context (prelude `crate::opt14`).
///
/// NOTE: a defaulted `occurrences`/`existence` of `0..1` is a
/// *fallback for non-conformant input only* — conformant OPTs always carry the
/// element. It is a guess (a node that should be `1..1` is silently made
/// optional-single), so any downstream multiplicity check (composition validation) must
/// resolve multiplicity from the `definition`/archetype, never trust a defaulted
/// `0..1` from this reader.
///
/// The match is on the field name **and its declared XSD type**, never the name
/// alone: `existence`/`occurrences` are `IntervalOfInteger` elements in the OPT
/// closure but `MultiplicityInterval` elements in the AOM2 model closure, and a
/// name-only match would emit an `Intervalofinteger` literal into a
/// `Multiplicityinterval` field. Keeping the type exact also keeps the concession
/// confined to the shapes it was adjudicated for.
fn lenient_default(field_name: &str, type_name: &str, prelude: &str) -> Option<String> {
    match (field_name, type_name) {
        // `rm_type_name` joins the lenient set for differential
        // `T_COMPLEX_OBJECT` children in real exports: `<constraints>` overlay
        // nodes may carry only `default_value` + `differential_path`
        // (e.g. the corpus `non_unique_aql_paths.opt`).
        ("node_id" | "purpose" | "rm_type_name", "xs:string") => Some("String::new()".to_owned()),
        ("occurrences" | "existence", "IntervalOfInteger") => Some(format!(
            "{prelude}::Intervalofinteger {{ \
             lower_included: Some(true), upper_included: Some(true), \
             lower_unbounded: false, upper_unbounded: false, \
             lower: Some(0), upper: Some(1) }}"
        )),
        _ => None,
    }
}

/// The Rust name of the emitted error a refusing facet reader returns.
///
/// One per generated module: the module's types are self-contained (the
/// zero-re-exports rule means nothing is shared across them), so each closure
/// carries its own copy.
const FACET_ERROR: &str = "UnknownFacetValue";

/// Whether a facet reader over this `xs:restriction` base compares the TRIMMED
/// element text against the declared facet values.
///
/// XSD fixes `whiteSpace` to `collapse` on every built-in datatype except
/// `xs:string`, which alone keeps `preserve`
/// (<https://www.w3.org/TR/xmlschema11-2/#rf-whiteSpace>). The vendored
/// archetype schemas restrict `xs:integer` throughout, so their facet values are
/// compared after trimming — exactly as every other numeric leaf already reads.
fn facet_base_collapses_whitespace(base: &str) -> bool {
    !matches!(base, "xs:string" | "xsd:string")
}

/// The Rust variant identifier for one `xs:enumeration` facet.
///
/// The openEHR schemas annotate every enumeration facet with an `id`
/// (`<xs:enumeration value="2001" id="equal"/>`), which is the schema's own
/// name for the value; a facet without one is named from its value, so a
/// re-vendoring that drops the annotation still emits.
fn facet_variant_ident(facet: &XsdEnumFacet) -> String {
    let source = facet.id.clone().unwrap_or_else(|| {
        let sanitized: String = facet
            .value
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if sanitized.starts_with(|c: char| c.is_ascii_alphabetic()) {
            sanitized
        } else {
            format!("v_{sanitized}")
        }
    });
    naming::type_name(&source)
}

/// A Rust string literal (including the quotes) carrying `s` verbatim.
fn rust_str_literal(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// How a referenced XSD type name resolves for a generated field.
enum Resolved {
    /// A Rust primitive (`String`/`bool`/`i32`/`i64`/`f64`).
    Primitive(&'static str),
    /// `xs:anyType`/`xs:anySimpleType`/anonymous-inline → the verbatim XML
    /// subtree carrier `crate::xml::runtime::XmlAny` (attributes, text and children in
    /// document order, re-emitted as read).
    ///
    /// NOTE: `AM aom14 §EXPR_LEAF Class` types `item: Any`, so the payload
    /// domain of the schema's open slots is open too and no closed set of
    /// generated types can be dispatched to.
    Any,
    /// A repeated `StringDictionaryItem` element group → order-preserving
    /// `IndexMap<String,String>` (target `OrderedDict`).
    Hash,
    /// A type emitted by `openehr-base` — the full generation-module type path.
    Base(String),
    /// A type emitted by `openehr-rm` — the full generation-module type path.
    Rm(String),
    /// A generated `opt14` type; the flag is `true` when it is a generated enum
    /// (a polymorphic slot), which single-valued fields must `Box` to stay sized.
    Gen(String, bool),
    /// A named `xs:simpleType` whose `xs:restriction` declares `xs:enumeration`
    /// facets → the generated fieldless enum over that closed value space.
    Facet(String),
}

/// Everything about the emission TARGET that varies between the XSD-driven
/// closures this emitter serves (`opt14`, `aom2`, `aom2_model`).
///
/// All of it is a model parameter rather than a constant for the same reason the
/// prelude is: three modules come out of one emitter, and a hardcoded value
/// silently stamps one module's identity onto another's files — an `aom2_model`
/// impl referencing `crate::opt14` types, or an `emit-aom2` output whose banner
/// tells the reader to re-run `emit-opt`.
pub(crate) struct ModelTarget {
    /// The `openehr-codegen` subcommand that regenerates the target, named in
    /// every `// @generated` banner.
    pub(crate) generator: &'static str,
    /// The `openehr-its` module path the emitted types live at
    /// (e.g. [`OPT_PRELUDE`]).
    pub(crate) prelude: &'static str,
    /// What `types.rs` is a model OF ("OPT 1.4 operational templates").
    pub(crate) types_subject: &'static str,
    /// The model name used in the `impls.rs` title ("OPT 1.4").
    pub(crate) impls_subject: &'static str,
    /// The spec-family label substituted into a per-type doc comment: "AOM/OPT"
    /// yields `openEHR AOM/OPT <spec name>.`
    pub(crate) type_label: &'static str,
    /// The spec-family label substituted into a per-field doc comment: "OPT"
    /// yields `… attribute/element of the OPT <spec name> XSD type.`
    pub(crate) field_label: &'static str,
}

/// The generate/resolve model for one XSD closure.
pub(crate) struct OptModel<'a> {
    xsd: &'a XsdModel,
    /// Spec class name → full generation-module path, per dependency crate
    /// (the openehr-base / openehr-rm generation the shared XSD types resolve
    /// to — full defining-module paths, never a prelude).
    base_paths: &'a BTreeMap<String, String>,
    rm_paths: &'a BTreeMap<String, String>,
    /// Concrete + abstract complexTypes we generate (spec names).
    generate: BTreeSet<String>,
    /// The subset of `generate` that are abstract polymorphic slots → enums.
    enum_specs: BTreeSet<String>,
    /// The emission target's identity (module path, banners, doc labels).
    target: &'static ModelTarget,
}

/// A generated field: the `emit_xml` [`XmlField`] plus the Rust type for its
/// struct-field declaration (which `emit_xml`'s impls then infer against).
struct OptField {
    xml: XmlField,
    decl_type: String,
}

impl<'a> OptModel<'a> {
    /// Build the model from the parsed XSD closure and the base/rm
    /// spec-name → module-path maps.
    #[must_use]
    pub(crate) fn new(
        xsd: &'a XsdModel,
        base_paths: &'a BTreeMap<String, String>,
        rm_paths: &'a BTreeMap<String, String>,
        target: &'static ModelTarget,
    ) -> Self {
        let generate: BTreeSet<String> = xsd
            .types
            .keys()
            .filter(|n| {
                n.as_str() != STRING_DICT_ITEM
                    && !OPAQUE_TYPES.contains(&n.as_str())
                    && (FORCE_GENERATE.contains(&n.as_str())
                        || (!base_paths.contains_key(*n) && !rm_paths.contains_key(*n)))
            })
            .cloned()
            .collect();
        let enum_specs: BTreeSet<String> = generate
            .iter()
            .filter(|n| {
                xsd.types.get(*n).is_some_and(|t| t.is_abstract) && !xsd.descendants(n).is_empty()
            })
            .cloned()
            .collect();
        Self {
            xsd,
            base_paths,
            rm_paths,
            generate,
            enum_specs,
            target,
        }
    }

    /// Resolve an XSD type name (element/attribute `type`) to a Rust binding.
    fn resolve(&self, type_name: &str) -> Resolved {
        if type_name.is_empty() {
            return Resolved::Any; // anonymous inline complexType
        }
        if type_name == STRING_DICT_ITEM {
            return Resolved::Hash;
        }
        if OPAQUE_TYPES.contains(&type_name) {
            return Resolved::Any; // the differential/presentation envelope, carried verbatim
        }
        // XSD-namespace primitive (`xs:` / `xsd:`).
        if let Some(local) = type_name
            .strip_prefix("xs:")
            .or_else(|| type_name.strip_prefix("xsd:"))
        {
            return Self::xs_primitive(local);
        }
        if self.xsd.types.contains_key(type_name) {
            let rust = naming::type_name(type_name);
            // A generated type wins over the base/rm resolution (the FORCE_GENERATE
            // resource types live in both sets — the generated one is authoritative).
            if self.generate.contains(type_name) {
                return Resolved::Gen(rust, self.enum_specs.contains(type_name));
            }
            if let Some(path) = self.base_paths.get(type_name) {
                return Resolved::Base(format!("{path}::{rust}"));
            }
            if let Some(path) = self.rm_paths.get(type_name) {
                return Resolved::Rm(format!("{path}::{rust}"));
            }
        }
        // A named `xs:simpleType` restricting a base with `xs:enumeration`
        // facets declares a CLOSED value space (`OPERATOR_KIND`,
        // `VALIDITY_KIND`, `PROPORTION_KIND`), so the slot is that enum.
        if self
            .xsd
            .simple_types
            .get(type_name)
            .is_some_and(|t| !t.enumerations.is_empty())
        {
            return Resolved::Facet(naming::type_name(type_name));
        }
        // Every other named `xs:simpleType` restricts a text space with lexical
        // facets only (`Iso8601Date`, `DateConstraintPattern`, `matchString`):
        // still text on the wire.
        Resolved::Primitive("String")
    }

    /// Map an `xs:`-local primitive to a Rust type.
    fn xs_primitive(local: &str) -> Resolved {
        match local {
            "anyType" | "anySimpleType" => Resolved::Any,
            "boolean" => Resolved::Primitive("bool"),
            "int" | "integer" | "nonNegativeInteger" | "positiveInteger" | "short" => {
                Resolved::Primitive("i32")
            }
            "long" => Resolved::Primitive("i64"),
            "decimal" | "double" | "float" => Resolved::Primitive("f64"),
            // string, token, normalizedString, anyURI, base64Binary, dateTime, …
            _ => Resolved::Primitive("String"),
        }
    }

    /// The unwrapped Rust type text + the declared-slot spec name (for the
    /// `xsi:type`-suppression `declared` argument).
    fn base_decl(res: &Resolved, raw_spec: &str) -> (String, String) {
        match res {
            Resolved::Primitive(p) => ((*p).to_string(), String::new()),
            // A simple type is never a complexType, so it never carries an
            // `xsi:type`: the declared slot is empty, as for a primitive.
            Resolved::Facet(n) => (n.clone(), String::new()),
            Resolved::Any => ("crate::xml::runtime::XmlAny".to_string(), String::new()),
            Resolved::Hash => (
                "indexmap::IndexMap<String, String>".to_string(),
                String::new(),
            ),
            Resolved::Base(n) | Resolved::Rm(n) | Resolved::Gen(n, _) => {
                (n.clone(), raw_spec.to_string())
            }
        }
    }

    /// The flattened fields (attributes then elements, ancestor-first) of a
    /// concrete generated type.
    fn fields(&self, spec: &str) -> Vec<OptField> {
        let (attrs, elems) = self.xsd.flattened(spec);
        let mut out = Vec::new();
        for a in &attrs {
            let rust_name = naming::field_ident(&a.name);
            let decl_type = if a.required {
                "String".to_string()
            } else {
                "Option<String>".to_string()
            };
            out.push(OptField {
                xml: XmlField {
                    wire_name: a.name.clone(),
                    rust_name,
                    optional: !a.required,
                    multiple: false,
                    target: String::new(),
                    map_value: None,
                    default: None,
                    nonempty: false,
                },
                decl_type,
            });
        }
        for e in &elems {
            out.push(self.element_field(e));
        }
        out
    }

    /// One element's generated field: its declared Rust type and its XML view.
    ///
    /// A single-valued reference to a generated enum is boxed — those slots
    /// (`EXPR_ITEM`, `C_PRIMITIVE`, `STATE`) are recursive. The container
    /// wrapping is [`Self::declared_type`]'s and the wire fallback is
    /// [`Self::scalar_default`]'s; an `xs:anyType` hash takes the separate
    /// [`Self::hash_field`] shape.
    fn element_field(&self, e: &crate::load::xsd::XsdElem) -> OptField {
        let res = self.resolve(&e.type_name);
        let rust_name = naming::field_ident(&e.name);
        let (base, target) = Self::base_decl(&res, &e.type_name);

        if matches!(res, Resolved::Hash) {
            return Self::hash_field(e, rust_name, base);
        }

        let inner = if !e.multiple && matches!(res, Resolved::Gen(_, true)) {
            format!("Box<{base}>")
        } else {
            base
        };
        OptField {
            xml: XmlField {
                wire_name: e.name.clone(),
                rust_name,
                optional: e.optional && !e.multiple,
                multiple: e.multiple,
                target,
                map_value: None,
                default: self.scalar_default(e, &res),
                nonempty: false,
            },
            decl_type: Self::declared_type(e, inner),
        }
    }

    /// The generated field for an `xs:anyType` hash element: a string map,
    /// never boxed and never defaulted.
    fn hash_field(e: &crate::load::xsd::XsdElem, rust_name: String, base: String) -> OptField {
        let decl_type = if e.optional {
            format!("Option<{base}>")
        } else {
            base
        };
        OptField {
            xml: XmlField {
                wire_name: e.name.clone(),
                rust_name,
                optional: e.optional,
                multiple: false,
                target: "OrderedDict".to_string(),
                map_value: Some("String".to_string()),
                default: None,
                nonempty: false,
            },
            decl_type,
        }
    }

    /// Wraps an element's inner type in the container its XSD multiplicity and
    /// optionality call for.
    fn declared_type(e: &crate::load::xsd::XsdElem, inner: String) -> String {
        if e.multiple {
            format!("Vec<{inner}>")
        } else if e.optional {
            format!("Option<{inner}>")
        } else {
            inner
        }
    }

    /// The wire default for a mandatory scalar element, if it has one.
    ///
    /// An absent mandatory bool falls back to `false` (openEHR `Interval`
    /// boundedness flags), and some XSD-mandatory fields are omitted by
    /// real-world OPT exports (Ocean/tool laxity), so those are defaulted
    /// leniently — a wire adapter must ingest imperfect real OPTs. A
    /// multi-valued or optional element never defaults.
    fn scalar_default(&self, e: &crate::load::xsd::XsdElem, res: &Resolved) -> Option<String> {
        if e.optional || e.multiple {
            return None;
        }
        if matches!(res, Resolved::Primitive("bool")) {
            return Some("false".to_string());
        }
        lenient_default(&e.name, &e.type_name, self.target.prelude)
    }

    /// The closure's `xs:enumeration`-faceted simple types, name-ordered.
    ///
    /// These are the named simple types whose restriction fixes a closed value
    /// space; each emits a fieldless Rust enum over the XSD's own vocabulary.
    fn facet_types(&self) -> Vec<&'a XsdSimpleType> {
        self.xsd
            .simple_types
            .values()
            .filter(|t| !t.enumerations.is_empty())
            .collect()
    }

    /// Every generated struct's fields as `(wire name, declared Rust type)` —
    /// the exact pairs [`OptModel::emit_types`] writes, keyed by spec name.
    ///
    /// Exposed so the emitter invariants can assert what a slot is TYPED as
    /// rather than re-deriving it, or scraping the emitted text.
    #[must_use]
    pub(crate) fn declared_field_types(&self) -> BTreeMap<String, Vec<(String, String)>> {
        self.generate
            .iter()
            .filter(|spec| !self.enum_specs.contains(*spec))
            .map(|spec| {
                let fields = self
                    .fields(spec)
                    .into_iter()
                    .map(|f| (f.xml.wire_name, f.decl_type))
                    .collect();
                (spec.clone(), fields)
            })
            .collect()
    }

    /// Emit the fieldless enum for one `xs:enumeration`-faceted simple type,
    /// plus its wire accessors.
    ///
    /// The wire form is the facet `value` VERBATIM, so a document round-trips
    /// byte-identically; a value outside the declared set is refused, which is
    /// what an `xs:enumeration` facet means
    /// (<https://www.w3.org/TR/xmlschema11-2/#rf-enumeration>).
    fn emit_facet_enum(&self, b: &mut String, st: &XsdSimpleType) {
        let rust = naming::type_name(&st.name);
        let variants: Vec<(String, &XsdEnumFacet)> = st
            .enumerations
            .iter()
            .map(|f| (facet_variant_ident(f), f))
            .collect();
        let mut seen = BTreeSet::new();
        for (ident, facet) in &variants {
            assert!(
                seen.insert(ident.clone()),
                "xs:simpleType {:?}: two xs:enumeration facets name the same Rust variant \
                 {ident:?} (facet value {:?}) — the schema's own facet ids no longer \
                 distinguish its values",
                st.name,
                facet.value
            );
        }
        let _ = write!(
            b,
            "/// openEHR {} `{}` — an `xs:enumeration`-faceted XSD simple type\n\
             /// restricting `{}`.\n\
             ///\n\
             /// The wire form is the facet value verbatim; text outside the declared set\n\
             /// is refused by [`{rust}::from_wire`].\n\
             #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n\
             pub enum {rust} {{\n",
            self.target.type_label, st.name, st.base
        );
        for (ident, facet) in &variants {
            match &facet.id {
                Some(id) => {
                    let _ = writeln!(
                        b,
                        "    /// The `{}` facet value (XSD facet id `{id}`).",
                        facet.value
                    );
                }
                None => {
                    let _ = writeln!(b, "    /// The `{}` facet value.", facet.value);
                }
            }
            let _ = writeln!(b, "    {ident},");
        }
        b.push_str("}\n\n");

        let _ = write!(
            b,
            "impl {rust} {{\n\
             /// Returns the `xs:enumeration` facet value this variant carries on the wire.\n\
             pub const fn as_wire(&self) -> &'static str {{\n\
             match self {{\n"
        );
        for (ident, facet) in &variants {
            let _ = writeln!(b, "Self::{ident} => {},", rust_str_literal(&facet.value));
        }
        b.push_str("}\n}\n\n");
        let _ = write!(
            b,
            "/// Parses one declared `xs:enumeration` facet value of `{}`.\n\
             ///\n\
             /// # Errors\n\
             /// Returns [`{FACET_ERROR}`] when `text` is outside the declared facet set.\n\
             pub fn from_wire(text: &str) -> Result<Self, {FACET_ERROR}> {{\n\
             match text {{\n",
            st.name
        );
        for (ident, facet) in &variants {
            let _ = writeln!(
                b,
                "{} => Ok(Self::{ident}),",
                rust_str_literal(&facet.value)
            );
        }
        let _ = write!(
            b,
            "_ => Err({FACET_ERROR} {{ simple_type: {}, value: text.to_owned() }}),\n\
             }}\n}}\n}}\n\n",
            rust_str_literal(&st.name)
        );
        let _ = write!(
            b,
            "impl std::fmt::Display for {rust} {{\n\
             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n\
             f.write_str(self.as_wire())\n}}\n}}\n\n"
        );
    }

    /// Emit the shared refusal error the facet readers return.
    fn emit_facet_error(b: &mut String) {
        let _ = write!(
            b,
            "/// Text outside the `xs:enumeration` facet set of one of this module's\n\
             /// generated XSD simple types.\n\
             #[derive(Debug, Clone, PartialEq, Eq)]\n\
             pub struct {FACET_ERROR} {{\n\
             /// The XSD simple type whose declared facet set refused the text.\n\
             pub simple_type: &'static str,\n\
             /// The refused text, verbatim as it appeared on the wire.\n\
             pub value: String,\n\
             }}\n\n\
             impl std::fmt::Display for {FACET_ERROR} {{\n\
             fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n\
             write!(f, \"{{:?}} is not a declared xs:enumeration value of {{}}\", self.value, self.simple_type)\n\
             }}\n}}\n\n\
             impl std::error::Error for {FACET_ERROR} {{}}\n\n"
        );
    }

    /// Emit `ToXml`/`FromXml` for one faceted simple type's enum: the facet
    /// value as element text out, the refusing reader in.
    fn emit_facet_impls(&self, b: &mut String, st: &XsdSimpleType) {
        let rust = naming::type_name(&st.name);
        let prelude = self.target.prelude;
        let _ = write!(
            b,
            "impl crate::xml::runtime::ToXml for {prelude}::{rust} {{\n\
             fn write_xml(&self, w: &mut crate::xml::runtime::XmlWriter, tag: &str, _declared: Option<&str>) -> Result<(), crate::xml::runtime::XmlError> {{\n\
             w.write_text_element(tag, self.as_wire())\n}}\n}}\n\n"
        );
        let read = if facet_base_collapses_whitespace(&st.base) {
            "__s.trim()"
        } else {
            "__s.as_str()"
        };
        let _ = write!(
            b,
            "impl crate::xml::runtime::FromXml for {prelude}::{rust} {{\n\
             fn from_xml(reader: &mut crate::xml::runtime::XmlReader, start: &crate::xml::runtime::StartTag) -> Result<Self, crate::xml::runtime::XmlError> {{\n\
             let __s = <::std::string::String as crate::xml::runtime::FromXml>::from_xml(reader, start)?;\n\
             {prelude}::{rust}::from_wire({read}).map_err(|__e| crate::xml::runtime::XmlError::parse_source({}, __e))\n\
             }}\n}}\n\n",
            rust_str_literal(&format!("element text of {}", st.name))
        );
    }

    /// Build the [`XmlType`] for a generated spec (for the `emit_xml` impls).
    fn xml_type(&self, spec: &str) -> Option<XmlType> {
        self.xsd.types.get(spec)?;
        let rust = naming::type_name(spec);
        if self.enum_specs.contains(spec) {
            let descendants = self.xsd.descendants(spec);
            let variants = descendants
                .iter()
                .map(|d| XmlVariant {
                    ident: naming::type_name(d),
                })
                .collect();
            let dispatch = descendants
                .iter()
                .map(|d| (d.clone(), naming::type_name(d)))
                .collect();
            Some(XmlType::Enum {
                spec: spec.to_string(),
                rust,
                generics: Vec::new(),
                variants,
                dispatch,
            })
        } else {
            Some(XmlType::Struct {
                spec: spec.to_string(),
                rust,
                generics: Vec::new(),
                fields: self.fields(spec).into_iter().map(|f| f.xml).collect(),
            })
        }
    }

    /// Emit the type declarations (the target module's `types.rs`).
    #[must_use]
    pub(crate) fn emit_types(&self) -> String {
        let mut b = String::new();
        let _ = write!(
            b,
            "// @generated by openehr-codegen ({}) — DO NOT EDIT.\n\
             //! Typed Rust model for openEHR {}.\n\n",
            self.target.generator, self.target.types_subject
        );
        b.push_str(
            "#![allow(\n    \
             dead_code,\n    \
             non_snake_case,\n    \
             non_camel_case_types,\n    \
             clippy::all,\n    \
             clippy::pedantic,\n    \
             clippy::nursery,\n    \
             reason = \"mechanically generated model text: the XSD is emitted in \
             full under its own spec-owned element/attribute spellings, so naming, \
             style and dead-code lints do not apply — the hand-written runtime \
             carries the lint bar\"\n\
             )]\n\n",
        );
        // The faceted simple types first: they are the closure's leaf
        // vocabularies, and the structs below declare fields of them.
        let facets = self.facet_types();
        if !facets.is_empty() {
            Self::emit_facet_error(&mut b);
        }
        let generated_type_names: BTreeSet<String> =
            self.generate.iter().map(|s| naming::type_name(s)).collect();
        for st in facets {
            let rust = naming::type_name(&st.name);
            assert!(
                !generated_type_names.contains(&rust),
                "xs:simpleType {:?} and a complexType of this closure both emit the Rust type \
                 {rust:?}",
                st.name
            );
            self.emit_facet_enum(&mut b, st);
        }
        for spec in &self.generate {
            let Some(ty) = self.xsd.types.get(spec) else {
                continue;
            };
            let rust = naming::type_name(spec);
            let mut doc = format!("/// openEHR {} `{spec}`.\n", self.target.type_label);
            // An `abstract="true"` complexType that NOTHING in the closure derives
            // from cannot be an `xsi:type` dispatch enum, and it is still a slot
            // type real documents must fill (`Archetype.xsd` declares
            // `C_ATTRIBUTE` abstract yet types `C_COMPLEX_OBJECT.attributes` with
            // it). It is emitted as the plain shape a document has to present —
            // never dropped, which would leave a dangling field type.
            //
            // NOTE: such a type is NOT a variant of the enums it descends from —
            // the XSD `xsi:type` rule — while its concrete descendants all are,
            // which `the_concrete_only_variant_reading_loses_no_document_shape` pins.
            if ty.is_abstract && !self.enum_specs.contains(spec) {
                let _ = writeln!(
                    b,
                    "/// openEHR {} `{spec}` — declared `abstract` in the XSD with no\n\
                     /// concrete subtype in this schema closure, so it is emitted as the\n\
                     /// plain shape a conforming document must present at its slots.",
                    self.target.type_label
                );
                doc = String::new();
            }
            if self.enum_specs.contains(spec) {
                let descendants = self.xsd.descendants(spec);
                b.push_str(&doc);
                // These are XML-only models (`ToXml`/`FromXml`); they carry no
                // serde — the types are plain data records parsed from XML.
                b.push_str("#[derive(Debug, Clone, PartialEq)]\n");
                let _ = writeln!(b, "pub enum {rust} {{");
                for d in &descendants {
                    let ident = naming::type_name(d);
                    // A variant is a public item `missing_docs` checks; the XSD
                    // carries no per-subtype prose, so name the subtype.
                    let _ = writeln!(
                        b,
                        "    /// The {} `{d}` subtype of `{spec}`.",
                        self.target.field_label
                    );
                    let _ = writeln!(b, "    {ident}({ident}),");
                }
                b.push_str("}\n\n");
            } else {
                b.push_str(&doc);
                b.push_str("#[derive(Debug, Clone, PartialEq)]\n");
                let _ = writeln!(b, "pub struct {rust} {{");
                for f in self.fields(spec) {
                    let _ = writeln!(
                        b,
                        "    /// The `{}` attribute/element of the {} `{spec}` XSD type.",
                        f.xml.wire_name, self.target.field_label
                    );
                    let _ = writeln!(b, "    pub {}: {},", f.xml.rust_name, f.decl_type);
                }
                b.push_str("}\n\n");
            }
        }
        b
    }

    /// Emit the `ToXml`/`FromXml` impls (the target module's `impls.rs`), reusing
    /// the `emit-xml` per-type emitters over the XSD-derived [`XmlType`]s.
    #[must_use]
    pub(crate) fn emit_impls(&self, unmatched: &mut Vec<(String, String)>) -> String {
        let mut b = String::new();
        let _ = write!(
            b,
            "// @generated by openehr-codegen ({}) — DO NOT EDIT.\n\
             //! Canonical-XML `ToXml`/`FromXml` impls for the {} model.\n\n",
            self.target.generator, self.target.impls_subject
        );
        b.push_str(
            "#![allow(\n    \
             non_snake_case,\n    \
             clippy::all,\n    \
             clippy::pedantic,\n    \
             clippy::nursery,\n    \
             unused_variables,\n    \
             unused_mut,\n    \
             unused_qualifications,\n    \
             unused_imports,\n    \
             reason = \"mechanically generated codec text: every runtime item is \
             named by its full path and every branch shape is emitted uniformly, \
             so style and unused-binding lints do not apply — the hand-written \
             runtime carries the lint bar\"\n\
             )]\n\
             use crate::xml::runtime::{ToXml, FromXml, XmlEvent, XmlError};\n\n",
        );
        for st in self.facet_types() {
            self.emit_facet_impls(&mut b, st);
        }
        for spec in &self.generate {
            if let Some(ty) = self.xml_type(spec) {
                emit_to_xml(&mut b, &ty, self.target.prelude, self.xsd, unmatched);
                emit_from_xml(&mut b, &ty, self.target.prelude, self.xsd, None);
            }
        }
        b
    }
}

/// One `from_xml`/`to_xml` pair to emit for a generated XML module.
pub(crate) struct EntryPoint {
    /// Appended to `from_xml`/`to_xml` so several roots can coexist in one
    /// module (`""` for the primary pair).
    pub(crate) suffix: &'static str,
    /// The generated Rust type of the document root.
    pub(crate) root_rust: &'static str,
    /// The XML root element name.
    pub(crate) root_element: &'static str,
    /// Human phrase for the doc comment ("operational template").
    pub(crate) what: &'static str,
    /// The indefinite article for [`Self::root_rust`] ("an"/"a"), so the
    /// generated prose stays grammatical across roots.
    pub(crate) article: &'static str,
    /// The wire label used in the serialize doc ("OPT 1.4 XML").
    pub(crate) wire: &'static str,
    /// The spec/XSD type name, named in the module docs.
    pub(crate) spec_name: &'static str,
}

/// What a generated XML module's `mod.rs` should say and expose.
pub(crate) struct ModuleSpec {
    /// The `openehr-codegen` subcommand that produced it.
    pub(crate) generator: &'static str,
    /// The module's one-line `//!` title.
    pub(crate) title: &'static str,
    /// The document roots to expose.
    pub(crate) entry_points: &'static [EntryPoint],
    /// Extra `//!` paragraphs appended after the entry-point lines — the
    /// module's adjudications (root-type choice, corpus ceiling). One entry per
    /// paragraph; a blank line is emitted between them.
    pub(crate) notes: &'static [&'static str],
}

/// The `opt14/mod.rs` surface.
pub(crate) static OPT_MODULE: ModuleSpec = ModuleSpec {
    generator: "emit-opt",
    title: "openEHR OPT 1.4 operational-template model + canonical-XML codec.",
    entry_points: &[EntryPoint {
        suffix: "",
        root_rust: "OperationalTemplate",
        root_element: "template",
        what: "operational template",
        article: "an",
        wire: "OPT 1.4 XML",
        spec_name: "OPERATIONAL_TEMPLATE",
    }],
    notes: &[],
};

/// The `aom2/mod.rs` surface (the AOM2 persistent form).
pub(crate) static AOM2_MODULE: ModuleSpec = ModuleSpec {
    generator: "emit-aom2",
    title: "openEHR AOM2 persistent-form archetype model + canonical-XML codec.",
    entry_points: &[EntryPoint {
        suffix: "",
        root_rust: "PAuthoredArchetype",
        root_element: "archetype",
        what: "persistent-form AOM2 archetype",
        article: "a",
        wire: "AOM2 persistent-form archetype XML",
        spec_name: "P_AUTHORED_ARCHETYPE",
    }],
    notes: &[
        "This is the PERSISTENT (`P_AOM`) AOM2 serialization — `P_Archetype.xsd`, whose\n\
         own header calls it \"uses P_AOM types - much more space efficient\". It is the\n\
         form the bundle's 8 `AOM2/examples/*.xml` documents carry\n\
         (`xsi:schemaLocation=\"… ../P_Archetype.xsd\"`). The bundle's other archetype\n\
         serialization — the AOM model form — is [`crate::aom2_model`].",
    ],
};

/// The `aom2_model/mod.rs` surface (the AOM2 model form).
pub(crate) static AOM2_MODEL_MODULE: ModuleSpec = ModuleSpec {
    generator: "emit-aom2",
    title: "openEHR AOM2 model-form archetype model + canonical-XML codec.",
    entry_points: &[EntryPoint {
        suffix: "",
        root_rust: "AuthoredArchetype",
        root_element: "archetype",
        what: "model-form AOM2 archetype",
        article: "a",
        wire: "AOM2 model-form archetype XML",
        spec_name: "AUTHORED_ARCHETYPE",
    }],
    notes: &[
        "This is the AOM MODEL form — `Archetype.xsd`, whose own header calls it \"uses\n\
         AOM-like types - not space-efficient\": the AOM2 classes themselves\n\
         (`C_COMPLEX_OBJECT`, `C_ATTRIBUTE`, `ARCHETYPE_TERMINOLOGY`,\n\
         `MultiplicityInterval`), as opposed to the persistent `P_AOM` form in\n\
         [`crate::aom2`]. Both schemas declare the top-level element `<archetype>`, so\n\
         a document's ROOT TYPE — not its element name — decides which module reads it.",
        "Root type: `Archetype.xsd` declares\n\
         `<xs:element name=\"archetype\" type=\"ARCHETYPE\"/>`, but `ARCHETYPE` is\n\
         `abstract=\"true\"` and no complexType in the closure derives from it —\n\
         `AUTHORED_ARCHETYPE` extends `AUTHORED_RESOURCE` and re-uses the archetype body\n\
         through `<xs:group ref=\"ARCHETYPE\"/>` instead. `AUTHORED_ARCHETYPE` is\n\
         therefore the only instantiable archetype root the schema offers, and the entry\n\
         points here are typed to it; `Archetype` itself is not emitted, because an\n\
         abstract type with no concrete subtype can never appear on the wire.",
        "Corpus ceiling: openEHR publishes NO model-form instance documents. All 8\n\
         `AOM2/examples/*.xml` are persistent-form, `openEHR/adl-archetypes` publishes\n\
         ADL text only, and `specifications-ITS-XML` has no further branch to vendor. So\n\
         this codec is gated by construct → serialize → parse self-consistency\n\
         (`openehr-its` `tests/it/aom2_model_xml.rs`), not by an upstream corpus. That\n\
         ceiling is stated rather than implied.",
    ],
};

/// Emit a generated XML module's `mod.rs` — wiring, re-export, and one
/// `from_xml`/`to_xml` pair per document root.
///
/// Shared by every XSD-driven emitter so the module surface stays identical
/// across them; the roots are the only thing that varies.
#[must_use]
pub(crate) fn emit_module(spec: &ModuleSpec) -> String {
    let mut b = String::new();
    let _ = writeln!(
        b,
        "// @generated by openehr-codegen ({}) — DO NOT EDIT.",
        spec.generator
    );
    let _ = writeln!(b, "//! {}", spec.title);
    let _ = writeln!(b, "//!");
    for e in spec.entry_points {
        let _ = writeln!(
            b,
            "//! Parse {} {} with [`from_xml{}`]; the root element is `<{}>` (`{}`).",
            e.article, e.what, e.suffix, e.root_element, e.spec_name
        );
    }
    for note in spec.notes {
        let _ = writeln!(b, "//!");
        for line in note.lines() {
            let _ = writeln!(b, "//! {line}");
        }
    }
    // `pub mod types`, never a star re-export: the zero-re-exports rule means an
    // import names its defining module, and a glob would also hide which of the
    // three generated modules a name came from at the use site.
    let _ = writeln!(b, "\nmod impls;\npub mod types;");
    for e in spec.entry_points {
        let _ = writeln!(
            b,
            "\n/// Parse {} {} XML document into {} [`types::{}`].\n\
             ///\n\
             /// # Errors\n\
             /// Propagates canonical-XML parse errors.\n\
             pub fn from_xml{}(xml: &str) -> Result<types::{}, crate::xml::runtime::XmlError> {{\n\
             crate::xml::runtime::from_xml(xml)\n\
             }}",
            e.article, e.what, e.article, e.root_rust, e.suffix, e.root_rust
        );
        let _ = writeln!(
            b,
            "\n/// Serialize {} [`types::{}`] back to {} (root `<{}>`,\n\
             /// `http://schemas.openehr.org/v1`).\n\
             ///\n\
             /// # Errors\n\
             /// Propagates canonical-XML serialization errors.\n\
             pub fn to_xml{}(value: &types::{}) -> Result<String, crate::xml::runtime::XmlError> {{\n\
             crate::xml::runtime::to_xml(value, \"{}\", crate::xml::runtime::Namespace::V1)\n\
             }}",
            e.article, e.root_rust, e.wire, e.root_element, e.suffix, e.root_rust, e.root_element
        );
    }
    b
}
