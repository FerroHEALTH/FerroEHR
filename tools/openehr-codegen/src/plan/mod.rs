// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 3 — PLAN. The emission-decision layer: for every analysed class,
//! decide the Rust shape it emits as (struct / closed enum / polymorphic enum /
//! enumeration literals / transparent newtype / skip) and the XML shape it
//! classifies as. The decision *data* — the class bindings, back-references,
//! type overrides, field defaults, the primitive/mapped-class tables, and the
//! emit-xml BMM-only allowlist — lives as declarative const tables in
//! [`overrides`], each entry carrying its spec citation; the crate → schema
//! merge table lives in [`composition`]. This stage makes decisions only — the
//! text is produced in [`crate::render`]. The construction-door decisions (which
//! classes hide their fields behind a validating constructor) live in
//! [`construction`], on the same declarative, spec-cited pattern.

pub(crate) mod composition;
pub(crate) mod construction;
pub(crate) mod overrides;

use crate::analyze::Model;
use crate::load::bmm::{BmmClass, BmmEnumeration, BmmPropKind, BmmSchema, BmmType};
use crate::plan::overrides::{back_reference, cardinality_contradicted, field_default, primitive};
use crate::render::naming;
use std::collections::BTreeSet;

/// What to emit for a class.
pub(crate) enum Emission<'a> {
    Struct,
    /// Untagged enum for an *abstract* polymorphic slot: one variant per
    /// immediate concrete subtype (the abstract class itself is not instantiable).
    Enum(Vec<String>),
    /// Untagged enum for a *concrete* class that also has subtypes
    /// (`DV_TEXT` → `DV_CODED_TEXT`): a field typed as the parent accepts either.
    /// Emits a `{Name}Data` struct for the class's own instances plus a `{Name}`
    /// enum over `{Name}Data` and each immediate concrete subtype.
    PolyEnum(Vec<String>),
    /// A BMM enumeration class (`BMM_ENUMERATION`) — a real Rust enum over its
    /// named constants plus a tolerance-preserving `Other(String|i32)` catch-all,
    /// with hand-written serde byte-identical to the bare primitive it replaces.
    EnumLiterals(&'a BmmEnumeration),
    /// Transparent newtype over a Rust primitive (a genuine primitive alias).
    /// After enumeration classes route to [`Emission::EnumLiterals`] this arm is
    /// the fallback for a 0-field concrete leaf over a primitive that carries no
    /// enumeration facet (none exist in the current vendored BMM).
    Newtype(&'a str),
    Skip,
}

/// Decide how a class is emitted.
pub(crate) fn decide<'a>(
    model: &Model,
    class: &'a BmmClass,
    used: &BTreeSet<String>,
) -> Emission<'a> {
    if Model::is_mapped(&class.name) {
        return Emission::Skip;
    }
    // A BMM enumeration is a typed literal set on the wire, regardless of how the
    // rest of the BMM shapes the class. This preempts BOTH the transparent-newtype
    // path (`VALIDITY_KIND`) AND the polymorphic-enum path that `PROPORTION_KIND`
    // wrongly fell into — the RM BMM lists `PROPORTION_KIND` in
    // `DV_PROPORTION.ancestors`, so `enum_variants` is non-empty and the concrete
    // branch below would emit a nonsense `ProportionKind`/`ProportionKindData`
    // poly enum. The enumeration facet is authoritative, so it wins first.
    if let Some(enumeration) = &class.enumeration {
        return Emission::EnumLiterals(enumeration);
    }
    if class.is_abstract {
        let variants = model.enum_variants(&class.name);
        if !variants.is_empty() {
            // A closed polymorphic slot. Emit the untagged enum whenever the
            // class has concrete descendants — even if *this* schema never uses
            // it as a field type, because a downstream crate may (e.g. `Interval`
            // is a BASE foundation type referenced by AM's `Interval<Integer>`).
            Emission::Enum(variants)
        } else if used.contains(&class.name) {
            // Abstract, referenced as a field type, but no concrete descendants
            // in this schema (e.g. `AUTHORED_RESOURCE` in BASE — its concretes
            // live in AM). Emit its own fields as a struct so the reference
            // resolves; a cross-schema pass can promote it to an enum later.
            //
            // NOTE: when such a class also declares NO attributes the emission
            // is an instantiable EMPTY struct, so abstractness is not encoded —
            // no better shape exists without forking the vendored model.
            Emission::Struct
        } else {
            Emission::Skip
        }
    } else {
        // Concrete but with its own concrete subtypes: a field typed as this
        // class accepts the subtype too (`DV_TEXT` holds a `DV_CODED_TEXT`, a
        // coded name), so emit a polymorphic enum plus the `{Name}Data` struct.
        let variants = model.enum_variants(&class.name);
        if !variants.is_empty() {
            return Emission::PolyEnum(variants);
        }
        // Concrete leaf: a 0-field class whose sole ancestor is a primitive is
        // an enumeration-style newtype (VALIDITY_KIND → String).
        let flattened = model.flattened_props(class);
        if flattened.is_empty()
            && let [sole_ancestor] = class.ancestors.as_slice()
            && let Some(prim) = primitive(sole_ancestor)
        {
            return Emission::Newtype(prim);
        }
        Emission::Struct
    }
}

