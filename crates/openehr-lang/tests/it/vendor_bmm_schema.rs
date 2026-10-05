// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Public-API battery for the P_BMM schema pipeline
//! (`openehr_lang::v1_1::bmm_persistence`) over every vendored `.bmm` schema under
//! `tests/vendor/**`: ODIN text → `P_BMM_SCHEMA` → inclusion resolution →
//! `BMM_MODEL`.
//!
//! `vendor_bmm_odin.rs` already pins that all 38 `tests/vendor/bmm/**` files
//! PARSE as ODIN; this module pins what they mean as P_BMM schemas, and adds the
//! five `.bmm` schemas under `tests/vendor/odin/odin/`.
//!
//! **Every file has an adjudicated expected outcome** — either a materialised
//! `BMM_MODEL` (with its class count) or a typed refusal at a named stage with
//! the spec ground for the refusal. There are no silent skips. The archie
//! fixtures under `bmm/org/openehr/bmm/v2/persistence/validation/` are
//! deliberately defective schemas, and this table records which stage of the
//! openEHR-specified pipeline each defect surfaces at.
//!
//! A defect that breaks no `BMM_*` construction surfaces ABOVE the pipeline
//! instead, in the collecting model-validity pass
//! (`openehr_lang::v1_1::bmm_persistence::validate`); its second table
//! ([`finding_cases`]) is adjudicated the same way, and a schema absent from it
//! must validate clean.
//!
//! A third table ([`pinned_cases`]) covers the openEHR component schemas
//! `openehr-codegen` vendors — all 18 released generations, each with its class
//! count + findings or its adjudicated refusal.
//!
//! Spec oracle: `docs/specs/openehr/LANG/docs/bmm_persistence/`
//! (`master02-overview.adoc` §Conceptual Approach for the three stages,
//! `master04-syntax.adoc` for the ODIN form) plus the class docs under
//! `docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm*.adoc`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![allow(
    clippy::doc_markdown,
    reason = "the module docs name the archie fixture directories and openEHR schema files as prose, not code refs"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openehr_lang::v1_1::bmm_persistence::create_model::create_bmm_model;
use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::include_resolution::resolve_includes;
use openehr_lang::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use openehr_lang::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;
use openehr_lang::v1_1::bmm_persistence::validate::validate_schema;

/// The pipeline stage an outcome is observed at
/// (`master02-overview.adoc` §Conceptual Approach).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// `read_schema` — the ODIN → `P_BMM_SCHEMA` walk.
    Read,
    /// `resolve_includes` — schema inclusion resolution.
    Resolve,
    /// `create_bmm_model` — the `P_BMM` → `BMM` transform.
    Model,
}

/// The typed error discriminant an adjudicated refusal must carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// [`PBmmReadError::UnknownAttribute`].
    UnknownAttribute,
    /// [`PBmmReadError::UnexpectedTypeMarker`].
    UnexpectedTypeMarker,
    /// [`PBmmReadError::KeyNameMismatch`].
    KeyNameMismatch,
    /// [`PBmmReadError::QualifiedNestedPackage`].
    QualifiedNestedPackage,
    /// [`PBmmReadError::MissingInclude`].
    MissingInclude,
    /// [`PBmmReadError::UnknownAncestor`].
    UnknownAncestor,
    /// [`PBmmReadError::UnknownType`].
    UnknownType,
    /// [`PBmmReadError::ClassNotInAnyPackage`].
    ClassNotInAnyPackage,
    /// [`PBmmReadError::ClassNotDefined`].
    ClassNotDefined,
    /// [`PBmmReadError::ContainerTargetTypeMissing`].
    ContainerTargetTypeMissing,
    /// [`PBmmReadError::TypeDefinitionMissing`].
    TypeDefinitionMissing,
    /// [`PBmmReadError::UndeclaredGenericParameter`].
    UndeclaredGenericParameter,
}

/// The discriminant of `error`.
fn kind_of(error: &PBmmReadError) -> Kind {
    match error {
        PBmmReadError::UnknownAttribute { .. } => Kind::UnknownAttribute,
        PBmmReadError::UnexpectedTypeMarker { .. } => Kind::UnexpectedTypeMarker,
        PBmmReadError::KeyNameMismatch { .. } => Kind::KeyNameMismatch,
        PBmmReadError::QualifiedNestedPackage { .. } => Kind::QualifiedNestedPackage,
        PBmmReadError::MissingInclude { .. } => Kind::MissingInclude,
        PBmmReadError::UnknownAncestor { .. } => Kind::UnknownAncestor,
        PBmmReadError::UnknownType { .. } => Kind::UnknownType,
        PBmmReadError::ClassNotInAnyPackage { .. } => Kind::ClassNotInAnyPackage,
        PBmmReadError::ClassNotDefined { .. } => Kind::ClassNotDefined,
        PBmmReadError::ContainerTargetTypeMissing { .. } => Kind::ContainerTargetTypeMissing,
        PBmmReadError::TypeDefinitionMissing { .. } => Kind::TypeDefinitionMissing,
        PBmmReadError::UndeclaredGenericParameter { .. } => Kind::UndeclaredGenericParameter,
        other => panic!("unexpected error kind in the vendored corpus: {other:?}"),
    }
}

/// What the pipeline must do with one vendored schema.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    /// Reads, resolves and materialises; the model's class count.
    Model(usize),
    /// An adjudicated refusal: the stage, the typed discriminant, and a
    /// substring of the message that pins WHICH element is at fault.
    Refused(Stage, Kind, &'static str),
}

/// One row of the corpus expectation table.
struct Case {
    /// Path relative to `tests/vendor`.
    path: &'static str,
    /// The adjudicated outcome.
    outcome: Outcome,
    /// The spec ground for a refusal, or the reason a model is expected.
    /// Documentation for the reader; not asserted.
    #[expect(dead_code, reason = "the adjudication rationale documents the row")]
    adjudication: &'static str,
}

