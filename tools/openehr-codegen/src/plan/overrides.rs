// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The emitter's **declarative decision data**: every override / mapping the
//! generator applies, as checked-in const tables rather than logic buried in
//! `match` arms. Each entry carries (a) its key, (b) the decision, (c) a spec
//! citation or the explicit "no openEHR spec governs this — our own design"
//! flag, and (d) a one-line reason.
//!
//! `tests/emitter_invariants.rs` machine-checks the tables: every entry names a
//! class or field that exists in the loaded schemas and carries a non-empty
//! citation. The lookup functions ([`back_reference`], [`class_binding`],
//! [`type_override`], [`field_default`], [`primitive`], [`is_mapped_class`]) are
//! thin scans, so a decision change is a data edit, never a control-flow edit.

use std::collections::BTreeMap;

use crate::analyze::invariants::{Bucket, classify};
use crate::load::bmm::{BmmPropKind, BmmProperty, BmmType};

// ─────────────────────────────────────────────────────────────────────────────
// Primitive type map (spec foundation type → Rust type)
// ─────────────────────────────────────────────────────────────────────────────

/// One primitive spec type mapped to a Rust type.
pub(crate) struct Primitive {
    /// The BMM foundation type name (`Integer`, `String`, …).
    pub spec: &'static str,
    /// The Rust type it lowers to.
    pub rust: &'static str,
    /// Spec citation, or the explicit our-own-design flag.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// The codegen primitive type map: BASE foundation scalar types → Rust scalars.
/// `Real`/`Double` both lower to `f64`; `String`/`Uri` both to `String`
/// (openEHR URI semantics are broader than any crate URL type — it stays a
/// plain string, the strong type living in hand-written behaviour if ever).
pub(crate) const PRIMITIVES: &[Primitive] = &[
    Primitive {
        spec: "Boolean",
        rust: "bool",
        citation: "BASE foundation_types (Boolean)",
        reason: "Boolean scalar → Rust bool.",
    },
    Primitive {
        spec: "Integer",
        rust: "i32",
        citation: "BASE foundation_types (Integer, 32-bit)",
        reason: "Integer scalar → i32.",
    },
    Primitive {
        spec: "Integer64",
        rust: "i64",
        citation: "BASE foundation_types (Integer64)",
        reason: "64-bit integer scalar → i64.",
    },
    Primitive {
        spec: "Real",
        rust: "f64",
        citation: "BASE foundation_types (Real)",
        reason: "Real scalar → f64.",
    },
    Primitive {
        spec: "Double",
        rust: "f64",
        citation: "BASE foundation_types (Double)",
        reason: "Double scalar → f64.",
    },
    Primitive {
        spec: "String",
        rust: "String",
        citation: "BASE foundation_types (String)",
        reason: "String scalar → Rust String.",
    },
    Primitive {
        spec: "Uri",
        rust: "String",
        citation: "no openEHR spec governs a URI newtype — our own design: openEHR URI \
                   semantics are broader than any crate URL type, so it stays a plain String",
        reason: "Uri lowers to String until a strong-newtype override is justified.",
    },
    Primitive {
        spec: "Octet",
        rust: "u8",
        citation: "BASE foundation_types (Octet)",
        reason: "Octet scalar → u8 (Octet containers become byte vectors).",
    },
    Primitive {
        spec: "Character",
        rust: "char",
        citation: "BASE foundation_types (Character)",
        reason: "Character scalar → Rust char.",
    },
];

/// The Rust type a primitive spec type maps to, or `None` if `name` is not a
/// primitive.
pub(crate) fn primitive(name: &str) -> Option<&'static str> {
    PRIMITIVES.iter().find(|p| p.spec == name).map(|p| p.rust)
}

// ─────────────────────────────────────────────────────────────────────────────
// Mapped/skipped classes (mapped to Rust, never emitted)
// ─────────────────────────────────────────────────────────────────────────────

/// A foundation class mapped to Rust and never emitted as a spec type.
pub(crate) struct MappedClass {
    /// The BMM class name.
    pub name: &'static str,
    /// Spec citation, or the explicit our-own-design flag.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// Foundation classes handled by the Rust type system rather than emitted:
/// container types (→ `Vec`/map/`BTreeSet` via container properties), abstract
/// marker/algebraic traits (no data), functional types, service interfaces (no
/// data), and constant-holder classes (their constants live in `*_impl.rs`).
/// Scalar primitives are handled by [`PRIMITIVES`]. `Multiplicity_interval` and
/// `Cardinality` are NOT here — they emit as real structs (their inherited
/// `Interval<Integer>` binding comes from [`CLASS_BINDINGS`]).
pub(crate) const MAPPED_CLASSES: &[MappedClass] = &[
    // ── containers → Vec / map / set, handled by container properties ──
    MappedClass {
        name: "Container",
        citation: "BASE foundation_types (Container)",
        reason: "Abstract container base → handled by the container property kind.",
    },
    MappedClass {
        name: "List",
        citation: "BASE foundation_types (List<T>)",
        reason: "List<T> → Vec<T>.",
    },
    MappedClass {
        name: "Set",
        citation: "BASE foundation_types (Set<T>)",
        reason: "Set<T> → BTreeSet<T>.",
    },
    MappedClass {
        name: "Array",
        citation: "BASE foundation_types (Array<T>)",
        reason: "Array<T> → Vec<T>.",
    },
    MappedClass {
        name: "Hash",
        citation: "BASE foundation_types (Hash<K,V>)",
        reason: "Hash<K,V> → BTreeMap<K,V>.",
    },
    // ── abstract marker / algebraic traits (no data) ──
    MappedClass {
        name: "Any",
        citation: "BASE foundation_types (Any)",
        reason: "The universal supertype → carries no fields (renders as free-form JSON).",
    },
    MappedClass {
        name: "Ordered",
        citation: "BASE foundation_types (Ordered)",
        reason: "Algebraic ordering trait, no data.",
    },
    MappedClass {
        name: "Numeric",
        citation: "BASE foundation_types (Numeric)",
        reason: "Algebraic numeric trait, no data.",
    },
    MappedClass {
        name: "Ordered_Numeric",
        citation: "BASE foundation_types (Ordered_Numeric)",
        reason: "Algebraic trait, no data.",
    },
    MappedClass {
        name: "Comparable",
        citation: "BASE foundation_types (Comparable)",
        reason: "Algebraic comparison trait, no data.",
    },
    MappedClass {
        name: "Temporal",
        citation: "BASE foundation_types (Temporal)",
        reason: "Algebraic temporal trait, no data.",
    },
    // ── functional types ──
    MappedClass {
        name: "TUPLE",
        citation: "BASE foundation_types (functional package)",
        reason: "Functional tuple type, not a spec data class.",
    },
    MappedClass {
        name: "TUPLE1",
        citation: "BASE foundation_types (functional package)",
        reason: "Functional tuple type, not a spec data class.",
    },
    MappedClass {
        name: "TUPLE2",
        citation: "BASE foundation_types (functional package)",
        reason: "Functional tuple type, not a spec data class.",
    },
    MappedClass {
        name: "ROUTINE",
        citation: "BASE foundation_types (functional package)",
        reason: "Functional routine type, not a spec data class.",
    },
    MappedClass {
        name: "FUNCTION",
        citation: "BASE foundation_types (functional package)",
        reason: "Functional function type, not a spec data class.",
    },
    MappedClass {
        name: "PROCEDURE",
        citation: "BASE foundation_types (functional package)",
        reason: "Functional procedure type, not a spec data class.",
    },
    // ── service interfaces (no data) ──
    MappedClass {
        name: "Env",
        citation: "BASE base_types.builtins (Env service interface; Base Types §Built-in types)",
        reason: "Service interface, no data.",
    },
    MappedClass {
        name: "Locale",
        citation: "BASE base_types.builtins (Locale service interface; Base Types §Built-in types)",
        reason: "Service interface, no data.",
    },
    MappedClass {
        name: "Math",
        citation: "BASE base_types.builtins (Math service interface; Base Types §Built-in types)",
        reason: "Service interface, no data.",
    },
    MappedClass {
        name: "Quantity_converter",
        citation: "BASE base_types.builtins (Quantity_converter service interface; Base Types §Built-in types)",
        reason: "Service interface, no data.",
    },
    MappedClass {
        name: "Statistical_evaluator",
        citation: "BASE base_types.builtins (Statistical_evaluator service interface; Base Types §Built-in types)",
        reason: "Service interface, no data.",
    },
    // ── constant-holder classes (no data; become assoc consts in *_impl.rs) ──
    MappedClass {
        name: "Time_Definitions",
        citation: "BASE foundation_types (Time_Definitions)",
        reason: "Constant holder; its constants become associated consts in *_impl.rs.",
    },
    MappedClass {
        name: "BASIC_DEFINITIONS",
        citation: "BASE base_types.definitions (BASIC_DEFINITIONS; Base Types §Definitions Package)",
        reason: "Constant holder; its constants live in base_types/definitions/definitions_impl.rs.",
    },
    MappedClass {
        name: "OPENEHR_DEFINITIONS",
        citation: "BASE base_types.definitions (OPENEHR_DEFINITIONS; Base Types §Definitions Package)",
        reason: "Constant holder; its constants live in base_types/definitions/definitions_impl.rs.",
    },
    // ── spec-declared open extension points → the validated verbatim carrier ──
    MappedClass {
        name: "ACCESS_CONTROL_SETTINGS",
        citation: "RM ehr_access.adoc §settings (\"allowing for the use of different access \
                   control schemes. Currently implementation dependent.\")",
        reason: "Open extension point: a valid instance is a scheme-defined subtype the \
                 published model cannot name → openehr_base::serde_support::OpenSubtype.",
    },
];

/// The Rust realization of a spec-declared OPEN extension point, when `name`
/// is one: a class whose instances are scheme-defined subtypes outside the
/// published model, carried verbatim by the validated
/// `openehr_base::serde_support::OpenSubtype`.
pub(crate) fn open_extension_point(name: &str) -> Option<&'static str> {
    (name == "ACCESS_CONTROL_SETTINGS").then_some("openehr_base::serde_support::OpenSubtype")
}

