// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The P_BMM schema reader: ODIN `.bmm` text → a typed `P_BMM_SCHEMA` object
//! graph.
//!
//! "A BMM schema is a serialisation of the `P_BMM_*` object graph … The ODIN
//! form is described here. The structures are direct ODIN serialisations of the
//! `P_BMM_XXX` classes in the `persistence` package"
//! (`LANG/docs/bmm_persistence/master04-syntax.adoc` §Overview +
//! §Serialisation Formats). This module implements exactly that reading: it
//! parses the text with [`crate::v1_1::odin::parse`] and walks the resulting value
//! tree into the generated `P_BMM_*` types, attribute by attribute, per the
//! class docs under
//! `LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.*.adoc`.
//!
//! It is the first of the three stages `master02-overview.adoc` §Conceptual
//! Approach describes — "A schema reading component has to resolve the schema
//! inclusions and ultimately `BMM_*` object instantiations to obtain the
//! in-memory form of the model": read
//! ([`read_schema`]) → resolve inclusions
//! ([`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`]) →
//! instantiate ([`crate::v1_1::bmm_persistence::create_model::create_bmm_model`]).
//!
//! **Strict read.** An attribute the P_BMM class docs do not declare is a typed
//! [`PBmmReadError::UnknownAttribute`], never silently dropped — with the two
//! adjudicated tolerances [`TOLERATED_SCHEMA_ATTRIBUTES`] and
//! [`TOLERATED_PROPERTY_ATTRIBUTES`], each carrying its reason.
//!
//! **In-memory-only attributes are not read.** `master03-model.adoc` §Overview:
//! "attributes named `_bmm_xxx_` and of type `BMM_XXX` … are in-memory only
//! references to reconstructed instances", so they are left `None` here and
//! [`crate::v1_1::bmm_persistence::create_model`] reconstructs them.
//!
//! Two attributes documented as set by processing ARE stamped here, because
//! their inputs exist only while reading the file:
//! `P_BMM_CLASS.source_schema_id` takes this schema's own `schema_id`, and
//! `P_BMM_CLASS.uid` is numbered from 1 in document order over `primitive_types`
//! then `class_definitions`. A `(P_BMM_INTERFACE)` entry declares neither
//! (`…p_bmm_interface.adoc` §Attributes) and stamps neither, leaving the
//! remaining `uid`s document-ordered and unique.

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use indexmap::IndexMap;
use openehr_base::v1_3::prelude::Interval;
use openehr_base::v1_3::prelude::ProperInterval;
use openehr_base::v1_3::prelude::ProperIntervalData;

use crate::v1_1::bmm::core::bmm_include_spec::BmmIncludeSpec;
use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClassData;
use crate::v1_1::bmm_persistence::p_bmm_constant::PBmmConstant;
use crate::v1_1::bmm_persistence::p_bmm_container_function_parameter::PBmmContainerFunctionParameter;
use crate::v1_1::bmm_persistence::p_bmm_container_property::PBmmContainerProperty;
use crate::v1_1::bmm_persistence::p_bmm_container_property::PBmmContainerPropertyData;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerTypeData;
use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumeration;
use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumerationData;
use crate::v1_1::bmm_persistence::p_bmm_enumeration_integer::PBmmEnumerationInteger;
use crate::v1_1::bmm_persistence::p_bmm_enumeration_string::PBmmEnumerationString;
use crate::v1_1::bmm_persistence::p_bmm_function::PBmmFunction;
use crate::v1_1::bmm_persistence::p_bmm_function_parameter::PBmmFunctionParameter;
use crate::v1_1::bmm_persistence::p_bmm_generic_function_parameter::PBmmGenericFunctionParameter;
use crate::v1_1::bmm_persistence::p_bmm_generic_parameter::PBmmGenericParameter;
use crate::v1_1::bmm_persistence::p_bmm_generic_property::PBmmGenericProperty;
use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
use crate::v1_1::bmm_persistence::p_bmm_indexed_container_property::PBmmIndexedContainerProperty;
use crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType;
use crate::v1_1::bmm_persistence::p_bmm_interface::PBmmInterface;
use crate::v1_1::bmm_persistence::p_bmm_open_type::PBmmOpenType;
use crate::v1_1::bmm_persistence::p_bmm_package::PBmmPackage;
use crate::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::p_bmm_schema_impl::compose_schema_id;
use crate::v1_1::bmm_persistence::p_bmm_simple_type::PBmmSimpleType;
use crate::v1_1::bmm_persistence::p_bmm_single_function_parameter::PBmmSingleFunctionParameter;
use crate::v1_1::bmm_persistence::p_bmm_single_function_parameter_open::PBmmSingleFunctionParameterOpen;
use crate::v1_1::bmm_persistence::p_bmm_single_property::PBmmSingleProperty;
use crate::v1_1::bmm_persistence::p_bmm_single_property_open::PBmmSinglePropertyOpen;
use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;
use crate::v1_1::odin::OdinInterval;
use crate::v1_1::odin::OdinKey;
use crate::v1_1::odin::OdinValue;
use openehr_base::containers::present;

/// Schema-level attributes the pinned P_BMM model does not declare, accepted
/// and discarded.
///
/// `model_name` is written by `master04-syntax.adoc` §Header Items itself
/// (`model_name = <"TEST_PKG">`) and by the vendored openEHR `adltest` schema,
/// but neither `P_BMM_SCHEMA` nor `BMM_SCHEMA_CORE` declares it
/// (`org.openehr.lang.bmm_persistence.p_bmm_schema.adoc` +
/// `…bmm.bmm_schema_core.adoc` §Attributes); it is the v3
/// `BMM_MODEL.model_name` (`…bmm3.bmm_model.adoc` §Attributes), outside this
/// generation. Refusing a form the syntax chapter writes would reject valid
/// schema text, so it is tolerated.
pub const TOLERATED_SCHEMA_ATTRIBUTES: &[&str] = &["model_name"];

/// Property-level attributes the pinned P_BMM model does not declare, accepted
/// and discarded.
///
/// `default` appears on `(P_BMM_SINGLE_PROPERTY)` blocks throughout the
/// vendored openEHR BASE/RM/LANG/TERM ODIN schemas, but no attribute of
/// `P_BMM_PROPERTY` or `P_BMM_SINGLE_PROPERTY` corresponds to it
/// (`org.openehr.lang.bmm_persistence.p_bmm_property.adoc` +
/// `…p_bmm_single_property.adoc` §Attributes) and `master04-syntax.adoc` never
/// writes one. NOTE: no openEHR spec governs this attribute — accepting and
/// discarding it is our own decision, taken so the published openEHR schemas
/// read rather than being refused wholesale.
pub const TOLERATED_PROPERTY_ATTRIBUTES: &[&str] = &["default"];

/// Read a P_BMM schema from its ODIN serialisation.
///
/// The returned `P_BMM_SCHEMA` is the *unresolved* persisted form: its
/// `includes` are recorded but not merged (that is
/// [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`]) and its
/// `bmm_*` in-memory caches are `None` (that is
/// [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`]).
///
/// # Errors
/// Returns [`PBmmReadError::Odin`] when the text is not well-formed ODIN, and
/// one of the schema-shape variants (missing/unknown attribute, wrong value
/// shape, unexpected or missing type marker, key/name mismatch, qualified
/// nested package, unsupported cardinality) when the ODIN tree does not match
/// the P_BMM class model.
pub fn read_schema(src: &str) -> Result<PBmmSchema, PBmmReadError> {
    let root = crate::v1_1::odin::parse(src)?;
    let OdinValue::Object(members) = &root else {
        return Err(PBmmReadError::NotASchemaObject {
            found: kind_name(&root),
        });
    };
    let mut block = Block::new(String::new(), members);

    let bmm_version = block.required_string("bmm_version")?;
    let rm_publisher = block.required_string("rm_publisher")?;
    let schema_name = block.required_string("schema_name")?;
    let rm_release = block.required_string("rm_release")?;
    let schema_id = compose_schema_id(&rm_publisher, &schema_name, &rm_release);

    let schema_revision = block.documentation_string("schema_revision")?;
    let schema_lifecycle_state = block.documentation_string("schema_lifecycle_state")?;
    let schema_author = block.documentation_string("schema_author")?;
    let schema_description = block.documentation_string("schema_description")?;
    let schema_contributors = block.string_list("schema_contributors")?;
    let archetype_parent_class = block.optional_string("archetype_parent_class")?;
    let archetype_data_value_parent_class =
        block.optional_string("archetype_data_value_parent_class")?;
    let archetype_rm_closure_packages = block.string_list("archetype_rm_closure_packages")?;
    let archetype_visualise_descendants_of = block.visualise_descendants_of()?;

    let includes = read_includes(&mut block)?;
    let mut uid: i32 = 0;
    let primitive_types = read_class_list(&mut block, "primitive_types", &schema_id, &mut uid)?;
    let class_definitions = read_class_list(&mut block, "class_definitions", &schema_id, &mut uid)?;
    let packages = read_packages(&mut block, "packages", PackageLevel::Top)?;
    block.finish(TOLERATED_SCHEMA_ATTRIBUTES)?;

    Ok(PBmmSchema {
        packages,
        rm_publisher,
        rm_release,
        schema_name,
        schema_revision,
        schema_lifecycle_state,
        schema_author,
        schema_description,
        schema_contributors: present(schema_contributors),
        archetype_parent_class,
        archetype_data_value_parent_class,
        archetype_rm_closure_packages: present(archetype_rm_closure_packages),
        archetype_visualise_descendants_of,
        bmm_version,
        includes,
        primitive_types: present(primitive_types),
        class_definitions: present(class_definitions),
    })
}

/// Whether a package block sits at the schema's top level, where "only
/// top-level package ids can be paths"
/// (`master04-syntax.adoc` §Package Definition, first NOTE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageLevel {
    /// A package declared directly under the schema's `packages` attribute.
    Top,
    /// A package declared under another package's `packages` attribute.
    Nested,
}

/// The ODIN value kind, for error text.
fn kind_name(value: &OdinValue) -> &'static str {
    match value {
        OdinValue::Object(_) => "an attribute object",
        OdinValue::KeyedList(_) => "a keyed list",
        OdinValue::List(_) => "a list",
        OdinValue::Typed { .. } => "a type-marked value",
        OdinValue::PathList(_) | OdinValue::Path(_) => "an object reference",
        OdinValue::Empty => "an empty block",
        OdinValue::String(_) => "a string",
        OdinValue::Integer(_) => "an integer",
        OdinValue::Real(_) => "a real",
        OdinValue::Boolean(_) => "a boolean",
        OdinValue::Character(_) => "a character",
        OdinValue::Date(_) => "a date",
        OdinValue::Time(_) => "a time",
        OdinValue::DateTime(_) => "a date/time",
        OdinValue::Duration(_) => "a duration",
        OdinValue::Interval(_) => "an interval",
        OdinValue::TermCode(_) => "a term code",
        OdinValue::Uri(_) => "a URI",
        OdinValue::PlugIn { .. } => "a plug-in-syntax block",
        OdinValue::ListContinue => "a list-continuation marker",
    }
}

/// A keyed-list key as written, without ODIN quoting.
fn key_text(key: &OdinKey) -> String {
    match key {
        OdinKey::String(text)
        | OdinKey::Date(text)
        | OdinKey::Time(text)
        | OdinKey::DateTime(text) => text.clone(),
        OdinKey::Integer(value) => value.to_string(),
    }
}