/// The complete outcome table for every vendored `.bmm` schema — 43 files.
///
/// Class counts are the materialised `BMM_MODEL.class_definitions` size after
/// inclusion resolution against [`include_map`].
#[expect(
    clippy::too_many_lines,
    reason = "one row per vendored fixture; splitting the table hides the corpus"
)]
fn cases() -> Vec<Case> {
    vec![
        // ── the published openEHR RM 1.0.2 inclusion chain ──────────────────
        Case {
            path: "bmm/openehr/openehr_primitive_types_102.bmm",
            outcome: Outcome::Model(22),
            adjudication: "self-contained primitive_types schema; master04 §Classes for Primitive Types",
        },
        Case {
            path: "bmm/openehr/openehr_basic_types_102.bmm",
            outcome: Outcome::Model(71),
            adjudication: "includes openehr_primitive_types_1.0.2",
        },
        Case {
            path: "bmm/openehr/openehr_structures_102.bmm",
            outcome: Outcome::Model(105),
            adjudication: "transitively includes basic_types → primitive_types",
        },
        Case {
            path: "bmm/openehr/openehr_ehr_102.bmm",
            outcome: Outcome::Model(124),
            adjudication: "transitively includes structures → basic_types → primitive_types",
        },
        Case {
            path: "bmm/openehr/openehr_demographic_102.bmm",
            outcome: Outcome::Model(117),
            adjudication: "transitively includes structures → basic_types → primitive_types",
        },
        Case {
            path: "bmm/openehr/openehr_rm_102.bmm",
            outcome: Outcome::Model(136),
            adjudication: "the whole RM 1.0.2: includes ehr + demographic, four levels deep",
        },
        // ── further published openEHR schemas ───────────────────────────────
        Case {
            path: "bmm/openehr/openehr_base_110.bmm",
            outcome: Outcome::Model(56),
            adjudication: "self-contained BASE 1.1.0 schema",
        },
        Case {
            path: "bmm/openehr/openehr_base_for_aom.bmm",
            outcome: Outcome::Model(52),
            adjudication: "a second schema rendering the id openehr_base_1.1.0; excluded from the include map (first path wins) but self-contained",
        },
        Case {
            path: "bmm/openehr/openEHR_aom_206.bmm",
            outcome: Outcome::Model(117),
            adjudication: "includes openehr_base_1.1.0; references BOOLEAN where BASE defines Boolean — master04 §Non-primitive Classes: 'any capitalisation can be used'",
        },
        Case {
            path: "bmm/openehr/openehr_adltest_100.bmm",
            outcome: Outcome::Model(94),
            adjudication: "the master04 §Header Items / §Inheritance example schema: model_name tolerance, generic inheritance via ancestor_defs, mixed-case type refs (Iso8601_date vs ISO8601_DATE)",
        },
        // ── the CIMI reference models ───────────────────────────────────────
        Case {
            path: "bmm/cimi/CIMI_RM_CORE.v.0.0.2.bmm",
            outcome: Outcome::Model(41),
            adjudication: "self-contained CIMI core",
        },
        Case {
            path: "bmm/cimi/CIMI_RM_FOUNDATION.v.0.0.2.bmm",
            outcome: Outcome::Model(55),
            adjudication: "includes cimi_rm_core_0.0.2",
        },
        Case {
            path: "bmm/cimi/CIMI_RM_CLINICAL.v.0.0.2.bmm",
            outcome: Outcome::Model(198),
            adjudication: "includes cimi_rm_core_0.0.2 + cimi_rm_foundation_0.0.2",
        },
        Case {
            path: "odin/odin/CIMI_RM_CORE.v.0.0.1.bmm",
            outcome: Outcome::Model(37),
            adjudication: "the 0.0.1 generation of the same CIMI core",
        },
        Case {
            path: "odin/odin/CIMI_RM_FOUNDATION.v.0.0.1.bmm",
            outcome: Outcome::Model(50),
            adjudication: "includes cimi_rm_core_0.0.1",
        },
        Case {
            path: "odin/odin/CIMI_RM_CLINICAL.v.0.0.1.bmm",
            outcome: Outcome::Model(144),
            adjudication: "includes cimi_rm_core_0.0.1 + cimi_rm_foundation_0.0.1",
        },
        Case {
            path: "bmm/CIMI-RM-3.0.5.bmm",
            outcome: Outcome::Refused(Stage::Read, Kind::KeyNameMismatch, "bmmType"),
            adjudication: "PARTICIPATION declares [\"type\"] with name = <\"bmmType\">, violating master04 §Package Definition's third NOTE ('make sure that the ODIN keys are the same as the name attributes in each block')",
        },
        Case {
            path: "bmm/cimi/CIMI-RM-3.0.5.bmm",
            outcome: Outcome::Refused(Stage::Read, Kind::KeyNameMismatch, "bmmType"),
            adjudication: "byte-identical copy of the file above; same key/name defect",
        },
        Case {
            path: "odin/odin/CIMI-RM-3.0.5.bmm",
            outcome: Outcome::Refused(Stage::Read, Kind::UnknownAttribute, "bmmType"),
            adjudication: "ARCHETYPED.archetype_id writes `bmmType = <\"String\">`; P_BMM_SINGLE_PROPERTY declares `type`, not `bmmType` (…p_bmm_single_property.adoc §Attributes)",
        },
        Case {
            path: "odin/odin/CIMI-RM-3.0.5_tweaked.bmm",
            outcome: Outcome::Refused(Stage::Read, Kind::UnknownAttribute, "bmmType"),
            adjudication: "hand-tweaked variant of the file above; same undeclared attribute",
        },
        // ── the archie P_BMM validation fixtures ────────────────────────────
        Case {
            path: "bmm/testbmm/TestBmm1.bmm",
            outcome: Outcome::Refused(Stage::Resolve, Kind::MissingInclude, "my_include.2.1.12"),
            adjudication: "declares two includes that exist in no repository; P_BMM_SCHEMA.merge's precondition includes_to_process.has(...) cannot be met",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/valid.bmm",
            outcome: Outcome::Model(3),
            adjudication: "the archie baseline schema its BasicSchemaValidationsTest mutates; semantically valid",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/duplicate_class.bmm",
            outcome: Outcome::Model(3),
            adjudication: "lists ParentType1 twice in one package; a duplicate entry in BMM_PACKAGE.classes is not a construction failure, so the model materialises and the containment defect is a collected finding instead (see finding_cases)",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/illegal_sibling_packages.bmm",
            outcome: Outcome::Model(3),
            adjudication: "sibling packages ParentPackage / ParentPackages. There is no prefix prohibition to violate: master05-core-model.adoc §Packages says package paths 'are only used in BMM to specify package structures in the serialised form in an efficient way' and 'are not used as namespaces as in UML' — the rule the section states is that all CLASS names be unique, which these two packages do not breach. The model materialises and validates clean",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/overridden_property_non_conformance.bmm",
            outcome: Outcome::Model(4),
            adjudication: "ChildType1 redefines property_1 to a non-conformant type; conformance is a validation question above the transform, so the model materialises and the defect is a collected finding instead (see finding_cases)",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/include_not_found.bmm",
            outcome: Outcome::Refused(Stage::Resolve, Kind::MissingInclude, "my_include.2.1.12"),
            adjudication: "an includes entry naming a schema absent from the repository",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/ancestor_def_doesnt_exist.bmm",
            outcome: Outcome::Refused(Stage::Read, Kind::UnexpectedTypeMarker, "P_BMM_SIMPLE_TYPE"),
            adjudication: "ancestor_defs holds a (P_BMM_SIMPLE_TYPE) entry; P_BMM_CLASS.ancestor_defs is List<P_BMM_GENERIC_TYPE> (class doc §Attributes) and master04 §Inheritance uses it only for generic ancestors, and a simple type states no root_type",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/package_illegal_qualified_name.bmm",
            outcome: Outcome::Refused(
                Stage::Read,
                Kind::QualifiedNestedPackage,
                "invalid.ChildPackage",
            ),
            adjudication: "master04 §Package Definition, first NOTE: 'only top-level package ids can be paths (i.e. contain the . character)'",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/ancestor_doesnt_exist.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownAncestor, "unknown"),
            adjudication: "BMM_CLASS.ancestors is a map of BMM_CLASS, so an unresolvable parent cannot be materialised",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/ancestor_name_empty.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownAncestor, "ancestor ``"),
            adjudication: "the empty ancestor name resolves to no class; same BMM_CLASS.ancestors reason",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/class_not_in_definition.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::ClassNotDefined, "String"),
            adjudication: "master04 §Package Definition, second NOTE: 'only classes defined in the same schema can be referenced in the package section in that schema'",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/package_class_name_empty.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::ClassNotDefined, "lists class ``"),
            adjudication: "an empty class name in a package's classes list names no definition; same second NOTE",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/class_not_in_packages.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::ClassNotInAnyPackage, "UnknownClass"),
            adjudication: "BMM_CLASS.package is 1..1 ('Package this class belongs to'), so a class no package lists cannot be materialised",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/container_target_type_empty.bmm",
            outcome: Outcome::Refused(
                Stage::Model,
                Kind::ContainerTargetTypeMissing,
                "careProvider",
            ),
            adjudication: "the container type_def states container_type but neither type nor type_def; BMM_CONTAINER_TYPE.base_type is 1..1",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/container_target_type_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "Something"),
            adjudication: "the container's target type names no class definition",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/container_type_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "List"),
            adjudication: "master04 §Classes for Primitive Types: 'all container types such as List<T>, Hash<V,K> etc are explicit in a BMM schema' — an undefined container class cannot be materialised",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/generic_container_property_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "X"),
            adjudication: "the container target type X names no class definition",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/generic_parameter_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "Quantity"),
            adjudication: "an actual generic parameter naming no class definition",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/generic_parameter_type_missing.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "unknown"),
            adjudication: "BMM_GENERIC_PARAMETER.conforms_to_type 'must be another valid class name' (…bmm.bmm_generic_parameter.adoc §Attributes)",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/generic_property_type_def_undefined.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::TypeDefinitionMissing, "property_1"),
            adjudication: "a (P_BMM_GENERIC_PROPERTY) with no type_def at all; BMM_PROPERTY.type is 1..1",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/generic_root_type_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "Intervals"),
            adjudication: "the generic type's root_type names no class definition",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/single_property_type_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UnknownType, "Unknown"),
            adjudication: "BMM_SIMPLE_TYPE.base_class IS a BMM_CLASS, so a property type naming no definition cannot be materialised",
        },
        Case {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/single_open_property_type_not_found.bmm",
            outcome: Outcome::Refused(Stage::Model, Kind::UndeclaredGenericParameter, "`X`"),
            adjudication: "'The parameter must be in the type declaration of the owning BMM_CLASS' (…bmm.bmm_open_type.adoc §Description); ParentType1 declares T, not X",
        },
    ]
}

