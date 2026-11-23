// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 3, plan: decides the Rust shape each analysed class emits as and the
//! XML shape it classifies as. The decision data lives in [`overrides`],
//! [`composition`] and [`construction`]; [`crate::render`] writes the text.

pub(crate) mod composition;
pub(crate) mod construction;
pub(crate) mod drafts;
pub(crate) mod elided_arguments;
pub(crate) mod its_json_bounds;
pub(crate) mod layout;
pub(crate) mod ordered_hashes;
pub(crate) mod overrides;
pub(crate) mod rest_tagged;
pub(crate) mod rest_text_types;
pub(crate) mod xml_overrides;

use crate::analyze::Model;
use crate::load::bmm::{BmmClass, BmmEnumeration, BmmPropKind, BmmSchema, BmmType};
use crate::plan::overrides::{
    ContradictionKind, back_reference, contradicted, field_default, primitive,
};
use crate::plan::xml_overrides::xml_wire_rename;
use crate::render::naming;
use std::collections::BTreeSet;

/// What to emit for a class.
pub(crate) enum Emission<'a> {
    Struct,
    /// Untagged enum for an *abstract* polymorphic slot: one variant per
    /// immediate concrete subtype (the abstract class itself is not instantiable).
    Enum(Vec<String>),
    /// A concrete class with subtypes (`DV_TEXT` → `DV_CODED_TEXT`): a
    /// `{Name}Data` struct plus a `{Name}` enum over it and each concrete subtype.
    PolyEnum(Vec<String>),
    /// A BMM enumeration class: a Rust enum over its constants plus an `Other`
    /// catch-all.
    EnumLiterals(&'a BmmEnumeration),
    /// A transparent newtype over a Rust primitive.
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
    // The enumeration facet wins first: the RM BMM lists `PROPORTION_KIND` among
    // `DV_PROPORTION.ancestors`.
    if let Some(enumeration) = &class.enumeration {
        return Emission::EnumLiterals(enumeration);
    }
    if class.is_abstract {
        let variants = model.enum_variants(&class.name);
        if !variants.is_empty() {
            // Emitted whenever there are concrete descendants: a downstream crate may
            // use it as a field type.
            Emission::Enum(variants)
        } else if used.contains(&class.name) {
            // Abstract and referenced, with its concretes in another schema
            // (`AUTHORED_RESOURCE` in BASE): its own fields as a struct.
            Emission::Struct
        } else {
            Emission::Skip
        }
    } else {
        // A field typed as this class also accepts its subtypes.
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

// XML view of the generated types for `emit_xml`; the wire shape comes from
// the XSD reader.

/// One field of an instantiable type: `wire_name` is the openEHR property
/// name, `rust_name` the accessor, `target` the value's spec type.
pub(crate) struct XmlField {
    pub wire_name: String,
    pub rust_name: String,
    /// The Rust field is `Option<…>`; an optional container also sets
    /// [`XmlField::multiple`].
    pub optional: bool,
    pub multiple: bool,
    /// The container is `NonEmptyVec<T>`, read through its fallible constructor.
    pub nonempty: bool,
    pub target: String,
    /// For a `Hash<String, V>` field, the value type's spec name.
    pub map_value: Option<String>,
    /// The field's default expression ([`field_default`]) for an absent mandatory
    /// element.
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
        /// `xsi:type` dispatch: every concrete descendant spec → the direct variant
        /// it routes into.
        dispatch: Vec<(String, String)>,
    },
    /// A transparent newtype over a primitive, written as element text.
    Newtype { spec: String, rust: String },
    /// A BMM enumeration emitted as a typed enum, written as its wire token or
    /// integer.
    EnumLiterals {
        spec: String,
        rust: String,
        /// `true` for a STRING-underlying enum (`as_str`/`from_wire`), `false`
        /// for an INTEGER-underlying enum (`value`/`from_value`).
        string_backed: bool,
    },
}

// JSON view of the generated types for `emit_json`: wire name, accessor and
// omission kind.

/// One field of an instantiable type on the canonical-JSON wire.
pub(crate) struct JsonField {
    pub wire_name: String,
    pub rust_name: String,
    pub kind: JsonFieldKind,
    /// The field's default expression ([`field_default`]) for an omitted
    /// mandatory field.
    pub default: Option<String>,
    /// An [`JsonFieldKind::OptionalContainer`] whose present-but-empty array
    /// the reader refuses ([`its_json_bounds::ITS_JSON_MIN_ITEMS`]).
    pub refuses_empty: bool,
}

/// How a field is written on serialize.
pub(crate) enum JsonFieldKind {
    /// `Option<T>` — omitted when `None`.
    Optional,
    /// A mandatory container (`Vec<T>`, or an adjudicated `Hash`): read as empty
    /// when absent, omitted when empty.
    Container,
    /// `NonEmptyVec<T>`: always present, an absent or empty array refused at
    /// parse.
    NonEmptyContainer,
    /// `Option<Vec<T>>`: omitted when `None` or empty
    /// (`overview/Resources.md` §JSON Format), read back as `None` or
    /// `Some(vec![])`.
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
    /// An untagged enum, dispatched per [`JsonEnumDispatch`].
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

/// How a [`JsonType::Enum`] selects a variant on deserialize.
pub(crate) enum JsonEnumDispatch {
    /// `_type`-keyed dispatch: `arms` maps each concrete descendant to its direct
    /// variant; `self_ident` is the default for a `_type`-less value on a concrete
    /// slot.
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
    /// The flattened fields of a concrete class for canonical JSON, in struct
    /// order, back-references omitted.
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
                // A byte buffer is a `String`, never a container.
                let octet = matches!(&p.kind,
                    BmmPropKind::Container { item, .. } if item.root_name() == "Octet");
                let multiple = matches!(&p.kind, BmmPropKind::Container { .. }) && !octet;
                let lower_bound_one = matches!(&p.kind,
                    BmmPropKind::Container { cardinality, .. }
                        if cardinality.as_ref().is_some_and(|c| c.lower >= 1))
                    && !contradicted(&rp.owner, &p.name, ContradictionKind::LowerBound);
                let kind = if multiple {
                    match (p.is_mandatory, lower_bound_one) {
                        (true, true) => JsonFieldKind::NonEmptyContainer,
                        (true, false) => JsonFieldKind::Container,
                        (false, _) => JsonFieldKind::OptionalContainer,
                    }
                } else if p.is_mandatory
                    && contradicted(&rp.owner, &p.name, ContradictionKind::Existence)
                {
                    // A contradicted `1..1` `Hash`: absent reads as empty, empty is omitted.
                    JsonFieldKind::Container
                } else if p.is_mandatory {
                    JsonFieldKind::Plain
                } else {
                    JsonFieldKind::Optional
                };
                JsonField {
                    wire_name: p.name.clone(),
                    rust_name: naming::field_ident(&p.name),
                    kind,
                    default: field_default(&rp.owner, p, |name| self.get(name)),
                    refuses_empty: its_json_bounds::refuses_empty(&rp.owner, &p.name),
                }
            })
            .collect()
    }

    /// The instantiable canonical-JSON types of a schema, in class order.
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

    /// Decides how a canonical-JSON enum deserializes: `_type` dispatch when every
    /// concrete target carries a `_type`, else the structural fallback.
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
            // A back-reference is not a struct field; its XSD element is `minOccurs=0`.
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
                    wire_name: xml_wire_rename(&rp.owner, &p.name)
                        .map_or_else(|| p.name.clone(), str::to_owned),
                    rust_name: naming::field_ident(&p.name),
                    optional: !p.is_mandatory,
                    multiple,
                    nonempty: multiple
                        && ((p.is_mandatory
                            && matches!(&p.kind,
                                BmmPropKind::Container { cardinality, .. }
                                    if cardinality.as_ref().is_some_and(|c| c.lower >= 1))
                            && !contradicted(&rp.owner, &p.name, ContradictionKind::LowerBound))
                            // An optional container under a present-implies-non-empty invariant,
                            // possibly declared on an ancestor.
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
                    default: field_default(&rp.owner, p, |name| self.get(name)),
                }
            })
            .collect()
    }

    /// The `xsi:type` dispatch map for an enum: every concrete descendant spec →
    /// the direct variant it routes into.
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

    /// Whether a concrete class carries a `_type` on the canonical-JSON wire.
    pub(crate) fn concrete_carries_type(&self, name: &str) -> bool {
        let Some(class) = self.get(name) else {
            return false;
        };
        // An enumeration is a bare scalar; checked first, since `PROPORTION_KIND` has
        // a concrete descendant in the RM BMM.
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