/// Joins an ODIN attribute path segment onto a parent path.
fn join_path(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_owned()
    } else {
        format!("{parent}/{segment}")
    }
}

/// One ODIN attribute object being read, tracking which of its members have been
/// consumed so [`Block::finish`] can refuse the rest.
struct Block<'a> {
    /// ODIN attribute path of this block, for error reporting.
    path: String,
    /// The block's members, in document order.
    members: &'a IndexMap<String, OdinValue>,
    /// Names consumed so far.
    consumed: BTreeSet<&'a str>,
}

impl<'a> Block<'a> {
    /// A block over `members` at `path`.
    fn new(path: String, members: &'a IndexMap<String, OdinValue>) -> Self {
        Self {
            path,
            members,
            consumed: BTreeSet::new(),
        }
    }

    /// The block's members, if `value` is an attribute object (or an empty
    /// block, which reads as no members).
    fn open(
        path: &str,
        value: &'a OdinValue,
        empty: &'a IndexMap<String, OdinValue>,
    ) -> Result<Self, PBmmReadError> {
        match value {
            OdinValue::Object(members) => Ok(Self::new(path.to_owned(), members)),
            OdinValue::Empty => Ok(Self::new(path.to_owned(), empty)),
            other => Err(PBmmReadError::WrongValueShape {
                path: path.to_owned(),
                expected: "an attribute object",
                found: kind_name(other),
            }),
        }
    }

    /// Marks `name` consumed and returns its value, if present.
    fn take(&mut self, name: &'static str) -> Option<&'a OdinValue> {
        self.consumed.insert(name);
        self.members.get(name)
    }

    /// The path of member `name`.
    fn member_path(&self, name: &str) -> String {
        join_path(&self.path, name)
    }

    /// A mandatory (`1..1`) `String` attribute.
    fn required_string(&mut self, name: &'static str) -> Result<String, PBmmReadError> {
        let path = self.member_path(name);
        match self.take(name) {
            Some(value) => as_string(&path, value),
            None => Err(PBmmReadError::MissingAttribute {
                path: self.path.clone(),
                attribute: name,
            }),
        }
    }

    /// An optional (`0..1`) `String` attribute.
    fn optional_string(&mut self, name: &'static str) -> Result<Option<String>, PBmmReadError> {
        let path = self.member_path(name);
        match self.take(name) {
            Some(value) => as_string(&path, value).map(Some),
            None => Ok(None),
        }
    }

    /// A `String` attribute the class docs declare `1..1` but
    /// `master04-syntax.adoc` §Header Items treats as optional.
    ///
    /// NOTE: `BMM_SCHEMA_CORE` declares the four documentation headers `1..1`
    /// (`org.openehr.lang.bmm.bmm_schema_core.adoc` §Attributes) yet the
    /// spec's own §Header Items example and most published schemas omit
    /// `schema_author` — an absent documentation header reads as the empty
    /// string rather than refusing published schema text; the four
    /// IDENTIFYING headers stay mandatory (`schema_id` derives from them).
    fn documentation_string(&mut self, name: &'static str) -> Result<String, PBmmReadError> {
        Ok(self.optional_string(name)?.unwrap_or_default())
    }

    /// `BMM_SCHEMA_CORE.archetype_visualise_descendants_of`, accepting the
    /// American spelling the vendored `TestBmm1` schema writes.
    ///
    /// NOTE (adjudicated): the spec attribute is
    /// `archetype_visualise_descendants_of`
    /// (`org.openehr.lang.bmm.bmm_schema_core.adoc` §Attributes); the
    /// `archetype_visualize_descendants_of` spelling found in schema text is
    /// read onto the same attribute rather than refused, since the two differ
    /// only in orthography and no second attribute exists to hold it.
    fn visualise_descendants_of(&mut self) -> Result<Option<String>, PBmmReadError> {
        let spec_spelling = self.optional_string("archetype_visualise_descendants_of")?;
        let variant_spelling = self.optional_string("archetype_visualize_descendants_of")?;
        Ok(spec_spelling.or(variant_spelling))
    }

    /// An optional attribute holding a scalar literal in its persisted
    /// (serialised) form.
    ///
    /// `P_BMM_CONSTANT.value` is "The literal value of this constant, in its
    /// persisted (serialised) form" typed `String`
    /// (`org.openehr.lang.bmm_persistence.p_bmm_constant.adoc` §Attributes), and
    /// `master04-syntax.adoc` §Constants writes it quoted
    /// (`value = <"local">`). The vendored openEHR BASE schemas also write bare
    /// ODIN scalars there (`value = <60>` on `Time_Definitions`), which are the
    /// serialised form of a non-`String` constant, so any scalar is accepted and
    /// its literal text taken.
    fn literal_text(&mut self, name: &'static str) -> Result<Option<String>, PBmmReadError> {
        let path = self.member_path(name);
        match self.take(name) {
            None => Ok(None),
            Some(value) => literal_text_of(&path, value).map(Some),
        }
    }

    /// An optional (`0..1`) `Boolean` attribute.
    fn optional_bool(&mut self, name: &'static str) -> Result<Option<bool>, PBmmReadError> {
        let path = self.member_path(name);
        match self.take(name) {
            Some(OdinValue::Boolean(flag)) => Ok(Some(*flag)),
            Some(other) => Err(PBmmReadError::WrongValueShape {
                path,
                expected: "a boolean",
                found: kind_name(other),
            }),
            None => Ok(None),
        }
    }

    /// A `List<String>` attribute, empty when absent.
    fn string_list(&mut self, name: &'static str) -> Result<Vec<String>, PBmmReadError> {
        let path = self.member_path(name);
        match self.take(name) {
            Some(value) => string_list_of(&path, value),
            None => Ok(Vec::new()),
        }
    }

    /// A `Hash<String, String>` attribute (assertion or alias tables), `None`
    /// when absent.
    fn string_map(
        &mut self,
        name: &'static str,
    ) -> Result<Option<BTreeMap<String, String>>, PBmmReadError> {
        let path = self.member_path(name);
        let Some(value) = self.take(name) else {
            return Ok(None);
        };
        let mut out = BTreeMap::new();
        for (key, item) in keyed_entries(&path, value)? {
            let key = key_text(key);
            let entry_path = join_path(&path, &key);
            let text = as_string(&entry_path, item)?;
            out.insert(key, text);
        }
        Ok(Some(out))
    }

    /// A `cardinality` attribute — an ODIN integer range
    /// (`master04-syntax.adoc` §Container Properties: "The optional
    /// `_cardinality_` meta-property indicates cardinality of the container, and
    /// is expressed as a ODIN range").
    fn cardinality(&mut self) -> Result<Option<Interval<i32>>, PBmmReadError> {
        let path = self.member_path("cardinality");
        match self.take("cardinality") {
            Some(OdinValue::Interval(interval)) => integer_interval(&path, interval).map(Some),
            Some(other) => Err(PBmmReadError::WrongValueShape {
                path,
                expected: "an interval",
                found: kind_name(other),
            }),
            None => Ok(None),
        }
    }

    /// Refuses any member neither consumed nor listed in `tolerated`.
    fn finish(&self, tolerated: &[&str]) -> Result<(), PBmmReadError> {
        for name in self.members.keys() {
            if self.consumed.contains(name.as_str()) || tolerated.contains(&name.as_str()) {
                continue;
            }
            return Err(PBmmReadError::UnknownAttribute {
                path: self.path.clone(),
                attribute: name.clone(),
            });
        }
        Ok(())
    }
}

/// A `String` leaf.
fn as_string(path: &str, value: &OdinValue) -> Result<String, PBmmReadError> {
    match value {
        OdinValue::String(text) => Ok(text.clone()),
        other => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "a string",
            found: kind_name(other),
        }),
    }
}

/// One scalar ODIN leaf as its literal text (see [`Block::literal_text`]).
///
/// `True`/`False` are the ODIN boolean literals
/// (`LANG/docs/odin/master06-primitive_types` §Boolean).
fn literal_text_of(path: &str, value: &OdinValue) -> Result<String, PBmmReadError> {
    match value {
        OdinValue::String(text)
        | OdinValue::Date(text)
        | OdinValue::Time(text)
        | OdinValue::DateTime(text)
        | OdinValue::Duration(text)
        | OdinValue::TermCode(text)
        | OdinValue::Uri(text) => Ok(text.clone()),
        OdinValue::Integer(number) => Ok(number.to_string()),
        OdinValue::Real(number) => Ok(number.to_string()),
        OdinValue::Character(character) => Ok(character.to_string()),
        OdinValue::Boolean(true) => Ok("True".to_owned()),
        OdinValue::Boolean(false) => Ok("False".to_owned()),
        other => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "a scalar literal",
            found: kind_name(other),
        }),
    }
}

/// A `List<String>` leaf: a single string, a string list, or an empty block.
///
/// NOTE: a trailing ODIN list-continuation marker (`ancestors = <"Any", ...>`,
/// the form `master04-syntax.adoc` §Simple Classes and the vendored schemas
/// write) marks the list as open — `LANG/docs/odin/master05-content` §Container
/// Objects — and contributes no member, so it is dropped.
fn string_list_of(path: &str, value: &OdinValue) -> Result<Vec<String>, PBmmReadError> {
    match value {
        OdinValue::String(text) => Ok(vec![text.clone()]),
        OdinValue::Empty => Ok(Vec::new()),
        OdinValue::List(items) => items
            .iter()
            .filter(|item| !matches!(item, OdinValue::ListContinue))
            .map(|item| as_string(path, item))
            .collect(),
        other => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "a string list",
            found: kind_name(other),
        }),
    }
}

/// A `List<Any>` leaf of enumeration item values, as JSON scalars.
///
/// `P_BMM_ENUMERATION.item_values` is `List<Any>` (class doc §Attributes) and
/// `master04-syntax.adoc` §Enumerated Types writes integer and string members
/// (`item_values = <0, 1001, 1002, 1003>`, `item_values = <"<=", ">=", "=", "~">`).
fn any_list_of(path: &str, value: &OdinValue) -> Result<Vec<serde_json::Value>, PBmmReadError> {
    match value {
        OdinValue::Empty => Ok(Vec::new()),
        OdinValue::List(items) => items
            .iter()
            .filter(|item| !matches!(item, OdinValue::ListContinue))
            .map(|item| any_scalar(path, item))
            .collect(),
        single => any_scalar(path, single).map(|scalar| vec![scalar]),
    }
}

/// One enumeration item value as a JSON scalar.
fn any_scalar(path: &str, value: &OdinValue) -> Result<serde_json::Value, PBmmReadError> {
    match value {
        OdinValue::String(text) => Ok(serde_json::Value::String(text.clone())),
        OdinValue::Integer(number) => Ok(serde_json::Value::from(*number)),
        OdinValue::Boolean(flag) => Ok(serde_json::Value::Bool(*flag)),
        OdinValue::Character(character) => Ok(serde_json::Value::String(character.to_string())),
        OdinValue::Real(number) => serde_json::Number::from_f64(*number)
            .map(serde_json::Value::Number)
            .ok_or_else(|| PBmmReadError::WrongValueShape {
                path: path.to_owned(),
                expected: "a finite real",
                found: "a non-finite real",
            }),
        other => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "an enumeration item value",
            found: kind_name(other),
        }),
    }
}