/// The `tests/vendor` root.
fn vendor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vendor")
}

/// The ODIN serialisations of the openEHR component schemas that
/// `openehr-codegen` vendors.
///
/// Referenced by path (never copied) so the boundary test below pins the reader
/// against the schemas the project actually pins (`docs/VERSIONS.md`). Codegen
/// itself consumes the `.bmm.json` serialisation of these same models, so these
/// ODIN files are not part of this crate's committed corpus.
const CODEGEN_VENDOR_ODIN: &str = "../../tools/openehr-codegen/vendor/bmm/components";

/// Reads `path` relative to `tests/vendor`.
fn source(path: &str) -> String {
    let full = vendor_root().join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()))
}

/// The schemas available for inclusion resolution: every vendored file that
/// reads cleanly, keyed by its own `schema_id`, FIRST path in table order
/// winning a collision.
///
/// A collision is real in this corpus: `openehr_base_110.bmm` and
/// `openehr_base_for_aom.bmm` both render `openehr_base_1.1.0`, and the archie
/// validation fixtures all render `my publisher_duplicate_class_3.1`. Keeping
/// the first makes the resolution deterministic; the losing files are still
/// exercised as roots by [`every_vendored_schema_reaches_its_adjudicated_outcome`].
fn include_map(cases: &[Case]) -> BTreeMap<String, PBmmSchema> {
    let mut out: BTreeMap<String, PBmmSchema> = BTreeMap::new();
    for case in cases {
        if let Ok(schema) = read_schema(&source(case.path)) {
            out.entry(schema.schema_id()).or_insert(schema);
        }
    }
    out
}