// ── XML codegen support ─────────────────────────────────────────────
// A thin, semantic view of the generated types for the XML emitter (`emit_xml`).
// The XML wire *shape* (element order, attribute-vs-element, xsi:type) comes from
// the XSD reader; this supplies the matching Rust facts (field idents, Option/Vec,
// enum variants, generics) so the generated impls compile against the emitted
// structs. Boxing is transparent to `.write_xml()`, so it is deliberately ignored.

/// One field of an instantiable type. The XML element/attribute name is the
/// openEHR property name (`wire_name`); the Rust accessor is `rust_name`.
/// `target` is the spec type of the value (item type for containers), passed as
/// the declared type so a polymorphic value emits `xsi:type`.
pub(crate) struct XmlField {
    pub wire_name: String,
    pub rust_name: String,
    /// The Rust field is wrapped in `Option<…>`. Orthogonal to
    /// [`XmlField::multiple`]: an optional container is `Option<Vec<T>>`, so
    /// both flags are set.
    pub optional: bool,
    pub multiple: bool,
    /// The container is `NonEmptyVec<T>` — a `1..*` bound, or an optional
    /// container carrying a present-implies-non-empty invariant
    /// (`Option<NonEmptyVec<T>>`): the reader builds it through the
    /// type's fallible constructor.
    pub nonempty: bool,
    pub target: String,
    /// For a `Hash<String, V>` field (`target == "Hash"`), the value type's spec
    /// name (`V`); `None` otherwise. `Some("String")` is serialized inline as the
    /// openEHR `StringDictionaryItem` shape.
    pub map_value: Option<String>,
    /// A mandatory field archie omits at its default (the `Interval` inclusivity/
    /// boundedness flags): the Rust default expression (`true`/`false`) to use on
    /// deserialization when the element is absent. `None` = genuinely required.
    pub default: Option<String>,
}

/// One variant of an untagged enum, for the forwarding `ToXml`/`FromXml` impl.
pub(crate) struct XmlVariant {
    /// Rust variant identifier (`DvCodedText`, or the enum's own name for the
    /// polymorphic-concrete self-data variant).
    pub ident: String,
}

/// An instantiable type needing a `ToXml`/`FromXml` impl.
pub(crate) enum XmlType {
    /// A struct: a plain `Struct` class, or a `PolyEnum`'s `{Name}Data`.
    Struct {
        spec: String,
        rust: String,
        generics: Vec<String>,
        fields: Vec<XmlField>,
    },
    /// An untagged enum (abstract slot or polymorphic-concrete) — forwards to
    /// the active variant's payload.
    Enum {
        spec: String,
        rust: String,
        generics: Vec<String>,
        variants: Vec<XmlVariant>,
        /// xsi:type deserialization map: every concrete descendant spec (and the
        /// enum's own spec, if concrete) → the direct variant ident it routes
        /// into. A deep type (`DV_CODED_TEXT` in a `DATA_VALUE` slot) routes into
        /// the intermediate variant (`DvText`), which recurses.
        dispatch: Vec<(String, String)>,
    },
    /// A transparent newtype over a primitive — writes its inner value as
    /// element text. No enumeration class emits as a newtype anymore (they route
    /// to [`XmlType::EnumLiterals`]); kept for a future genuine primitive alias.
    Newtype { spec: String, rust: String },
    /// A BMM enumeration emitted as a typed enum (`VALIDITY_KIND`,
    /// `PROPORTION_KIND`) — writes its wire token (`as_str`) or integer (`value`)
    /// as element text; reads the bare primitive back through `from_wire`/
    /// `from_value`.
    EnumLiterals {
        spec: String,
        rust: String,
        /// `true` for a STRING-underlying enum (`as_str`/`from_wire`), `false`
        /// for an INTEGER-underlying enum (`value`/`from_value`).
        string_backed: bool,
    },
}