/// The entries of a keyed container attribute (`["k"] = <…>`), or none for an
/// empty block.
fn keyed_entries<'a>(
    path: &str,
    value: &'a OdinValue,
) -> Result<&'a [(OdinKey, OdinValue)], PBmmReadError> {
    match value {
        OdinValue::KeyedList(entries) => Ok(entries.as_slice()),
        OdinValue::Empty => Ok(&[]),
        other => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "a keyed list",
            found: kind_name(other),
        }),
    }
}

/// An ODIN range as an `Interval<Integer>`.
///
/// A `None` endpoint is unbounded on that side (the representation
/// [`crate::v1_1::odin::OdinInterval`] already uses), so `|>=1|` reads as
/// `lower = 1, upper unbounded` and `|0..*|` as `lower = 0, upper unbounded`.
/// Always the `Proper_interval` form: `Multiplicity_interval` is "any two-sided
/// or one-sided interval" (`BASE/docs/foundation_types` `Proper_interval`), and
/// a point cardinality `|1|` is the two-sided `1..1`.
fn integer_interval(path: &str, interval: &OdinInterval) -> Result<Interval<i32>, PBmmReadError> {
    let OdinInterval::Range {
        lower,
        lower_included,
        upper,
        upper_included,
    } = interval
    else {
        return Err(PBmmReadError::UnsupportedCardinality {
            path: path.to_owned(),
        });
    };
    let lower = interval_bound(path, lower.as_deref())?;
    let upper = interval_bound(path, upper.as_deref())?;
    Ok(Interval::ProperInterval(ProperInterval::ProperInterval(
        ProperIntervalData {
            lower,
            upper,
            lower_unbounded: lower.is_none(),
            upper_unbounded: upper.is_none(),
            lower_included: *lower_included,
            upper_included: *upper_included,
        },
    )))
}

/// One interval endpoint as an `Integer`.
fn interval_bound(path: &str, bound: Option<&OdinValue>) -> Result<Option<i32>, PBmmReadError> {
    match bound {
        None => Ok(None),
        Some(OdinValue::Integer(number)) => i32::try_from(*number).map(Some).map_err(|_overflow| {
            PBmmReadError::UnsupportedCardinality {
                path: path.to_owned(),
            }
        }),
        Some(_) => Err(PBmmReadError::UnsupportedCardinality {
            path: path.to_owned(),
        }),
    }
}

/// The ODIN type marker on `value`, and the value it wraps.
///
/// `master04-syntax.adoc` §Serialisation Formats: "ODIN uses a type marker
/// `(P_BMM_SINGLE_PROPERTY)`"; "Where the type is unambiguous from context
/// (such as a class definition or a package), no discriminator is needed".
fn split_marker(value: &OdinValue) -> (Option<&str>, &OdinValue) {
    match value {
        OdinValue::Typed { rm_type, value } => (Some(rm_type.as_str()), value.as_ref()),
        plain => (None, plain),
    }
}

/// Reads the schema's `includes` attribute.
///
/// `P_BMM_SCHEMA.includes` is "Other schemas included by this schema, keyed by
/// schema id" (class doc §Attributes), while `master04-syntax.adoc`
/// §Inclusions writes the blocks under ordinal keys (`["1"] = < id = <"…"> >`)
/// and the vendored openEHR component schemas key them by the id itself.
///
/// NOTE (adjudicated): the map is therefore keyed by each block's own `id`, the
/// keying the class doc mandates; the ODIN key is positional in the §Inclusions
/// form and carries no information the `id` does not.
fn read_includes(
    block: &mut Block<'_>,
) -> Result<Option<BTreeMap<String, BmmIncludeSpec>>, PBmmReadError> {
    let path = block.member_path("includes");
    let Some(value) = block.take("includes") else {
        return Ok(None);
    };
    let empty = IndexMap::new();
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let entry_path = join_path(&path, &key_text(key));
        let mut entry_block = Block::open(&entry_path, entry, &empty)?;
        let id = entry_block.required_string("id")?;
        entry_block.finish(&[])?;
        out.insert(id.clone(), BmmIncludeSpec { id });
    }
    Ok(Some(out))
}

/// Reads a `packages` attribute into a `P_BMM_PACKAGE` tree.
///
/// `master04-syntax.adoc` §Package Definition: "just name the classes and
/// packages in a recursive fashion", with its three NOTEs enforced —
/// only a top-level package id may be a path
/// ([`PBmmReadError::QualifiedNestedPackage`]), and the ODIN key must equal the
/// block's `name` ([`PBmmReadError::KeyNameMismatch`]). The third NOTE (a
/// package may reference only classes the same schema defines) is schema-global
/// rather than local to one block, so it is enforced where the whole schema is
/// in scope — [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`]'s
/// [`PBmmReadError::ClassNotDefined`].
fn read_packages(
    block: &mut Block<'_>,
    name: &'static str,
    level: PackageLevel,
) -> Result<BTreeMap<String, PBmmPackage>, PBmmReadError> {
    let path = block.member_path(name);
    let Some(value) = block.take(name) else {
        return Ok(BTreeMap::new());
    };
    let empty = IndexMap::new();
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        let mut entry_block = Block::open(&entry_path, entry, &empty)?;
        let package_name = read_name(&mut entry_block, &key)?;
        if level == PackageLevel::Nested && package_name.contains('.') {
            return Err(PBmmReadError::QualifiedNestedPackage {
                path: entry_path,
                name: package_name,
            });
        }
        let documentation = entry_block.optional_string("documentation")?;
        let classes = entry_block.string_list("classes")?;
        let packages = read_packages(&mut entry_block, "packages", PackageLevel::Nested)?;
        entry_block.finish(&[])?;
        out.insert(
            key,
            PBmmPackage {
                packages,
                documentation,
                name: package_name,
                classes: present(classes),
                bmm_package_definition: None,
            },
        );
    }
    Ok(out)
}

/// The `name` attribute of a keyed block, defaulting to its ODIN key.
///
/// `master04-syntax.adoc` §Non-primitive Classes: "Since `name` is a BMM
/// meta-model attribute, the class definition always contains its ODIN key";
/// §Package Definition NOTE: "make sure that the ODIN 'keys' are the same as
/// the 'name' attributes in each block". A stated `name` that disagrees with the
/// key is a [`PBmmReadError::KeyNameMismatch`]; an omitted `name` takes the key,
/// which the chapter says it equals.
fn read_name(block: &mut Block<'_>, key: &str) -> Result<String, PBmmReadError> {
    match block.optional_string("name")? {
        None => Ok(key.to_owned()),
        Some(name) if name == key => Ok(name),
        Some(name) => Err(PBmmReadError::KeyNameMismatch {
            path: block.path.clone(),
            key: key.to_owned(),
            name,
        }),
    }
}

/// Reads a `primitive_types` or `class_definitions` attribute.
///
/// `master04-syntax.adoc` §Classes for Primitive Types: primitive-type
/// definitions "are just normal class definitions within a `primitive_types`
/// block", so both lists read identically; the distinction is carried into the
/// BMM model by `BMM_CLASS.is_primitive_type`.
fn read_class_list(
    block: &mut Block<'_>,
    name: &'static str,
    schema_id: &str,
    uid: &mut i32,
) -> Result<Vec<PBmmClass>, PBmmReadError> {
    let path = block.member_path(name);
    let Some(value) = block.take(name) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        *uid = uid.saturating_add(1);
        out.push(read_class(&entry_path, &key, entry, schema_id, *uid)?);
    }
    Ok(out)
}

/// The attributes every `P_BMM_CLASS` leaf carries, read once and distributed
/// over the concrete generated struct the type marker selects.
struct ClassParts {
    /// `P_BMM_MODEL_ELEMENT.documentation`.
    documentation: Option<String>,
    /// `P_BMM_CLASS.name`.
    name: String,
    /// `P_BMM_CLASS.ancestors` (optional container: `None` = the class
    /// declares no ancestors).
    ancestors: Option<Vec<String>>,
    /// `P_BMM_CLASS.constants`.
    constants: Option<BTreeMap<String, PBmmConstant>>,
    /// `P_BMM_CLASS.properties`.
    properties: Option<BTreeMap<String, PBmmProperty>>,
    /// `P_BMM_CLASS.functions`.
    functions: Option<BTreeMap<String, PBmmFunction>>,
    /// `P_BMM_CLASS.invariants`.
    invariants: Option<BTreeMap<String, String>>,
    /// `P_BMM_CLASS.is_abstract`.
    is_abstract: Option<bool>,
    /// `P_BMM_CLASS.is_override`.
    is_override: Option<bool>,
    /// `P_BMM_CLASS.generic_parameter_defs`.
    generic_parameter_defs: Option<BTreeMap<String, PBmmGenericParameter>>,
    /// `P_BMM_CLASS.source_schema_id`, stamped by the reader.
    source_schema_id: String,
    /// `P_BMM_CLASS.uid`, stamped by the reader.
    uid: i32,
    /// `P_BMM_CLASS.ancestor_defs` (optional container).
    ancestor_defs: Option<Vec<PBmmGenericType>>,
}

/// The enumeration-only attributes of `P_BMM_ENUMERATION`.
struct EnumerationParts {
    /// `P_BMM_ENUMERATION.item_names` (optional container).
    item_names: Option<Vec<String>>,
    /// `P_BMM_ENUMERATION.item_values` (optional container).
    item_values: Option<Vec<serde_json::Value>>,
    /// `P_BMM_ENUMERATION.item_documentations` (optional container).
    item_documentations: Option<Vec<String>>,
}

/// Reads one class definition, dispatching on its optional type marker.
///
/// A `(P_BMM_INTERFACE)`-marked entry reads into
/// [`PBmmClass::PBmmInterface`]: `master02-overview.adoc` §Conceptual Approach
/// says the model "can also represent pure interfaces via `P_BMM_INTERFACE`,
/// i.e. class-like definitions that declare only functions and carry no state",
/// and openEHR's own published schemas serialise them as members of
/// `class_definitions` (the vendored BASE 1.3.0 and RM 1.2.0 ODIN schemas mark
/// `Env`, `Locale`, `Math`, `Quantity_converter`, `Statistical_evaluator`,
/// `CODE_SET_ACCESS` and `TERMINOLOGY_ACCESS` that way). It declares exactly
/// three attributes — `name`, `documentation` (inherited) and `functions`
/// (`…p_bmm_interface.adoc` §Attributes) — so no other class attribute is read
/// for it.
fn read_class(
    path: &str,
    key: &str,
    value: &OdinValue,
    schema_id: &str,
    uid: i32,
) -> Result<PBmmClass, PBmmReadError> {
    let (marker, body) = split_marker(value);
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    if marker == Some("P_BMM_INTERFACE") {
        let name = read_name(&mut block, key)?;
        let documentation = block.optional_string("documentation")?;
        let functions = read_functions(&mut block)?;
        block.finish(&[])?;
        return Ok(PBmmClass::PBmmInterface(PBmmInterface {
            documentation,
            name,
            functions,
        }));
    }
    let parts = read_class_parts(&mut block, key, schema_id, uid)?;
    let class = match marker {
        None | Some("P_BMM_CLASS") => {
            block.finish(&[])?;
            PBmmClass::PBmmClass(class_data(parts))
        }
        Some("P_BMM_ENUMERATION") => {
            let items = read_enumeration_parts(&mut block)?;
            block.finish(&[])?;
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumeration(enumeration_data(
                parts, items,
            )))
        }
        Some("P_BMM_ENUMERATION_INTEGER") => {
            let items = read_enumeration_parts(&mut block)?;
            block.finish(&[])?;
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumerationInteger(
                enumeration_integer(parts, items),
            ))
        }
        Some("P_BMM_ENUMERATION_STRING") => {
            let items = read_enumeration_parts(&mut block)?;
            block.finish(&[])?;
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumerationString(enumeration_string(
                parts, items,
            )))
        }
        Some(other) => {
            return Err(PBmmReadError::UnexpectedTypeMarker {
                path: path.to_owned(),
                marker: other.to_owned(),
                expected: "P_BMM_CLASS",
            });
        }
    };
    Ok(class)
}