/// One row of the model-validity expectation table.
struct FindingCase {
    /// Path relative to `tests/vendor`.
    path: &'static str,
    /// Every finding's rendered text, in the order the pass emits them.
    findings: &'static [&'static str],
    /// The spec ground for the findings. Documentation for the reader; not
    /// asserted.
    #[expect(dead_code, reason = "the adjudication rationale documents the row")]
    adjudication: &'static str,
}

/// The adjudicated `validate_schema` findings of every vendored schema that
/// materialises a model.
///
/// A schema that materialises and is ABSENT from this table must validate
/// clean — [`every_materialising_schema_reaches_its_adjudicated_findings`]
/// asserts that, so a new finding anywhere in the corpus fails the build.
fn finding_cases() -> Vec<FindingCase> {
    vec![
        FindingCase {
            path: "bmm/openehr/openEHR_aom_206.bmm",
            findings: &[
                "class `TRANSLATION_DETAILS` is contained within 2 package listing(s) (default, org.openehr.base.base_types.resource); a class must be contained within exactly one package",
                "class `AUTHORED_RESOURCE` is contained within 2 package listing(s) (default, org.openehr.base.base_types.resource); a class must be contained within exactly one package",
                "class `RESOURCE_DESCRIPTION` is contained within 2 package listing(s) (default, org.openehr.base.base_types.resource); a class must be contained within exactly one package",
                "class `RESOURCE_DESCRIPTION_ITEM` is contained within 2 package listing(s) (default, org.openehr.base.base_types.resource); a class must be contained within exactly one package",
            ],
            adjudication: "this schema (its own header says `auto-generated experiment`, `autogenerated as implemented in Archie`) lists four BASE classes in its flat `default` package AND includes openehr_base_1.1.0, which lists the same four in org.openehr.base.base_types.resource; after the merge each is contained twice, against master05-core-model.adoc §Packages ('every class is contained within exactly one package')",
        },
        FindingCase {
            path: "bmm/cimi/CIMI_RM_CLINICAL.v.0.0.2.bmm",
            findings: &[
                "class `BaseAssertion` redefines property `name` as `CODED_TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `ContactInformation` redefines property `name` as `PersonName`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `MedicationOrder` redefines property `prnReason` as `List<Justification>`, which does not conform to `List<CODED_TEXT>` as declared by `Request`",
                "class `Qualification` redefines property `name` as `TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `Street` redefines property `name` as `CODED_TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
            ],
            adjudication: "CIMI's LOCATABLE declares `name: String` while these descendants redefine it to the DATA_VALUE-rooted TEXT hierarchy, which does not conform to String under master06-core-types.adoc §Type Conformance; `prnReason` narrows the contained type to a non-descendant the same way",
        },
        FindingCase {
            path: "odin/odin/CIMI_RM_CLINICAL.v.0.0.1.bmm",
            findings: &[
                "class `Assertion` redefines property `name` as `CODED_TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `ContactInformation` redefines property `name` as `PersonName`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `MaterialEntity` redefines property `name` as `TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `Procedure` redefines property `name` as `CODED_TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `Qualification` redefines property `name` as `TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
                "class `Street` redefines property `name` as `CODED_TEXT`, which does not conform to `String` as declared by `LOCATABLE`",
            ],
            adjudication: "the 0.0.1 generation of the same CIMI defect set",
        },
        FindingCase {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/duplicate_class.bmm",
            findings: &[
                "class `ParentType1` is contained within 2 package listing(s) (ParentPackage, ParentPackage); a class must be contained within exactly one package",
            ],
            adjudication: "ParentPackage lists ParentType1 twice, so the class is contained twice — master05-core-model.adoc §Packages. The transform still materialises the model (a duplicate list entry breaks no BMM_* construction), which is why this is a collected finding rather than one of the fail-fast refusals",
        },
        FindingCase {
            path: "bmm/org/openehr/bmm/v2/persistence/validation/overridden_property_non_conformance.bmm",
            findings: &[
                "class `ChildType1` redefines property `property_1` as `ParentType1`, which does not conform to `String` as declared by `ParentType1`",
            ],
            adjudication: "ChildType1 redefines the inherited `property_1: String` as `ParentType1`, which has only the implicit `Any` ancestor and so fails the base-class test of master06-core-types.adoc §Type Conformance",
        },
    ]
}

/// What the pipeline must do with one vendored openEHR component schema.
enum PinnedOutcome {
    /// Reads, resolves and materialises: the class count and every
    /// model-validity finding, in the order the pass emits them.
    Model(usize, &'static [&'static str]),
    /// An adjudicated refusal: the stage, the typed discriminant, and a
    /// substring of the message pinning WHICH element is at fault.
    Refused(Stage, Kind, &'static str),
}

/// One row of the pinned-generation outcome table.
struct PinnedCase {
    /// Path relative to [`CODEGEN_VENDOR_ODIN`].
    file: &'static str,
    /// The adjudicated outcome.
    outcome: PinnedOutcome,
    /// The spec ground for the outcome. Documentation for the reader; not
    /// asserted.
    #[expect(dead_code, reason = "the adjudication rationale documents the row")]
    adjudication: &'static str,
}

