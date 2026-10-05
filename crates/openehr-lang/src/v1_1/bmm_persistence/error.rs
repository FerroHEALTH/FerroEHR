// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The typed failures of the P_BMM schema reader, include resolver and
//! `BMM_MODEL` transform.
//!
//! Every variant is a discriminant a caller can branch on; the display text is
//! never a decision input. Spec anchors are named per variant.

/// A P_BMM schema could not be read, its inclusions could not be resolved, or
/// the in-memory `BMM_MODEL` could not be materialised from it.
///
/// The three stages
/// ([`crate::v1_1::bmm_persistence::reader::read_schema`],
/// [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`],
/// [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`]) share one error
/// type because they are one pipeline: "A schema reading component has to
/// resolve the schema inclusions and ultimately `BMM_*` object instantiations
/// to obtain the in-memory form of the model"
/// (`LANG/docs/bmm_persistence/master02-overview.adoc` §Conceptual Approach).
///
/// `path`-carrying variants name the ODIN location as a `/`-joined attribute
/// path (`class_definitions/ELEMENT/properties/items/type_def`); `context`
/// carrying variants name the model element being materialised.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PBmmReadError {
    /// The schema text is not well-formed ODIN.
    #[error("the schema text is not valid ODIN: {0}")]
    Odin(#[from] crate::v1_1::odin::OdinError),

    /// The ODIN document root is not the attribute object a schema file is:
    /// "A schema file begins with the header (meta) items corresponding to the
    /// persisted attributes of `P_BMM_SCHEMA`"
    /// (`master04-syntax.adoc` §Serialisation Formats).
    #[error("the schema text is not a P_BMM schema object (the ODIN root is {found})")]
    NotASchemaObject {
        /// The ODIN value kind found at the document root.
        found: &'static str,
    },

    /// A mandatory (`1..1`) attribute of the block at `path` is absent.
    #[error("{path}: mandatory attribute `{attribute}` is missing")]
    MissingAttribute {
        /// ODIN attribute path of the block.
        path: String,
        /// The absent attribute's name.
        attribute: &'static str,
    },

    /// The block at `path` carries an attribute the P_BMM model does not
    /// declare and this reader does not tolerate.
    #[error("{path}: attribute `{attribute}` is not part of the P_BMM model")]
    UnknownAttribute {
        /// ODIN attribute path of the block.
        path: String,
        /// The undeclared attribute's name.
        attribute: String,
    },

    /// The ODIN value at `path` has the wrong shape for its P_BMM attribute.
    #[error("{path}: expected {expected}, found {found}")]
    WrongValueShape {
        /// ODIN attribute path of the value.
        path: String,
        /// What the P_BMM attribute's type admits.
        expected: &'static str,
        /// The ODIN value kind found.
        found: &'static str,
    },

    /// An ODIN type marker at `path` names a class that is not one of the
    /// declared attribute's subtypes.
    #[error("{path}: ODIN type marker `({marker})` is not a {expected}")]
    UnexpectedTypeMarker {
        /// ODIN attribute path of the marked value.
        path: String,
        /// The marker as written.
        marker: String,
        /// The P_BMM class the slot declares.
        expected: &'static str,
    },

    /// A polymorphic slot at `path` states no subtype and none can be inferred:
    /// "Wherever a value may be one of several `P_BMM_*` subtypes … the
    /// concrete subtype is always stated explicitly via a `_type`
    /// discriminator" (`master04-syntax.adoc` §Serialisation Formats).
    #[error("{path}: no ODIN type marker and no inferable {expected} subtype")]
    MissingTypeMarker {
        /// ODIN attribute path of the value.
        path: String,
        /// The P_BMM class the slot declares.
        expected: &'static str,
    },

    /// A keyed block's ODIN key and its `name` attribute disagree — "make sure
    /// that the ODIN 'keys' are the same as the 'name' attributes in each
    /// block" (`master04-syntax.adoc` §Package Definition, third NOTE).
    #[error("{path}: ODIN key `{key}` does not match the block's `name` attribute `{name}`")]
    KeyNameMismatch {
        /// ODIN attribute path of the block.
        path: String,
        /// The key as written.
        key: String,
        /// The `name` attribute as written.
        name: String,
    },

    /// A nested package name is a path — "only top-level package ids can be
    /// paths (i.e. contain the '.' character)"
    /// (`master04-syntax.adoc` §Package Definition, first NOTE).
    #[error(
        "{path}: nested package name `{name}` is qualified; only a top-level package id may contain '.'"
    )]
    QualifiedNestedPackage {
        /// ODIN attribute path of the package block.
        path: String,
        /// The qualified name as written.
        name: String,
    },

    /// A `cardinality` interval is not an integer range — `P_BMM_CONTAINER_PROPERTY.cardinality`
    /// "is expressed as a ODIN range" over the `Integer` multiplicity of
    /// `Multiplicity_interval` (`master04-syntax.adoc` §Container Properties).
    #[error("{path}: cardinality is not an integer ODIN range")]
    UnsupportedCardinality {
        /// ODIN attribute path of the `cardinality` attribute.
        path: String,
    },

    /// A schema includes another that was not supplied to the resolver.
    ///
    /// `P_BMM_SCHEMA.merge` has precondition
    /// `includes_to_process.has (included_schema.schema_id)`
    /// (`…p_bmm_schema.adoc` §Functions); an include that cannot be located
    /// leaves the inclusion unresolvable.
    #[error("schema `{requester}` includes `{id}`, which was not supplied")]
    MissingInclude {
        /// `schema_id` of the including schema.
        requester: String,
        /// `BMM_INCLUDE_SPEC.id` of the missing schema.
        id: String,
    },

    /// The inclusion graph is cyclic.
    ///
    /// NOTE: no openEHR spec governs this — our own design/extension. The
    /// `P_BMM_SCHEMA` load states (`…p_bmm_schema.adoc` §Functions) describe a
    /// terminating include-processing sequence but state no acyclicity rule, so
    /// the resolver detects the cycle itself rather than recursing forever.
    #[error("schema inclusion cycle: {chain}")]
    IncludeCycle {
        /// The cycle, as `a -> b -> a`.
        chain: String,
    },

    /// Two supplied schemas render the same `schema_id`, so an include cannot
    /// name one of them unambiguously (`BMM_SCHEMA_CORE.schema_id`,
    /// `org.openehr.lang.bmm.bmm_schema_core.adoc` §Functions).
    #[error("two supplied schemas share the schema id `{id}`")]
    DuplicateSchemaId {
        /// The shared id.
        id: String,
    },

    /// A class names an inheritance parent no class in the schema defines.
    ///
    /// `BMM_CLASS.ancestors` is a map of `BMM_CLASS`
    /// (`org.openehr.lang.bmm.bmm_class.adoc` §Attributes), so an unresolvable
    /// parent cannot be materialised.
    #[error("class `{class}` names ancestor `{ancestor}`, which no class in the schema defines")]
    UnknownAncestor {
        /// The inheriting class's name.
        class: String,
        /// The unresolvable parent's name.
        ancestor: String,
    },

    /// A type reference names a class no class definition in the schema
    /// declares.
    ///
    /// Every `BMM_TYPE` form roots in a `BMM_CLASS` (`BMM_SIMPLE_TYPE.base_class`,
    /// `BMM_GENERIC_TYPE.base_class`, `BMM_CONTAINER_TYPE.container_type`), so
    /// an unresolvable name cannot be materialised without inventing a class
    /// the schema does not declare.
    #[error("{context}: type `{type_name}` is not defined by any class in the schema")]
    UnknownType {
        /// The model element being materialised.
        context: String,
        /// The unresolvable type name.
        type_name: String,
    },

    /// A class definition is in no package of the schema.
    ///
    /// `BMM_CLASS.package` is `1..1` ("Package this class belongs to",
    /// `org.openehr.lang.bmm.bmm_class.adoc` §Attributes), so a class listed in
    /// no package cannot be materialised.
    #[error("class `{class}` is not listed in any package of the schema")]
    ClassNotInAnyPackage {
        /// The orphaned class's name.
        class: String,
    },

    /// A package lists a class the schema does not define — "only classes
    /// defined in the same schema can be referenced in the package section in
    /// that schema" (`master04-syntax.adoc` §Package Definition, second NOTE).
    #[error("package `{package}` lists class `{class}`, which the schema does not define")]
    ClassNotDefined {
        /// The listing package's name.
        package: String,
        /// The undefined class name.
        class: String,
    },

    /// A container type states neither `type` nor `type_def`, so it has no
    /// target type — `BMM_CONTAINER_TYPE.base_type` is `1..1`
    /// (`org.openehr.lang.bmm.bmm_container_type.adoc` §Attributes).
    #[error("{context}: the container type states no target type")]
    ContainerTargetTypeMissing {
        /// The model element being materialised.
        context: String,
    },

    /// A property or function result states no type at all, so its `BMM_TYPE`
    /// cannot be materialised (`BMM_PROPERTY.type` is `1..1`,
    /// `org.openehr.lang.bmm.bmm_property.adoc` §Attributes).
    #[error("{context}: no type is stated")]
    TypeDefinitionMissing {
        /// The model element being materialised.
        context: String,
    },

    /// A `P_BMM_SINGLE_PROPERTY_OPEN` names a generic parameter the owning
    /// class does not declare — "The parameter must be in the type declaration
    /// of the owning `BMM_CLASS`" (`org.openehr.lang.bmm.bmm_open_type.adoc`
    /// §Description).
    #[error(
        "class `{class}` property `{property}` is of open type `{parameter}`, which is not a generic parameter of the class or of any ancestor"
    )]
    UndeclaredGenericParameter {
        /// The owning class's name.
        class: String,
        /// The property's name.
        property: String,
        /// The undeclared parameter name.
        parameter: String,
    },

    /// An enumeration class states more than one inheritance ancestor.
    ///
    /// "the `BMM_ENUMERATION` meta-type is defined as a descendant of
    /// `BMM_SIMPLE_CLASS`, and may have only one ancestor"
    /// (`LANG/docs/bmm3/master07-core-classes.adoc` §Range-Constrained Classes);
    /// the class definition repeats it — "Only one inheritance ancestor is
    /// allowed in order to provide the base type to which the range constraint
    /// is applied" (`org.openehr.lang.bmm3.bmm_enumeration.adoc` §Description).
    /// The single ancestor IS the enumeration's underlying type
    /// (`BMM_ENUMERATION.underlying_type_name` is "the name of type bound to 'T'",
    /// `org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes), so a second
    /// ancestor leaves the underlying type ambiguous.
    #[error(
        "enumeration class `{class}` states {} ancestors ({}); an enumeration may have only one",
        ancestors.len(),
        ancestors.join(", ")
    )]
    EnumerationAncestorCount {
        /// The enumeration class's name.
        class: String,
        /// The ancestors as stated, in schema order.
        ancestors: Vec<String>,
    },

    /// An enumeration states `item_values` that are not 1:1 with `item_names`.
    ///
    /// `BMM_ENUMERATION.item_values` is an "Optional list of specific values.
    /// Must be 1:1 with `item_names` list"
    /// (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes, identically in
    /// `org.openehr.lang.bmm3.bmm_enumeration.adoc`). Omitting the values
    /// entirely stays legal — "If no values are supplied, the integer values 0,
    /// 1, 2, ... are assumed" (same §Attributes) — so only a non-empty list of
    /// the wrong length is a violation.
    #[error(
        "enumeration class `{class}` states {values} item value(s) for {names} item name(s); \
         `item_values` must be 1:1 with `item_names`"
    )]
    EnumerationItemListsNotOneToOne {
        /// The enumeration class's name.
        class: String,
        /// The number of `item_names` stated.
        names: usize,
        /// The number of `item_values` stated.
        values: usize,
    },

    /// An integer enumeration states an item value that is not an `Integer`.
    ///
    /// `BMM_ENUMERATION_INTEGER` redefines `item_values` to
    /// `List<BMM_INTEGER_VALUE>`
    /// (`org.openehr.lang.bmm3.bmm_enumeration_integer.adoc` §Attributes), and
    /// `BMM_INTEGER_VALUE.value` is a "Native Integer value"
    /// (`org.openehr.lang.bmm3.bmm_integer_value.adoc` §Attributes) — an
    /// `Integer` being a 32-bit integer
    /// (`BASE/docs/foundation_types/master03-primitive_types.adoc` §Overview).
    /// `P_BMM_ENUMERATION.item_values` persists `List<Any>`
    /// (`org.openehr.lang.bmm_persistence.p_bmm_enumeration.adoc` §Attributes),
    /// so a persisted scalar of another kind, or outside that range, names no
    /// `BMM_INTEGER_VALUE` that can be constructed.
    #[error(
        "enumeration class `{class}` item {} states value `{value}`, which is not an Integer",
        item.as_ref().map_or_else(
            || format!("at position {index}"),
            |name| format!("`{name}`"),
        )
    )]
    EnumerationItemValueNotAnInteger {
        /// The enumeration class's name.
        class: String,
        /// The value's 0-based position in `item_values`.
        index: usize,
        /// The `item_names` entry at the same position, where the two lists
        /// are 1:1.
        item: Option<String>,
        /// The persisted value's serial form.
        value: String,
    },

    /// A generic type's root class is not generic — `BMM_GENERIC_TYPE.base_class`
    /// is a `BMM_GENERIC_CLASS` (`org.openehr.lang.bmm.bmm_generic_type.adoc`
    /// §Attributes).
    #[error("{context}: `{type_name}` is not a generic class")]
    NotAGenericClass {
        /// The model element being materialised.
        context: String,
        /// The non-generic root type's name.
        type_name: String,
    },
}