/// Whether `name` is a mapped/skipped foundation class (never emitted).
pub(crate) fn is_mapped_class(name: &str) -> bool {
    MAPPED_CLASSES.iter().any(|m| m.name == name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Ancestor-generic class bindings
// ─────────────────────────────────────────────────────────────────────────────

/// One ancestor-generic binding: `class`'s generic parameter `param` is bound to
/// the concrete spec type `concrete` (a binding the BMM drops).
pub(crate) struct ClassBinding {
    pub class: &'static str,
    pub param: &'static str,
    pub concrete: &'static str,
    pub citation: &'static str,
    pub reason: &'static str,
}

/// Ancestor-generic bindings the BMM drops (it records `ancestors` and some
/// generic-content property types as bare class names, losing the `<Integer>` /
/// `<COMPOSITION>` argument). The emitter substitutes the concrete type instead
/// of degrading the field to `serde_json::Value`.
pub(crate) const CLASS_BINDINGS: &[ClassBinding] = &[
    ClassBinding {
        class: "Multiplicity_interval",
        param: "T",
        concrete: "Integer",
        citation: "BASE foundation_types — Multiplicity_interval is \"an Interval of Integer\"",
        reason: "openEHR files it under primitive_types without carrying the Interval<Integer> binding.",
    },
    ClassBinding {
        class: "BMM_CONTAINER_VALUE",
        param: "T",
        concrete: "BMM_CONTAINER_TYPE",
        citation: "LANG bmm3 master09-core-values.adoc §Container Literals — a container literal's \"`_type_` will be `BMM_CONTAINER_TYPE`\"",
        reason: "BMM_LITERAL_VALUE<T>.type is inherited open; the chapter states the narrowing the bmm3 class definition omits (openEHR's own LANG 1.0.0 BMM declares `type: BMM_CONTAINER_TYPE` here).",
    },
    ClassBinding {
        class: "BMM_INDEXED_CONTAINER_VALUE",
        param: "T",
        concrete: "BMM_INDEXED_CONTAINER_TYPE",
        citation: "LANG bmm3 master09-core-values.adoc §Container Literals — a `Hash<K,V>` literal \"has as its meta-type `BMM_INDEXED_CONTAINER_VALUE`\", the indexed analogue of the container-literal narrowing",
        reason: "BMM_LITERAL_VALUE<T>.type is inherited open; openEHR's own LANG 1.0.0 BMM declares `type: BMM_INDEXED_CONTAINER_TYPE` here.",
    },
    ClassBinding {
        class: "X_VERSIONED_COMPOSITION",
        param: "T",
        concrete: "COMPOSITION",
        citation: "RM ehr_extract (X_VERSIONED_COMPOSITION : X_VERSIONED_OBJECT<COMPOSITION>)",
        reason: "Binds the versioned-content type X_VERSIONED_OBJECT<T> leaves open.",
    },
    ClassBinding {
        class: "X_VERSIONED_EHR_ACCESS",
        param: "T",
        concrete: "EHR_ACCESS",
        citation: "RM ehr_extract (X_VERSIONED_EHR_ACCESS : X_VERSIONED_OBJECT<EHR_ACCESS>)",
        reason: "Binds the versioned-content type X_VERSIONED_OBJECT<T> leaves open.",
    },
    ClassBinding {
        class: "X_VERSIONED_EHR_STATUS",
        param: "T",
        concrete: "EHR_STATUS",
        citation: "RM ehr_extract (X_VERSIONED_EHR_STATUS : X_VERSIONED_OBJECT<EHR_STATUS>)",
        reason: "Binds the versioned-content type X_VERSIONED_OBJECT<T> leaves open.",
    },
    ClassBinding {
        class: "X_VERSIONED_PARTY",
        param: "T",
        concrete: "PARTY",
        citation: "RM ehr_extract (X_VERSIONED_PARTY : X_VERSIONED_OBJECT<PARTY>)",
        reason: "Binds the versioned-content type X_VERSIONED_OBJECT<T> leaves open.",
    },
    ClassBinding {
        class: "X_VERSIONED_FOLDER",
        param: "T",
        concrete: "FOLDER",
        citation: "RM ehr_extract (X_VERSIONED_FOLDER : X_VERSIONED_OBJECT<FOLDER>)",
        reason: "Binds the versioned-content type X_VERSIONED_OBJECT<T> leaves open.",
    },
];

/// The generic-parameter → concrete-type bindings for `class` (empty if none).
pub(crate) fn class_binding(class: &str) -> BTreeMap<String, String> {
    CLASS_BINDINGS
        .iter()
        .filter(|b| b.class == class)
        .map(|b| (b.param.to_string(), b.concrete.to_string()))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Additional polymorphic subtype members (subtype seams the BMM under-declares)
// ─────────────────────────────────────────────────────────────────────────────

/// One additional member of a polymorphic subtype set: `subtype` becomes a
/// variant of `parent`'s untagged enum although the vendored BMM records no
/// inheritance edge from `subtype` to `parent`.
pub(crate) struct SubtypeExtension {
    /// The polymorphic parent whose variant set widens.
    pub parent: &'static str,
    /// The class that additionally becomes one of its variants.
    pub subtype: &'static str,
    /// Spec citation, or the explicit our-own-design flag.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// Subtype sets the vendored BMM under-declares versus the normative spec text
/// and openEHR's OWN published schema artifacts.
///
/// An entry adds one inheritance edge to the analysed model
/// ([`crate::analyze::Model::inherits`]), so the class joins its parent's
/// polymorphic enum exactly as a declared descendant would. Property
/// flattening is deliberately NOT affected: it reads
/// `crate::load::bmm::BmmClass::ancestors` verbatim, so an extension subtype
/// gains none of the parent's attributes — it stays the class the BMM declares,
/// reachable through the parent's slot.
pub(crate) const SUBTYPE_EXTENSIONS: &[SubtypeExtension] = &[SubtypeExtension {
    parent: "P_BMM_CLASS",
    subtype: "P_BMM_INTERFACE",
    citation: "LANG bmm_persistence master02-overview.adoc §Conceptual Approach — \"In \
               addition to ordinary classes, the model can also represent pure interfaces via \
               P_BMM_INTERFACE, i.e. class-like definitions that declare only functions and \
               carry no state\" — and openEHR's OWN published schemas serialise them as \
               members of `class_definitions` (`(P_BMM_INTERFACE)`-marked entries in the \
               vendored BASE 1.3.0 + RM 1.2.0 ODIN schemas, `\"_type\": \"P_BMM_INTERFACE\"` \
               members in openehr_base_1.3.0.bmm.json), whereas \
               `P_BMM_SCHEMA.primitive_types`/`class_definitions` are \
               `List<P_BMM_CLASS>` (org.openehr.lang.bmm_persistence.p_bmm_schema.adoc \
               §Attributes) and P_BMM_INTERFACE inherits only P_BMM_MODEL_ELEMENT \
               (…p_bmm_interface.adoc §Inherit). The docs text plus the published artifacts \
               win over the under-declared inheritance edge.",
    reason: "A persisted P_BMM_INTERFACE has to be readable where a schema's class list \
             admits P_BMM_CLASS.",
}];

/// The parents `subtype` additionally belongs to.
pub(crate) fn subtype_extension_parents(subtype: &str) -> impl Iterator<Item = &'static str> {
    SUBTYPE_EXTENSIONS
        .iter()
        .filter(move |e| e.subtype == subtype)
        .map(|e| e.parent)
}

// ─────────────────────────────────────────────────────────────────────────────
// REST DTO required-list overrides (docs-text-wins corrections)
// ─────────────────────────────────────────────────────────────────────────────

/// One REST-contract required-list correction: `(dto, field)` is emitted as
/// OPTIONAL although the vendored OAS schema lists it under `required`. Used
/// only where the ITS-REST **docs text** (which wins every conflict with the
/// released OAS — owner rulings 2026-07-24 + 2026-07-28) contradicts the OAS
/// shape.
pub(crate) struct RestOptionalOverride {
    pub dto: &'static str,
    pub field: &'static str,
    pub citation: &'static str,
    pub reason: &'static str,
}

/// The docs-text-wins required-list corrections for the generated ITS-REST
/// contract (`emit-rest`).
pub(crate) const REST_OPTIONAL_OVERRIDES: &[RestOptionalOverride] = &[
    RestOptionalOverride {
        dto: "UpdateVersion",
        field: "data",
        citation: "RM common original_version.adoc §Attributes — `data` is `0..1`; RM common \
                   master06-change_control_package.adoc §Logical Deletion — the deleting commit \
                   \"delete[s] its `_data_`\" and sets `lifecycle_state` to `deleted`",
        reason: "A logical-deletion member carries no data; the released OAS lists `data` as \
                 required, which makes the RM's own deletion procedure unrepresentable.",
    },
    RestOptionalOverride {
        dto: "Query",
        field: "offset",
        citation: "ITS-REST docs/query/Request.md §Common Headers and Query Parameters — \
                   \"`offset` … default `0`\"",
        reason: "A required member cannot default; the stored-query execute body must \
                 accept `{}` (a parameterless stored query).",
    },
    RestOptionalOverride {
        dto: "Query",
        field: "fetch",
        citation: "ITS-REST docs/query/Request.md §Common Headers and Query Parameters — \
                   \"`fetch` … default depends on the implementation\"",
        reason: "A required member cannot default; the stored-query execute body must \
                 accept `{}`.",
    },
    RestOptionalOverride {
        dto: "Query",
        field: "query_parameters",
        citation: "ITS-REST docs/query/Request.md §Query parameters — parameters exist \
                   \"Depending on each query definition\"; a parameterless stored query \
                   binds none",
        reason: "A stored query with no $parameters must be executable with an empty body.",
    },
];

/// The docs-text-wins correction for `(dto, field)`, or `None` — the field is
/// emitted optional and the generated code carries the citation.
pub(crate) fn rest_optional_override(
    dto: &str,
    field: &str,
) -> Option<&'static RestOptionalOverride> {
    REST_OPTIONAL_OVERRIDES
        .iter()
        .find(|o| o.dto == dto && o.field == field)
}

/// One REST-contract member that reads as its `Default` when the wire omits
/// it, although the vendored OAS schema lists it under `required`: the
/// generated field keeps its type and carries `#[serde(default)]`.
pub(crate) struct RestDefaultMember {
    pub dto: &'static str,
    pub field: &'static str,
    pub citation: &'static str,
    pub reason: &'static str,
}

/// The members of the generated ITS-REST contract that read as their
/// `Default` when absent (`emit-rest`).
pub(crate) const REST_DEFAULT_MEMBERS: &[RestDefaultMember] = &[RestDefaultMember {
    dto: "Error",
    field: "validationErrors",
    citation: "ITS-REST `responses/400.yaml`: \"The response body MAY contain error \
               details\" — the released text makes the whole error body optional and \
               binds no client's reading of one; no openEHR spec governs a client's \
               tolerance of a partial `Error` — our own design",
    reason: "A service that answers `{\"message\": …}` alone still sends its \
             diagnostics; an empty list reads them, a decode refusal loses them.",
}];

/// The default-when-absent correction for `(dto, field)`, or `None`.
pub(crate) fn rest_default_member(dto: &str, field: &str) -> Option<&'static RestDefaultMember> {
    REST_DEFAULT_MEMBERS
        .iter()
        .find(|o| o.dto == dto && o.field == field)
}

// ─────────────────────────────────────────────────────────────────────────────
// REST request headers the docs text defines (no OAS parameter declares them)
// ─────────────────────────────────────────────────────────────────────────────

/// One request header the ITS-REST docs text defines for a set of operations
/// that the vendored OAS declares no parameter for. `emit-rest` appends it to
/// each named operation's parameters as an optional header, so the param
/// struct, the server trait and the generated client all carry it.
pub(crate) struct RestDocsTextHeader {
    /// The header name on the wire.
    pub name: &'static str,
    /// Whether the header MAY repeat (one field line per value): a `Vec<String>`
    /// parameter, one field line per item, rather than a `String`.
    pub list: bool,
    /// The `operationId`s the header applies to.
    pub operations: &'static [&'static str],
    pub citation: &'static str,
    pub reason: &'static str,
}

/// The operations that commit a change-controlled resource directly, on which
/// the docs text mandates the committal-metadata headers: the `PUT`, `POST`
/// and `DELETE` operations on COMPOSITION, `EHR_STATUS`, the directory FOLDER,
/// the EHR creation that commits the first `EHR_STATUS` version, and the
/// demographic `PARTY` and `PARTY_RELATIONSHIP` resources. A `CONTRIBUTION` carries
/// its own `AUDIT_DETAILS` and `VERSION` attributes in the body, and the `ITEM_TAG`
/// operations stand outside change control.
const COMMIT_OPERATIONS: &[&str] = &[
    "ehr_create",
    "ehr_create_with_id",
    "ehr_status_update",
    "composition_create",
    "composition_update",
    "composition_delete",
    "directory_create",
    "directory_update",
    "directory_delete",
    "agent_create",
    "agent_update",
    "agent_delete",
    "group_create",
    "group_update",
    "group_delete",
    "organisation_create",
    "organisation_update",
    "organisation_delete",
    "person_create",
    "person_update",
    "person_delete",
    "role_create",
    "role_update",
    "role_delete",
    "party_relationship_create",
    "party_relationship_update",
    "party_relationship_delete",
];

/// The request headers the ITS-REST docs text defines beyond the OAS
/// parameters (`emit-rest`).
pub(crate) const REST_DOCS_TEXT_HEADERS: &[RestDocsTextHeader] = &[
    RestDocsTextHeader {
        name: "openehr-version",
        list: false,
        operations: COMMIT_OPERATIONS,
        citation: "ITS-REST `docs/overview/Requests_and_responses.md` §openehr-version and \
                   openehr-audit-details — \"services MUST accept `openehr-version` and \
                   `openehr-audit-details` custom request headers\" on the `PUT`, `POST` \
                   and `DELETE` methods of every change-controlled resource",
        reason: "The VERSION attributes of the commit (`lifecycle_state.code_string=\"532\"`); \
                 the OAS declares no parameter for it.",
    },
    RestDocsTextHeader {
        name: "openehr-audit-details",
        list: true,
        operations: COMMIT_OPERATIONS,
        citation: "ITS-REST `docs/overview/Requests_and_responses.md` §openehr-version and \
                   openehr-audit-details — the same mandate; the section's example sends \
                   the header as several field lines, one per AUDIT_DETAILS attribute",
        reason: "The AUDIT_DETAILS attributes of the commit (`change_type`, `description`, \
                 `committer`, `system_id`); the OAS declares no parameter for it.",
    },
    RestDocsTextHeader {
        name: "openehr-template-id",
        list: false,
        operations: &[
            "composition_create",
            "composition_update",
            "contribution_create",
        ],
        citation: "ITS-REST `docs/overview/Requests_and_responses.md` §openehr-template-id — \
                   \"MUST be used whenever committing COMPOSITION (via `PUT` or `POST` \
                   methods) using a Simplified Format which does not support TEMPLATE_ID \
                   value\"; a CONTRIBUTION `POST` commits COMPOSITIONs too",
        reason: "The template a Simplified Formats COMPOSITION body is validated against; \
                 the OAS declares no parameter for it.",
    },
];

/// The docs-text headers that apply to `operation_id`, in table order.
pub(crate) fn rest_docs_text_headers(
    operation_id: &str,
) -> impl Iterator<Item = &'static RestDocsTextHeader> {
    REST_DOCS_TEXT_HEADERS
        .iter()
        .filter(move |h| h.operations.contains(&operation_id))
}

/// The docs-text header `name` on `operation_id`, or `None` when the OAS
/// declared it.
pub(crate) fn rest_docs_text_header(
    operation_id: &str,
    name: &str,
) -> Option<&'static RestDocsTextHeader> {
    rest_docs_text_headers(operation_id).find(|h| h.name == name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Field-level type overrides
// ─────────────────────────────────────────────────────────────────────────────

/// One field-level type override: `(class, field)` uses `rust_type` instead of
/// the BMM primitive.
pub(crate) struct TypeOverride {
    pub class: &'static str,
    pub field: &'static str,
    pub rust_type: &'static str,
    pub citation: &'static str,
    pub reason: &'static str,
}

/// Field type overrides mapping a `(class, field)` to a proven Rust crate type
/// instead of the BMM primitive. Only unambiguous mappings belong here — where
/// openEHR's semantics are broader than a crate (partial-precision ISO 8601,
/// plain-text URIs) the field stays `String` and the crate is used in the
/// hand-written `*_impl.rs` behaviour instead.
pub(crate) const TYPE_OVERRIDES: &[TypeOverride] = &[TypeOverride {
    class: "UUID",
    field: "value",
    rust_type: "uuid::Uuid",
    citation: "BASE base_types (UUID is an RFC-4122 canonical UUID); the `uuid` crate is our own \
               design choice for the Rust type",
    reason: "A UUID.value is an RFC-4122 canonical UUID — use `uuid::Uuid`. (ISO_OID / \
             INTERNET_ID / OBJECT_VERSION_ID are NOT plain UUIDs.)",
}];

/// The Rust type override for `(class, field)`, or `None`.
pub(crate) fn type_override(class: &str, field: &str) -> Option<&'static str> {
    TYPE_OVERRIDES
        .iter()
        .find(|o| o.class == class && o.field == field)
        .map(|o| o.rust_type)
}

// ─────────────────────────────────────────────────────────────────────────────
// Adjudicated free-form (`serde_json::Value`) fields
// ─────────────────────────────────────────────────────────────────────────────

/// One adjudicated free-form field: `(class, field)` renders as
/// `serde_json::Value` and that degrade is a recorded decision, not an
/// oversight. The emitter writes the citation + reason as a `// NOTE:` directly
/// above the generated field, so the adjudication is readable where the untyped
/// slot is.
pub(crate) struct UntypedField {
    /// The BMM class owning (or inheriting) the field.
    pub class: &'static str,
    /// The BMM property name.
    pub field: &'static str,
    /// Spec citation, or the explicit our-own-design flag.
    pub citation: &'static str,
    /// One-line reason the field cannot be narrowed here.
    pub reason: &'static str,
}

/// Fields whose free-form JSON rendering is adjudicated rather than accidental.
///
/// Two distinct causes, both real and both recorded at the site:
/// * an inherited OPEN generic parameter no vendored declaration narrows
///   (`BMM_LITERAL_VALUE<T>.type` on `BMM_INTERVAL_VALUE`) — genuine spec
///   silence, so no [`CLASS_BINDINGS`] entry can be justified;
/// * an upstream→downstream layering inversion (a LANG class referencing an AM
///   class), where the typed field DOES appear in the downstream re-emission.
pub(crate) const UNTYPED_FIELDS: &[UntypedField] = &[
    UntypedField {
        class: "BMM_INTERVAL_VALUE",
        field: "type",
        citation: "LANG bmm3 master09-core-values.adoc §Container Literals narrows the container \
                   literals only; `…bmm3.bmm_interval_value.adoc` declares no attributes at all, \
                   so `BMM_LITERAL_VALUE<T>.type` stays open — no openEHR spec text narrows T for \
                   an interval literal",
        reason: "Spec silence: unlike BMM_CONTAINER_VALUE/BMM_INDEXED_CONTAINER_VALUE, no chapter \
                 or class definition states the interval literal's concrete type meta-class, so \
                 binding T here would be a guess.",
    },
    UntypedField {
        class: "EL_CASE",
        field: "value_constraint",
        citation: "LANG `…bmm3.el_case.adoc` §Attributes types `value_constraint: C_OBJECT` (1..1), \
                   but C_OBJECT is an AM class (`AM aom2 …c_object.adoc`) and AM depends on LANG — \
                   an upstream→downstream dependency inversion in the vendored model",
        reason: "openehr-lang cannot name an openehr-am type; the AM24 downstream re-emission of \
                 the same class DOES carry `value_constraint: CObject`, so the typed form exists \
                 exactly where C_OBJECT does.",
    },
];

/// The free-form-field adjudication for `(class, field)`, or `None`.
pub(crate) fn untyped_field(class: &str, field: &str) -> Option<&'static UntypedField> {
    UNTYPED_FIELDS
        .iter()
        .find(|u| u.class == class && u.field == field)
}

// ─────────────────────────────────────────────────────────────────────────────
// Serde field defaults (wire-omittable fields)
// ─────────────────────────────────────────────────────────────────────────────

/// One serde default: the field `(owner, field)` defaults to the literal Rust
/// expression `default` when the canonical wire omits it.
pub(crate) struct FieldDefault {
    pub owner: &'static str,
    pub field: &'static str,
    pub default: &'static str,
    pub citation: &'static str,
    pub reason: &'static str,
}