/// The schemas offered for inclusion resolution, one file per `schema_id`.
///
/// `LANG/odin/openehr_lang_1.1.0-bmm3.bmm` is deliberately absent: it renders
/// the same `schema_id` (`openehr_lang_1.1.0`) as the v2.x LANG file, so the
/// two published files cannot both answer one `includes` entry. The
/// tie-break is exercised from both sides —
/// [`am_2_4_0_is_refused_against_either_published_lang_1_1_0_schema`] pins that
/// neither choice materialises AM 2.4.0.
const INCLUDE_SOURCES: &[&str] = &[
    "AM/odin/openehr_am_1.4.0.bmm",
    "AM/odin/openehr_am_2.2.0.bmm",
    "AM/odin/openehr_am_2.3.0.bmm",
    "AM/odin/openehr_am_2.4.0.bmm",
    "BASE/odin/openehr_base_1.0.4.bmm",
    "BASE/odin/openehr_base_1.1.0.bmm",
    "BASE/odin/openehr_base_1.2.0.bmm",
    "BASE/odin/openehr_base_1.3.0.bmm",
    "LANG/odin/openehr_lang_1.0.0.bmm",
    "LANG/odin/openehr_lang_1.1.0.bmm",
    "RM/odin/openehr_rm_1.0.2.bmm",
    "RM/odin/openehr_rm_1.0.3.bmm",
    "RM/odin/openehr_rm_1.0.4.bmm",
    "RM/odin/openehr_rm_1.1.0.bmm",
    "RM/odin/openehr_rm_1.2.0.bmm",
    "TERM/odin/openehr_term_3.0.0.bmm",
    "TERM/odin/openehr_term_3.1.0.bmm",
];

/// The adjudicated outcome of EVERY openEHR component schema `openehr-codegen`
/// vendors, as ODIN — 18 files, the complete pinned matrix.
///
/// Seven materialise; eleven are refused, and each refusal is a first-hand
/// adjudication that the RELEASED schema references something its own
/// inclusion closure does not define. The pipeline is fail-fast
/// (`master02-overview.adoc` §Conceptual Approach), so a refusal row names the
/// FIRST unresolvable element, not the schema's whole defect set.
#[expect(
    clippy::too_many_lines,
    reason = "one row per vendored component schema; splitting the table hides the pinned matrix"
)]
fn pinned_cases() -> Vec<PinnedCase> {
    vec![
        PinnedCase {
            file: "AM/odin/openehr_am_1.4.0.bmm",
            outcome: PinnedOutcome::Model(
                110,
                &[
                    "class `Cardinality` is contained within 2 package listing(s) (org.openehr.am.aom14.archetype.constraint_model, org.openehr.base.foundation_types.interval); a class must be contained within exactly one package",
                    "class `CARDINALITY` is contained within 2 package listing(s) (org.openehr.am.aom14.archetype.constraint_model, org.openehr.base.foundation_types.interval); a class must be contained within exactly one package",
                    "2 class definitions share one name (Cardinality, CARDINALITY); all classes in a BMM model must be uniquely named",
                    "class `ARCHETYPE` redefines property `uid` as `HIER_OBJECT_ID`, which does not conform to `UUID` as declared by `AUTHORED_RESOURCE`",
                ],
            ),
            adjudication: "AM 1.4.0's own CARDINALITY and the included BASE 1.3.0's Cardinality are DIFFERENT classes whose names are equal under master05-core-model.adoc §Naming Convention ('the class name \"Hashable\" refers to the same class as \"HASHABLE\"'), so §Packages' uniqueness rule is violated and the two containment rows are that collision seen through the same case-insensitive fold; separately ARCHETYPE redefines the inherited `uid: UUID` as HIER_OBJECT_ID, which is not a UUID descendant",
        },
        PinnedCase {
            file: "AM/odin/openehr_am_2.2.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `ARCHETYPE` property `rules`: type `STATEMENT`",
            ),
            adjudication: "AM 2.2.0 includes openehr_base_1.1.0 and nothing else, yet types ARCHETYPE.rules as List<STATEMENT> — a class published only in the LANG schema. master04-syntax.adoc §Package Definition, second NOTE ('only classes defined in the same schema can be referenced') and §Inclusions make the inclusion list the only way to widen the class pool, and BMM_CONTAINER_TYPE's target roots in a BMM_CLASS, so the reference is unresolvable",
        },
        PinnedCase {
            file: "AM/odin/openehr_am_2.3.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `ARCHETYPE` property `rules`: type `STATEMENT_SET`",
            ),
            adjudication: "AM 2.3.0 includes openehr_lang_1.0.0, which defines no STATEMENT_SET (that class is published in LANG 1.1.0's beom package); the include therefore names the wrong LANG generation and the reference stays unresolvable",
        },
        PinnedCase {
            file: "AM/odin/openehr_am_2.4.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `BMM_ENUMERATION` property `item_values`: type `T`",
            ),
            adjudication: "AM 2.4.0 includes openehr_lang_1.1.0 and so inherits that schema's own defect below; against the other published openehr_lang_1.1.0 file it is refused for STATEMENT_SET instead — see am_2_4_0_is_refused_against_either_published_lang_1_1_0_schema",
        },
        PinnedCase {
            file: "BASE/odin/openehr_base_1.0.4.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `P_BMM_SINGLE_PROPERTY_OPEN` property `bmm_property`: type `BMM_OPEN_TYPE`",
            ),
            adjudication: "the schema declares no includes and defines no BMM_OPEN_TYPE, yet types P_BMM_SINGLE_PROPERTY_OPEN.bmm_property as BMM_PROPERTY<BMM_OPEN_TYPE>; BMM_GENERIC_TYPE's actual parameters root in BMM_CLASS, so the generic parameter cannot be materialised",
        },
        PinnedCase {
            file: "BASE/odin/openehr_base_1.1.0.bmm",
            outcome: PinnedOutcome::Model(66, &[]),
            adjudication: "self-contained (its own primitive_types block) and model-valid",
        },
        PinnedCase {
            file: "BASE/odin/openehr_base_1.2.0.bmm",
            outcome: PinnedOutcome::Model(71, &[]),
            adjudication: "self-contained and model-valid",
        },
        PinnedCase {
            file: "BASE/odin/openehr_base_1.3.0.bmm",
            outcome: PinnedOutcome::Model(72, &[]),
            adjudication: "the pinned BASE generation is self-contained and model-valid",
        },
        PinnedCase {
            file: "LANG/odin/openehr_lang_1.0.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `BMM_ACTION_TABLE` property `items`: type `List`",
            ),
            adjudication: "LANG 1.0.0 declares no includes and no primitive_types block, so none of the container and primitive classes it references exists: master04-syntax.adoc §Classes for Primitive Types is explicit that 'all container types such as List<T>, Hash<V,K> etc are exlicit in a BMM schema, and consequently, such types are normally defined ... in a BMM schema'. Every property of the schema is affected; List on BMM_ACTION_TABLE.items is simply the first",
        },
        PinnedCase {
            file: "LANG/odin/openehr_lang_1.1.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `BMM_ENUMERATION` property `item_values`: type `T`",
            ),
            adjudication: "BMM_ENUMERATION types item_values as List<T> while declaring no generic_parameter_defs, and its only ancestor BMM_CLASS declares none either — the released class doc (org.openehr.lang.bmm.bmm_enumeration.adoc, Inherit: BMM_CLASS, no generic parameter) says the same. bmm_open_type.adoc §Description requires 'The parameter must be in the type declaration of the owning BMM_CLASS', so T is neither an open type nor a class here. This is the schema's ONLY defect: with item_values removed the schema materialises 158 classes",
        },
        PinnedCase {
            file: "LANG/odin/openehr_lang_1.1.0-bmm3.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `EL_CASE` property `value_constraint`: type `C_OBJECT`",
            ),
            adjudication: "the v3 LANG schema types EL_CASE.value_constraint as C_OBJECT, an AM AOM2 class, while including only openehr_base_1.3.0; AM includes LANG, so the missing include would also be circular. This is the schema's ONLY defect: with value_constraint removed it materialises 191 classes",
        },
        PinnedCase {
            file: "RM/odin/openehr_rm_1.0.2.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `LOCATABLE` property `archetype_node_id`: type `String`",
            ),
            adjudication: "RM 1.0.2 declares no includes and no primitive_types block, so String, List, Integer and every other foundation class it references is undefined; the RM line first declares an include at 1.0.4",
        },
        PinnedCase {
            file: "RM/odin/openehr_rm_1.0.3.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `OBJECT_REF` property `id_namespace`: type `String`",
            ),
            adjudication: "the 1.0.3 generation of the same defect",
        },
        PinnedCase {
            file: "RM/odin/openehr_rm_1.0.4.bmm",
            outcome: PinnedOutcome::Model(
                206,
                &[
                    "class `AUTHORED_RESOURCE` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `RESOURCE_DESCRIPTION` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `TRANSLATION_DETAILS` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `RESOURCE_DESCRIPTION_ITEM` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `OPENEHR_DEFINITIONS` is contained within 2 package listing(s) (org.openehr.base.base_types.definitions, org.openehr.rm.support.definition); a class must be contained within exactly one package",
                    "class `BASIC_DEFINITIONS` is contained within 2 package listing(s) (org.openehr.base.base_types.definitions, org.openehr.rm.support.definition); a class must be contained within exactly one package",
                ],
            ),
            adjudication: "the first RM generation to declare an include (openehr_base_1.1.0); the six classes it lists in its own packages are listed again by BASE, against master05-core-model.adoc §Packages",
        },
        PinnedCase {
            file: "RM/odin/openehr_rm_1.1.0.bmm",
            outcome: PinnedOutcome::Model(
                222,
                &[
                    "class `AUTHORED_RESOURCE` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `RESOURCE_DESCRIPTION` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `TRANSLATION_DETAILS` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `RESOURCE_DESCRIPTION_ITEM` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                ],
            ),
            adjudication: "the same double containment against BASE 1.2.0, less the two definitions classes BASE 1.2.0 no longer duplicates",
        },
        PinnedCase {
            file: "RM/odin/openehr_rm_1.2.0.bmm",
            outcome: PinnedOutcome::Model(
                213,
                &[
                    "class `AUTHORED_RESOURCE` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `RESOURCE_DESCRIPTION` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `TRANSLATION_DETAILS` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `RESOURCE_DESCRIPTION_ITEM` is contained within 2 package listing(s) (org.openehr.base.resource, org.openehr.rm.common.resource); a class must be contained within exactly one package",
                    "class `CODE_PHRASE` is contained within 2 package listing(s) (org.openehr.base.foundation_types.terminology, org.openehr.rm.data_types.text); a class must be contained within exactly one package",
                ],
            ),
            adjudication: "RM 1.2.0 includes openehr_base_1.3.0 and both schemas list these five classes in a package of their own, so each is contained twice after the merge — master05-core-model.adoc §Packages",
        },
        PinnedCase {
            file: "TERM/odin/openehr_term_3.0.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `CODE` property `description`: type `String`",
            ),
            adjudication: "TERM 3.0.0 declares no includes and no primitive_types block, so String, List and Iso8601_date are all undefined; primitives are ordinary class definitions a schema must carry or include (master04-syntax.adoc §Classes for Primitive Types), never implicit",
        },
        PinnedCase {
            file: "TERM/odin/openehr_term_3.1.0.bmm",
            outcome: PinnedOutcome::Refused(
                Stage::Model,
                Kind::UnknownType,
                "class `CODE` property `description`: type `String`",
            ),
            adjudication: "the pinned TERM generation carries the same defect as 3.0.0",
        },
    ]
}