// ── JSON codegen support ─────────────────────────────────────────────
// A thin, semantic view of the generated types for the canonical-JSON emitter
// (`emit_json`). The JSON wire shape is entirely BMM-driven — field order =
// declaration order, `_type` first, `None`/empty-`Vec` omitted — so unlike the
// XML view there is no XSD input; this view only needs each field's wire name,
// Rust accessor, and omission kind. It must reproduce, byte-for-byte, the
// `#[derive(OpenEhrType)]` serde `Serialize` (`openehr-derive`).

/// One field of an instantiable type on the canonical-JSON wire. `wire_name` is
/// the JSON key (the openEHR property name, which the derive's rename logic
/// always resolves back to), `rust_name` the Rust accessor.
pub(crate) struct JsonField {
    pub wire_name: String,
    pub rust_name: String,
    pub kind: JsonFieldKind,
    /// A literal default (`"true"`/`"false"`) for a mandatory (`Plain`) field the
    /// wire may omit — the `Interval` `*_included`/`*_unbounded` flags. When set,
    /// a missing field deserializes to this default instead of erroring, matching
    /// the retired derive's `#[openehr(default = "…")]`.
    pub default: Option<String>,
}

/// How a field is written on serialize — matching the derive's classification by
/// Rust type head (`Option` → omit when `None`; `Vec` → omit when empty; else
/// always present).
pub(crate) enum JsonFieldKind {
    /// `Option<T>` — omitted when `None`.
    Optional,
    /// `Vec<T>` (a mandatory container) — omitted when empty.
    Container,
    /// `NonEmptyVec<T>` (a mandatory `1..*` container) — always present, and
    /// read through the type's own fallible constructor, so an absent or empty
    /// array is refused at parse rather than validated later.
    NonEmptyContainer,
    /// `Option<Vec<T>>` (an optional container) — omitted when `None` **and**
    /// when `Some` but empty, per
    /// `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
    /// §JSON Format; read back as `None` when absent and `Some(vec![])` when
    /// present-but-empty.
    OptionalContainer,
    /// Anything else (including a mandatory `Box<T>`, a `BTreeMap`, or a
    /// `#[openehr(default)]` flag) — always emitted.
    Plain,
}

/// An instantiable type needing a `ToJson` impl. Mirrors [`XmlType`] but carries
/// only what the JSON serialize side needs.
pub(crate) enum JsonType {
    /// A struct: a plain `Struct` class, or a `PolyEnum`'s `{Name}Data`. Emits
    /// `_type` first, then the fields.
    Struct {
        spec: String,
        rust: String,
        generics: Vec<String>,
        fields: Vec<JsonField>,
    },
    /// An untagged enum (abstract slot or polymorphic-concrete). Serialize
    /// forwards to the active variant's payload (`_type` comes from the payload);
    /// deserialize dispatches per [`JsonEnumDispatch`].
    Enum {
        spec: String,
        rust: String,
        generics: Vec<String>,
        /// The Rust variant identifiers, in the same order the struct/enum
        /// emitter declares them (a `PolyEnum`'s self-data variant is last).
        variant_idents: Vec<String>,
        /// How the deserialize side selects a variant.
        dispatch: JsonEnumDispatch,
    },
    /// A transparent newtype over a primitive — forwards to its inner value.
    Newtype { spec: String, rust: String },
    /// A BMM enumeration emitted as a typed enum — writes its wire token
    /// (`as_str`) or integer (`value`), byte-identical to the bare primitive.
    EnumLiterals {
        spec: String,
        rust: String,
        string_backed: bool,
    },
}

/// How a [`JsonType::Enum`] selects a variant on deserialize — the exact split
/// `emit_enum` makes for the serde reader (`_type` dispatch vs structural
/// `#[serde(untagged)]`), projected for the native `FromJson`.
pub(crate) enum JsonEnumDispatch {
    /// `_type`-keyed dispatch (every concrete target carries a `_type`). `arms`
    /// maps each concrete descendant spec → its direct variant ident (deep
    /// descendants collapse onto their intermediate variant, which recurses).
    /// `self_ident` is `Some` for a concrete polymorphic slot (a `_type`-less
    /// value defaults to it) and `None` for an abstract slot (a `_type`-less value
    /// is rejected). `spec_name` + `expected` build the error messages.
    ByType {
        arms: Vec<(String, String)>,
        self_ident: Option<String>,
        spec_name: String,
        expected: String,
    },
    /// Structural fallback (a target does not carry `_type`): try each variant in
    /// declaration order, first success wins — mirrors `#[serde(untagged)]`.
    Structural { variant_idents: Vec<String> },
}