/// Reads the `P_BMM_CLASS` attribute set shared by every class form.
fn read_class_parts(
    block: &mut Block<'_>,
    key: &str,
    schema_id: &str,
    uid: i32,
) -> Result<ClassParts, PBmmReadError> {
    let name = read_name(block, key)?;
    Ok(ClassParts {
        documentation: block.optional_string("documentation")?,
        ancestors: present(block.string_list("ancestors")?),
        constants: read_constants(block)?,
        properties: read_properties(block)?,
        functions: read_functions(block)?,
        invariants: block.string_map("invariants")?,
        is_abstract: block.optional_bool("is_abstract")?,
        is_override: block.optional_bool("is_override")?,
        generic_parameter_defs: read_generic_parameter_defs(block)?,
        ancestor_defs: present(read_ancestor_defs(block)?),
        source_schema_id: schema_id.to_owned(),
        uid,
        name,
    })
}

/// Reads the `P_BMM_ENUMERATION` item lists.
fn read_enumeration_parts(block: &mut Block<'_>) -> Result<EnumerationParts, PBmmReadError> {
    let values_path = block.member_path("item_values");
    let item_values = match block.take("item_values") {
        Some(value) => any_list_of(&values_path, value)?,
        None => Vec::new(),
    };
    Ok(EnumerationParts {
        item_names: present(block.string_list("item_names")?),
        item_documentations: present(block.string_list("item_documentations")?),
        item_values: present(item_values),
    })
}

/// The least-rich `P_BMM_CLASS` form.
fn class_data(parts: ClassParts) -> PBmmClassData {
    PBmmClassData {
        documentation: parts.documentation,
        name: parts.name,
        ancestors: parts.ancestors,
        constants: parts.constants,
        properties: parts.properties,
        functions: parts.functions,
        invariants: parts.invariants,
        is_abstract: parts.is_abstract,
        is_override: parts.is_override,
        generic_parameter_defs: parts.generic_parameter_defs,
        source_schema_id: parts.source_schema_id,
        bmm_class: None,
        uid: parts.uid,
        ancestor_defs: parts.ancestor_defs,
    }
}

/// The least-rich `P_BMM_ENUMERATION` form.
fn enumeration_data(parts: ClassParts, items: EnumerationParts) -> PBmmEnumerationData {
    PBmmEnumerationData {
        documentation: parts.documentation,
        name: parts.name,
        ancestors: parts.ancestors,
        constants: parts.constants,
        properties: parts.properties,
        functions: parts.functions,
        invariants: parts.invariants,
        is_abstract: parts.is_abstract,
        is_override: parts.is_override,
        generic_parameter_defs: parts.generic_parameter_defs,
        source_schema_id: parts.source_schema_id,
        bmm_class: None,
        uid: parts.uid,
        ancestor_defs: parts.ancestor_defs,
        item_names: items.item_names,
        item_values: items.item_values,
        item_documentations: items.item_documentations,
    }
}

/// The `P_BMM_ENUMERATION_INTEGER` form.
fn enumeration_integer(parts: ClassParts, items: EnumerationParts) -> PBmmEnumerationInteger {
    PBmmEnumerationInteger {
        documentation: parts.documentation,
        name: parts.name,
        ancestors: parts.ancestors,
        constants: parts.constants,
        properties: parts.properties,
        functions: parts.functions,
        invariants: parts.invariants,
        is_abstract: parts.is_abstract,
        is_override: parts.is_override,
        generic_parameter_defs: parts.generic_parameter_defs,
        source_schema_id: parts.source_schema_id,
        bmm_class: None,
        uid: parts.uid,
        ancestor_defs: parts.ancestor_defs,
        item_names: items.item_names,
        item_values: items.item_values,
        item_documentations: items.item_documentations,
    }
}

/// The `P_BMM_ENUMERATION_STRING` form.
fn enumeration_string(parts: ClassParts, items: EnumerationParts) -> PBmmEnumerationString {
    PBmmEnumerationString {
        documentation: parts.documentation,
        name: parts.name,
        ancestors: parts.ancestors,
        constants: parts.constants,
        properties: parts.properties,
        functions: parts.functions,
        invariants: parts.invariants,
        is_abstract: parts.is_abstract,
        is_override: parts.is_override,
        generic_parameter_defs: parts.generic_parameter_defs,
        source_schema_id: parts.source_schema_id,
        bmm_class: None,
        uid: parts.uid,
        ancestor_defs: parts.ancestor_defs,
        item_names: items.item_names,
        item_values: items.item_values,
        item_documentations: items.item_documentations,
    }
}

/// Reads a class's `generic_parameter_defs`.
///
/// `master04-syntax.adoc` §Generic Classes: "the usual ODIN keyed hash
/// structure is used with each member being keyed by a generic parameter name".
fn read_generic_parameter_defs(
    block: &mut Block<'_>,
) -> Result<Option<BTreeMap<String, PBmmGenericParameter>>, PBmmReadError> {
    let path = block.member_path("generic_parameter_defs");
    let Some(value) = block.take("generic_parameter_defs") else {
        return Ok(None);
    };
    let empty = IndexMap::new();
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        let mut entry_block = Block::open(&entry_path, entry, &empty)?;
        let name = read_name(&mut entry_block, &key)?;
        let documentation = entry_block.optional_string("documentation")?;
        let conforms_to_type = entry_block.optional_string("conforms_to_type")?;
        entry_block.finish(&[])?;
        out.insert(
            key,
            PBmmGenericParameter {
                documentation,
                name,
                conforms_to_type,
                bmm_generic_parameter: None,
            },
        );
    }
    Ok(Some(out))
}

/// Reads a class's `ancestor_defs`.
///
/// `P_BMM_CLASS.ancestor_defs` is `List<P_BMM_GENERIC_TYPE>` (class doc
/// §Attributes) and `master04-syntax.adoc` §Inheritance uses it only "In the
/// case of generic inheritance", where "the ancestors are generic types"; the
/// keys are the rendered generic type signatures
/// (`["GENERIC_PARENT<T,SUPPLIER_B>"]`), not names, so they are not matched
/// against a `name` attribute.
fn read_ancestor_defs(block: &mut Block<'_>) -> Result<Vec<PBmmGenericType>, PBmmReadError> {
    let path = block.member_path("ancestor_defs");
    let Some(value) = block.take("ancestor_defs") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let entry_path = join_path(&path, &key_text(key));
        out.push(read_generic_type(&entry_path, entry)?);
    }
    Ok(out)
}

/// Reads a class's `constants`.
///
/// `master04-syntax.adoc` §Constants: "Each constant is a `P_BMM_CONSTANT`
/// carrying its `_type_` (a simple class name) and its literal `_value_` in
/// serialised form."
fn read_constants(
    block: &mut Block<'_>,
) -> Result<Option<BTreeMap<String, PBmmConstant>>, PBmmReadError> {
    let path = block.member_path("constants");
    let Some(value) = block.take("constants") else {
        return Ok(None);
    };
    let empty = IndexMap::new();
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        let mut entry_block = Block::open(&entry_path, entry, &empty)?;
        let name = read_name(&mut entry_block, &key)?;
        let documentation = entry_block.optional_string("documentation")?;
        let r#type = entry_block.required_string("type")?;
        let value = entry_block.literal_text("value")?;
        entry_block.finish(&[])?;
        out.insert(
            key,
            PBmmConstant {
                documentation,
                name,
                r#type,
                value,
            },
        );
    }
    Ok(Some(out))
}

/// Reads a class's `functions`.
///
/// `master04-syntax.adoc` §Functions: functions are "expressed as ODIN object
/// blocks under the `_functions_` keyword, keyed by function name … whose
/// formal parameters appear under `_parameters_`, keyed by parameter name, and
/// whose return type, if any, is stated in `_result_`. A function with no
/// `_result_` is a procedure."
fn read_functions(
    block: &mut Block<'_>,
) -> Result<Option<BTreeMap<String, PBmmFunction>>, PBmmReadError> {
    let path = block.member_path("functions");
    let Some(value) = block.take("functions") else {
        return Ok(None);
    };
    let empty = IndexMap::new();
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        let mut entry_block = Block::open(&entry_path, entry, &empty)?;
        let name = read_name(&mut entry_block, &key)?;
        let documentation = entry_block.optional_string("documentation")?;
        let aliases = read_aliases(&mut entry_block)?;
        let is_abstract = entry_block.optional_bool("is_abstract")?;
        let parameters = read_parameters(&mut entry_block)?;
        let pre_conditions = entry_block.string_map("pre_conditions")?;
        let post_conditions = entry_block.string_map("post_conditions")?;
        let result_path = entry_block.member_path("result");
        let result = match entry_block.take("result") {
            Some(value) => Some(read_type(&result_path, value)?),
            None => None,
        };
        let is_nullable = entry_block.optional_bool("is_nullable")?;
        entry_block.finish(&[])?;
        out.insert(
            key,
            PBmmFunction {
                documentation,
                name,
                aliases,
                is_abstract,
                parameters,
                pre_conditions,
                post_conditions,
                result,
                is_nullable,
            },
        );
    }
    Ok(Some(out))
}

/// Reads a function's `aliases`.
///
/// NOTE (adjudicated): `P_BMM_FUNCTION.aliases` is declared
/// `Hash<String, String>` "Optional alias names for this function, keyed by
/// alias" (class doc §Attributes), while the vendored openEHR schemas write it
/// as a plain ODIN string list (`aliases = <"=", "==">`). Each alias is
/// therefore entered under itself — the keying the class doc mandates — since
/// the list form carries no separate key.
fn read_aliases(block: &mut Block<'_>) -> Result<Option<BTreeMap<String, String>>, PBmmReadError> {
    let path = block.member_path("aliases");
    let Some(value) = block.take("aliases") else {
        return Ok(None);
    };
    if matches!(value, OdinValue::KeyedList(_)) {
        let mut out = BTreeMap::new();
        for (key, entry) in keyed_entries(&path, value)? {
            let key = key_text(key);
            let entry_path = join_path(&path, &key);
            let text = as_string(&entry_path, entry)?;
            out.insert(key, text);
        }
        return Ok(Some(out));
    }
    let aliases = string_list_of(&path, value)?;
    Ok(Some(
        aliases
            .into_iter()
            .map(|alias| (alias.clone(), alias))
            .collect(),
    ))
}

/// Reads a function's `parameters`.
///
/// `master04-syntax.adoc` §Functions: "Each parameter carries an ODIN type
/// marker indicating its `P_BMM_FUNCTION_PARAMETER` subtype, in direct analogy
/// with the property meta-types."
fn read_parameters(
    block: &mut Block<'_>,
) -> Result<Option<BTreeMap<String, PBmmFunctionParameter>>, PBmmReadError> {
    let path = block.member_path("parameters");
    let Some(value) = block.take("parameters") else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        out.insert(key.clone(), read_parameter(&entry_path, &key, entry)?);
    }
    Ok(Some(out))
}