/// Runs the three stages over `path` and returns the observed outcome, or the
/// stage + error it failed at.
fn run(
    path: &str,
    includes: &BTreeMap<String, PBmmSchema>,
) -> Result<usize, (Stage, PBmmReadError)> {
    let schema = read_schema(&source(path)).map_err(|error| (Stage::Read, error))?;
    let resolved = resolve_includes(schema, includes).map_err(|error| (Stage::Resolve, error))?;
    let model = create_bmm_model(&resolved).map_err(|error| (Stage::Model, error))?;
    Ok(model.class_definitions.as_ref().map_or(0, BTreeMap::len))
}

#[test]
fn every_vendored_schema_reaches_its_adjudicated_outcome() {
    let cases = cases();
    let includes = include_map(&cases);
    for case in &cases {
        let observed = run(case.path, &includes);
        match (&case.outcome, observed) {
            (Outcome::Model(expected), Ok(classes)) => assert_eq!(
                classes, *expected,
                "{}: class count changed — re-adjudicate before updating",
                case.path
            ),
            (Outcome::Model(_), Err((stage, error))) => {
                panic!(
                    "{}: expected a model, got {stage:?} error {error}",
                    case.path
                )
            }
            (Outcome::Refused(stage, kind, detail), Err((observed_stage, error))) => {
                assert_eq!(observed_stage, *stage, "{}: wrong stage", case.path);
                assert_eq!(kind_of(&error), *kind, "{}: wrong error kind", case.path);
                let message = error.to_string();
                assert!(
                    message.contains(detail),
                    "{}: message {message:?} does not name {detail:?}",
                    case.path
                );
            }
            (Outcome::Refused(stage, kind, _), Ok(classes)) => panic!(
                "{}: expected {stage:?} refusal {kind:?}, got a {classes}-class model",
                case.path
            ),
        }
    }
}