impl Model {
    /// The flattened fields of a concrete class for canonical-JSON emission —
    /// same order and flattening as struct emission, with the same designated
    /// back-reference fields omitted (they are not emitted as struct fields, so
    /// the codec must not name them). The omission kind mirrors the derive's
    /// classification by Rust type head.
    #[must_use]
    pub(crate) fn json_fields(&self, class_name: &str) -> Vec<JsonField> {
        let Some(class) = self.get(class_name) else {
            return Vec::new();
        };
        self.flattened_props(class)
            .iter()
            .filter(|rp| back_reference(&rp.owner, &rp.prop.name).is_none())
            .map(|rp| {
                let p = rp.prop;
                // A byte buffer (`Array<Octet>`) renders as a `String`/`Option<
                // String>`, not a `Vec` — so it is never a Container; every other
                // container is a `Vec<T>`.
                let octet = matches!(&p.kind,
                    BmmPropKind::Container { item, .. } if item.root_name() == "Octet");
                let multiple = matches!(&p.kind, BmmPropKind::Container { .. }) && !octet;
                let lower_bound_one = matches!(&p.kind,
                    BmmPropKind::Container { cardinality, .. }
                        if cardinality.as_ref().is_some_and(|c| c.lower >= 1))
                    && !cardinality_contradicted(&rp.owner, &p.name);
                let kind = if multiple {
                    match (p.is_mandatory, lower_bound_one) {
                        (true, true) => JsonFieldKind::NonEmptyContainer,
                        (true, false) => JsonFieldKind::Container,
                        (false, _) => JsonFieldKind::OptionalContainer,
                    }
                } else if p.is_mandatory {
                    JsonFieldKind::Plain
                } else {
                    JsonFieldKind::Optional
                };
                JsonField {
                    wire_name: p.name.clone(),
                    rust_name: naming::field_ident(&p.name),
                    kind,
                    default: field_default(&rp.owner, p),
                }
            })
            .collect()
    }

    /// The instantiable canonical-JSON types of a schema, in class order (the
    /// same decisions [`Model::xml_types`] makes, projected to the JSON view).
    ///
    #[must_use]
    pub(crate) fn json_types(&self, schema: &BmmSchema) -> Vec<JsonType> {
        let used = self.used_as_type();
        let mut out = Vec::new();
        for (name, class) in &schema.classes {
            let generics: Vec<String> = self
                .used_generic_params(name)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            let rust = naming::type_name(name);
            match decide(self, class, &used) {
                Emission::Struct => out.push(JsonType::Struct {
                    spec: name.clone(),
                    rust,
                    generics,
                    fields: self.json_fields(name),
                }),
                Emission::PolyEnum(variants) => {
                    // The `{Name}Data` struct (own instances) + the `{Name}` enum.
                    out.push(JsonType::Struct {
                        spec: name.clone(),
                        rust: format!("{rust}Data"),
                        generics: generics.clone(),
                        fields: self.json_fields(name),
                    });
                    let mut idents: Vec<String> =
                        variants.iter().map(|v| naming::type_name(v)).collect();
                    // The polymorphic-concrete self-data variant is emitted last;
                    // its identifier is the enum's own name (`DvText(DvTextData)`).
                    idents.push(rust.clone());
                    let dispatch = self.json_enum_dispatch(name, &variants, &idents);
                    out.push(JsonType::Enum {
                        spec: name.clone(),
                        rust,
                        generics,
                        variant_idents: idents,
                        dispatch,
                    });
                }
                Emission::Enum(variants) => {
                    let idents: Vec<String> =
                        variants.iter().map(|v| naming::type_name(v)).collect();
                    let dispatch = self.json_enum_dispatch(name, &variants, &idents);
                    out.push(JsonType::Enum {
                        spec: name.clone(),
                        rust,
                        generics,
                        variant_idents: idents,
                        dispatch,
                    });
                }
                Emission::EnumLiterals(enumeration) => out.push(JsonType::EnumLiterals {
                    spec: name.clone(),
                    rust,
                    string_backed: enumeration.underlying_type != "INTEGER",
                }),
                Emission::Newtype(_) => out.push(JsonType::Newtype {
                    spec: name.clone(),
                    rust,
                }),
                Emission::Skip => {}
            }
        }
        out
    }