/// Reads one function parameter.
fn read_parameter(
    path: &str,
    key: &str,
    value: &OdinValue,
) -> Result<PBmmFunctionParameter, PBmmReadError> {
    let (marker, body) = split_marker(value);
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    let name = read_name(&mut block, key)?;
    let documentation = block.optional_string("documentation")?;
    let is_nullable = block.optional_bool("is_nullable")?;
    let parameter = match marker {
        Some("P_BMM_SINGLE_FUNCTION_PARAMETER") => {
            let r#type = block.required_string("type")?;
            block.finish(&[])?;
            PBmmFunctionParameter::PBmmSingleFunctionParameter(PBmmSingleFunctionParameter {
                documentation,
                name,
                is_nullable,
                r#type,
            })
        }
        Some("P_BMM_SINGLE_FUNCTION_PARAMETER_OPEN") => {
            let r#type = block.required_string("type")?;
            block.finish(&[])?;
            PBmmFunctionParameter::PBmmSingleFunctionParameterOpen(
                PBmmSingleFunctionParameterOpen {
                    documentation,
                    name,
                    is_nullable,
                    r#type,
                },
            )
        }
        Some("P_BMM_CONTAINER_FUNCTION_PARAMETER") => {
            let type_def = read_required_container_type(&mut block)?;
            let cardinality = block.cardinality()?;
            block.finish(&[])?;
            PBmmFunctionParameter::PBmmContainerFunctionParameter(PBmmContainerFunctionParameter {
                documentation,
                name,
                is_nullable,
                type_def,
                cardinality,
            })
        }
        Some("P_BMM_GENERIC_FUNCTION_PARAMETER") => {
            let type_def_path = block.member_path("type_def");
            let Some(type_def) = block.take("type_def") else {
                return Err(PBmmReadError::MissingAttribute {
                    path: path.to_owned(),
                    attribute: "type_def",
                });
            };
            let type_def = read_generic_type(&type_def_path, type_def)?;
            block.finish(&[])?;
            PBmmFunctionParameter::PBmmGenericFunctionParameter(PBmmGenericFunctionParameter {
                documentation,
                name,
                is_nullable,
                type_def,
            })
        }
        Some(other) => {
            return Err(PBmmReadError::UnexpectedTypeMarker {
                path: path.to_owned(),
                marker: other.to_owned(),
                expected: "P_BMM_FUNCTION_PARAMETER",
            });
        }
        None => {
            return Err(PBmmReadError::MissingTypeMarker {
                path: path.to_owned(),
                expected: "P_BMM_FUNCTION_PARAMETER",
            });
        }
    };
    Ok(parameter)
}

/// Reads a mandatory `type_def` of `P_BMM_CONTAINER_TYPE`.
fn read_required_container_type(block: &mut Block<'_>) -> Result<PBmmContainerType, PBmmReadError> {
    let path = block.member_path("type_def");
    match block.take("type_def") {
        Some(value) => read_container_type(&path, value),
        None => Err(PBmmReadError::MissingAttribute {
            path: block.path.clone(),
            attribute: "type_def",
        }),
    }
}

/// Reads a class's `properties`.
///
/// `master04-syntax.adoc` §Class properties: "Class properties from the
/// original model are expressed using ODIN object blocks keyed by property
/// name. Since there are multiple possible descendants of `P_BMM_PROPERTY`,
/// ODIN type markers must be used to indicate which subtypes is used in each
/// case."
fn read_properties(
    block: &mut Block<'_>,
) -> Result<Option<BTreeMap<String, PBmmProperty>>, PBmmReadError> {
    let path = block.member_path("properties");
    let Some(value) = block.take("properties") else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let key = key_text(key);
        let entry_path = join_path(&path, &key);
        out.insert(key.clone(), read_property(&entry_path, &key, entry)?);
    }
    Ok(Some(out))
}

/// The attributes every `P_BMM_PROPERTY` leaf carries.
struct PropertyParts {
    /// `P_BMM_MODEL_ELEMENT.documentation`.
    documentation: Option<String>,
    /// `P_BMM_PROPERTY.name`.
    name: String,
    /// `P_BMM_PROPERTY.is_mandatory`.
    is_mandatory: Option<bool>,
    /// `P_BMM_PROPERTY.is_computed`.
    is_computed: Option<bool>,
    /// `P_BMM_PROPERTY.is_im_infrastructure`.
    is_im_infrastructure: Option<bool>,
    /// `P_BMM_PROPERTY.is_im_runtime`.
    is_im_runtime: Option<bool>,
}

/// Reads one class property.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per P_BMM_PROPERTY subtype; splitting the dispatch would hide the five-way marker → generated-struct mapping master04 §Class properties defines"
)]
fn read_property(path: &str, key: &str, value: &OdinValue) -> Result<PBmmProperty, PBmmReadError> {
    let (marker, body) = split_marker(value);
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    let parts = PropertyParts {
        name: read_name(&mut block, key)?,
        documentation: block.optional_string("documentation")?,
        is_mandatory: block.optional_bool("is_mandatory")?,
        is_computed: block.optional_bool("is_computed")?,
        is_im_infrastructure: block.optional_bool("is_im_infrastructure")?,
        is_im_runtime: block.optional_bool("is_im_runtime")?,
    };
    let property = match marker {
        Some("P_BMM_SINGLE_PROPERTY") => {
            let r#type = block.optional_string("type")?;
            let simple_ref = read_optional_simple_type(&mut block)?;
            let structural = read_optional_type(&mut block)?;
            block.finish(TOLERATED_PROPERTY_ATTRIBUTES)?;
            PBmmProperty::PBmmSingleProperty(PBmmSingleProperty {
                documentation: parts.documentation,
                name: parts.name,
                is_mandatory: parts.is_mandatory,
                is_computed: parts.is_computed,
                is_im_infrastructure: parts.is_im_infrastructure,
                is_im_runtime: parts.is_im_runtime,
                type_def: structural,
                bmm_property: None,
                r#type,
                type_ref: simple_ref,
            })
        }
        Some("P_BMM_SINGLE_PROPERTY_OPEN") => {
            let r#type = block.optional_string("type")?;
            let open_ref = read_optional_open_type(&mut block)?;
            let structural = read_optional_type(&mut block)?;
            block.finish(TOLERATED_PROPERTY_ATTRIBUTES)?;
            PBmmProperty::PBmmSinglePropertyOpen(PBmmSinglePropertyOpen {
                documentation: parts.documentation,
                name: parts.name,
                is_mandatory: parts.is_mandatory,
                is_computed: parts.is_computed,
                is_im_infrastructure: parts.is_im_infrastructure,
                is_im_runtime: parts.is_im_runtime,
                type_def: structural,
                bmm_property: None,
                type_ref: open_ref,
                r#type,
            })
        }
        Some("P_BMM_GENERIC_PROPERTY") => {
            let type_def_path = block.member_path("type_def");
            let type_def = match block.take("type_def") {
                Some(value) => Some(read_generic_type(&type_def_path, value)?),
                None => None,
            };
            block.finish(TOLERATED_PROPERTY_ATTRIBUTES)?;
            PBmmProperty::PBmmGenericProperty(PBmmGenericProperty {
                documentation: parts.documentation,
                name: parts.name,
                is_mandatory: parts.is_mandatory,
                is_computed: parts.is_computed,
                is_im_infrastructure: parts.is_im_infrastructure,
                is_im_runtime: parts.is_im_runtime,
                type_def,
                bmm_property: None,
            })
        }
        Some("P_BMM_CONTAINER_PROPERTY") => {
            let type_def_path = block.member_path("type_def");
            let type_def = match block.take("type_def") {
                Some(value) => Some(read_container_type(&type_def_path, value)?),
                None => None,
            };
            let cardinality = block.cardinality()?;
            block.finish(TOLERATED_PROPERTY_ATTRIBUTES)?;
            PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
                PBmmContainerPropertyData {
                    documentation: parts.documentation,
                    name: parts.name,
                    is_mandatory: parts.is_mandatory,
                    is_computed: parts.is_computed,
                    is_im_infrastructure: parts.is_im_infrastructure,
                    is_im_runtime: parts.is_im_runtime,
                    type_def,
                    bmm_property: None,
                    cardinality,
                },
            ))
        }
        Some("P_BMM_INDEXED_CONTAINER_PROPERTY") => {
            let type_def_path = block.member_path("type_def");
            let type_def = match block.take("type_def") {
                Some(value) => Some(read_indexed_container_type(&type_def_path, value)?),
                None => None,
            };
            let cardinality = block.cardinality()?;
            block.finish(TOLERATED_PROPERTY_ATTRIBUTES)?;
            PBmmProperty::PBmmContainerProperty(
                PBmmContainerProperty::PBmmIndexedContainerProperty(PBmmIndexedContainerProperty {
                    documentation: parts.documentation,
                    name: parts.name,
                    is_mandatory: parts.is_mandatory,
                    is_computed: parts.is_computed,
                    is_im_infrastructure: parts.is_im_infrastructure,
                    is_im_runtime: parts.is_im_runtime,
                    type_def,
                    bmm_property: None,
                    cardinality,
                }),
            )
        }
        Some(other) => {
            return Err(PBmmReadError::UnexpectedTypeMarker {
                path: path.to_owned(),
                marker: other.to_owned(),
                expected: "P_BMM_PROPERTY",
            });
        }
        None => {
            return Err(PBmmReadError::MissingTypeMarker {
                path: path.to_owned(),
                expected: "P_BMM_PROPERTY",
            });
        }
    };
    Ok(property)
}

/// Reads an optional `type_def` of the polymorphic `P_BMM_TYPE` slot.
fn read_optional_type(block: &mut Block<'_>) -> Result<Option<PBmmType>, PBmmReadError> {
    let path = block.member_path("type_def");
    match block.take("type_def") {
        Some(value) => read_type(&path, value).map(Some),
        None => Ok(None),
    }
}

/// Reads an optional `type_ref` of `P_BMM_SIMPLE_TYPE`.
///
/// `master04-syntax.adoc` §Value-set Constraints: "The use of the
/// `value_constraint` attribute forces the use of the `type_ref` structural form
/// of the type definition within a `P_BMM_SINGLE_PROPERTY` instance, rather than
/// the simple `String` form."
fn read_optional_simple_type(
    block: &mut Block<'_>,
) -> Result<Option<PBmmSimpleType>, PBmmReadError> {
    let path = block.member_path("type_ref");
    match block.take("type_ref") {
        Some(value) => read_simple_type(&path, value).map(Some),
        None => Ok(None),
    }
}

/// Reads an optional `type_ref` of `P_BMM_OPEN_TYPE`.
fn read_optional_open_type(block: &mut Block<'_>) -> Result<Option<PBmmOpenType>, PBmmReadError> {
    let path = block.member_path("type_ref");
    match block.take("type_ref") {
        Some(value) => read_open_type(&path, value).map(Some),
        None => Ok(None),
    }
}