/// Serde defaults the vendored BMM does NOT state, for fields the canonical
/// wire may omit.
///
/// **This table is the residue, not the source.** The vendored schemas carry a
/// `default` facet on 44 properties ([`BmmProperty::default`]), and
/// [`vendored_default`] renders it wherever it is renderable — that derivation
/// is the primary path. Entries survive here only where the vendored input
/// states nothing and a decision is still required.
///
/// The four that remain are `Interval`'s inclusivity/boundedness flags. The
/// BASE 1.3.0 `Interval` primitive type declares all four as mandatory with NO
/// `default` facet, yet its own `Point_interval` descendant REDECLARES the same
/// four properties carrying exactly the values below — so the schema states the
/// convention once, on one descendant, and leaves the inherited sites silent.
/// These entries propagate that same statement to the inherited sites
/// (`Proper_interval`, `Multiplicity_interval`), which is also what archie and
/// EHRbase emit on the wire: a bounded limit is *included* by default, an
/// unstated limit is *bounded* by default. The value is a literal Rust
/// expression.
pub(crate) const FIELD_DEFAULTS: &[FieldDefault] = &[
    FieldDefault {
        owner: "Interval",
        field: "lower_included",
        default: "true",
        citation: "BASE docs/UML/classes/org.openehr.base.foundation_types.interval.adoc \
                   (Interval) for the semantics; the identical value is the vendored `default` \
                   facet on Point_interval.lower_included. No openEHR spec governs the wire \
                   omission itself — archie/EHRbase interop convention (our own design)",
        reason: "A bounded interval limit is included by default.",
    },
    FieldDefault {
        owner: "Interval",
        field: "upper_included",
        default: "true",
        citation: "BASE docs/UML/classes/org.openehr.base.foundation_types.interval.adoc \
                   (Interval) for the semantics; the identical value is the vendored `default` \
                   facet on Point_interval.upper_included. No openEHR spec governs the wire \
                   omission itself — archie/EHRbase interop convention (our own design)",
        reason: "A bounded interval limit is included by default.",
    },
    FieldDefault {
        owner: "Interval",
        field: "lower_unbounded",
        default: "false",
        citation: "BASE docs/UML/classes/org.openehr.base.foundation_types.interval.adoc \
                   (Interval) for the semantics; the identical value is the vendored `default` \
                   facet on Point_interval.lower_unbounded. No openEHR spec governs the wire \
                   omission itself — archie/EHRbase interop convention (our own design)",
        reason: "An unstated interval limit is bounded by default.",
    },
    FieldDefault {
        owner: "Interval",
        field: "upper_unbounded",
        default: "false",
        citation: "BASE docs/UML/classes/org.openehr.base.foundation_types.interval.adoc \
                   (Interval) for the semantics; the identical value is the vendored `default` \
                   facet on Point_interval.upper_unbounded. No openEHR spec governs the wire \
                   omission itself — archie/EHRbase interop convention (our own design)",
        reason: "An unstated interval limit is bounded by default.",
    },
];

/// Render a property's vendored `default` facet as a literal Rust expression of
/// the field's own type, or `None` when the facet cannot be one.
///
/// The facet is an **undeclared extension**: LANG
/// `docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_property.adoc`
/// §Attributes declares `name`, `is_mandatory`, `is_computed`,
/// `is_im_infrastructure`, `is_im_runtime`, `type_def` and `bmm_property` — no
/// `default` — so the vendored schemas' 44 occurrences carry no normative
/// reading and the emitter derives from them only where the facet is
/// unambiguous in the property's own declared type:
///
/// * `Boolean` ← the ODIN literals `False`/`True` (the JSON export stringifies
///   them) or a real JSON boolean.
/// * `String` ← an ODIN-QUOTED literal (`<"Boolean">` arrives as `"\"Boolean\""`).
///   The quoting is what distinguishes a string value from a stray annotation:
///   the one bare non-empty facet in the vendored set is RM
///   `RESOURCE_DESCRIPTION.parent_resource = "0..1"`, a cardinality that leaked
///   into the default slot of an `AUTHORED_RESOURCE`-typed property.
///
/// Everything else is un-renderable and is listed in [`UNRENDERABLE_DEFAULTS`],
/// so the vendored set is partitioned totally: derived, or named with a reason.
pub(crate) fn vendored_default(prop: &BmmProperty) -> Option<String> {
    let facet = prop.default.as_deref()?;
    let BmmPropKind::Single(BmmType::Simple(ty)) = &prop.kind else {
        return None;
    };
    match ty.as_str() {
        "Boolean" => match facet {
            "False" | "false" => Some("false".to_string()),
            "True" | "true" => Some("true".to_string()),
            _ => None,
        },
        "String" => {
            let inner = facet.strip_prefix('"')?.strip_suffix('"')?;
            (!inner.is_empty()).then(|| format!("::std::string::String::from({inner:?})"))
        }
        _ => None,
    }
}

/// One vendored `default` facet the emitter deliberately does NOT realize.
pub(crate) struct UnrenderableDefault {
    /// The declaring BMM class.
    pub owner: &'static str,
    /// The property carrying the facet.
    pub field: &'static str,
    /// The spec/schema evidence for leaving it unrealized.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// Every vendored `default` facet [`vendored_default`] declines to render, named
/// and cited — the completeness half of the partition, so a facet is never just
/// dropped.
///
/// A new un-renderable facet (a re-vendored schema, a pin bump) fails the
/// emitter's own test suite until it is adjudicated here.
pub(crate) const UNRENDERABLE_DEFAULTS: &[UnrenderableDefault] = &[
    UnrenderableDefault {
        owner: "RESOURCE_DESCRIPTION",
        field: "parent_resource",
        citation: "The property is typed AUTHORED_RESOURCE, not a primitive: the BASE 1.3.0 \
                   schema gives it the facet `\"\"` and the RM 1.2.0 schema `\"0..1\"` (a \
                   cardinality string in the default slot). It is also a designated owner/parent \
                   back-reference (see BACK_REFERENCES), so no struct field exists to default.",
        reason: "Class-typed back-reference; both vendored facet values are authoring slips.",
    },
    UnrenderableDefault {
        owner: "CODE_SET",
        field: "status",
        citation: "TERM 3.1.0 types the property TERMINOLOGY_STATUS and gives it the facet \
                   `\"\"`; an empty string is not a value of that type.",
        reason: "Empty facet on a non-primitive type.",
    },
    UnrenderableDefault {
        owner: "TERMINOLOGY_GROUP",
        field: "status",
        citation: "TERM 3.1.0 types the property TERMINOLOGY_STATUS and gives it the facet \
                   `\"\"`; an empty string is not a value of that type.",
        reason: "Empty facet on a non-primitive type.",
    },
    UnrenderableDefault {
        owner: "CODE",
        field: "status",
        citation: "TERM 3.1.0 types the property TERMINOLOGY_STATUS and gives it the facet \
                   `\"\"`; an empty string is not a value of that type.",
        reason: "Empty facet on a non-primitive type.",
    },
    UnrenderableDefault {
        owner: "TERMINOLOGY_CONCEPT",
        field: "status",
        citation: "TERM 3.1.0 types the property TERMINOLOGY_STATUS and gives it the facet \
                   `\"\"`; an empty string is not a value of that type.",
        reason: "Empty facet on a non-primitive type.",
    },
];

/// Whether `(owner, field)`'s vendored `default` facet is an adjudicated
/// un-renderable one.
#[must_use]
pub(crate) fn default_unrenderable(owner: &str, field: &str) -> bool {
    UNRENDERABLE_DEFAULTS
        .iter()
        .any(|u| u.owner == owner && u.field == field)
}

/// A container attribute whose vendored cardinality lower bound is CONTRADICTED
/// by the same release's normative syntax, with the citation that resolves it.
///
/// The emitter never softens a bound on its own — a `1..*` is emitted as
/// `NonEmptyVec<T>` precisely so the model carries it. An entry here exists
/// only where the vendored release states the bound BOTH ways and the syntax
/// chapter (which defines the persistence form and shows conformant instances)
/// governs over the UML class table.
struct CardinalityContradiction {
    /// The declaring BMM class.
    owner: &'static str,
    /// The container attribute.
    field: &'static str,
    /// The spec text that resolves the contradiction.
    #[expect(
        dead_code,
        reason = "the citation is this decision map's spec record — it exists to be read at review, exactly like every other override map's citation field, and is deliberately not consumed by emitted text"
    )]
    citation: &'static str,
    /// One-line reason.
    #[expect(
        dead_code,
        reason = "the reason is this decision map's spec record — read at review, not consumed by emitted text"
    )]
    reason: &'static str,
}

/// Every adjudicated cardinality contradiction in the vendored inputs.
static CARDINALITY_CONTRADICTIONS: &[CardinalityContradiction] = &[CardinalityContradiction {
    owner: "P_BMM_GENERIC_TYPE",
    field: "generic_parameter_defs",
    citation: "docs/specs/openehr/LANG/docs/bmm_persistence/master04-syntax.adoc \u{a7}Generic                Classes: \"within `P_BMM_GENERIC_TYPE`, use `_generic_parameters_` for a list of                string types; use `_generic_parameter_defs_` for a list of complex type                references\" \u{2014} the two are ALTERNATIVES, and the chapter\u{2019}s own                example writes `root_type = <\"DV_INTERVAL\"> generic_parameters =                <\"DV_QUANTITY\">` with no `generic_parameter_defs` at all. The UML class table                (docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_generic_type.adoc)                states 1..1 for the same attribute, which would make that example \u{2014} and                every string-parameterised generic type in a real .bmm schema \u{2014} invalid,                and would leave the 0..1 `generic_parameters` attribute dead. The syntax                chapter governs.",
    reason: "Reported upstream as a spec defect (tracker issue #1717); the syntax chapter's reading is emitted.",
}];

/// Whether `(owner, field)`'s vendored cardinality lower bound is an adjudicated
/// contradiction that the emitter must NOT realize as a non-empty container.
#[must_use]
pub(crate) fn cardinality_contradicted(owner: &str, field: &str) -> bool {
    CARDINALITY_CONTRADICTIONS
        .iter()
        .any(|c| c.owner == owner && c.field == field)
}