    /// Decide how a canonical-JSON enum deserializes, reproducing exactly the
    /// split `emit_enum` makes for the serde reader: `_type` dispatch when every
    /// concrete target carries a `_type`, else the structural `#[serde(untagged)]`
    /// fallback. `variant_idents` are the ToJson-order variant idents used by the
    /// structural path.
    fn json_enum_dispatch(
        &self,
        spec: &str,
        variants: &[String],
        variant_idents: &[String],
    ) -> JsonEnumDispatch {
        let dispatch = self.xsi_dispatch(spec, variants);
        let type_dispatch = !dispatch.is_empty()
            && dispatch
                .iter()
                .all(|(target, _)| self.concrete_carries_type(target));
        if type_dispatch {
            let self_ident = dispatch
                .iter()
                .find(|(target, _)| target == spec)
                .map(|(_, id)| id.clone());
            let expected = dispatch
                .iter()
                .map(|(s, _)| s.clone())
                .collect::<Vec<_>>()
                .join(", ");
            JsonEnumDispatch::ByType {
                arms: dispatch,
                self_ident,
                spec_name: spec.to_string(),
                expected,
            }
        } else {
            JsonEnumDispatch::Structural {
                variant_idents: variant_idents.to_vec(),
            }
        }
    }

    /// The flattened fields of a concrete class for XML emission (same order and
    /// flattening as struct emission).
    #[must_use]
    pub(crate) fn xml_fields(&self, class_name: &str) -> Vec<XmlField> {
        let Some(class) = self.get(class_name) else {
            return Vec::new();
        };
        self.flattened_props(class)
            .iter()
            // A designated owner/parent back-reference is omitted from the
            // emitted struct (see `back_reference` / `render_struct_def`), so it
            // must be omitted from the canonical-XML codec too — otherwise
            // `ToXml`/`FromXml` would name a struct field that no longer exists.
            // On the wire these fields are XSD-`minOccurs=0` (optional), so
            // omitting them keeps the XML schema-valid; the XSD element becomes a
            // skipped "XSD-only" slot on write and a `skip_element` on read.
            .filter(|rp| back_reference(&rp.owner, &rp.prop.name).is_none())
            .map(|rp| {
                let p = rp.prop;
                let octet = matches!(&p.kind,
                    BmmPropKind::Container { item, .. } if item.root_name() == "Octet");
                let (multiple, target) = match &p.kind {
                    BmmPropKind::Single(t) => (false, t.root_name().to_string()),
                    BmmPropKind::Container { item, .. } => (!octet, item.root_name().to_string()),
                };
                // The value type of a `Hash<K, V>` field (second generic arg).
                let map_value = match &p.kind {
                    BmmPropKind::Single(BmmType::Generic { root, params }) if root == "Hash" => {
                        params.get(1).map(|v| v.root_name().to_string())
                    }
                    _ => None,
                };
                XmlField {
                    wire_name: p.name.clone(),
                    rust_name: naming::field_ident(&p.name),
                    optional: !p.is_mandatory,
                    multiple,
                    nonempty: multiple
                        && ((p.is_mandatory
                            && matches!(&p.kind,
                                BmmPropKind::Container { cardinality, .. }
                                    if cardinality.as_ref().is_some_and(|c| c.lower >= 1))
                            && !cardinality_contradicted(&rp.owner, &p.name))
                            // An OPTIONAL container carrying a
                            // present-implies-non-empty invariant emits
                            // `Option<NonEmptyVec<T>>`, so its reader
                            // builds through the same fallible constructor.
                            // The invariant's declaring class may be an
                            // ANCESTOR of the flattened property's owner (a
                            // subclass overriding the attribute to narrow its
                            // type keeps the inherited rule).
                            || (!p.is_mandatory
                                && crate::analyze::nonempty_optional_lists_cached(self)
                                    .iter()
                                    .any(|(decl, attr)| {
                                        attr == &p.name
                                            && (decl == &rp.owner
                                                || self.inherits(&rp.owner, decl))
                                    }))),
                    target,
                    map_value,
                    default: field_default(&rp.owner, p),
                }
            })
            .collect()
    }