#[test]
fn every_materialising_schema_reaches_its_adjudicated_findings() {
    // master05-core-model.adoc §Packages: "A model validity checker ensures
    // that every class is contained within exactly one package"; "all classes
    // in a BMM model should be uniquely named".
    let cases = cases();
    let includes = include_map(&cases);
    let expected = finding_cases();
    for case in &cases {
        let Ok(schema) = read_schema(&source(case.path)) else {
            continue;
        };
        let Ok(resolved) = resolve_includes(schema, &includes) else {
            continue;
        };
        let Ok(model) = create_bmm_model(&resolved) else {
            continue;
        };
        let observed: Vec<String> = validate_schema(&resolved, &model)
            .iter()
            .map(ToString::to_string)
            .collect();
        let claimed: &[&str] = expected
            .iter()
            .find(|row| row.path == case.path)
            .map_or(&[], |row| row.findings);
        assert_eq!(
            observed, claimed,
            "{}: model-validity findings changed — re-adjudicate before updating",
            case.path
        );
    }
}

#[test]
fn the_finding_table_names_only_schemas_that_materialise() {
    let paths: Vec<&str> = cases().iter().map(|case| case.path).collect();
    for row in finding_cases() {
        assert!(
            paths.contains(&row.path),
            "{}: the finding table names a file the corpus table does not",
            row.path
        );
        assert!(
            !row.findings.is_empty(),
            "{}: a clean schema is recorded by ABSENCE, not an empty row",
            row.path
        );
    }
}

/// Reads one file under [`CODEGEN_VENDOR_ODIN`] as a `P_BMM_SCHEMA`.
fn read_pinned(file: &str) -> PBmmSchema {
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(CODEGEN_VENDOR_ODIN)
        .join(file);
    let src =
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    read_schema(&src).unwrap_or_else(|e| panic!("{file}: the pinned schema reads: {e}"))
}

/// [`INCLUDE_SOURCES`] keyed by `schema_id`.
fn pinned_include_map() -> BTreeMap<String, PBmmSchema> {
    let mut available: BTreeMap<String, PBmmSchema> = BTreeMap::new();
    for file in INCLUDE_SOURCES {
        let schema = read_pinned(file);
        let id = schema.schema_id();
        assert!(
            available.insert(id.clone(), schema).is_none(),
            "{file}: a second inclusion source renders the schema_id {id}"
        );
    }
    available
}

#[test]
fn every_pinned_openehr_odin_schema_reaches_its_adjudicated_outcome() {
    // The complete matrix openehr-codegen vendors, run as models rather than
    // just read: seven materialise (their class count + §Packages findings
    // pinned) and eleven are refused where the RELEASED schema references a
    // class its own inclusion closure does not define.
    let available = pinned_include_map();
    for case in pinned_cases() {
        let schema = read_pinned(case.file);
        let id = schema.schema_id();
        let observed = resolve_includes(schema, &available)
            .map_err(|error| (Stage::Resolve, error))
            .and_then(|resolved| match create_bmm_model(&resolved) {
                Ok(model) => {
                    let findings: Vec<String> = validate_schema(&resolved, &model)
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    Ok((
                        model.class_definitions.as_ref().map_or(0, BTreeMap::len),
                        findings,
                    ))
                }
                Err(error) => Err((Stage::Model, error)),
            });
        match (case.outcome, observed) {
            (
                PinnedOutcome::Model(classes, findings),
                Ok((observed_classes, observed_findings)),
            ) => {
                assert_eq!(
                    observed_classes, classes,
                    "{id}: class count changed — re-adjudicate before updating"
                );
                assert_eq!(
                    observed_findings, findings,
                    "{id}: model-validity findings changed — re-adjudicate before updating"
                );
            }
            (PinnedOutcome::Model(..), Err((stage, error))) => {
                panic!("{id}: expected a model, got {stage:?} error {error}")
            }
            (PinnedOutcome::Refused(stage, kind, detail), Err((observed_stage, error))) => {
                assert_eq!(observed_stage, stage, "{id}: wrong stage");
                assert_eq!(kind_of(&error), kind, "{id}: wrong error kind");
                let message = error.to_string();
                assert!(
                    message.contains(detail),
                    "{id}: message {message:?} does not name {detail:?}"
                );
            }
            (PinnedOutcome::Refused(stage, kind, _), Ok((classes, _))) => {
                panic!("{id}: expected {stage:?} refusal {kind:?}, got a {classes}-class model")
            }
        }
    }
}

#[test]
fn the_pinned_table_covers_every_vendored_component_schema() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CODEGEN_VENDOR_ODIN);
    let mut on_disk: Vec<String> = Vec::new();
    collect_bmm(&root, &root, &mut on_disk);
    on_disk.sort();
    let mut claimed: Vec<String> = pinned_cases()
        .iter()
        .map(|case| case.file.to_owned())
        .collect();
    claimed.sort();
    assert_eq!(
        claimed, on_disk,
        "the pinned-generation table and the vendored component ODIN schemas disagree"
    );
    // Five components × their released generations.
    assert_eq!(on_disk.len(), 18);
}