/// Reads a `P_BMM_TYPE`, from its marker where stated and from its member set
/// otherwise.
///
/// `master04-syntax.adoc` §Serialisation Formats requires the discriminator
/// only "Wherever a value may be one of several `P_BMM_*` subtypes": the
/// chapter's own examples omit it on a `type_def` whose declaring attribute is
/// a concrete type (`§Container Properties`, `§Generic Classes`). Where the slot
/// IS polymorphic and no marker is written, the subtype is inferred from the
/// attributes present, using the chapter's own rules (§Generic Classes: "use
/// 'type' for simple string type refs; use `_type_def_` for structure types;
/// within `P_BMM_GENERIC_TYPE`, use `_generic_parameters_` for a list of string
/// types") — `container_type` ⇒ a container type (indexed when `index_type` is
/// present), `root_type` ⇒ a generic type, `type` alone ⇒ a simple type.
fn read_type(path: &str, value: &OdinValue) -> Result<PBmmType, PBmmReadError> {
    let (marker, body) = split_marker(value);
    match marker {
        Some("P_BMM_SIMPLE_TYPE") => read_simple_type(path, value).map(PBmmType::PBmmSimpleType),
        Some("P_BMM_OPEN_TYPE") => read_open_type(path, value).map(PBmmType::PBmmOpenType),
        Some("P_BMM_GENERIC_TYPE") => read_generic_type(path, value).map(PBmmType::PBmmGenericType),
        Some("P_BMM_CONTAINER_TYPE" | "P_BMM_INDEXED_CONTAINER_TYPE") => {
            read_container_type(path, value).map(PBmmType::PBmmContainerType)
        }
        Some(other) => Err(PBmmReadError::UnexpectedTypeMarker {
            path: path.to_owned(),
            marker: other.to_owned(),
            expected: "P_BMM_TYPE",
        }),
        None => match infer_type_kind(body) {
            Some(TypeKind::Container | TypeKind::IndexedContainer) => {
                read_container_type(path, value).map(PBmmType::PBmmContainerType)
            }
            Some(TypeKind::Generic) => {
                read_generic_type(path, value).map(PBmmType::PBmmGenericType)
            }
            Some(TypeKind::Simple) => read_simple_type(path, value).map(PBmmType::PBmmSimpleType),
            None => Err(PBmmReadError::MissingTypeMarker {
                path: path.to_owned(),
                expected: "P_BMM_TYPE",
            }),
        },
    }
}

/// Reads a `P_BMM_BASE_TYPE` — the non-container half of the type family
/// (`…p_bmm_base_type.adoc` §Description: "Persistent form of a proper
/// (non-container) BMM type").
fn read_base_type(path: &str, value: &OdinValue) -> Result<PBmmBaseType, PBmmReadError> {
    let (marker, body) = split_marker(value);
    match marker {
        Some("P_BMM_SIMPLE_TYPE") => {
            read_simple_type(path, value).map(PBmmBaseType::PBmmSimpleType)
        }
        Some("P_BMM_OPEN_TYPE") => read_open_type(path, value).map(PBmmBaseType::PBmmOpenType),
        Some("P_BMM_GENERIC_TYPE") => {
            read_generic_type(path, value).map(PBmmBaseType::PBmmGenericType)
        }
        Some(other) => Err(PBmmReadError::UnexpectedTypeMarker {
            path: path.to_owned(),
            marker: other.to_owned(),
            expected: "P_BMM_BASE_TYPE",
        }),
        None => match infer_type_kind(body) {
            Some(TypeKind::Generic) => {
                read_generic_type(path, value).map(PBmmBaseType::PBmmGenericType)
            }
            Some(TypeKind::Simple) => {
                read_simple_type(path, value).map(PBmmBaseType::PBmmSimpleType)
            }
            Some(TypeKind::Container | TypeKind::IndexedContainer) | None => {
                Err(PBmmReadError::MissingTypeMarker {
                    path: path.to_owned(),
                    expected: "P_BMM_BASE_TYPE",
                })
            }
        },
    }
}

/// Which `P_BMM_TYPE` subtype an unmarked type block's members indicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeKind {
    /// `container_type` + `index_type` present.
    IndexedContainer,
    /// `container_type` present.
    Container,
    /// `root_type` present.
    Generic,
    /// `type` present.
    Simple,
}

/// Infers the subtype of an unmarked type block from its members.
fn infer_type_kind(body: &OdinValue) -> Option<TypeKind> {
    let OdinValue::Object(members) = body else {
        return None;
    };
    if members.contains_key("container_type") {
        if members.contains_key("index_type") {
            return Some(TypeKind::IndexedContainer);
        }
        return Some(TypeKind::Container);
    }
    if members.contains_key("root_type") {
        return Some(TypeKind::Generic);
    }
    if members.contains_key("type") {
        return Some(TypeKind::Simple);
    }
    None
}

/// Reads a `P_BMM_SIMPLE_TYPE` (`type`, plus the optional
/// `P_BMM_BASE_TYPE.value_constraint`).
fn read_simple_type(path: &str, value: &OdinValue) -> Result<PBmmSimpleType, PBmmReadError> {
    let body = expect_marker(path, value, "P_BMM_SIMPLE_TYPE", &["P_BMM_SIMPLE_TYPE"])?;
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    let r#type = block.required_string("type")?;
    let value_constraint = block.optional_string("value_constraint")?;
    block.finish(&[])?;
    Ok(PBmmSimpleType {
        bmm_type: None,
        value_constraint,
        r#type,
    })
}

/// Reads a `P_BMM_OPEN_TYPE` (a generic parameter name, plus the optional
/// `value_constraint`).
fn read_open_type(path: &str, value: &OdinValue) -> Result<PBmmOpenType, PBmmReadError> {
    let body = expect_marker(path, value, "P_BMM_OPEN_TYPE", &["P_BMM_OPEN_TYPE"])?;
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    let r#type = block.required_string("type")?;
    let value_constraint = block.optional_string("value_constraint")?;
    block.finish(&[])?;
    Ok(PBmmOpenType {
        bmm_type: None,
        value_constraint,
        r#type,
    })
}

/// Reads a `P_BMM_GENERIC_TYPE`.
///
/// `master04-syntax.adoc` §Generic Classes: "`DV_INTERVAL` is the `_root_type_`";
/// "within `P_BMM_GENERIC_TYPE`, use `_generic_parameters_` for a list of
/// string types; use `_generic_parameter_defs_` for a list of complex type
/// references" — the latter "a logical list in general, since there can always
/// be more than one generic parameter", written as an ODIN keyed hash whose
/// document order is the declaration order the class doc requires.
fn read_generic_type(path: &str, value: &OdinValue) -> Result<PBmmGenericType, PBmmReadError> {
    let body = expect_marker(path, value, "P_BMM_GENERIC_TYPE", &["P_BMM_GENERIC_TYPE"])?;
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    let root_type = block.required_string("root_type")?;
    let (generic_parameters, mut generic_parameter_defs) = read_generic_parameters(&mut block)?;
    let defs_path = block.member_path("generic_parameter_defs");
    if let Some(defs) = block.take("generic_parameter_defs") {
        for (key, entry) in keyed_entries(&defs_path, defs)? {
            let entry_path = join_path(&defs_path, &key_text(key));
            generic_parameter_defs.push(read_type(&entry_path, entry)?);
        }
    }
    let value_constraint = block.optional_string("value_constraint")?;
    block.finish(&[])?;
    Ok(PBmmGenericType {
        bmm_type: None,
        value_constraint,
        root_type,
        generic_parameter_defs,
        generic_parameters: present(generic_parameters),
    })
}

/// Reads a generic type's `generic_parameters` attribute.
///
/// The spec form is a plain ODIN string list — "use `_generic_parameters_` for a
/// list of string types" (`master04-syntax.adoc` §Generic Classes) — and reads
/// into `P_BMM_GENERIC_TYPE.generic_parameters`.
///
/// NOTE: the vendored openEHR AM schemas also write an INTEGER-KEYED list
/// mixing plain names with type-marked blocks — no openEHR spec governs that
/// positional form — and it is read entirely into `generic_parameter_defs`
/// (bare names promoted to `P_BMM_SIMPLE_TYPE`), the only shape preserving
/// the declaration ORDER the class doc requires.
fn read_generic_parameters(
    block: &mut Block<'_>,
) -> Result<(Vec<String>, Vec<PBmmType>), PBmmReadError> {
    let path = block.member_path("generic_parameters");
    let Some(value) = block.take("generic_parameters") else {
        return Ok((Vec::new(), Vec::new()));
    };
    if !matches!(value, OdinValue::KeyedList(_)) {
        return Ok((string_list_of(&path, value)?, Vec::new()));
    }
    let mut defs = Vec::new();
    for (key, entry) in keyed_entries(&path, value)? {
        let entry_path = join_path(&path, &key_text(key));
        match entry {
            OdinValue::String(name) => defs.push(PBmmType::PBmmSimpleType(PBmmSimpleType {
                bmm_type: None,
                value_constraint: None,
                r#type: name.clone(),
            })),
            structured => defs.push(read_type(&entry_path, structured)?),
        }
    }
    Ok((Vec::new(), defs))
}

/// Reads a `P_BMM_CONTAINER_TYPE`, selecting the indexed form by marker or by
/// the presence of `index_type`.
fn read_container_type(path: &str, value: &OdinValue) -> Result<PBmmContainerType, PBmmReadError> {
    let (marker, body) = split_marker(value);
    match marker {
        Some("P_BMM_INDEXED_CONTAINER_TYPE") => read_indexed_container_type(path, value)
            .map(PBmmContainerType::PBmmIndexedContainerType),
        Some("P_BMM_CONTAINER_TYPE") | None => {
            if infer_type_kind(body) == Some(TypeKind::IndexedContainer) {
                return read_indexed_container_type(path, value)
                    .map(PBmmContainerType::PBmmIndexedContainerType);
            }
            let empty = IndexMap::new();
            let mut block = Block::open(path, body, &empty)?;
            let container_type = block.required_string("container_type")?;
            let r#type = block.optional_string("type")?;
            let type_def = read_optional_base_type(&mut block)?;
            block.finish(&[])?;
            Ok(PBmmContainerType::PBmmContainerType(
                PBmmContainerTypeData {
                    bmm_type: None,
                    container_type,
                    type_def,
                    r#type,
                },
            ))
        }
        Some(other) => Err(PBmmReadError::UnexpectedTypeMarker {
            path: path.to_owned(),
            marker: other.to_owned(),
            expected: "P_BMM_CONTAINER_TYPE",
        }),
    }
}

/// Reads a `P_BMM_INDEXED_CONTAINER_TYPE`.
///
/// `master04-syntax.adoc` §Container Properties: "The meta-element `index_type`
/// is used to state the key type."
fn read_indexed_container_type(
    path: &str,
    value: &OdinValue,
) -> Result<PBmmIndexedContainerType, PBmmReadError> {
    let body = expect_marker(
        path,
        value,
        "P_BMM_INDEXED_CONTAINER_TYPE",
        &["P_BMM_INDEXED_CONTAINER_TYPE"],
    )?;
    let empty = IndexMap::new();
    let mut block = Block::open(path, body, &empty)?;
    let container_type = block.required_string("container_type")?;
    let index_type = block.required_string("index_type")?;
    let r#type = block.optional_string("type")?;
    let type_def = read_optional_base_type(&mut block)?;
    block.finish(&[])?;
    Ok(PBmmIndexedContainerType {
        bmm_type: None,
        container_type,
        type_def,
        r#type,
        index_type,
    })
}

/// Reads a container type's nested `type_def` (a `P_BMM_BASE_TYPE`).
fn read_optional_base_type(block: &mut Block<'_>) -> Result<Option<PBmmBaseType>, PBmmReadError> {
    let path = block.member_path("type_def");
    match block.take("type_def") {
        Some(value) => read_base_type(&path, value).map(Some),
        None => Ok(None),
    }
}