    /// The xsi:type deserialization map for an enum: every concrete descendant
    /// spec (and the enum's own spec, if concrete) → the direct variant ident it
    /// routes into. `direct` is the enum's immediate variant specs. A deep type
    /// routes into its intermediate direct variant, which recurses.
    pub(crate) fn xsi_dispatch(&self, enum_spec: &str, direct: &[String]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (name, class) in &self.classes {
            if class.is_abstract || Self::is_mapped(name) {
                continue;
            }
            if name != enum_spec && !self.inherits(name, enum_spec) {
                continue;
            }
            let ident = if name == enum_spec {
                naming::type_name(enum_spec) // polymorphic-concrete self-data variant
            } else if let Some(v) = direct
                .iter()
                .find(|v| v.as_str() == name || self.inherits(name, v))
            {
                naming::type_name(v)
            } else {
                continue;
            };
            out.push((name.clone(), ident));
        }
        out
    }

    /// Does a *concrete* class carry a `_type` discriminator on the
    /// canonical-JSON wire? A `Struct` or `PolyEnum` does (it derives
    /// `OpenEhrType`, which emits `_type` first); a transparent enumeration
    /// `Newtype` (`VALIDITY_KIND` → a bare JSON string) does not. Used to decide
    /// whether an enum's variants can be dispatched on `_type`:
    /// `_type` dispatch is only valid when every concrete target carries one.
    pub(crate) fn concrete_carries_type(&self, name: &str) -> bool {
        let Some(class) = self.get(name) else {
            return false;
        };
        // A BMM enumeration is a bare scalar on the wire (string/int), never a
        // `_type`-tagged object — so it is never a `_type`-dispatch target. This
        // must precede the `enum_variants` check below: `PROPORTION_KIND` has a
        // spurious concrete descendant (`DV_PROPORTION` inherits it in the RM
        // BMM), which would otherwise report `true`.
        if class.enumeration.is_some() {
            return false;
        }
        if !self.enum_variants(name).is_empty() {
            return true; // PolyEnum — the `{Name}` enum + `{Name}Data` both tag.
        }
        // Mirror `decide`'s concrete newtype rule: a 0-field concrete leaf whose
        // sole ancestor is a primitive is a transparent newtype (no `_type`).
        let flattened = self.flattened_props(class);
        !(flattened.is_empty()
            && matches!(class.ancestors.as_slice(), [sole] if primitive(sole).is_some()))
    }

    /// Generic parameter names a type exposes (`Version<T>` → `["T"]`).
    #[must_use]
    pub(crate) fn xml_generics(&self, class_name: &str) -> Vec<String> {
        self.used_generic_params(class_name)
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    /// The instantiable XML types of a schema, in class order.
    #[must_use]
    pub(crate) fn xml_types(&self, schema: &BmmSchema) -> Vec<XmlType> {
        let used = self.used_as_type();
        let mut out = Vec::new();
        for (name, class) in &schema.classes {
            let generics = self.xml_generics(name);
            let rust = naming::type_name(name);
            match decide(self, class, &used) {
                Emission::Struct => out.push(XmlType::Struct {
                    spec: name.clone(),
                    rust,
                    generics,
                    fields: self.xml_fields(name),
                }),
                Emission::PolyEnum(variants) => {
                    out.push(XmlType::Struct {
                        spec: name.clone(),
                        rust: format!("{rust}Data"),
                        generics: generics.clone(),
                        fields: self.xml_fields(name),
                    });
                    let mut vs: Vec<XmlVariant> = variants
                        .iter()
                        .map(|v| XmlVariant {
                            ident: naming::type_name(v),
                        })
                        .collect();
                    // The polymorphic-concrete self-data variant is emitted last,
                    // its identifier is the enum's own name (`DvText(DvTextData)`).
                    vs.push(XmlVariant {
                        ident: rust.clone(),
                    });
                    let dispatch = self.xsi_dispatch(name, &variants);
                    out.push(XmlType::Enum {
                        spec: name.clone(),
                        rust,
                        generics,
                        variants: vs,
                        dispatch,
                    });
                }
                Emission::Enum(variants) => {
                    let dispatch = self.xsi_dispatch(name, &variants);
                    out.push(XmlType::Enum {
                        spec: name.clone(),
                        rust,
                        generics,
                        variants: variants
                            .iter()
                            .map(|v| XmlVariant {
                                ident: naming::type_name(v),
                            })
                            .collect(),
                        dispatch,
                    });
                }
                Emission::EnumLiterals(enumeration) => out.push(XmlType::EnumLiterals {
                    spec: name.clone(),
                    rust,
                    string_backed: enumeration.underlying_type != "INTEGER",
                }),
                Emission::Newtype(_) => out.push(XmlType::Newtype {
                    spec: name.clone(),
                    rust,
                }),
                Emission::Skip => {}
            }
        }
        out
    }
}