#[test]
fn am_2_4_0_is_refused_against_either_published_lang_1_1_0_schema() {
    // openEHR publishes TWO openehr_lang_1.1.0 schemas — the v2.x BMM and the
    // v3 line — so AM 2.4.0's `includes = <["openehr_lang_1.1.0"]>` names no
    // unique schema. Neither choice materialises it, and each is refused for a
    // class the other file defines: master04-syntax.adoc §Inclusions keys an
    // include by schema id alone, so no third reading exists.
    for (lang, detail) in [
        (
            "LANG/odin/openehr_lang_1.1.0.bmm",
            "class `BMM_ENUMERATION` property `item_values`: type `T`",
        ),
        (
            "LANG/odin/openehr_lang_1.1.0-bmm3.bmm",
            "class `ARCHETYPE` property `rules`: type `STATEMENT_SET`",
        ),
    ] {
        let mut available: BTreeMap<String, PBmmSchema> = BTreeMap::new();
        for file in ["BASE/odin/openehr_base_1.3.0.bmm", lang] {
            let schema = read_pinned(file);
            available.insert(schema.schema_id(), schema);
        }
        let resolved = resolve_includes(read_pinned("AM/odin/openehr_am_2.4.0.bmm"), &available)
            .unwrap_or_else(|e| panic!("{lang}: inclusion resolution: {e}"));
        let error = create_bmm_model(&resolved)
            .err()
            .unwrap_or_else(|| panic!("{lang}: AM 2.4.0 unexpectedly materialised"));
        assert_eq!(
            kind_of(&error),
            Kind::UnknownType,
            "{lang}: wrong error kind"
        );
        let message = error.to_string();
        assert!(
            message.contains(detail),
            "{lang}: message {message:?} does not name {detail:?}"
        );
    }
}

#[test]
fn the_table_covers_every_vendored_bmm_file() {
    let root = vendor_root();
    let mut on_disk: Vec<String> = Vec::new();
    collect_bmm(&root, &root, &mut on_disk);
    on_disk.sort();
    let mut claimed: Vec<String> = cases().iter().map(|case| case.path.to_owned()).collect();
    claimed.sort();
    assert_eq!(
        claimed, on_disk,
        "the P_BMM expectation table and the vendored .bmm files disagree"
    );
    // 38 under bmm/** + 5 under odin/odin/.
    assert_eq!(on_disk.len(), 43);
}

/// Collects every `.bmm` path under `dir`, relative to `root`.
fn collect_bmm(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir: {e}")) {
        let path = entry.unwrap_or_else(|e| panic!("dir entry: {e}")).path();
        if path.is_dir() {
            collect_bmm(&path, root, out);
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("bmm") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or_else(|e| panic!("strip_prefix: {e}"));
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn the_pinned_openehr_odin_schemas_read_their_persisted_interfaces() {
    // master02-overview.adoc §Conceptual Approach: "In addition to ordinary
    // classes, the model can also represent pure interfaces via
    // P_BMM_INTERFACE, i.e. class-like definitions that declare only functions
    // and carry no state" — and the RM and BASE ODIN schemas this project pins
    // (docs/VERSIONS.md: RM 1.2.0, BASE 1.3.0) serialise them as
    // `(P_BMM_INTERFACE)`-marked members of `class_definitions`. Those real
    // artefacts (not a fixture) pin that the reader materialises each one with
    // its declared functions, and that the whole schema still reads.
    for (file, interfaces) in [
        (
            "RM/odin/openehr_rm_1.2.0.bmm",
            ["CODE_SET_ACCESS", "TERMINOLOGY_ACCESS"].as_slice(),
        ),
        (
            "BASE/odin/openehr_base_1.3.0.bmm",
            ["Env", "Locale", "Math", "Quantity_converter"].as_slice(),
        ),
    ] {
        let schema = read_pinned(file);
        for name in interfaces {
            let class = schema
                .primitive_types
                .iter()
                .flatten()
                .chain(schema.class_definitions.iter().flatten())
                .find(|class| class.name() == *name)
                .unwrap_or_else(|| panic!("{file}: {name} is not in the class list"));
            assert!(
                matches!(class, PBmmClass::PBmmInterface(_)),
                "{file}: {name} did not read as a P_BMM_INTERFACE",
            );
            assert!(
                class.functions().is_some_and(|f| !f.is_empty()),
                "{file}: {name} carries no functions",
            );
            // A pure interface declares only functions and carries no state.
            assert!(class.properties().is_none(), "{file}: {name} has state");
            assert!(class.is_abstract(), "{file}: {name} is not instantiable");
        }
    }
}

#[test]
fn the_pinned_openehr_odin_schemas_materialise_their_interfaces_as_abstract_classes() {
    // The whole pinned RM 1.2.0 + BASE 1.3.0 pair runs the three stages: the
    // interfaces are listed in packages and referenced as property types
    // (`TERMINOLOGY_SERVICE.terminology: TERMINOLOGY_ACCESS`), so the transform
    // has to resolve those references — which it does by materialising each
    // interface as an abstract `BMM_CLASS` with no properties (see
    // `create_model::Builder::build_class`).
    let base = read_pinned("BASE/odin/openehr_base_1.3.0.bmm");
    let rm = read_pinned("RM/odin/openehr_rm_1.2.0.bmm");
    let mut available: BTreeMap<String, PBmmSchema> = BTreeMap::new();
    available.insert(base.schema_id(), base.clone());

    for (schema, interfaces) in [
        (base, ["Env", "Locale", "Math"].as_slice()),
        (rm, ["CODE_SET_ACCESS", "TERMINOLOGY_ACCESS"].as_slice()),
    ] {
        let id = schema.schema_id();
        let resolved = resolve_includes(schema, &available)
            .unwrap_or_else(|e| panic!("{id}: inclusion resolution: {e}"));
        let model =
            create_bmm_model(&resolved).unwrap_or_else(|e| panic!("{id}: materialisation: {e}"));
        let classes = model
            .class_definitions
            .as_ref()
            .unwrap_or_else(|| panic!("{id}: the model defines no classes"));
        for name in interfaces {
            let class = classes
                .get(*name)
                .unwrap_or_else(|| panic!("{id}: {name} is missing from the model"));
            assert!(
                class.is_abstract(),
                "{id}: {name} did not materialise as an abstract class",
            );
            assert!(
                class.properties().is_none(),
                "{id}: {name} materialised with state",
            );
        }
    }
}