/// Unwraps a type block whose ODIN marker, when present, must be one of
/// `allowed`.
fn expect_marker<'a>(
    path: &str,
    value: &'a OdinValue,
    expected: &'static str,
    allowed: &[&str],
) -> Result<&'a OdinValue, PBmmReadError> {
    let (marker, body) = split_marker(value);
    match marker {
        None => Ok(body),
        Some(found) if allowed.contains(&found) => Ok(body),
        Some(found) => Err(PBmmReadError::UnexpectedTypeMarker {
            path: path.to_owned(),
            marker: found.to_owned(),
            expected,
        }),
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "the Book ch11 test shape: `?` propagates the read/resolve/model plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
    )]
    use super::read_schema;
    use crate::v1_1::bmm_persistence::error::PBmmReadError;
    use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
    use crate::v1_1::bmm_persistence::p_bmm_container_property::PBmmContainerProperty;
    use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
    use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
    use crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType;
    use crate::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;
    use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
    use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;
    use openehr_base::v1_3::prelude::Interval;
    use openehr_base::v1_3::prelude::ProperInterval;

    /// The four identifying header items every fixture below needs.
    const HEADER: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"adltest">
        rm_release = <"1.0.2">
    "#;

    /// Reads `HEADER` plus `body`.
    fn read(body: &str) -> Result<PBmmSchema, PBmmReadError> {
        read_schema(&format!("{HEADER}{body}"))
    }

    /// The class definition named `name` in `schema`.
    fn class<'a>(schema: &'a PBmmSchema, name: &str) -> &'a PBmmClass {
        schema
            .class_definitions
            .iter()
            .flatten()
            .find(|class| class.name() == name)
            .expect("the fixture defines the class")
    }

    /// The property named `property` of the class named `class_name`.
    fn property<'a>(schema: &'a PBmmSchema, class_name: &str, property: &str) -> &'a PBmmProperty {
        class(schema, class_name)
            .properties()
            .expect("the class declares properties")
            .get(property)
            .expect("the class declares the property")
    }

    #[test]
    fn header_items_read_verbatim() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Header Items, verbatim (including `model_name`,
        // which P_BMM_SCHEMA does not declare and the reader tolerates).
        let schema = read_schema(
            r#"
            bmm_version = <"2.3">
            rm_publisher = <"openehr">
            schema_name = <"adltest">
            rm_release = <"1.0.2">
            model_name = <"TEST_PKG">
            schema_revision = <"1.0.36">
            schema_lifecycle_state = <"stable">
            schema_description = <"openEHR schema to support test archetypes">
        "#,
        )?;
        assert_eq!(schema.bmm_version, "2.3");
        assert_eq!(schema.schema_id(), "openehr_adltest_1.0.2");
        assert_eq!(schema.schema_revision, "1.0.36");
        assert_eq!(schema.schema_lifecycle_state, "stable");
        // Declared 1..1 but omitted by the chapter's own example.
        assert_eq!(schema.schema_author, "");
        Ok(())
    }

    #[test]
    fn a_missing_identifying_header_is_refused() {
        let error = read_schema(r#"bmm_version = <"2.4">"#).expect_err("rm_publisher is missing");
        assert_eq!(
            error,
            PBmmReadError::MissingAttribute {
                path: String::new(),
                attribute: "rm_publisher",
            }
        );
    }

    #[test]
    fn an_undeclared_attribute_is_refused_by_name() {
        let error = read(r#"not_a_bmm_attribute = <"x">"#).expect_err("the attribute is unknown");
        assert_eq!(
            error,
            PBmmReadError::UnknownAttribute {
                path: String::new(),
                attribute: "not_a_bmm_attribute".to_owned(),
            }
        );
    }

    #[test]
    fn includes_are_keyed_by_their_own_id() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Inclusions, verbatim.
        let schema = read(
            r#"
            includes = <
                ["1"] = <
                    id = <"openehr_basic_types_1.0.2">
                >
            >
        "#,
        )?;
        let includes = schema.includes.expect("the schema declares an include");
        assert_eq!(includes.len(), 1);
        assert_eq!(
            includes
                .get("openehr_basic_types_1.0.2")
                .map(|spec| spec.id.as_str()),
            Some("openehr_basic_types_1.0.2")
        );
        Ok(())
    }

    #[test]
    fn packages_read_recursively_and_only_top_level_ids_may_be_paths() -> Result<(), PBmmReadError>
    {
        // master04-syntax.adoc §Package Definition, verbatim.
        let schema = read(
            r#"
            packages = <
                ["org.openehr.test_pkg"] = <
                    name = <"org.openehr.test_pkg">
                    classes = <"WHOLE", "SOME_TYPE", "BOOK">
                >
            >
        "#,
        )?;
        let package = schema
            .packages
            .get("org.openehr.test_pkg")
            .expect("the top-level package reads");
        assert_eq!(package.classes.as_ref().map_or(0, Vec::len), 3);

        let error = read(
            r#"
            packages = <
                ["ParentPackage"] = <
                    name = <"ParentPackage">
                    packages = <
                        ["invalid.ChildPackage"] = <
                            name = <"invalid.ChildPackage">
                        >
                    >
                >
            >
        "#,
        )
        .expect_err("a nested package id may not be a path");
        assert!(
            matches!(error, PBmmReadError::QualifiedNestedPackage { .. }),
            "expected a qualified-nested-package refusal, got {error:?}"
        );
        Ok(())
    }

    #[test]
    fn a_key_that_disagrees_with_the_name_attribute_is_refused() {
        let error = read(
            r#"
            packages = <
                ["ParentPackage"] = <
                    name = <"OtherName">
                >
            >
        "#,
        )
        .expect_err("the key and name disagree");
        assert!(
            matches!(error, PBmmReadError::KeyNameMismatch { .. }),
            "expected a key/name refusal, got {error:?}"
        );
    }

    #[test]
    fn single_and_container_properties_read_the_master04_examples() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Class properties + §Container Properties.
        let schema = read(
            r#"
            class_definitions = <
                ["ELEMENT"] = <
                    name = <"ELEMENT">
                    ancestors = <"ITEM">
                    properties = <
                        ["null_flavour"] = (P_BMM_SINGLE_PROPERTY) <
                            name = <"null_flavour">
                            type = <"DV_CODED_TEXT">
                            is_mandatory = <True>
                        >
                        ["items"] = (P_BMM_CONTAINER_PROPERTY) <
                            name = <"items">
                            type_def = <
                                container_type = <"List">
                                type = <"ITEM">
                            >
                            cardinality = <|>=1|>
                            is_mandatory = <True>
                        >
                        ["custom_actions"] = (P_BMM_INDEXED_CONTAINER_PROPERTY) <
                            name = <"custom_actions">
                            type_def = <
                                container_type = <"Hash">
                                index_type = <"String">
                                type = <"EVENT_ACTION">
                            >
                            cardinality = <|>=0|>
                        >
                    >
                >
            >
        "#,
        )?;
        assert_eq!(class(&schema, "ELEMENT").ancestors(), ["ITEM"]);
        assert_eq!(class(&schema, "ELEMENT").uid(), Some(1));
        assert_eq!(
            class(&schema, "ELEMENT").source_schema_id(),
            Some("openehr_adltest_1.0.2")
        );

        let PBmmProperty::PBmmSingleProperty(single) = property(&schema, "ELEMENT", "null_flavour")
        else {
            panic!("null_flavour is a P_BMM_SINGLE_PROPERTY");
        };
        assert_eq!(single.r#type.as_deref(), Some("DV_CODED_TEXT"));
        assert_eq!(single.is_mandatory, Some(true));

        let PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
            container,
        )) = property(&schema, "ELEMENT", "items")
        else {
            panic!("items is a P_BMM_CONTAINER_PROPERTY");
        };
        let type_def = container
            .type_def
            .as_ref()
            .expect("the container states a type_def");
        assert_eq!(type_def.as_type_string(), "List<ITEM>");
        // `|>=1|` is lower-bounded and upper-unbounded.
        let Some(Interval::ProperInterval(ProperInterval::ProperInterval(cardinality))) =
            container.cardinality.as_ref()
        else {
            panic!("the cardinality reads as a proper interval");
        };
        assert_eq!(cardinality.lower, Some(1));
        assert!(cardinality.lower_included);
        assert_eq!(cardinality.upper, None);
        assert!(cardinality.upper_unbounded);

        let PBmmProperty::PBmmContainerProperty(
            PBmmContainerProperty::PBmmIndexedContainerProperty(indexed),
        ) = property(&schema, "ELEMENT", "custom_actions")
        else {
            panic!("custom_actions is a P_BMM_INDEXED_CONTAINER_PROPERTY");
        };
        assert_eq!(
            indexed
                .type_def
                .as_ref()
                .map(PBmmIndexedContainerType::as_type_string)
                .as_deref(),
            Some("Hash<String,EVENT_ACTION>")
        );
        Ok(())
    }

    #[test]
    fn a_property_without_a_type_marker_is_refused() {
        let error = read(
            r#"
            class_definitions = <
                ["ELEMENT"] = <
                    name = <"ELEMENT">
                    properties = <
                        ["value"] = <
                            name = <"value">
                            type = <"DATA_VALUE">
                        >
                    >
                >
            >
        "#,
        )
        .expect_err("§Class properties requires the marker");
        assert_eq!(
            error,
            PBmmReadError::MissingTypeMarker {
                path: "class_definitions/ELEMENT/properties/value".to_owned(),
                expected: "P_BMM_PROPERTY",
            }
        );
    }

    #[test]
    fn crazy_type_reads_every_mixed_generic_parameter_form() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Generic Classes CRAZY_TYPE, verbatim.
        let schema = read(
            r#"
            class_definitions = <
                ["CRAZY_TYPE"] = <
                    name = <"CRAZY_TYPE">
                    ancestors = <"Any">
                    properties = <
                        ["range"] = (P_BMM_GENERIC_PROPERTY) <
                            name = <"range">
                            type_def = <
                                root_type = <"REFERENCE_RANGE">
                                generic_parameter_defs = <
                                    ["T"] = (P_BMM_GENERIC_TYPE) <
                                        root_type = <"DV_INTERVAL">
                                        generic_parameters = <"DV_QUANTITY">
                                    >
                                    ["U"] = (P_BMM_SIMPLE_TYPE) <
                                        type = <"Integer">
                                    >
                                    ["V"] = (P_BMM_CONTAINER_TYPE) <
                                        type = <"DV_QUANTITY">
                                        container_type = <"List">
                                    >
                                    ["W"] = (P_BMM_CONTAINER_TYPE) <
                                        type_def = (P_BMM_GENERIC_TYPE) <
                                            root_type = <"DV_INTERVAL">
                                            generic_parameters = <"DV_QUANTITY">
                                        >
                                        container_type = <"List">
                                    >
                                >
                            >
                        >
                    >
                >
            >
        "#,
        )?;
        let PBmmProperty::PBmmGenericProperty(generic) = property(&schema, "CRAZY_TYPE", "range")
        else {
            panic!("range is a P_BMM_GENERIC_PROPERTY");
        };
        let type_def = generic.type_def.as_ref().expect("range states a type_def");
        assert_eq!(
            type_def.as_type_string(),
            "REFERENCE_RANGE<DV_INTERVAL<DV_QUANTITY>,Integer,List<DV_QUANTITY>,List<DV_INTERVAL<DV_QUANTITY>>>"
        );
        Ok(())
    }

    #[test]
    fn value_set_constraints_read_in_both_positions() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Value-set Constraints, both forms.
        let schema = read(
            r#"
            class_definitions = <
                ["TRANSLATION_DETAILS"] = <
                    name = <"TRANSLATION_DETAILS">
                    properties = <
                        ["encoding"] = (P_BMM_SINGLE_PROPERTY) <
                            name = <"encoding">
                            type_ref = <
                                type = <"CODE_PHRASE">
                                value_constraint = <"openEHR::languages">
                            >
                        >
                        ["language"] = (P_BMM_CONTAINER_PROPERTY) <
                            name = <"language">
                            type_def = <
                                container_type = <"List">
                                type_def = (P_BMM_SIMPLE_TYPE) <
                                    type = <"Coding">
                                    value_constraint = <"hl7::Languages">
                                >
                            >
                        >
                    >
                >
            >
        "#,
        )?;
        let PBmmProperty::PBmmSingleProperty(single) =
            property(&schema, "TRANSLATION_DETAILS", "encoding")
        else {
            panic!("encoding is a P_BMM_SINGLE_PROPERTY");
        };
        let type_ref = single
            .type_ref
            .as_ref()
            .expect("encoding states a type_ref");
        assert_eq!(type_ref.r#type, "CODE_PHRASE");
        assert_eq!(
            type_ref.value_constraint.as_deref(),
            Some("openEHR::languages")
        );

        let PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
            container,
        )) = property(&schema, "TRANSLATION_DETAILS", "language")
        else {
            panic!("language is a P_BMM_CONTAINER_PROPERTY");
        };
        assert_eq!(
            container
                .type_def
                .as_ref()
                .map(PBmmContainerType::as_type_string)
                .as_deref(),
            Some("List<Coding>")
        );
        Ok(())
    }

    #[test]
    fn functions_constants_and_invariants_read() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Functions, §Constants, §Invariants, verbatim.
        let schema = read(
            r#"
            class_definitions = <
                ["BMM_MODEL"] = <
                    name = <"BMM_MODEL">
                    ancestors = <"BMM_PACKAGE_CONTAINER">
                    functions = <
                        ["class_definition"] = <
                            name = <"class_definition">
                            parameters = <
                                ["a_name"] = (P_BMM_SINGLE_FUNCTION_PARAMETER) <
                                    name = <"a_name">
                                    type = <"String">
                                >
                            >
                            result = (P_BMM_SIMPLE_TYPE) <
                                type = <"BMM_CLASS">
                            >
                        >
                    >
                >
                ["OPENEHR_DEFINITIONS"] = <
                    name = <"OPENEHR_DEFINITIONS">
                    constants = <
                        ["Local_terminology_id"] = <
                            name = <"Local_terminology_id">
                            type = <"String">
                            value = <"local">
                        >
                    >
                >
                ["LOCATABLE"] = <
                    name = <"LOCATABLE">
                    invariants = <
                        ["Links_valid"] = <"links /= Void implies not links.is_empty">
                        ["Archetype_node_id_valid"] = <"not archetype_node_id.is_empty">
                    >
                >
            >
        "#,
        )?;
        let functions = class(&schema, "BMM_MODEL")
            .functions()
            .expect("BMM_MODEL declares functions");
        let function = functions
            .get("class_definition")
            .expect("class_definition reads");
        assert_eq!(
            function
                .result
                .as_ref()
                .map(PBmmType::as_type_string)
                .as_deref(),
            Some("BMM_CLASS")
        );
        assert_eq!(
            function
                .parameters
                .as_ref()
                .map(std::collections::BTreeMap::len),
            Some(1)
        );

        let constants = class(&schema, "OPENEHR_DEFINITIONS")
            .constants()
            .expect("OPENEHR_DEFINITIONS declares constants");
        assert_eq!(
            constants
                .get("Local_terminology_id")
                .and_then(|constant| constant.value.as_deref()),
            Some("local")
        );

        let invariants = class(&schema, "LOCATABLE")
            .invariants()
            .expect("LOCATABLE declares invariants");
        assert_eq!(invariants.len(), 2);
        assert_eq!(
            invariants.get("Links_valid").map(String::as_str),
            Some("links /= Void implies not links.is_empty")
        );
        Ok(())
    }

    #[test]
    fn enumerations_read_names_and_values() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Enumerated Types, verbatim.
        let schema = read(
            r#"
            class_definitions = <
                ["PROPORTION_KIND_2"] = (P_BMM_ENUMERATION_INTEGER) <
                    name = <"PROPORTION_KIND_2">
                    ancestors = <"Integer">
                    item_names = <"pk_ratio", "pk_unitary", "pk_percent", "pk_fraction">
                    item_values = <0, 1001, 1002, 1003>
                >
                ["MAGNITUDE_STATUS"] = (P_BMM_ENUMERATION_STRING) <
                    name = <"MAGNITUDE_STATUS">
                    ancestors = <"String", ...>
                    item_names = <"le", "ge", "eq", "approx_eq">
                    item_values = <"<=", ">=", "=", "~">
                >
            >
        "#,
        )?;
        let PBmmClass::PBmmEnumeration(integer) = class(&schema, "PROPORTION_KIND_2") else {
            panic!("PROPORTION_KIND_2 is a P_BMM_ENUMERATION_INTEGER");
        };
        assert_eq!(integer.item_names().len(), 4);
        assert_eq!(integer.item_values().len(), 4);
        assert_eq!(integer.underlying_type_name(), "INTEGER");

        let PBmmClass::PBmmEnumeration(string) = class(&schema, "MAGNITUDE_STATUS") else {
            panic!("MAGNITUDE_STATUS is a P_BMM_ENUMERATION_STRING");
        };
        assert_eq!(string.underlying_type_name(), "STRING");
        // The open-list marker of `ancestors = <"String", ...>` is not an item.
        assert_eq!(class(&schema, "MAGNITUDE_STATUS").ancestors(), ["String"]);
        Ok(())
    }

    #[test]
    fn generic_inheritance_reads_ancestor_defs() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Inheritance, GENERIC_CHILD_OPEN_T.
        let schema = read(
            r#"
            class_definitions = <
                ["GENERIC_CHILD_OPEN_T"] = <
                    name = <"GENERIC_CHILD_OPEN_T">
                    ancestor_defs = <
                        ["GENERIC_PARENT<T,SUPPLIER_B>"] = (P_BMM_GENERIC_TYPE) <
                            root_type = <"GENERIC_PARENT">
                            generic_parameters = <"T", "SUPPLIER_B">
                        >
                    >
                    generic_parameter_defs = <
                        ["T"] = <
                            name = <"T">
                            conforms_to_type = <"SUPPLIER">
                        >
                    >
                    properties = <
                        ["gen_child_open_t_prop"] = (P_BMM_SINGLE_PROPERTY) <
                            name = <"gen_child_open_t_prop">
                            type = <"String">
                        >
                    >
                >
            >
        "#,
        )?;
        let child = class(&schema, "GENERIC_CHILD_OPEN_T");
        assert!(child.is_generic());
        let defs = child.ancestor_defs();
        assert_eq!(defs.len(), 1);
        let signature = defs
            .first()
            .map(PBmmGenericType::as_type_string)
            .expect("the ancestor_defs entry reads");
        assert_eq!(signature, "GENERIC_PARENT<T,SUPPLIER_B>");
        Ok(())
    }

    #[test]
    fn an_ancestor_def_that_is_not_a_generic_type_is_refused() {
        // `P_BMM_CLASS.ancestor_defs` is List<P_BMM_GENERIC_TYPE> (class doc
        // §Attributes) and §Inheritance uses it only for generic ancestors, so a
        // (P_BMM_SIMPLE_TYPE) entry — the shape of the vendored fixture
        // `tests/vendor/bmm/…/persistence/validation/ancestor_def_doesnt_exist.bmm`
        // — states no `root_type` and cannot be materialised.
        let error = read(
            r#"
            class_definitions = <
                ["ParentType1"] = <
                    name = <"ParentType1">
                    ancestor_defs = <
                        ["UNKNOWN"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"UNKNOWN">
                        >
                    >
                >
            >
        "#,
        )
        .expect_err("ancestor_defs admits only P_BMM_GENERIC_TYPE");
        assert_eq!(
            error,
            PBmmReadError::UnexpectedTypeMarker {
                path: "class_definitions/ParentType1/ancestor_defs/UNKNOWN".to_owned(),
                marker: "P_BMM_SIMPLE_TYPE".to_owned(),
                expected: "P_BMM_GENERIC_TYPE",
            }
        );
    }

    #[test]
    fn a_persisted_interface_reads_its_name_documentation_and_functions()
    -> Result<(), PBmmReadError> {
        // master02-overview.adoc §Conceptual Approach: the model "can also
        // represent pure interfaces via P_BMM_INTERFACE, i.e. class-like
        // definitions that declare only functions and carry no state".
        let schema = read(
            r#"
            class_definitions = <
                ["Math"] = (P_BMM_INTERFACE) <
                    name = <"Math">
                    documentation = <"Mathematical functions.">
                    functions = <
                        ["abs"] = <
                            name = <"abs">
                            parameters = <
                                ["v"] = (P_BMM_SINGLE_FUNCTION_PARAMETER) <
                                    name = <"v">
                                    type = <"Real">
                                >
                            >
                            result = (P_BMM_SIMPLE_TYPE) <
                                type = <"Real">
                            >
                        >
                    >
                >
            >
        "#,
        )?;
        let interface = class(&schema, "Math");
        assert!(matches!(interface, PBmmClass::PBmmInterface(_)));
        assert_eq!(interface.documentation(), Some("Mathematical functions."));
        let functions = interface.functions().expect("the declared functions");
        assert_eq!(functions.keys().collect::<Vec<&String>>(), ["abs"]);
        // An interface carries no state and no processing-stamped attribute.
        assert!(interface.properties().is_none());
        assert!(interface.is_abstract());
        assert_eq!(interface.source_schema_id(), None);
        assert_eq!(interface.uid(), None);
        Ok(())
    }

    #[test]
    fn a_persisted_interface_refuses_a_state_carrying_attribute() {
        // `…p_bmm_interface.adoc` §Attributes declares `name` and `functions`
        // only (plus the inherited `documentation`), so a property block on an
        // interface is not part of the P_BMM model.
        let error = read(
            r#"
            class_definitions = <
                ["Env"] = (P_BMM_INTERFACE) <
                    name = <"Env">
                    properties = <
                        ["home"] = (P_BMM_SINGLE_PROPERTY) <
                            name = <"home">
                            type = <"String">
                        >
                    >
                >
            >
        "#,
        )
        .expect_err("an interface declares no properties");
        assert_eq!(
            error,
            PBmmReadError::UnknownAttribute {
                path: "class_definitions/Env".to_owned(),
                attribute: "properties".to_owned(),
            }
        );
    }

    #[test]
    fn primitive_types_read_as_ordinary_class_definitions() -> Result<(), PBmmReadError> {
        // master04-syntax.adoc §Classes for Primitive Types, verbatim.
        let schema = read(
            r#"
            primitive_types = <
                ["Any"] = <
                    name = <"Any">
                    is_abstract = <True>
                >
                ["Ordered"] = <
                    name = <"Ordered">
                    is_abstract = <True>
                    ancestors = <"Any">
                >
            >
        "#,
        )?;
        assert_eq!(schema.primitive_types.as_ref().map_or(0, Vec::len), 2);
        assert!(
            schema
                .primitive_types
                .iter()
                .flatten()
                .all(PBmmClass::is_abstract)
        );
        // uid numbering runs over primitive_types first, then class_definitions.
        assert_eq!(
            schema
                .primitive_types
                .iter()
                .flatten()
                .map(PBmmClass::uid)
                .collect::<Vec<Option<i32>>>(),
            [Some(1), Some(2)]
        );
        Ok(())
    }
}