/// The serde default expression for a resolved property, or `None`.
///
/// The vendored `default` facet on the DECLARING class wins; [`FIELD_DEFAULTS`]
/// supplies only what the schemas leave unstated. The two can never disagree —
/// an entry in the hand table for a property that carries a renderable facet
/// fails the emitter's own test suite.
pub(crate) fn field_default(owner: &str, prop: &BmmProperty) -> Option<String> {
    vendored_default(prop).or_else(|| {
        FIELD_DEFAULTS
            .iter()
            .find(|d| d.owner == owner && d.field == prop.name)
            .map(|d| d.default.to_string())
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// ITS-REST OAS schema names that ARE spec types under a different spelling
// ─────────────────────────────────────────────────────────────────────────────

/// One ITS-REST OAS component schema whose *key* name does not match the spec
/// class it renders, mapped to the generated spec type it must resolve to.
pub(crate) struct OasMonomorphization {
    /// The OAS `components/schemas` key.
    pub schema: &'static str,
    /// The `title` that same schema declares — the openEHR spec name, which is
    /// what grounds the mapping in the vendored input rather than in a guess.
    pub title: &'static str,
    /// The fully-qualified generated Rust type.
    pub rust_type: &'static str,
    /// The spec/OAS evidence.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// OAS component schemas that ARE generated spec types spelled differently.
///
/// `emit_rest` binds a `$ref` to a spec type by matching the schema KEY against
/// the emittable class names, which works for the ~200 schemas the released
/// bundles name after their class (`Ehr`, `DvText`, …) and fails for the five
/// below: OAS keys must be unique and ASCII, so the released bundles rename a
/// class whose Rust name collides (`Clstr`) and give each generic
/// INSTANTIATION its own key (`DvIntervalOfDate`, `ObjectRefOfHierObjectId`).
/// Every one of them declares its real spec name in `title`, so the mapping is
/// read out of the vendored bundle, not invented — the emitter's test suite
/// asserts each entry's `title` against the bundle.
///
/// Without the mapping they emit as DTO structs, which is doubly wrong: the
/// payload loses the spec type's strict canonical-JSON reader, and the DTO is
/// `allOf`-truncated (it carries only the schema's OWN `properties`, so
/// `Clstr` loses every `ITEM`/`LOCATABLE` member and `ObjectRefOfHierObjectId`
/// loses the mandatory `namespace` and `type`).
pub(crate) const OAS_MONOMORPHIZATIONS: &[OasMonomorphization] = &[
    OasMonomorphization {
        schema: "Clstr",
        title: "CLUSTER",
        rust_type: "openehr_rm::prelude::Cluster",
        citation: "The schema declares `title: CLUSTER` and
                   `x-discriminator-value: CLUSTER`, and every ITEM discriminator mapping in the \
                   released bundles routes `CLUSTER: '#/components/schemas/Clstr'`. RM \
                   docs/data_structures/master03-item_structure_package.adoc §CLUSTER is the \
                   class; `openehr_rm::prelude::Cluster` is its generated form.",
        reason: "OAS key abbreviated to avoid a name clash; the class is RM CLUSTER.",
    },
    OasMonomorphization {
        schema: "DvIntervalOfDate",
        title: "DV_INTERVAL_of_DATE",
        rust_type: "openehr_rm::prelude::DvInterval<openehr_rm::prelude::DvDate>",
        citation: "The schema declares `title: DV_INTERVAL_of_DATE`, composes \
                   `allOf: [DvInterval]`, and narrows `lower`/`upper` to `DvDate`. RM \
                   docs/data_types/master04-quantity_package.adoc §DV_INTERVAL is the generic \
                   class `DV_INTERVAL<T: DV_ORDERED>`; this is its DV_DATE instantiation.",
        reason: "A generic instantiation the OAS must give a flat key; the class is DV_INTERVAL.",
    },
    OasMonomorphization {
        schema: "DvIntervalOfDateTime",
        title: "DV_INTERVAL_of_DATE_TIME",
        rust_type: "openehr_rm::prelude::DvInterval<openehr_rm::prelude::DvDateTime>",
        citation: "The schema declares `title: DV_INTERVAL_of_DATE_TIME`, composes \
                   `allOf: [DvInterval]`, and narrows `lower`/`upper` to `DvDateTime`. RM \
                   docs/data_types/master04-quantity_package.adoc §DV_INTERVAL is the generic \
                   class `DV_INTERVAL<T: DV_ORDERED>`; this is its DV_DATE_TIME instantiation.",
        reason: "A generic instantiation the OAS must give a flat key; the class is DV_INTERVAL.",
    },
    OasMonomorphization {
        schema: "ObjectRefOfHierObjectId",
        title: "OBJECT_REF",
        rust_type: "openehr_base::prelude::ObjectRef",
        citation: "The schema declares `title: OBJECT_REF`, composes `allOf: [ObjectRef]`, and \
                   narrows `id` to `HierObjectId`. BASE \
                   docs/base_types/master05-identification_package.adoc §OBJECT_REF declares \
                   `id: OBJECT_ID`, of which HIER_OBJECT_ID is a subtype, so the generated \
                   `openehr_base::prelude::ObjectRef` already accepts this narrowing — and \
                   carries the mandatory `namespace`/`type` members the truncated DTO dropped.",
        reason: "An id-narrowed OBJECT_REF given its own OAS key; the class is OBJECT_REF.",
    },
    OasMonomorphization {
        schema: "ObjectRefOfObjectVersionId",
        title: "OBJECT_REF",
        rust_type: "openehr_base::prelude::ObjectRef",
        citation: "The schema declares `title: OBJECT_REF`, composes `allOf: [ObjectRef]`, and \
                   narrows `id` to `ObjectVersionId`. BASE \
                   docs/base_types/master05-identification_package.adoc §OBJECT_REF declares \
                   `id: OBJECT_ID`, of which OBJECT_VERSION_ID is a subtype, so the generated \
                   `openehr_base::prelude::ObjectRef` already accepts this narrowing — and \
                   carries the mandatory `namespace`/`type` members the truncated DTO dropped.",
        reason: "An id-narrowed OBJECT_REF given its own OAS key; the class is OBJECT_REF.",
    },
];

/// The generated spec type an OAS component schema key resolves to, or `None`
/// when the key is not one of the renamed/monomorphized spellings.
#[must_use]
pub(crate) fn oas_monomorphization(schema: &str) -> Option<&'static str> {
    OAS_MONOMORPHIZATIONS
        .iter()
        .find(|m| m.schema == schema)
        .map(|m| m.rust_type)
}

// ─────────────────────────────────────────────────────────────────────────────
// Owner/parent back-references (cycle-breaking, non-emitted)
// ─────────────────────────────────────────────────────────────────────────────

/// One owner/parent back-reference: `(class, field)` is a navigational pointer
/// from a part to the whole that owns it, omitted from the emitted struct.
pub(crate) struct BackReference {
    pub class: &'static str,
    pub field: &'static str,
    /// The spec citation naming the field a back-reference. **This string is
    /// emitted verbatim** into a `// NOTE:` in the generated struct (see
    /// `render::emit::render_struct_def`), so it is byte-load-bearing: changing
    /// it changes generated output.
    pub citation: &'static str,
    /// One-line reason (the cycle it breaks) — documentation only, not emitted.
    pub reason: &'static str,
}

/// Mandatory single-valued properties designated as **owner/parent
/// back-references** — a navigational association pointing from a part to the
/// whole that owns it, not forward-owned data.
///
/// # Why the emitter special-cases these (owner ruling 2026-07-19)
///
/// The spec is written in reference-semantics languages where an owner pointer
/// is a trivially-satisfiable back-pointer. Under Rust value semantics an
/// OWNING mandatory back-reference makes the type a non-constructible infinite
/// value (every `ARCHETYPE` owns a `terminology` whose `owner_archetype` is an
/// `ARCHETYPE`), so each is emitted as a non-data back-reference: omitted from
/// the struct fields and from serde, behavioural access left to the
/// hand-written `*_impl.rs`. They appear on no canonical wire either.
///
/// Every genuine composition field stays mandatory: `Model::assert_constructible`
/// proves every remaining cycle is broken only at an edge designated here. The
/// BMM carries no `is_im_runtime`/`is_im_infrastructure` flag on these, so the
/// designation is an explicit, spec-cited override.
pub(crate) const BACK_REFERENCES: &[BackReference] = &[
    BackReference {
        class: "ARCHETYPE_TERMINOLOGY",
        field: "owner_archetype",
        citation: "AM AOM2 archetype_terminology (owner_archetype: Archetype that owns this terminology)",
        reason: "Back-pointer to the owning archetype; forms the ARCHETYPE ↔ \
                 ARCHETYPE_TERMINOLOGY cycle. \
                 (docs/specs/openehr/AM/docs/UML/classes/\
                 org.openehr.am.aom2.archetype_terminology.adoc)",
    },
    BackReference {
        class: "RESOURCE_DESCRIPTION",
        field: "parent_resource",
        citation: "BASE resource + RM common.resource resource_description (parent_resource: \
                   Reference to owning resource; the class name occurs in both components \
                   with the same back-reference)",
        reason: "Back-pointer to the owning resource; forms the AUTHORED_RESOURCE ↔ \
                 RESOURCE_DESCRIPTION cycle. \
                 (docs/specs/openehr/BASE/docs/UML/classes/\
                 org.openehr.base.resource.resource_description.adoc; \
                 docs/specs/openehr/RM/docs/UML/classes/\
                 org.openehr.rm.common.resource_description.adoc)",
    },
    BackReference {
        class: "ARCHETYPE_ONTOLOGY",
        field: "parent_archetype",
        citation: "AM AOM14 archetype_ontology (parent_archetype: Archetype which owns this terminology)",
        reason: "The ADL 1.4 owner back-reference (v1_4 analogue of v2_4 owner_archetype), with \
                 the invariant parent_archetype.ontology = Current; forms the ARCHETYPE ↔ \
                 ARCHETYPE_ONTOLOGY cycle. \
                 (docs/specs/openehr/AM/docs/UML/classes/\
                 org.openehr.am.aom14.archetype_ontology.adoc)",
    },
    // ── LANG 1.0.0 `scope`: the same declaring-context back-reference ─────────
    // The released 1.0.0 model declares it on BMM_DECLARATION (the
    // pre-SPECLANG-14 spelling of the v3 BMM_MODEL_ELEMENT edge below): an
    // owning field would make every declaration an infinite value.
    BackReference {
        class: "BMM_DECLARATION",
        field: "scope",
        citation: "LANG 1.0.0 BMM_DECLARATION (scope: the model element within which the \
                   declaration appears — the declaring-context edge the 1.1.0 line restates \
                   on BMM_MODEL_ELEMENT)",
        reason: "The declaring-context back-reference of the released 1.0.0 generation — the \
                 same owner/parent edge as the v3 entries below, keyed by that model's own \
                 declaring class. \
                 (tools/openehr-codegen/vendor/bmm/components/LANG/json/\
                 openehr_lang_1.0.0.bmm.json)",
    },
    // ── LANG BMM v3 `scope`: the declaring-context back-reference ──────────────
    // The v3 generation makes every model element name its declaring context,
    // and the root's context is ITSELF (`is_root_scope(): Result = (scope =
    // self)`), so an owning field would make every BMM_MODEL_ELEMENT an infinite
    // value. Four subtypes redefine the attribute more narrowly; each
    // redefinition is the same owner/parent edge and needs its own entry,
    // because the flattened property is attributed to its declaring class.
    BackReference {
        class: "BMM_MODEL_ELEMENT",
        field: "scope",
        citation: "LANG BMM3 bmm_model_element (scope: Model element within which an element is declared; Post_result of is_root_scope: Result = (scope = self))",
        reason: "The declaring-context back-reference of the BMM v3 generation, self-referential \
                 at the root of a model hierarchy. \
                 (docs/specs/openehr/LANG/docs/UML/classes/\
                 org.openehr.lang.bmm3.bmm_model_element.adoc)",
    },
    BackReference {
        class: "BMM_MODULE",
        field: "scope",
        citation: "LANG BMM3 bmm_module (scope: BMM_MODEL — redefinition of BMM_MODEL_ELEMENT.scope)",
        reason: "The same declaring-context back-reference, narrowed to the owning model; forms \
                 the BMM_MODEL ↔ BMM_CLASS cycle. \
                 (docs/specs/openehr/LANG/docs/UML/classes/\
                 org.openehr.lang.bmm3.bmm_module.adoc)",
    },
    BackReference {
        class: "BMM_PACKAGE_CONTAINER",
        field: "scope",
        citation: "LANG BMM3 bmm_package_container (scope: BMM_PACKAGE_CONTAINER — redefinition of BMM_MODEL_ELEMENT.scope)",
        reason: "The same declaring-context back-reference, narrowed to the enclosing package \
                 container; forms the BMM_PACKAGE ↔ BMM_PACKAGE_CONTAINER cycle. \
                 (docs/specs/openehr/LANG/docs/UML/classes/\
                 org.openehr.lang.bmm3.bmm_package_container.adoc)",
    },
    BackReference {
        class: "BMM_FEATURE",
        field: "scope",
        citation: "LANG BMM3 bmm_feature (scope: BMM_CLASS — redefinition of BMM_MODEL_ELEMENT.scope)",
        reason: "The same declaring-context back-reference, narrowed to the owning class; forms \
                 the BMM_CLASS ↔ BMM_FEATURE cycle. \
                 (docs/specs/openehr/LANG/docs/UML/classes/\
                 org.openehr.lang.bmm3.bmm_feature.adoc)",
    },
    BackReference {
        class: "BMM_VARIABLE",
        field: "scope",
        citation: "LANG BMM3 bmm_variable (scope: BMM_ROUTINE — redefinition of BMM_MODEL_ELEMENT.scope)",
        reason: "The same declaring-context back-reference, narrowed to the owning routine; forms \
                 the BMM_ROUTINE ↔ BMM_VARIABLE cycle. \
                 (docs/specs/openehr/LANG/docs/UML/classes/\
                 org.openehr.lang.bmm3.bmm_variable.adoc)",
    },
];

/// The spec citation if `(class, field)` is a designated owner/parent
/// back-reference, else `None`.
///
/// The returned string is emitted verbatim into generated output, so it is
/// byte-stable.
pub(crate) fn back_reference(class: &str, field: &str) -> Option<&'static str> {
    BACK_REFERENCES
        .iter()
        .find(|b| b.class == class && b.field == field)
        .map(|b| b.citation)
}

// ─────────────────────────────────────────────────────────────────────────────
// emit-xml BMM-only field allowlist (RM/BASE model vs vendored ITS-XML skew)
// ─────────────────────────────────────────────────────────────────────────────

/// One allowlisted BMM-only field: the owning concrete type's spec name, the
/// field's wire name, the governing spec-version skew, and a one-line reason
/// bucket.
pub(crate) struct XmlBmmOnlyField {
    /// The owning concrete type's spec name.
    pub spec: &'static str,
    /// The field's openEHR wire (property) name.
    pub wire_name: &'static str,
    /// Why the field is absent from the vendored XSD and safe to append — the
    /// concrete RM/BASE-model-vs-ITS-XML-XSD version delta.
    pub citation: &'static str,
    /// One-line reason bucket.
    pub reason: &'static str,
}

/// Allowlist of BMM fields that have **no** matching element or attribute in any
/// vendored ITS-XML XSD (the RM/BASE model outran the vendored ITS-XML schemas).
/// Each accepted `(spec, wire_name)` pair is emitted as a **deterministic
/// trailing element** in BMM field order (so it is never silently dropped), and
/// carries the spec delta that justifies it. Any XSD-covered struct with a BMM
/// field NOT on this list fails codegen (the `check_bmm_field_coverage` guard in
/// `render::emit_xml`), forcing an explicit decision instead of a silent drop.
///
/// The governing spec citation for every entry is the pinned-version delta: the
/// emitted model is RM 1.2.0 / BASE 1.3.0, while the
/// vendored canonical-XML schemas are ITS-XML 1.0.2 (namespace `.../v1`) and, for
/// the EHR/demographic/extract closure, ITS-XML 2.0.0 (RM 1.1.0). Where the model
/// added or renamed a field after those XSDs were cut, no XSD slot exists.
///
/// Empty is the desired state: it means every RM-1.2.0 BMM field of an
/// XSD-covered type maps to a vendored-XSD slot.
pub(crate) const XML_BMM_ONLY_ALLOWLIST: &[XmlBmmOnlyField] = &[
    // ── BASE 1.3.0 resource-class growth (ITS-XML 1.0.2 Resource.xsd predates it) ──
    XmlBmmOnlyField {
        spec: "AUTHORED_RESOURCE",
        wire_name: "uid",
        citation: "BASE 1.3.0 added AUTHORED_RESOURCE.uid (HIER_OBJECT_ID); absent from ITS-XML 1.0.2 Resource.xsd.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "AUTHORED_RESOURCE",
        wire_name: "annotations",
        citation: "BASE 1.3.0 added AUTHORED_RESOURCE.annotations (RESOURCE_ANNOTATIONS); absent from ITS-XML 1.0.2 Resource.xsd.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "title",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.title; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "original_namespace",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.original_namespace; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "original_publisher",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.original_publisher; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "custodian_namespace",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.custodian_namespace; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "custodian_organisation",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.custodian_organisation; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "copyright",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.copyright; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "licence",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.licence; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "ip_acknowledgements",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.ip_acknowledgements; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "references",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.references; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "conversion_details",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.conversion_details; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "TRANSLATION_DETAILS",
        wire_name: "version_last_translated",
        citation: "BASE 1.3.0 TRANSLATION_DETAILS.version_last_translated; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "TRANSLATION_DETAILS",
        wire_name: "other_contributors",
        citation: "BASE 1.3.0 TRANSLATION_DETAILS.other_contributors; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "TRANSLATION_DETAILS",
        wire_name: "accreditaton",
        citation: "BASE 1.3.0 BMM spells this field `accreditaton` (sic); ITS-XML 1.0.2 Resource.xsd uses `accreditation`. Canonical JSON derives from the BMM, so the BMM spelling is emitted for JSON/XML consistency.",
        reason: "BASE 1.3.0 BMM field spelling vs ITS-XML 1.0.2.",
    },
    // ── BASE 1.3.0 CODE_PHRASE.preferred_term ──
    XmlBmmOnlyField {
        spec: "CODE_PHRASE",
        wire_name: "preferred_term",
        citation: "BASE 1.3.0 added CODE_PHRASE.preferred_term; ITS-XML 1.0.2 BaseTypes.xsd CODE_PHRASE has terminology_id + code_string only.",
        reason: "BASE 1.3.0 field addition vs ITS-XML 1.0.2.",
    },
    // ── RM 1.2.0 ENTRY.work_flow_id → workflow_id rename (all ENTRY subtypes) ──
    XmlBmmOnlyField {
        spec: "ACTION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id. Canonical JSON corpus uses workflow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "ADMIN_ENTRY",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "EVALUATION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "INSTRUCTION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "OBSERVATION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    // ── RM 1.2.0 data-type / data-structure additions ──
    XmlBmmOnlyField {
        spec: "DV_QUANTITY",
        wire_name: "units_system",
        citation: "RM 1.2.0 added DV_QUANTITY.units_system; absent from ITS-XML 1.0.2 BaseTypes.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "DV_QUANTITY",
        wire_name: "units_display_name",
        citation: "RM 1.2.0 added DV_QUANTITY.units_display_name; absent from ITS-XML 1.0.2 BaseTypes.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "ELEMENT",
        wire_name: "null_reason",
        citation: "RM 1.2.0 added ELEMENT.null_reason; ITS-XML 1.0.2 Structure.xsd ELEMENT has value + null_flavour only.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "ISM_TRANSITION",
        wire_name: "reason",
        citation: "RM 1.2.0 added ISM_TRANSITION.reason (List<DV_TEXT>); ITS-XML 1.0.2 Content.xsd has current_state/transition/careflow_step only.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "FEEDER_AUDIT_DETAILS",
        wire_name: "other_details",
        citation: "RM 1.2.0 added FEEDER_AUDIT_DETAILS.other_details (ITEM_STRUCTURE); absent from ITS-XML 1.0.2 Common.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "FOLDER",
        wire_name: "details",
        citation: "RM 1.2.0 added FOLDER.details (ITEM_STRUCTURE); absent from ITS-XML 1.0.2 Composition.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "EHR",
        wire_name: "tags",
        citation: "RM 1.2.0 added EHR.tags; absent from ITS-XML 2.0.0 (RM 1.1.0) Ehr.xsd, the vendored EHR shape.",
        reason: "RM 1.2.0 field addition vs ITS-XML 2.0.0 (RM 1.1.0).",
    },
    // ── RM 1.2.0 EhrExtract includes_* → include_* renames ──
    XmlBmmOnlyField {
        spec: "EXTRACT_SPEC",
        wire_name: "include_multimedia",
        citation: "RM 1.2.0 renamed EXTRACT_SPEC.includes_multimedia → include_multimedia; ITS-XML 2.0.0 EhrExtract.xsd still declares includes_multimedia.",
        reason: "RM 1.2.0 EhrExtract includes_* → include_* rename vs ITS-XML 2.0.0.",
    },
    XmlBmmOnlyField {
        spec: "EXTRACT_VERSION_SPEC",
        wire_name: "include_revision_history",
        citation: "RM 1.2.0 renamed EXTRACT_VERSION_SPEC.includes_revision_history → include_revision_history; ITS-XML 2.0.0 EhrExtract.xsd still declares includes_revision_history.",
        reason: "RM 1.2.0 EhrExtract includes_* → include_* rename vs ITS-XML 2.0.0.",
    },
    XmlBmmOnlyField {
        spec: "EXTRACT_VERSION_SPEC",
        wire_name: "include_data",
        citation: "RM 1.2.0 renamed EXTRACT_VERSION_SPEC.includes_data → include_data; ITS-XML 2.0.0 EhrExtract.xsd still declares includes_data.",
        reason: "RM 1.2.0 EhrExtract includes_* → include_* rename vs ITS-XML 2.0.0.",
    },
    // ── VERSIONED_OBJECT base fields on the VERSIONED_* container types ──
    // uid/owner_id/time_created come from VERSIONED_OBJECT, defined ONLY in the
    // v2 Common.xsd (RM 1.1.0), which the emit-xml input deliberately does not
    // merge: v1 Version.xsd omits VERSIONED_OBJECT, and pulling in v2 Common.xsd
    // would re-shape the served VERSION-family types. These container types are
    // not served as canonical XML (406 at the REST edge); the base fields are
    // appended in canonical VERSIONED_OBJECT order so nothing is lost.
    XmlBmmOnlyField {
        spec: "VERSIONED_COMPOSITION",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd (see module note).",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_COMPOSITION",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_COMPOSITION",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_ACCESS",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_ACCESS",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_ACCESS",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_STATUS",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_STATUS",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_STATUS",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_PARTY",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_PARTY",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_PARTY",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
];

/// Is `(spec, wire_name)` an accepted BMM-only field (on the allowlist)?
pub(crate) fn xml_bmm_only_allowed(spec: &str, wire_name: &str) -> bool {
    XML_BMM_ONLY_ALLOWLIST
        .iter()
        .any(|e| e.spec == spec && e.wire_name == wire_name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Assertion-dialect leaf-predicate → runtime-function table
// ─────────────────────────────────────────────────────────────────────────────

/// One assertion-dialect leaf predicate the invariant classifier recognises
/// (`crate::analyze::invariants::RUNTIME_PREDICATES`), mapped to the named
/// runtime function the invariant-core layer calls to realize it. This is the
/// declarative bridge between the BMM assertion spelling and the hand-written
/// runtime in `openehr-rm`'s `validate.rs`.
pub(crate) struct DialectPredicate {
    /// The BMM assertion-dialect predicate spelling, as it appears in a
    /// `BMM_CLASS.invariants` expression (e.g. `valid_iso8601_date`).
    pub predicate: &'static str,
    /// The runtime function realizing it (in `openehr_rm::validate`, or a spec
    /// method for `is_valid_match_code`).
    pub runtime_fn: &'static str,
    /// Spec citation (BMM/RM section) for the predicate.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// The assertion-dialect predicate → runtime-function map. Its predicate set is
/// exactly `crate::analyze::invariants::RUNTIME_PREDICATES` (the classifier's
/// recognised runtime-backed leaves) — the emitter-invariant suite pins the two
/// in lockstep, so a new recognised predicate without a runtime hook fails CI.
pub(crate) const DIALECT_PREDICATES: &[DialectPredicate] = &[
    DialectPredicate {
        predicate: "valid_iso8601_date",
        runtime_fn: "openehr_rm::validate::is_valid_iso_date",
        citation: "BASE Iso8601_date (docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_date.adoc)",
        reason: "ISO-8601 date well-formedness (openEHR partial-precision subset).",
    },
    DialectPredicate {
        predicate: "valid_iso8601_time",
        runtime_fn: "openehr_rm::validate::is_valid_iso_time",
        citation: "BASE Iso8601_time (docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_time.adoc)",
        reason: "ISO-8601 time well-formedness (openEHR partial-precision subset).",
    },
    DialectPredicate {
        predicate: "valid_iso8601_date_time",
        runtime_fn: "openehr_rm::validate::is_valid_iso_date_time",
        citation: "BASE Iso8601_date_time (docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_date_time.adoc)",
        reason: "ISO-8601 date-time well-formedness (openEHR partial-precision subset).",
    },
    DialectPredicate {
        predicate: "valid_iso8601_duration",
        runtime_fn: "openehr_rm::validate::is_valid_iso_duration",
        citation: "BASE Iso8601_duration (docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_duration.adoc)",
        reason: "ISO-8601 duration well-formedness (openEHR sign + W designator deviation).",
    },
    DialectPredicate {
        predicate: "valid_percentage",
        runtime_fn: "openehr_rm::validate::valid_percentage",
        citation: "RM DV_AMOUNT.Accuracy_validity (docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_amount.adoc)",
        reason: "0 <= v <= 100 for a percent-recorded accuracy.",
    },
    DialectPredicate {
        predicate: "valid_magnitude_status",
        runtime_fn: "openehr_rm::validate::valid_magnitude_status",
        citation: "RM DV_QUANTIFIED.Magnitude_status_valid (docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_quantified.adoc)",
        reason: "magnitude_status is one of = < > <= >= ~.",
    },
    DialectPredicate {
        predicate: "valid_proportion_kind",
        runtime_fn: "openehr_rm::validate::valid_proportion_kind",
        citation: "RM PROPORTION_KIND (docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.proportion_kind.adoc)",
        reason: "proportion kind code is one of 0..=4.",
    },
    DialectPredicate {
        predicate: "is_valid_match_code",
        runtime_fn: "openehr_rm::data_types::text::term_mapping::TermMapping::is_valid_match_code",
        citation: "RM TERM_MAPPING.Match_valid (docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.term_mapping.adoc)",
        reason: "TERM_MAPPING.match is one of < = > ?.",
    },
];

// ─────────────────────────────────────────────────────────────────────────────
// Emittable-invariant realization register
// ─────────────────────────────────────────────────────────────────────────────

/// The vendored RM class-documentation directory every [`InvariantRealization`]
/// citation is relative to.
pub(crate) const RM_CLASS_DOCS: &str = "docs/specs/openehr/RM/docs/UML/classes";

/// The vendored class-doc directory for `spec_file`, by its component
/// package: `org.openehr.base.*` classes are documented under the BASE
/// component, everything else under the RM one.
#[must_use]
pub(crate) fn class_doc_dir(spec_file: &str) -> &'static str {
    if spec_file.starts_with("org.openehr.base.") {
        "docs/specs/openehr/BASE/docs/UML/classes"
    } else {
        RM_CLASS_DOCS
    }
}

/// Where an assertion-dialect **emittable** RM class invariant
/// (`crate::analyze::invariants::Bucket::Emitted`) is realized.
///
/// The classifier's `Emitted` verdict says an expression *could* be evaluated
/// mechanically — it does NOT say anything is evaluating it. Without this
/// register an invariant classified emittable but realized nowhere is
/// indistinguishable from one a core enforces, so it disappears silently; every
/// emittable invariant therefore carries a venue here, and the emitter-invariant
/// suite fails on any that carries none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvariantVenue {
    /// A generated invariant core in `openehr-rm`'s `validate/generated.rs`
    /// (`site` = the core function name).
    Core,
    /// A hand-written realization in `openehr-rm` (`site` = the file): either a
    /// typed `Validate` impl in a `*_impl.rs` sibling, or one of the JSON-level
    /// per-node checks in `validate.rs` that exist precisely because the typed
    /// model cannot express the rule (a BMM `List` emits as a `Vec`, so absent
    /// and present-but-empty collapse to one value once deserialized).
    Impl,
    /// The wire boundary in `openehr-its` (`site` = the file) — a rule whose
    /// inputs the wire walker holds and a per-node RM core does not.
    Wire,
    /// A write-boundary check in the application (`site` = the file under
    /// `app/`) — a rule whose inputs only the inbound request body still
    /// carries. The commonest case is a `X /= Void implies not X.is_empty`
    /// list rule: the BMM `List` emits as a `Vec`, so absent and
    /// present-but-empty collapse to one value in the typed model and neither
    /// a core nor the wire walker can separate them, while the raw JSON body
    /// still can.
    App,
    /// Adjudicated out of the per-node invariant layer: an aggregate /
    /// cross-object constraint owned by another layer, or an assertion that is
    /// vacuous on stored data. `site` is empty.
    Excluded,
    /// Classified emittable, realized nowhere yet — the honest pending list.
    /// `site` is empty.
    Unrealized,
}

impl InvariantVenue {
    /// The register-section heading this venue renders under.
    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Core => "Realized by a generated core in this file",
            Self::Impl => "Realized in hand-written `openehr-rm` code",
            Self::Wire => "Realized at the wire boundary (`openehr-its`)",
            Self::App => "Realized at the application write boundary (`app/`)",
            Self::Excluded => "Adjudicated out of the per-node invariant layer",
            Self::Unrealized => "Classified emittable, realized nowhere yet",
        }
    }
}

/// One emittable RM class invariant and the venue realizing it.
#[derive(Debug)]
pub(crate) struct InvariantRealization {
    /// The owning BMM class name.
    pub class: &'static str,
    /// The BMM invariant name.
    pub name: &'static str,
    /// Where it is realized.
    pub venue: InvariantVenue,
    /// The realizing site: a core function name ([`InvariantVenue::Core`]) or a
    /// repo-relative file ([`InvariantVenue::Impl`] / [`InvariantVenue::Wire`] /
    /// [`InvariantVenue::App`]); empty for the two non-realizing venues.
    pub site: &'static str,
    /// The class's vendored spec file under [`RM_CLASS_DOCS`] (§Invariants).
    pub spec_file: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// Every assertion-dialect-emittable RM class invariant, with the venue that
/// realizes it. Declarative decision data (the same shape as
/// [`DIALECT_PREDICATES`]): the *set* is derived — it must equal the
/// classifier's `Emitted` verdicts over the RM schema, which the
/// emitter-invariant suite checks in both directions — while each row's venue,
/// site and reason are adjudicated once, here, with the spec citation.
pub(crate) const INVARIANT_REALIZATIONS: &[InvariantRealization] = &[
    InvariantRealization {
        class: "ACTIVITY",
        name: "Action_archetype_id_valid",
        venue: InvariantVenue::Core,
        site: "activity_core",
        spec_file: "org.openehr.rm.composition.activity.adoc",
        reason: "the ACTIVITY core, called by `activity_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "ARCHETYPED",
        name: "Rm_version_valid",
        venue: InvariantVenue::Core,
        site: "archetyped_core",
        spec_file: "org.openehr.rm.common.archetyped.adoc",
        reason: "the ARCHETYPED core, called by `archetyped_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "CODE_PHRASE",
        name: "Code_string_valid",
        venue: InvariantVenue::Core,
        site: "code_phrase_core",
        spec_file: "org.openehr.rm.data_types.code_phrase.adoc",
        reason: "the CODE_PHRASE core, called by `code_phrase_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "COMPOSITION",
        name: "Is_archetype_root",
        venue: InvariantVenue::Core,
        site: "composition_core",
        spec_file: "org.openehr.rm.composition.composition.adoc",
        reason: "`is_archetype_root` is `archetype_details /= Void` (locatable.adoc §Functions).",
    },
    InvariantRealization {
        class: "DV_AMOUNT",
        name: "Accuracy_is_percent_validity",
        venue: InvariantVenue::Core,
        site: "dv_amount_core",
        spec_file: "org.openehr.rm.data_types.dv_amount.adoc",
        reason: "shared by every concrete DV_AMOUNT descendant.",
    },
    InvariantRealization {
        class: "DV_AMOUNT",
        name: "Accuracy_validity",
        venue: InvariantVenue::Core,
        site: "dv_amount_core",
        spec_file: "org.openehr.rm.data_types.dv_amount.adoc",
        reason: "shared by every concrete DV_AMOUNT descendant.",
    },
    InvariantRealization {
        class: "DV_DATE",
        name: "Value_valid",
        venue: InvariantVenue::Core,
        site: "temporal_value_core",
        spec_file: "org.openehr.rm.data_types.dv_date.adoc",
        reason: "the ISO-8601 date validator supplies the verdict.",
    },
    InvariantRealization {
        class: "DV_DATE_TIME",
        name: "Value_valid",
        venue: InvariantVenue::Core,
        site: "temporal_value_core",
        spec_file: "org.openehr.rm.data_types.dv_date_time.adoc",
        reason: "the ISO-8601 date-time validator supplies the verdict.",
    },
    InvariantRealization {
        class: "DV_DURATION",
        name: "Value_valid",
        venue: InvariantVenue::Core,
        site: "temporal_value_core",
        spec_file: "org.openehr.rm.data_types.dv_duration.adoc",
        reason: "the ISO-8601 duration validator supplies the verdict.",
    },
    InvariantRealization {
        class: "DV_TIME",
        name: "Value_valid",
        venue: InvariantVenue::Core,
        site: "temporal_value_core",
        spec_file: "org.openehr.rm.data_types.dv_time.adoc",
        reason: "the ISO-8601 time validator supplies the verdict.",
    },
    InvariantRealization {
        class: "DV_IDENTIFIER",
        name: "Id_valid",
        venue: InvariantVenue::Core,
        site: "dv_identifier_core",
        spec_file: "org.openehr.rm.data_types.dv_identifier.adoc",
        reason: "called by `dv_identifier_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "DV_PARSABLE",
        name: "Formalism_valid",
        venue: InvariantVenue::Core,
        site: "dv_parsable_core",
        spec_file: "org.openehr.rm.data_types.dv_parsable.adoc",
        reason: "called by `dv_parsable_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Fraction_validity",
        venue: InvariantVenue::Core,
        site: "dv_proportion_core",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "the DV_PROPORTION core evaluates all six own invariants.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Percent_validity",
        venue: InvariantVenue::Core,
        site: "dv_proportion_core",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "the DV_PROPORTION core evaluates all six own invariants.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Precision_validity",
        venue: InvariantVenue::Core,
        site: "dv_proportion_core",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "the DV_PROPORTION core evaluates all six own invariants.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Type_validity",
        venue: InvariantVenue::Core,
        site: "dv_proportion_core",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "the DV_PROPORTION core evaluates all six own invariants.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Unitary_validity",
        venue: InvariantVenue::Core,
        site: "dv_proportion_core",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "the DV_PROPORTION core evaluates all six own invariants.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Valid_denominator",
        venue: InvariantVenue::Core,
        site: "dv_proportion_core",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "the DV_PROPORTION core evaluates all six own invariants.",
    },
    InvariantRealization {
        class: "DV_QUANTIFIED",
        name: "Magnitude_status_valid",
        venue: InvariantVenue::Core,
        site: "magnitude_status_core",
        spec_file: "org.openehr.rm.data_types.dv_quantified.adoc",
        reason: "shared by every concrete DV_QUANTIFIED descendant.",
    },
    InvariantRealization {
        class: "DV_TEXT",
        name: "Formatting_valid",
        venue: InvariantVenue::Core,
        site: "dv_text_core",
        spec_file: "org.openehr.rm.data_types.dv_text.adoc",
        reason: "shared by DV_TEXT and DV_CODED_TEXT.",
    },
    InvariantRealization {
        class: "DV_TEXT",
        name: "Valid_value",
        venue: InvariantVenue::Core,
        site: "dv_text_core",
        spec_file: "org.openehr.rm.data_types.dv_text.adoc",
        reason: "shared by DV_TEXT and DV_CODED_TEXT.",
    },
    InvariantRealization {
        class: "DV_URI",
        name: "Value_valid",
        venue: InvariantVenue::Core,
        site: "dv_uri_core",
        spec_file: "org.openehr.rm.data_types.dv_uri.adoc",
        reason: "extended by `dv_ehr_uri_impl.rs` for the DV_EHR_URI scheme rule.",
    },
    InvariantRealization {
        class: "ENTRY",
        name: "Is_archetype_root",
        venue: InvariantVenue::Core,
        site: "entry_root_core",
        spec_file: "org.openehr.rm.composition.entry.adoc",
        reason: "shared by every concrete ENTRY subtype.",
    },
    InvariantRealization {
        class: "EVENT_CONTEXT",
        name: "location_valid",
        venue: InvariantVenue::Core,
        site: "event_context_core",
        spec_file: "org.openehr.rm.composition.event_context.adoc",
        reason: "called by `event_context_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "HISTORY",
        name: "Events_valid",
        venue: InvariantVenue::Core,
        site: "history_basic_core",
        spec_file: "org.openehr.rm.data_structures.history.adoc",
        reason: "called by `history_impl.rs` and the fast path.",
    },
    InvariantRealization {
        class: "LOCATABLE",
        name: "Archetype_node_id_valid",
        venue: InvariantVenue::Core,
        site: "archetype_node_id_core",
        spec_file: "org.openehr.rm.common.locatable.adoc",
        reason: "inherited by every concrete LOCATABLE descendant; the typed dispatcher closes out the classes with no typed impl from the generated concrete-descendant closure.",
    },
    InvariantRealization {
        class: "PARTY_IDENTIFIED",
        name: "Basic_validity",
        venue: InvariantVenue::Core,
        site: "party_identified_core",
        spec_file: "org.openehr.rm.common.party_identified.adoc",
        reason: "shared by PARTY_IDENTIFIED and PARTY_RELATED.",
    },
    InvariantRealization {
        class: "PARTY_IDENTIFIED",
        name: "Name_valid",
        venue: InvariantVenue::Core,
        site: "party_identified_core",
        spec_file: "org.openehr.rm.common.party_identified.adoc",
        reason: "shared by PARTY_IDENTIFIED and PARTY_RELATED.",
    },
    InvariantRealization {
        class: "TERM_MAPPING",
        name: "Match_valid",
        venue: InvariantVenue::Core,
        site: "term_mapping_core",
        spec_file: "org.openehr.rm.data_types.term_mapping.adoc",
        reason: "the match code is one of `< = > ?`.",
    },
    InvariantRealization {
        class: "AUDIT_DETAILS",
        name: "System_id_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/common/generic/audit_details_impl.rs",
        spec_file: "org.openehr.rm.common.audit_details.adoc",
        reason: "re-stated on the ATTESTATION subtype impl for its own RM type name.",
    },
    InvariantRealization {
        class: "FEEDER_AUDIT_DETAILS",
        name: "System_id_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/common/archetyped/feeder_audit_details_impl.rs",
        spec_file: "org.openehr.rm.common.feeder_audit_details.adoc",
        reason: "the FEEDER_AUDIT_DETAILS system id is checked on its own type.",
    },
    InvariantRealization {
        class: "DV_MULTIMEDIA",
        name: "Integrity_check_validity",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/encapsulated/dv_multimedia_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_multimedia.adoc",
        reason: "an integrity check requires its algorithm.",
    },
    InvariantRealization {
        class: "DV_MULTIMEDIA",
        name: "Not_empty",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/encapsulated/dv_multimedia_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_multimedia.adoc",
        reason: "inline data or an external URI must be present.",
    },
    InvariantRealization {
        class: "DV_MULTIMEDIA",
        name: "Size_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/encapsulated/dv_multimedia_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_multimedia.adoc",
        reason: "a negative encapsulated size is refused.",
    },
    InvariantRealization {
        class: "INSTRUCTION_DETAILS",
        name: "Activity_path_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/composition/content/entry/instruction_details_impl.rs",
        spec_file: "org.openehr.rm.composition.instruction_details.adoc",
        reason: "the activity id must be non-empty.",
    },
    InvariantRealization {
        class: "ITEM_TAG",
        name: "Inv_value_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/common/tags/item_tag_impl.rs",
        spec_file: "org.openehr.rm.common.item_tag.adoc",
        reason: "a present tag value must be non-empty.",
    },
    InvariantRealization {
        class: "REFERENCE_RANGE",
        name: "Range_is_simple",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/quantity/reference_range_impl.rs",
        spec_file: "org.openehr.rm.data_types.reference_range.adoc",
        reason: "each present interval limit must itself be simple (no nested reference ranges).",
    },
    InvariantRealization {
        class: "COMPOSITION",
        name: "Content_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.composition.composition.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "EVENT_CONTEXT",
        name: "Participations_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.composition.event_context.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "SECTION",
        name: "Items_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.composition.section.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "ENTRY",
        name: "Other_participations_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.composition.entry.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "INSTRUCTION",
        name: "Activities_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.composition.instruction.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "DV_TEXT",
        name: "Mappings_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_types.dv_text.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "DV_ORDERED",
        name: "Other_reference_ranges_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_types.dv_ordered.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "LOCATABLE",
        name: "Links_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.locatable.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "LOCATABLE",
        name: "Archetyped_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/validate.rs",
        spec_file: "org.openehr.rm.common.locatable.adoc",
        reason: "the enforceable arm reads the node's own `archetype_node_id` + `archetype_details` off the JSON value (`check_archetyped_valid`).",
    },
    InvariantRealization {
        class: "EHR",
        name: "Ehr_status_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "a cross-object reference resolved by the EHR service against the store, not a property of the value being validated.",
    },
    InvariantRealization {
        class: "EHR",
        name: "Ehr_access_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "a cross-object reference resolved by the EHR service against the store, not a property of the value being validated.",
    },
    InvariantRealization {
        class: "EHR",
        name: "Directory_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "a cross-object reference resolved by the EHR service against the store, not a property of the value being validated.",
    },
    InvariantRealization {
        class: "VERSIONED_OBJECT",
        name: "Uid_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.versioned_object.adoc",
        reason: "the versioning aggregate, owned by the versioning layer's commit path.",
    },
    InvariantRealization {
        class: "VERSION",
        name: "Preceding_version_uid_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.version.adoc",
        reason: "the version chain, owned by the versioning layer's commit path.",
    },
    InvariantRealization {
        class: "REVISION_HISTORY_ITEM",
        name: "Audit_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.revision_history_item.adoc",
        reason: "`not audits.is_empty`: satisfied by construction — `audits` emits as `NonEmptyVec<AuditDetails>`, so BOTH ingestion paths refuse an empty list at parse: the REST read side never accepts one as a request body (every builder pushes the commit audit first), and the OPT 1.4 template upload — the one real client-supplied REVISION_HISTORY carrier (Template.xsd) — constructs items through `NonEmptyVec::new` in the generated XML reader, pinned by the opt14 refusal twin (#1648).",
    },
    InvariantRealization {
        class: "DV_ORDERED",
        name: "Is_simple_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_types.dv_ordered.adoc",
        reason: "vacuous on stored data: `is_simple ()` is defined as `True if this quantity has no reference ranges` (§Functions), i.e. exactly the antecedent.",
    },
    InvariantRealization {
        class: "HISTORY",
        name: "Periodic_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_structures.history.adoc",
        reason: "vacuous on stored data: `is_periodic ()` is a derived function (§Functions) with no wire representation; the stored node carries only `period`.",
    },
    InvariantRealization {
        class: "ACTOR",
        name: "Roles_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.actor.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "ADDRESS",
        name: "Type_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.address.adoc",
        reason: "`type = name`: the invariant DEFINES the derived `type` function as the class's own `name` (§Functions), so nothing about an instance's data can violate it.",
    },
    InvariantRealization {
        class: "CONTACT",
        name: "Purpose_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.contact.adoc",
        reason: "`purpose = name`: the invariant DEFINES the derived `purpose` function as the class's own `name` (§Functions), so nothing about an instance's data can violate it.",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Contacts_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Identities_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "`not identities.is_empty`: holds BY CONSTRUCTION — `identities` is a mandatory `1..*` list emitted as `NonEmptyVec<PartyIdentity>`, so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse (pinned by `demographic/validate.rs` `identities_valid_is_enforced`).",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Is_archetype_root",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/ehr/validation.rs",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "unconditional (`is_archetype_root`), so every party body is a root LOCATABLE; the demographic write boundary (`service/demographic/validate.rs`) routes every party commit through the shared root-LOCATABLE validator at the named site, whose refusal names the invariant.",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Type_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "`type = name`: the invariant DEFINES the derived `type` function as the class's own `name` (§Functions), so nothing about an instance's data can violate it.",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Uid_mandatory",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "`uid /= Void`: satisfied by construction, not by a check — a demographic party's `uid` is the version container's, which the server injects at the storage/read boundary (`app/ferroehr/src/service/demographic/support.rs`), so an inbound body legitimately carries none (`app/ferroehr/src/service/demographic/validate.rs`).",
    },
    InvariantRealization {
        class: "PARTY_IDENTITY",
        name: "Purpose_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party_identity.adoc",
        reason: "`purpose = name`: the invariant DEFINES the derived `purpose` function as the class's own `name` (§Functions), so nothing about an instance's data can violate it.",
    },
    InvariantRealization {
        class: "PARTY_RELATIONSHIP",
        name: "Type_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party_relationship.adoc",
        reason: "`type = name`: the invariant DEFINES the derived `type` function as the class's own `name` (§Functions), so nothing about an instance's data can violate it.",
    },
    InvariantRealization {
        class: "ROLE",
        name: "Capabilities_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.role.adoc",
        reason: "the `x /= Void implies not x.empty` family (Eiffel spelling): holds BY CONSTRUCTION — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "EXTRACT",
        name: "Sequence_nr_valid",
        venue: InvariantVenue::Core,
        site: "extract_core",
        spec_file: "org.openehr.rm.ehr_extract.extract.adoc",
        reason: "`sequence_nr >= 1`, called by `extract_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "EXTRACT_CONTENT_ITEM",
        name: "Item_validity",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/message/import.rs",
        spec_file: "org.openehr.rm.ehr_extract.extract_content_item.adoc",
        reason: "`is_masked xor item /= Void`: realized on the EHR-Extract import path, which rejects a masked wrapper carrying an item and an unmasked one carrying none.",
    },
    InvariantRealization {
        class: "EXTRACT_UPDATE_SPEC",
        name: "Overall_validity",
        venue: InvariantVenue::Core,
        site: "extract_update_spec_core",
        spec_file: "org.openehr.rm.ehr_extract.extract_update_spec.adoc",
        reason: "`repeat_period /= Void or trigger_events /= Void`, called by `extract_update_spec_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "EXTRACT_UPDATE_SPEC",
        name: "Send_changes_only_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr_extract.extract_update_spec.adoc",
        reason: "invokes `send_changes_only`, an attribute the class does NOT declare (its table carries `update_method: CODE_PHRASE`; only the intro prose speaks of the flag) — an invariant over a nonexistent attribute is unevaluable; upstream defect, reported.",
    },
    InvariantRealization {
        class: "EXTRACT_UPDATE_SPEC",
        name: "Trigger_events_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr_extract.extract_update_spec.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "EXTRACT_VERSION_SPEC",
        name: "Includes_revision_history_valid",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/message/export.rs",
        spec_file: "org.openehr.rm.ehr_extract.extract_version_spec.adoc",
        reason: "`not include_data implies include_revision_history`: realized where an extract request's version spec is read, before any export runs.",
    },
    InvariantRealization {
        class: "AUTHORED_RESOURCE",
        name: "Current_revision_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.authored_resource.adoc",
        reason: "constrains the DERIVED `current_revision()` function (§Functions: the most recent revision when controlled, else `(uncontrolled)`) — no stored attribute exists for the rule to refuse; the function's own definition satisfies it by construction (the ORIGINAL_VERSION.Is_merged_validity precedent).",
    },
    InvariantRealization {
        class: "AUTHORED_RESOURCE",
        name: "Revision_history_valid",
        venue: InvariantVenue::Core,
        site: "authored_resource_core",
        spec_file: "org.openehr.rm.common.authored_resource.adoc",
        reason: "`is_controlled xor revision_history = Void`, evaluated only when `is_controlled` (0..1) is present — an xor against a Void operand is not evaluable, so an absent flag asserts nothing; called by `authored_resource_impl.rs`.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION",
        name: "Details_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_core",
        spec_file: "org.openehr.rm.common.resource_description.adoc",
        reason: "the three own non-empty rules, called by `resource_description_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION",
        name: "Lifecycle_state_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_core",
        spec_file: "org.openehr.rm.common.resource_description.adoc",
        reason: "the three own non-empty rules, called by `resource_description_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION",
        name: "Original_author_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_core",
        spec_file: "org.openehr.rm.common.resource_description.adoc",
        reason: "the three own non-empty rules, called by `resource_description_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION_ITEM",
        name: "Purpose_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_item_core",
        spec_file: "org.openehr.rm.common.resource_description_item.adoc",
        reason: "`not purpose.is_empty` plus the present-implies-non-empty string rules, called by `resource_description_item_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION_ITEM",
        name: "Use_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_item_core",
        spec_file: "org.openehr.rm.common.resource_description_item.adoc",
        reason: "`not purpose.is_empty` plus the present-implies-non-empty string rules, called by `resource_description_item_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION_ITEM",
        name: "copyright_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_item_core",
        spec_file: "org.openehr.rm.common.resource_description_item.adoc",
        reason: "`not purpose.is_empty` plus the present-implies-non-empty string rules, called by `resource_description_item_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION_ITEM",
        name: "misuse_valid",
        venue: InvariantVenue::Core,
        site: "resource_description_item_core",
        spec_file: "org.openehr.rm.common.resource_description_item.adoc",
        reason: "`not purpose.is_empty` plus the present-implies-non-empty string rules, called by `resource_description_item_impl.rs` and the typed dispatch.",
    },
    InvariantRealization {
        class: "ORIGINAL_VERSION",
        name: "Attestations_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.original_version.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "ORIGINAL_VERSION",
        name: "Is_merged_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.original_version.adoc",
        reason: "`other_input_version_ids = Void xor is_merged`: `is_merged` is the DERIVED function `True if this Version was created from more than just the preceding (checked out) version` (§Functions), i.e. exactly the emptiness of `other_input_version_uids` — the invariant defines it rather than constraining stored data.",
    },
    InvariantRealization {
        class: "ORIGINAL_VERSION",
        name: "Other_input_version_uids_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.original_version.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "ATTESTATION",
        name: "Items_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.attestation.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    InvariantRealization {
        class: "AUTHORED_RESOURCE",
        name: "Languages_available_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.authored_resource.adoc",
        reason: "`languages_available.has (original_language)`: constrains the DERIVED `languages_available()` function (§Functions), which builds its result from `original_language` — satisfied by the function's own definition (`authored_resource_impl.rs`; the `Current_revision_valid` precedent).",
    },
    InvariantRealization {
        class: "AUTHORED_RESOURCE",
        name: "Translations_valid",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/validation/opt/resource.rs",
        spec_file: "org.openehr.rm.common.authored_resource.adoc",
        reason: "a cross-member map rule over `translations`, realized where a whole authored resource is ingested: the OPT 1.4 template upload's resource-meta pass (the named site) and the ADL 1.4 source catalogue (`openehr-adl` `validate/resource_meta.rs`) — a present translations list is non-empty and never re-states the original language.",
    },
    InvariantRealization {
        class: "AUTHORED_RESOURCE",
        name: "Description_valid",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/validation/opt/resource.rs",
        spec_file: "org.openehr.rm.common.authored_resource.adoc",
        reason: "realized at the same whole-resource ingest seams as `Translations_valid` (the named site plus `openehr-adl` `validate/resource_meta.rs`), as the `RESOURCE_DESCRIPTION.Language_valid` membership — the literal (details ⊆ translations keys) would refuse the original language's own description item, so membership is checked against the original plus the translations.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION",
        name: "Language_valid",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/validation/opt/resource.rs",
        spec_file: "org.openehr.rm.common.resource_description.adoc",
        reason: "`details.for_all (d | parent_resource.languages_available.has (d.language.code_string))`: realized at the whole-resource ingest seams (the named site plus `openehr-adl` `validate/resource_meta.rs`), where the owner is in hand — each description detail's language must be the owner's original language or a listed translation.",
    },
    InvariantRealization {
        class: "RESOURCE_DESCRIPTION",
        name: "Parent_resource_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.resource_description.adoc",
        reason: "`parent_resource /= Void implies parent_resource.description = self`: reads the OWNING `parent_resource` back-reference, which the generated model deliberately breaks (`BACK_REFERENCES` — a back-reference is not forward-owned data), so nothing stored exists for the rule to constrain; where the pair is in hand the identity holds by construction of ownership.",
    },
    InvariantRealization {
        class: "DV_PARAGRAPH",
        name: "Items_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_types.dv_paragraph.adoc",
        reason: "`not items.is_empty`: `items` is a mandatory `1..*` list, emitted as `NonEmptyVec<DvText>`, so a present-but-empty value is unrepresentable and the strict reader refuses an empty wire list — the invariant holds by construction.",
    },
    InvariantRealization {
        class: "DV_PARSABLE",
        name: "Size_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_types.dv_parsable.adoc",
        reason: "`size >= 0`: `size` is the DERIVED function `size in bytes of value` (§Functions), realized as the Rust byte length (`dv_parsable_impl.rs::size`, a `usize`), so it cannot be negative — unlike DV_MULTIMEDIA, where `size` is a STORED attribute a wire value can contradict, nothing stored exists for this rule to constrain.",
    },
    InvariantRealization {
        class: "EHR_ACCESS",
        name: "Is_archetype_root",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/ehr/validation.rs",
        spec_file: "org.openehr.rm.ehr.ehr_access.adoc",
        reason: "unconditional (`is_archetype_root`), so every EHR_ACCESS is a root LOCATABLE; the commit-time validator runs the root-LOCATABLE checks it entails (`Archetyped_valid`, the root `archetype_node_id` rule).",
    },
    InvariantRealization {
        class: "EHR_ACCESS",
        name: "Scheme_valid",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/ehr/validation.rs",
        spec_file: "org.openehr.rm.ehr.ehr_access.adoc",
        reason: "`not scheme.is_empty`: `scheme` names the concrete ACCESS_CONTROL_SETTINGS subtype, which the typed model carries only as the payload's `_type`; the commit-time validator requires a present `settings` to name it (422).",
    },
    InvariantRealization {
        class: "EHR_STATUS",
        name: "Is_archetype_root",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/ehr/validation.rs",
        spec_file: "org.openehr.rm.ehr.ehr_status.adoc",
        reason: "unconditional (`is_archetype_root`), so every EHR_STATUS is a root LOCATABLE; the commit-time validator runs the root-LOCATABLE checks it entails (`Archetyped_valid`, the root `archetype_node_id` rule).",
    },
    InvariantRealization {
        class: "PARTY_IDENTIFIED",
        name: "Identifiers_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.party_identified.adoc",
        reason: "the `x /= Void implies not x.is_empty` family: holds BY CONSTRUCTION since #1730 — an optional container carrying the invariant emits `Option<NonEmptyVec<T>>` (`analyze::nonempty_optional_lists`), so a present-but-empty value is unrepresentable and the strict readers refuse `[]` at parse.",
    },
    // ── the complex-bucket adjudications: every classifier-Complex
    // invariant, venue-registered like the emitted bucket ──────────────────
    InvariantRealization {
        class: "DV_EHR_URI",
        name: "Scheme_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/uri/dv_ehr_uri_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_ehr_uri.adoc",
        reason: "`scheme.is_equal (Ehr_scheme)`: the DV_EHR_URI core extends the shared DV_URI core with the `ehr:` scheme rule.",
    },
    InvariantRealization {
        class: "DV_INTERVAL",
        name: "Limits_consistent",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/quantity/dv_interval_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_interval.adoc",
        reason: "both-bounded limits must compare and order (`lower <= upper`); evaluated on the typed interval, dispatched with a `DvOrdered` element type by `validate/typed_dispatch.rs`.",
    },
    InvariantRealization {
        class: "DV_ORDERED",
        name: "Normal_range_and_status_consistency",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/quantity/dv_ordered_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_ordered.adoc",
        reason: "the cross-member xor of `normal_status = N` against `normal_range.has (self)`, evaluated where both members are typed.",
    },
    InvariantRealization {
        class: "DV_PERIODIC_TIME_SPECIFICATION",
        name: "Value_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_types/time_specification/dv_periodic_time_specification_impl.rs",
        spec_file: "org.openehr.rm.data_types.dv_periodic_time_specification.adoc",
        reason: "`value.formalism` is `HL7:PIVL` or `HL7:EIVL`, checked with the inner syntax where the value is parsed.",
    },
    InvariantRealization {
        class: "DV_PROPORTION",
        name: "Is_integral_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_types.dv_proportion.adoc",
        reason: "`is_integral implies (numerator.floor = numerator …)`: `is_integral` is derived from `type`, and the `dv_proportion_core`'s `Fraction_validity` arm already refuses a non-integral numerator/denominator for exactly the integral kinds — a separate evaluation would double-report the same defect.",
    },
    InvariantRealization {
        class: "EHR",
        name: "Compositions_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "a type facet of the store-maintained reference list: the container rows are typed by `kind` in storage, so a member of the wrong container type is unrepresentable, and the ITS-REST EHR representation serves no such list (the `EHR.Directory_valid` cross-object precedent).",
    },
    InvariantRealization {
        class: "EHR",
        name: "Contributions_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "a type facet of the store-maintained reference list: contribution rows are their own storage relation, so a non-CONTRIBUTION member is unrepresentable, and the ITS-REST EHR representation serves no such list.",
    },
    InvariantRealization {
        class: "EHR",
        name: "Folders_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "a type facet of the store-maintained reference list: folder containers are typed by `kind` in storage, and the ITS-REST EHR representation serves no `folders` list.",
    },
    InvariantRealization {
        class: "EHR",
        name: "Directory_in_folders",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.ehr.ehr.adoc",
        reason: "`folders.item(1) = directory`: the store keeps ONE directory container per EHR and materializes no `folders` list, so the antecedent (`folders /= Void`) never holds on served data (the `EHR.Directory_valid` cross-object precedent).",
    },
    InvariantRealization {
        class: "ELEMENT",
        name: "Inv_is_null_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_structures.element.adoc",
        reason: "`is_null() = (value = Void)`: `is_null()` is the DERIVED function defined as exactly that emptiness, so nothing stored can contradict it (the `ORIGINAL_VERSION.Is_merged_validity` precedent).",
    },
    InvariantRealization {
        class: "ELEMENT",
        name: "Inv_null_flavour_indicated",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_structures/representation/element_impl.rs",
        spec_file: "org.openehr.rm.data_structures.element.adoc",
        reason: "`is_null() xor null_flavour = Void`: a value-less ELEMENT carries a null_flavour and a valued one does not.",
    },
    InvariantRealization {
        class: "ELEMENT",
        name: "Inv_null_reason_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_structures/representation/element_impl.rs",
        spec_file: "org.openehr.rm.data_structures.element.adoc",
        reason: "`null_reason /= Void implies is_null()`: a null_reason on a valued ELEMENT is refused.",
    },
    InvariantRealization {
        class: "ENTRY",
        name: "Subject_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.composition.entry.adoc",
        reason: "`subject_is_self implies subject.generating_type = PARTY_SELF`: `subject_is_self` is the DERIVED function defined by exactly that variant test (§Functions), so the rule restates its own definition.",
    },
    InvariantRealization {
        class: "EVENT",
        name: "Offset_validity1",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_structures.event.adoc",
        reason: "`offset = time.diff (parent.origin)`: `offset` is the DERIVED function defined as that difference, over the `parent` back-reference the generated model deliberately breaks — nothing stored exists for the rule to constrain.",
    },
    InvariantRealization {
        class: "HISTORY",
        name: "Period_consistency",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/data_structures/history/history_impl.rs",
        spec_file: "org.openehr.rm.data_structures.history.adoc",
        reason: "every periodic history's event offset is a whole multiple of `period`, evaluated on the typed HISTORY where the event offsets are computable.",
    },
    InvariantRealization {
        class: "INTERVAL_EVENT",
        name: "Interval_start_time_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_structures.interval_event.adoc",
        reason: "`interval_start_time = time - width`: `interval_start_time` is the DERIVED function defined as that subtraction (§Functions) — no stored attribute exists for the rule to refuse.",
    },
    InvariantRealization {
        class: "ITEM_LIST",
        name: "Valid_structure",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_structures.item_list.adoc",
        reason: "`items.forall (i | i.type = ELEMENT)`: holds BY CONSTRUCTION — the BMM types `items` `List<ELEMENT>`, emitted as `Vec<Element>`, so a non-ELEMENT member is unrepresentable and the strict readers refuse it at parse.",
    },
    InvariantRealization {
        class: "ITEM_TABLE",
        name: "Valid_structure",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.data_structures.item_table.adoc",
        reason: "rows of ELEMENT-only CLUSTERs: the BMM types `rows` `List<CLUSTER>` emitted as typed CLUSTERs whose ELEMENT-only membership the table's own row semantics carry; a non-CLUSTER row is unrepresentable at parse.",
    },
    InvariantRealization {
        class: "ITEM_TAG",
        name: "Inv_key_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-rm/src/v1_2/common/tags/item_tag_impl.rs",
        spec_file: "org.openehr.rm.common.item_tag.adoc",
        reason: "`not key.is_empty and key.is_justified`: the tag key rules, realized once and read by both service seams.",
    },
    InvariantRealization {
        class: "Interval",
        name: "Limits_comparable",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.base.foundation_types.interval.adoc",
        reason: "`lower.strictly_comparable_to (upper)`: holds BY CONSTRUCTION — the generic emits `Interval<T>` with one `T` for both limits and the ordering bound, so incomparable limits are unrepresentable.",
    },
    InvariantRealization {
        class: "Interval",
        name: "Limits_consistent",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/interval/point_interval_impl.rs",
        spec_file: "org.openehr.base.foundation_types.interval.adoc",
        reason: "both-bounded limits must order (`lower <= upper`), evaluated on the typed interval.",
    },
    InvariantRealization {
        class: "Iso8601_date",
        name: "Year_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.base.foundation_types.iso8601_date.adoc",
        reason: "`valid_year (year)`: holds at PARSE — the ISO-8601 lexical form admits only `yyyy` digit years and the reader validates through `time_definitions::valid_year` before a typed value exists.",
    },
    InvariantRealization {
        class: "Iso8601_date",
        name: "Month_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_date_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date.adoc",
        reason: "`not month_unknown implies valid_month (month)`, reported on the owning type by the shared date-component check.",
    },
    InvariantRealization {
        class: "Iso8601_date",
        name: "Day_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_date_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date.adoc",
        reason: "`not day_unknown implies valid_day (year, month, day)`, reported on the owning type by the shared date-component check.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Year_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "`valid_year (year)`: holds at PARSE — the ISO-8601 lexical form admits only `yyyy` digit years and the reader validates through `time_definitions::valid_year` before a typed value exists.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Month_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_date_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "`valid_month (month)`, reported on the owning type by the shared date-component check the date-time reader calls.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Day_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_date_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "`valid_day (year, month, day)`, reported on the owning type by the shared date-component check the date-time reader calls.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Hour_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "`valid_hour (hour, minute, second)`, reported on the owning type by the shared time-component check the date-time reader calls.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Minute_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "`not minute_unknown implies valid_minute (minute)`, reported on the owning type by the shared time-component check.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Second_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "`not second_unknown implies valid_second (second)`, reported on the owning type by the shared time-component check.",
    },
    InvariantRealization {
        class: "Iso8601_date_time",
        name: "Fractional_second_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_date_time.adoc",
        reason: "a fractional second requires a known second and a valid fraction, reported on the owning type by the shared time-component check.",
    },
    InvariantRealization {
        class: "Iso8601_time",
        name: "Hour_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_time.adoc",
        reason: "`valid_hour (hour, minute, second)`, evaluated where the time value is parsed.",
    },
    InvariantRealization {
        class: "Iso8601_time",
        name: "Minute_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_time.adoc",
        reason: "`not minute_unknown implies valid_minute (minute)`, evaluated where the time value is parsed.",
    },
    InvariantRealization {
        class: "Iso8601_time",
        name: "Second_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_time.adoc",
        reason: "`not second_unknown implies valid_second (second)`, evaluated where the time value is parsed.",
    },
    InvariantRealization {
        class: "Iso8601_time",
        name: "Fractional_second_valid",
        venue: InvariantVenue::Impl,
        site: "crates/openehr-base/src/v1_3/foundation_types/time/iso8601_time_impl.rs",
        spec_file: "org.openehr.base.foundation_types.iso8601_time.adoc",
        reason: "a fractional second requires a known second and a valid fraction, evaluated where the time value is parsed.",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Relationships_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "reads the source-party aggregate the SM's independently-versioned relationship containers deliberately do not maintain (the two representations are DISJOINT — RM demographic master02 §Party Relationships vs SM i_party_relationship; the adjudication lives at `app/ferroehr/src/service/demographic/relationship.rs`); an inline `relationships` list in a committed PARTY body stays validated and served verbatim.",
    },
    InvariantRealization {
        class: "PARTY",
        name: "Reverse_relationships_validity",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party.adoc",
        reason: "quantifies over `repository (demographics).all_party_relationships` — a whole-repository aggregate no per-node evaluation can read; the SM's disjoint relationship containers carry the served representation.",
    },
    InvariantRealization {
        class: "PARTY_RELATIONSHIP",
        name: "Source_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party_relationship.adoc",
        reason: "`source.relationships.has (self)` reads the source party's aggregate through the back-reference the generated model breaks; under the SM's disjoint containers the RM compositional linkage is deliberately not maintained (the `PARTY.Relationships_validity` adjudication).",
    },
    InvariantRealization {
        class: "PARTY_RELATIONSHIP",
        name: "Target_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.demographic.party_relationship.adoc",
        reason: "`not target.reverse_relationships.has (self)` reads the target party's aggregate through a broken back-reference; same disjoint-representation adjudication as `Source_valid`.",
    },
    InvariantRealization {
        class: "VERSION",
        name: "Owner_id_valid",
        venue: InvariantVenue::Excluded,
        site: "",
        spec_file: "org.openehr.rm.common.version.adoc",
        reason: "`owner_id.value = uid.object_id.value`: `owner_id` is the DERIVED function realized as exactly `uid.object_id()` (`version_impl.rs`), so the rule restates its own definition (the `ORIGINAL_VERSION.Is_merged_validity` precedent).",
    },
    InvariantRealization {
        class: "VERSIONED_COMPOSITION",
        name: "Archetype_node_id_valid",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/ehr/validation.rs",
        spec_file: "org.openehr.rm.ehr.versioned_composition.adoc",
        reason: "every version's root `archetype_node_id` equals the container's first version's — checked in the commit transaction on every composition update.",
    },
    InvariantRealization {
        class: "VERSIONED_COMPOSITION",
        name: "Persistent_validity",
        venue: InvariantVenue::App,
        site: "app/ferroehr/src/service/ehr/validation.rs",
        spec_file: "org.openehr.rm.ehr.versioned_composition.adoc",
        reason: "every version's persistence (`category` = persistent, the derived `is_persistent`) equals the container's first version's — checked in the commit transaction on every composition update.",
    },
];

/// The register row for `class.name`, if the invariant is registered.
pub(crate) fn invariant_realization(
    class: &str,
    name: &str,
) -> Option<&'static InvariantRealization> {
    INVARIANT_REALIZATIONS
        .iter()
        .find(|r| r.class == class && r.name == name)
}

/// One accounted emittable invariant: a classifier `Emitted` verdict paired
/// with the register row that says where it is realized. A `None` realization
/// is an **unaccounted** emit — an invariant the classifier calls mechanically
/// evaluable that no venue claims — which the generated file reports and the
/// emitter-invariant suite fails on.
#[derive(Debug)]
pub(crate) struct AccountedInvariant {
    /// The owning BMM class name.
    pub class: String,
    /// The BMM invariant name.
    pub name: String,
    /// The register row, or `None` when the invariant is unaccounted.
    pub realization: Option<&'static InvariantRealization>,
}

/// Account every **emittable** invariant among `invariants` — `(class, name,
/// assertion-expression)` triples, typically a BMM schema's own class
/// invariants — against [`INVARIANT_REALIZATIONS`], sorted by `(class, name)`.
///
/// The accounted *set* is derived from the classifier
/// ([`crate::analyze::invariants::classify`]), never from a list: an invariant
/// that changes bucket, appears with a spec bump, or is added to the vendored
/// BMM enters this accounting automatically.
pub(crate) fn account_emitted<'a>(
    invariants: impl Iterator<Item = (&'a str, &'a str, &'a str)>,
) -> Vec<AccountedInvariant> {
    let mut out: Vec<AccountedInvariant> = invariants
        .filter(|(_, _, expr)| classify(expr) == Bucket::Emitted)
        .map(|(class, name, _)| AccountedInvariant {
            class: class.to_owned(),
            name: name.to_owned(),
            realization: invariant_realization(class, name),
        })
        .collect();
    out.sort_by(|a, b| (&a.class, &a.name).cmp(&(&b.class, &b.name)));
    out
}

/// Account every **complex-bucket** invariant among `invariants` — the rules
/// the classifier judges NOT mechanically evaluable — against
/// [`INVARIANT_REALIZATIONS`], sorted by `(class, name)`.
///
/// A complex invariant is still normative: it is realized at a hand-written
/// or application venue, or carries an adjudicated exclusion — a complex rule
/// with no register row is a silent enforcement gap, the same defect class
/// the emitted-bucket accounting closes.
pub(crate) fn account_complex<'a>(
    invariants: impl Iterator<Item = (&'a str, &'a str, &'a str)>,
) -> Vec<AccountedInvariant> {
    let mut out: Vec<AccountedInvariant> = invariants
        .filter(|(_, _, expr)| matches!(classify(expr), Bucket::Complex(_)))
        .map(|(class, name, _)| AccountedInvariant {
            class: class.to_owned(),
            name: name.to_owned(),
            realization: invariant_realization(class, name),
        })
        .collect();
    out.sort_by(|a, b| (&a.class, &a.name).cmp(&(&b.class, &b.name)));
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Reference targets (the class an OBJECT_REF attribute points to)
// ─────────────────────────────────────────────────────────────────────────────

/// The class one reference-typed RM attribute points to, as the RM text states
/// it.
///
/// The BMM types every such attribute by the reference class alone
/// (`OBJECT_REF`, `PARTY_REF`, `LOCATABLE_REF`); the target is stated only in the
/// attribute's definition in the class text, which this entry quotes. `target`
/// is `None` where the text names no single class ("e.g.", "usually", an
/// external system), so every reference-typed attribute has exactly one entry
/// and `emit-rm-model` refuses one without.
pub(crate) struct RefTarget {
    /// The class declaring the attribute.
    pub class: &'static str,
    /// The attribute.
    pub attr: &'static str,
    /// The class the reference points to, when the text names one.
    pub target: Option<&'static str>,
    /// The RM class text the target is read from.
    pub citation: &'static str,
}

/// Every reference-typed attribute of the RM, with its target.
pub(crate) const REF_TARGETS: &[RefTarget] = &[
    RefTarget {
        class: "EHR",
        attr: "contributions",
        target: Some("CONTRIBUTION"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"List of contributions \
                   causing changes to this EHR.\"",
    },
    RefTarget {
        class: "EHR",
        attr: "ehr_status",
        target: Some("EHR_STATUS"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"Reference to `EHR_STATUS` \
                   object for this EHR.\"",
    },
    RefTarget {
        class: "EHR",
        attr: "ehr_access",
        target: Some("EHR_ACCESS"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"Reference to `EHR_ACCESS` \
                   object for this EHR.\"",
    },
    RefTarget {
        class: "EHR",
        attr: "compositions",
        target: Some("VERSIONED_COMPOSITION"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"Master list of all \
                   Versioned Composition references in this EHR.\"",
    },
    RefTarget {
        class: "EHR",
        attr: "directory",
        target: Some("FOLDER"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"a reference to the first \
                   member of `_folders_`\"; RM ehr master04-ehr_package.adoc §Folders: \
                   \"versioned hierarchies of `FOLDER` objects\"",
    },
    RefTarget {
        class: "EHR",
        attr: "folders",
        target: Some("FOLDER"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"Optional additional Folder \
                   structures for this EHR.\"; RM ehr master04-ehr_package.adoc §Folders: \
                   \"versioned hierarchies of `FOLDER` objects\"",
    },
    RefTarget {
        class: "EHR",
        attr: "tags",
        target: Some("ITEM_TAG"),
        citation: "RM ehr UML/classes/org.openehr.rm.ehr.ehr.adoc: \"Optional list of tags \
                   associated with this EHR.\"; RM ehr master04-ehr_package.adoc §Tags: \"Tags \
                   are associated with an EHR via the `EHR._tags_` attribute\"",
    },
    RefTarget {
        class: "VERSIONED_OBJECT",
        attr: "owner_id",
        target: None,
        citation: "RM common UML/classes/org.openehr.rm.common.versioned_object.adoc: \"e.g. the \
                   id of the containing EHR or other relevant owning entity\" names no single \
                   class",
    },
    RefTarget {
        class: "CONTRIBUTION",
        attr: "versions",
        target: Some("VERSION"),
        citation: "RM common UML/classes/org.openehr.rm.common.contribution.adoc: \"Set of \
                   references to Versions causing changes to this EHR.\"",
    },
    RefTarget {
        class: "VERSION",
        attr: "contribution",
        target: Some("CONTRIBUTION"),
        citation: "RM common UML/classes/org.openehr.rm.common.version.adoc: \"Contribution in \
                   which this version was added.\"",
    },
    RefTarget {
        class: "PARTY_PROXY",
        attr: "external_ref",
        target: None,
        citation: "RM common UML/classes/org.openehr.rm.common.party_proxy.adoc: \"information \
                   for this party, in an external system\" names no class",
    },
    RefTarget {
        class: "FOLDER",
        attr: "items",
        target: None,
        citation: "RM common UML/classes/org.openehr.rm.common.folder.adoc: \"references to \
                   other (usually) versioned objects\" names no single class",
    },
    RefTarget {
        class: "ITEM_TAG",
        attr: "owner_id",
        target: None,
        citation: "RM common UML/classes/org.openehr.rm.common.item_tag.adoc: \"Identifier of \
                   owner object, such as EHR.\" names no single class",
    },
    RefTarget {
        class: "EXTRACT_ACTION_REQUEST",
        attr: "request_id",
        target: Some("EXTRACT_REQUEST"),
        citation: "RM ehr_extract UML/classes/org.openehr.rm.ehr_extract.extract_action_request.adoc: \
                   \"Identifier of previous `EXTRACT_REQUEST`.\"",
    },
    RefTarget {
        class: "EXTRACT_ENTITY_MANIFEST",
        attr: "item_list",
        target: None,
        citation: "RM ehr_extract UML/classes/org.openehr.rm.ehr_extract.extract_entity_manifest.adoc: \
                   \"List of Uids of items to be included in the Extract\" names no single class",
    },
    RefTarget {
        class: "EXTRACT_ERROR",
        attr: "request_id",
        target: None,
        citation: "RM ehr_extract UML/classes/org.openehr.rm.ehr_extract.extract_error.adoc: the \
                   attribute carries no definition",
    },
    RefTarget {
        class: "X_VERSIONED_OBJECT",
        attr: "owner_id",
        target: Some("EHR"),
        citation: "RM ehr_extract UML/classes/org.openehr.rm.ehr_extract.x_versioned_object.adoc: \
                   \"Owner_id from original `VERSIONED_OBJECT`, which identifies source EHR.\"",
    },
    RefTarget {
        class: "ROLE",
        attr: "performer",
        target: Some("VERSIONED_PARTY"),
        citation: "RM demographic UML/classes/org.openehr.rm.demographic.role.adoc: \"Reference \
                   to Version container of Actor playing the role.\"",
    },
    RefTarget {
        class: "ACTOR",
        attr: "roles",
        target: Some("VERSIONED_PARTY"),
        citation: "RM demographic UML/classes/org.openehr.rm.demographic.actor.adoc: \
                   \"Identifiers of the Version container for each Role played by this Party.\"",
    },
    RefTarget {
        class: "PARTY_RELATIONSHIP",
        attr: "source",
        target: Some("VERSIONED_PARTY"),
        citation: "RM demographic master02-demographic_package.adoc: \"`OBJECT_REFs` containing \
                   `HIER_OBJECT_IDs` to denote the Version container of a Party\"",
    },
    RefTarget {
        class: "PARTY_RELATIONSHIP",
        attr: "target",
        target: Some("VERSIONED_PARTY"),
        citation: "RM demographic master02-demographic_package.adoc: \"`OBJECT_REFs` containing \
                   `HIER_OBJECT_IDs` to denote the Version container of a Party\"",
    },
    RefTarget {
        class: "PARTY",
        attr: "reverse_relationships",
        target: Some("PARTY_RELATIONSHIP"),
        citation: "RM demographic UML/classes/org.openehr.rm.demographic.party.adoc: \"References \
                   to relationships in which this Party takes part as target.\"",
    },
    RefTarget {
        class: "CITATION",
        attr: "source_ref",
        target: None,
        citation: "RM 1.1.0 BMM (openehr_rm_1.1.0.bmm.json) CITATION.source_ref: \"Reference to \
                   original object.\" names no class; the vendored RM text defines no CITATION",
    },
    RefTarget {
        class: "CITATION",
        attr: "source_parent_ref",
        target: None,
        citation: "RM 1.1.0 BMM (openehr_rm_1.1.0.bmm.json) CITATION.source_parent_ref: \
                   \"Reference to target's owning object.\" names no class; the vendored RM text \
                   defines no CITATION",
    },
    RefTarget {
        class: "CARE_ENTRY",
        attr: "guideline_id",
        target: None,
        citation: "RM ehr UML/classes/org.openehr.rm.composition.care_entry.adoc: \"Optional \
                   external identifier of guideline\" names no class",
    },
    RefTarget {
        class: "INSTRUCTION_DETAILS",
        attr: "instruction_id",
        target: Some("INSTRUCTION"),
        citation: "RM ehr UML/classes/org.openehr.rm.composition.instruction_details.adoc: \
                   \"Reference to causing Instruction.\"",
    },
    RefTarget {
        class: "ENTRY",
        attr: "workflow_id",
        target: None,
        citation: "RM ehr UML/classes/org.openehr.rm.composition.entry.adoc: \"Identifier of \
                   externally held workflow engine data\" names no class",
    },
];

/// The reference-target entry of `class.attr`, when the attribute is
/// reference-typed.
pub(crate) fn ref_target(class: &str, attr: &str) -> Option<&'static RefTarget> {
    REF_TARGETS
        .iter()
        .find(|r| r.class == class && r.attr == attr)
}

// ─────────────────────────────────────────────────────────────────────────────
// ITS-REST path identifiers (the openEHR identifier class a path parameter carries)
// ─────────────────────────────────────────────────────────────────────────────

/// The openEHR identifier class one ITS-REST path-parameter component carries.
///
/// The OAS states it only in the component's description, which names the RM
/// attribute the value is "taken from"; the class is that attribute's BMM
/// type. Every path-parameter component has exactly one entry, `class: None`
/// where the value is no openEHR identifier, and `emit-rest` emits a
/// `compile_error!` for a path parameter without one.
pub(crate) struct RestPathIdentifier {
    /// The `#/components/parameters/` name.
    pub component: &'static str,
    /// The BASE identifier class (`HIER_OBJECT_ID`, `OBJECT_VERSION_ID`,
    /// `UID_BASED_ID`), or `None`.
    pub class: Option<&'static str>,
    /// The OAS description and the RM attribute type the class is read from.
    pub citation: &'static str,
}

/// Every ITS-REST path-parameter component, with its identifier class.
pub(crate) const REST_PATH_IDENTIFIERS: &[RestPathIdentifier] = &[
    RestPathIdentifier {
        component: "ehr_id",
        class: Some("HIER_OBJECT_ID"),
        citation: "OAS `ehr_id`: \"EHR identifier taken from EHR.ehr_id.value.\"; RM BMM \
                   `EHR.ehr_id: HIER_OBJECT_ID`",
    },
    RestPathIdentifier {
        component: "version_uid",
        class: Some("OBJECT_VERSION_ID"),
        citation: "OAS `version_uid`: \"VERSION identifier taken from VERSION.uid.value.\"; RM \
                   BMM `VERSION.uid(): OBJECT_VERSION_ID`",
    },
    RestPathIdentifier {
        component: "version_uid_COMPOSITION",
        class: Some("OBJECT_VERSION_ID"),
        citation: "OAS `version_uid_COMPOSITION`: \"VERSION identifier taken from \
                   VERSION.uid.value.\"; RM BMM `VERSION.uid(): OBJECT_VERSION_ID`",
    },
    RestPathIdentifier {
        component: "versioned_object_uid_COMPOSITION",
        class: Some("HIER_OBJECT_ID"),
        citation: "OAS `versioned_object_uid_COMPOSITION`: \"VERSIONED_COMPOSITION identifier \
                   taken from VERSIONED_COMPOSITION.uid.value.\"; RM BMM \
                   `VERSIONED_OBJECT.uid: HIER_OBJECT_ID`",
    },
    RestPathIdentifier {
        component: "versioned_object_uid_PARTY",
        class: Some("HIER_OBJECT_ID"),
        citation: "OAS `versioned_object_uid_PARTY`: \"VERSIONED_PARTY identifier taken from \
                   VERSIONED_PARTY.uid.value.\"; RM BMM `VERSIONED_OBJECT.uid: HIER_OBJECT_ID`",
    },
    RestPathIdentifier {
        component: "contribution_uid",
        class: Some("HIER_OBJECT_ID"),
        citation: "OAS `contribution_uid`: \"The CONTRIBUTION uid.\"; RM BMM \
                   `CONTRIBUTION.uid: HIER_OBJECT_ID`",
    },
    RestPathIdentifier {
        component: "uid_based_id",
        class: Some("UID_BASED_ID"),
        citation: "OAS `uid_based_id`: \"it can take a form of an OBJECT_VERSION_ID identifier \
                   taken from VERSION.uid.value (i.e. a `version_uid`), or a form of a \
                   HIER_OBJECT_ID identifier taken from VERSIONED_OBJECT.uid.value\"; BASE BMM \
                   `UID_BASED_ID` is the ancestor of both",
    },
    RestPathIdentifier {
        component: "uid_based_id_as_version_uid",
        class: Some("OBJECT_VERSION_ID"),
        citation: "OAS `uid_based_id_as_version_uid`: \"An identifier in a form of an \
                   OBJECT_VERSION_ID identifier taken from VERSION.uid.value\"",
    },
    RestPathIdentifier {
        component: "uid_based_id_as_versioned_object_uid",
        class: Some("HIER_OBJECT_ID"),
        citation: "OAS `uid_based_id_as_versioned_object_uid`: \"An identifier in a form of a \
                   HIER_OBJECT_ID identifier taken from VERSIONED_OBJECT.uid.value\"",
    },
    RestPathIdentifier {
        component: "key",
        class: None,
        citation: "OAS `key`: \"The ITEM_TAG key.\"; RM BMM `ITEM_TAG.key: String`",
    },
    RestPathIdentifier {
        component: "template_id",
        class: None,
        citation: "OAS `template_id`: \"Template identifier or partial reference.\", a \
                   template name the REST API resolves, not an openEHR identifier object",
    },
    RestPathIdentifier {
        component: "template_id_adl2",
        class: None,
        citation: "OAS `template_id_adl2`: \"Template identifier or partial reference.\", a \
                   template name the REST API resolves, not an openEHR identifier object",
    },
    RestPathIdentifier {
        component: "version",
        class: None,
        citation: "OAS `version`: \"A SEMVER version number.\"",
    },
    RestPathIdentifier {
        component: "qualified_query_name",
        class: None,
        citation: "OAS `qualified_query_name`: \"The (fully qualified) name of the query to be \
                   executed\"",
    },
];

/// The identifier entry of path-parameter component `component`.
pub(crate) fn rest_path_identifier(component: &str) -> Option<&'static RestPathIdentifier> {
    REST_PATH_IDENTIFIERS
        .iter()
        .find(|r| r.component == component)
}

/// One extra `_type` tag a discriminated ITS-REST schema reads as its own
/// instantiable base, where the docs text tells servers to accept a tag the
/// OAS `discriminator.mapping` does not list. The value keeps the tag it was
/// sent with.
pub(crate) struct RestDiscriminatorAlias {
    /// The discriminated OAS component schema.
    pub schema: &'static str,
    /// The extra `_type` tag read as the schema's base.
    pub tag: &'static str,
    pub citation: &'static str,
    pub reason: &'static str,
}

/// The docs-text-wins discriminator aliases for the generated ITS-REST contract.
pub(crate) const REST_DISCRIMINATOR_ALIASES: &[RestDiscriminatorAlias] =
    &[RestDiscriminatorAlias {
        schema: "UpdateAudit",
        tag: "AUDIT_DETAILS",
        citation: "ITS-REST operations/contribution_create.yaml — \"Clients SHOULD send `_type: \
               \"UPDATE_AUDIT\"`; for interoperability servers SHOULD additionally accept `_type: \
               \"AUDIT_DETAILS\"` or an omitted `_type` for this attribute\"",
        reason: "An `UPDATE_AUDIT` is structurally an `AUDIT_DETAILS` minus the server-assigned \
             attributes, so the docs text has servers read either tag.",
    }];

/// The discriminator aliases of `schema`.
pub(crate) fn rest_discriminator_aliases(
    schema: &str,
) -> impl Iterator<Item = &'static RestDiscriminatorAlias> + '_ {
    REST_DISCRIMINATOR_ALIASES
        .iter()
        .filter(move |a| a.schema == schema)
}
