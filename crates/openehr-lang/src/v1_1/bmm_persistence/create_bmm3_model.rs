// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `P_BMM_SCHEMA` → **v3** (`org.openehr.lang.bmm3`) `BMM_MODEL` transform.
//!
//! The sibling of [`crate::v1_1::bmm_persistence::create_model`] against the
//! other BMM generation's shapes. It is a separate module because the two
//! generations give the same Rust NAMES to structurally different types and this
//! workspace forbids import renaming, so each transform imports one generation.
//!
//! Four things the v2.x materialisation must leave in the P_BMM graph have a
//! declared destination in v3 and so reach a model here:
//!
//! * **A generic ancestor's parameter binding.** v3 states inheritance as a map
//!   of TYPES (`org.openehr.lang.bmm3.bmm_class.adoc` §Description,
//!   `Hash<String, BMM_MODEL_TYPE>`), so `P_BMM_CLASS.ancestor_defs`'
//!   `GENERIC_PARENT<T,SUPPLIER_B>` materialises as a `BMM_GENERIC_TYPE`
//!   ancestor carrying its substitutions.
//! * **A class's routines and constants**, which v3 `BMM_CLASS` declares as
//!   `features`, `functions`, `procedures` and `static_properties`.
//! * **`value_constraint`**, which v3 puts on the TYPE
//!   (`…bmm3.bmm_model_type.adoc` §Attributes) — where
//!   `P_BMM_BASE_TYPE.value_constraint` belongs, split on `::` per
//!   `master07-core-classes.adoc` §Value-set Types.
//! * **Generic-substituted properties**: where a class binds an ancestor's
//!   formal parameter, the ancestor's properties typed by it reappear in the
//!   descendant with `is_synthesised_generic` set.
//!
//! Two shapes are COLLECTED as a
//! [`crate::v1_1::bmm_persistence::validate::PBmmValidityFinding`] and omitted
//! rather than refusing the schema: an assertion string that is not EL or whose
//! names do not resolve (v3 requires `BMM_ASSERTION`,
//! `LANG/docs/bmm3/master10-expressions.adoc` §Usage in BMM Models, where P_BMM
//! persists an opaque string), and a constant stating no value
//! (`P_BMM_CONSTANT.value` is `0..1` while `BMM_CONSTANT.generator` is `1..1`,
//! and openEHR's own schemas omit it on `BMM_DEFINITIONS.Bmm_internal_version`).
//! No empty serial form is invented.
//!
//! NOTE (embedding depth, the same adjudication as the v2.x transform): a v3
//! `BMM_CLASS.ancestors` entry is a type whose `base_class` is a `BMM_CLASS`, so
//! full embedding would not terminate. An ancestor type's base class is therefore
//! a name-bearing STUB (name, package, flags — no features, no ancestors, no
//! generic parameters), and the complete definition of every class is always
//! `BMM_MODEL.class_definitions` ("All classes in this model, keyed by type
//! name", `…bmm3.bmm_model.adoc` §Attributes).

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::v1_1::bmm_persistence::create_bmm3_assertion::AssertionScope;
use crate::v1_1::bmm_persistence::create_bmm3_assertion::build_assertions;
use crate::v1_1::bmm_persistence::create_model::Builder;
use crate::v1_1::bmm_persistence::create_model::ClassEntry;
use crate::v1_1::bmm_persistence::create_model::check_enumeration_validity;
use crate::v1_1::bmm_persistence::create_model::multiplicity_of;
use crate::v1_1::bmm_persistence::create_model::property_context;
use crate::v1_1::bmm_persistence::create_model::qualify;
use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::p_bmm_constant::PBmmConstant;
use crate::v1_1::bmm_persistence::p_bmm_container_property::PBmmContainerProperty;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumeration;
use crate::v1_1::bmm_persistence::p_bmm_function::PBmmFunction;
use crate::v1_1::bmm_persistence::p_bmm_function_parameter::PBmmFunctionParameter;
use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
use crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType;
use crate::v1_1::bmm_persistence::p_bmm_package::PBmmPackage;
use crate::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;
use crate::v1_1::bmm_persistence::validate::AssertionKind;
use crate::v1_1::bmm_persistence::validate::PBmmValidityFinding;
use crate::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use crate::v1_1::bmm3::core::entity::bmm_container_type::BmmContainerType;
use crate::v1_1::bmm3::core::entity::bmm_container_type::BmmContainerTypeData;
use crate::v1_1::bmm3::core::entity::bmm_effective_type::BmmEffectiveType;
use crate::v1_1::bmm3::core::entity::bmm_generic_class::BmmGenericClass;
use crate::v1_1::bmm3::core::entity::bmm_generic_type::BmmGenericType;
use crate::v1_1::bmm3::core::entity::bmm_indexed_container_type::BmmIndexedContainerType;
use crate::v1_1::bmm3::core::entity::bmm_model_type::BmmModelType;
use crate::v1_1::bmm3::core::entity::bmm_module::BmmModule;
use crate::v1_1::bmm3::core::entity::bmm_parameter_type::BmmParameterType;
use crate::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
use crate::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClassData;
use crate::v1_1::bmm3::core::entity::bmm_simple_type::BmmSimpleType;
use crate::v1_1::bmm3::core::entity::bmm_type::BmmType;
use crate::v1_1::bmm3::core::entity::bmm_unitary_type::BmmUnitaryType;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration::BmmEnumeration;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration::BmmEnumerationData;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration_integer::BmmEnumerationInteger;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration_string::BmmEnumerationString;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_value_set_spec::BmmValueSetSpec;
use crate::v1_1::bmm3::core::feature::bmm_constant::BmmConstant;
use crate::v1_1::bmm3::core::feature::bmm_container_property::BmmContainerProperty;
use crate::v1_1::bmm3::core::feature::bmm_container_property::BmmContainerPropertyData;
use crate::v1_1::bmm3::core::feature::bmm_feature::BmmFeature;
use crate::v1_1::bmm3::core::feature::bmm_feature_group::BmmFeatureGroup;
use crate::v1_1::bmm3::core::feature::bmm_function::BmmFunction;
use crate::v1_1::bmm3::core::feature::bmm_indexed_container_property::BmmIndexedContainerProperty;
use crate::v1_1::bmm3::core::feature::bmm_parameter::BmmParameter;
use crate::v1_1::bmm3::core::feature::bmm_procedure::BmmProcedure;
use crate::v1_1::bmm3::core::feature::bmm_property::BmmProperty;
use crate::v1_1::bmm3::core::feature::bmm_result::BmmResult;
use crate::v1_1::bmm3::core::feature::bmm_static::BmmStatic;
use crate::v1_1::bmm3::core::literal_value::bmm_integer_value::BmmIntegerValue;
use crate::v1_1::bmm3::core::literal_value::bmm_literal_value::BmmLiteralValue;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValue;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValueData;
use crate::v1_1::bmm3::core::model::bmm_model::BmmModel;
use crate::v1_1::bmm3::core::model::bmm_package::BmmPackage;
use crate::v1_1::bmm3::statement::bmm_assertion::BmmAssertion;
use openehr_base::containers::present;

/// The default feature-group name every feature is placed in.
///
/// "Name of this feature group; defaults to 'feature'"
/// (`org.openehr.lang.bmm3.bmm_feature_group.adoc` §Attributes;
/// `LANG/docs/bmm3/master08-core-features.adoc` §Feature Groups and Visibility).
pub const DEFAULT_FEATURE_GROUP_NAME: &str = "feature";

/// The `::` separator a persisted `value_constraint` is split on: "The
/// construction within the `<<>>` is parsed into two pieces around the `::`
/// separator, which are then used to populate the `BMM_VALUE_SET_SPEC` for a
/// type" (`LANG/docs/bmm3/master07-core-classes.adoc` §Value-set Types).
const VALUE_SET_SEPARATOR: &str = "::";

/// Materialises the in-memory **v3** `BMM_MODEL` of an inclusion-resolved
/// `P_BMM_SCHEMA`.
///
/// See the module docs for what this generation carries that the v2.x one
/// cannot, and for the two recorded boundaries.
///
/// # Errors
/// The same failures as [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`]:
/// [`PBmmReadError::UnknownAncestor`], [`PBmmReadError::UnknownType`],
/// [`PBmmReadError::ClassNotInAnyPackage`], [`PBmmReadError::ClassNotDefined`],
/// [`PBmmReadError::ContainerTargetTypeMissing`],
/// [`PBmmReadError::TypeDefinitionMissing`],
/// [`PBmmReadError::UndeclaredGenericParameter`],
/// [`PBmmReadError::NotAGenericClass`],
/// [`PBmmReadError::EnumerationAncestorCount`] and
/// [`PBmmReadError::EnumerationItemListsNotOneToOne`] — the two transforms share
/// one enumeration-validity check, so they refuse the same schemas there — plus
/// [`PBmmReadError::EnumerationItemValueNotAnInteger`], which only this
/// generation can raise because only v3 types the item values.
pub fn create_bmm3_model(schema: &PBmmSchema) -> Result<BmmModel, PBmmReadError> {
    create_bmm3_model_reporting(schema).map(|(model, _)| model)
}

/// Materialises the v3 `BMM_MODEL` and returns the materialisation findings
/// alongside it.
///
/// Same transform as [`create_bmm3_model`]; the second element is every
/// persisted invariant / pre-condition / post-condition string that could not
/// become a `BMM_ASSERTION`
/// ([`crate::v1_1::bmm_persistence::validate::PBmmValidityFinding::AssertionNotMaterialised`])
/// and every constant that states no value
/// ([`crate::v1_1::bmm_persistence::validate::PBmmValidityFinding::ConstantNotMaterialised`]).
/// The findings are COLLECTED rather than fatal, for the same reason
/// [`crate::v1_1::bmm_persistence::validate::validate_schema`]'s are: a validity
/// report is only useful whole.
///
/// # Errors
/// The same failures as [`create_bmm3_model`].
pub fn create_bmm3_model_reporting(
    schema: &PBmmSchema,
) -> Result<(BmmModel, Vec<PBmmValidityFinding>), PBmmReadError> {
    let builder = Builder::new(schema)?;
    let mut class_definitions: BTreeMap<String, BmmClass> = BTreeMap::new();
    for entry in builder.classes.values() {
        class_definitions.insert(
            entry.class.name().to_owned(),
            build_class(&builder, entry, Depth::Full, &mut BTreeSet::new())?,
        );
    }
    let modules: BTreeMap<String, BmmModule> = class_definitions
        .iter()
        .map(|(name, class)| (name.clone(), as_module(class)))
        .collect();
    let packages = build_packages(&builder, &schema.packages, "")?;
    let findings = builder.findings.take();
    Ok((
        BmmModel {
            // `BMM_MODEL_ELEMENT.name` of the model is the schema's own name —
            // `P_BMM_SCHEMA.schema_name` (`…bmm_persistence.p_bmm_schema.adoc`
            // §Attributes).
            name: schema.schema_name.clone(),
            // P_BMM_SCHEMA declares neither a keyed `documentation` nor `extensions`
            // (same §Attributes), so both inherited `BMM_MODEL_ELEMENT` attributes
            // have no persisted source.
            documentation: None,
            extensions: None,
            packages: (!packages.is_empty()).then_some(packages),
            rm_publisher: schema.rm_publisher.clone(),
            rm_release: schema.rm_release.clone(),
            class_definitions: (!class_definitions.is_empty()).then_some(class_definitions),
            // `BMM_MODEL.used_models` is v3's model-import list
            // (`org.openehr.lang.bmm3.bmm_model.adoc` §Attributes). P_BMM composes
            // models by INCLUSION instead, and inclusion is already resolved into
            // this schema before the transform runs
            // (`LANG/docs/bmm_persistence/master02-overview.adoc` §Conceptual
            // Approach), so a materialised schema uses no separate model.
            used_models: present(Vec::new()),
            // `BMM_MODEL.modules` — "All classes in this model, keyed by type name"
            // (same §Attributes): the same population as `class_definitions`, viewed
            // under the module meta-type every v3 class is one of
            // (`…bmm3.bmm_class.adoc` §Inherit — `BMM_CLASS : BMM_MODULE`).
            modules: (!modules.is_empty()).then_some(modules),
        },
        findings,
    ))
}

/// One class under the `BMM_MODULE` meta-type it inherits
/// (`org.openehr.lang.bmm3.bmm_class.adoc` §Inherit).
fn as_module(class: &BmmClass) -> BmmModule {
    match class {
        BmmClass::BmmGenericClass(generic) => BmmModule::BmmGenericClass(generic.clone()),
        BmmClass::BmmSimpleClass(simple) => BmmModule::BmmSimpleClass(simple.clone()),
    }
}

/// How much of a referenced class is embedded (see the module docs' depth
/// adjudication).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// A class definition: features, properties, routines and the ancestor chain
    /// built. Used for a `BMM_MODEL.class_definitions` entry and for every
    /// ancestor of one, so the class-level walks
    /// ([`crate::v1_1::bmm3::core::entity::bmm_class::BmmClass::all_ancestors`],
    /// `has_ancestor_class`, `flat_features`) see the whole lineage.
    Full,
    /// A name-bearing stub: no features, ancestors as stubs. Used for the base
    /// class of a TYPE, which is where full embedding would stop terminating
    /// (a property's type resolves to a class whose own properties have
    /// types …). Formal generic parameters ARE declared even on a stub — a
    /// `BMM_GENERIC_CLASS` without them is not a valid instance and could not
    /// generate its fully open type (`BMM_CLASS.type`); their constraint
    /// chains are cycle-cut in [`build_parameter_type`].
    Stub,
}

/// The `BMM_CLASS` attributes built once for every class form.
struct ClassCore {
    /// `BMM_CLASS.name`.
    name: String,
    /// `BMM_CLASS.documentation` — the persisted scalar text under the
    /// recommended `"purpose"` key (see [`documentation_of`]).
    documentation: Option<BTreeMap<String, serde_json::Value>>,
    /// `BMM_CLASS.feature_groups`.
    feature_groups: Vec<BmmFeatureGroup>,
    /// `BMM_CLASS.features`.
    features: Vec<BmmFeature>,
    /// `BMM_CLASS.ancestors` — as TYPES.
    ancestors: Option<BTreeMap<String, BmmModelType>>,
    /// `BMM_CLASS.package`.
    package: BmmPackage,
    /// `BMM_CLASS.properties`.
    properties: Option<BTreeMap<String, BmmProperty>>,
    /// `BMM_CLASS.static_properties`.
    static_properties: Option<BTreeMap<String, BmmStatic>>,
    /// `BMM_CLASS.functions`.
    functions: Option<BTreeMap<String, BmmFunction>>,
    /// `BMM_CLASS.procedures`.
    procedures: Option<BTreeMap<String, BmmProcedure>>,
    /// `BMM_CLASS.source_schema_id`.
    source_schema_id: String,
    /// `BMM_CLASS.is_abstract`.
    is_abstract: Option<bool>,
    /// `BMM_CLASS.is_primitive`.
    is_primitive: Option<bool>,
    /// `BMM_CLASS.is_override`.
    is_override: bool,
    /// `BMM_CLASS.invariants` — every persisted invariant string that
    /// materialised (see the module docs).
    invariants: Vec<BmmAssertion>,
}

/// The v3 keyed `documentation` form of a persisted scalar documentation string.
///
/// `BMM_MODEL_ELEMENT.documentation` is a `Hash<String, Any>` whose recommended
/// keys include `"purpose": String` and where "Other keys and value types may be
/// freely added" (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes),
/// while P_BMM persists a single `documentation: String`
/// (`…bmm_persistence.p_bmm_model_element.adoc` §Attributes). The scalar
/// therefore lands under `"purpose"`, the key the spec names for exactly that
/// content.
fn documentation_of(text: Option<&str>) -> Option<BTreeMap<String, serde_json::Value>> {
    text.map(|text| {
        let mut out = BTreeMap::new();
        out.insert(
            "purpose".to_owned(),
            serde_json::Value::String(text.to_owned()),
        );
        out
    })
}

/// The feature group a materialised feature belongs to: the default group, whose
/// own `features` list is left empty because `BMM_FEATURE.group` and
/// `BMM_FEATURE_GROUP.features` are the two directions of one relation
/// (`org.openehr.lang.bmm3.bmm_feature.adoc` /
/// `…bmm3.bmm_feature_group.adoc` §Attributes) — the group the CLASS carries
/// (`BMM_CLASS.feature_groups`) is the populated one.
fn group_back_reference() -> BmmFeatureGroup {
    BmmFeatureGroup {
        name: DEFAULT_FEATURE_GROUP_NAME.to_owned(),
        properties: BTreeMap::new(),
        features: present(Vec::new()),
        visibility: None,
    }
}

/// The `BMM_VALUE_SET_SPEC` of a persisted `value_constraint`, split on `::`
/// (`LANG/docs/bmm3/master07-core-classes.adoc` §Value-set Types). A constraint
/// with no separator is taken as a value-set id in an unnamed resource: BMM "does
/// not impose any particular format or resolution algorithm on these identifiers"
/// (same §Value-set Types), so neither half is refused.
fn value_set_spec(constraint: Option<&str>) -> Option<BmmValueSetSpec> {
    constraint.map(
        |constraint| match constraint.split_once(VALUE_SET_SEPARATOR) {
            Some((resource_id, value_set_id)) => BmmValueSetSpec {
                resource_id: resource_id.to_owned(),
                value_set_id: value_set_id.to_owned(),
            },
            None => BmmValueSetSpec {
                resource_id: String::new(),
                value_set_id: constraint.to_owned(),
            },
        },
    )
}

/// Builds one v3 `BMM_CLASS` at the given embedding depth.
fn build_class(
    builder: &Builder<'_>,
    entry: &ClassEntry<'_>,
    depth: Depth,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmClass, PBmmReadError> {
    let persisted = entry.class;
    let mut core = build_core(builder, entry, depth, visiting)?;
    synthesise_generic_properties(builder, depth, visiting, &mut core)?;
    if depth == Depth::Full {
        let scope = AssertionScope {
            builder,
            owner: persisted,
            properties: core.properties.as_ref(),
            statics: core.static_properties.as_ref(),
            functions: core.functions.as_ref(),
            result_type: None,
        };
        let mut findings: Vec<PBmmValidityFinding> = Vec::new();
        core.invariants = build_assertions(
            &scope,
            AssertionKind::Invariant,
            None,
            persisted.invariants(),
            &mut findings,
        );
        builder.findings.borrow_mut().extend(findings);
    }
    if let PBmmClass::PBmmEnumeration(enumeration) = persisted {
        check_enumeration_validity(core.name.as_str(), enumeration)?;
        return Ok(BmmClass::BmmSimpleClass(BmmSimpleClass::BmmEnumeration(
            build_enumeration(builder, core, enumeration, visiting)?,
        )));
    }
    if persisted.is_generic() {
        return Ok(BmmClass::BmmGenericClass(BmmGenericClass {
            name: core.name,
            documentation: core.documentation,
            extensions: None,
            feature_groups: present(core.feature_groups),
            features: present(core.features),
            ancestors: core.ancestors,
            package: core.package,
            properties: core.properties,
            source_schema_id: core.source_schema_id,
            // `BMM_CLASS.immediate_descendants` is `List<BMM_CLASS>` in v3
            // (`…bmm3.bmm_class.adoc` §Attributes) — a downward reference the
            // emitter cannot own without making the type non-constructible, so
            // the inverted graph stays a model-level query.
            immediate_descendants: present(Vec::new()),
            is_override: core.is_override,
            static_properties: core.static_properties,
            functions: core.functions,
            procedures: core.procedures,
            is_primitive: core.is_primitive,
            is_abstract: core.is_abstract,
            invariants: present(core.invariants),
            // `creators`/`converters` are subsets of `procedures` a schema
            // designates (`…bmm3.bmm_class.adoc` §Attributes); P_BMM has no
            // attribute designating them, so no subset can be computed.
            creators: None,
            converters: None,
            generic_parameters: build_generic_parameters(builder, persisted, visiting)?,
        }));
    }
    Ok(BmmClass::BmmSimpleClass(BmmSimpleClass::BmmSimpleClass(
        BmmSimpleClassData {
            name: core.name,
            documentation: core.documentation,
            extensions: None,
            feature_groups: present(core.feature_groups),
            features: present(core.features),
            ancestors: core.ancestors,
            package: core.package,
            properties: core.properties,
            source_schema_id: core.source_schema_id,
            immediate_descendants: present(Vec::new()),
            is_override: core.is_override,
            static_properties: core.static_properties,
            functions: core.functions,
            procedures: core.procedures,
            is_primitive: core.is_primitive,
            is_abstract: core.is_abstract,
            invariants: present(core.invariants),
            creators: None,
            converters: None,
        },
    )))
}

/// Builds the shared `BMM_CLASS` attributes.
fn build_core(
    builder: &Builder<'_>,
    entry: &ClassEntry<'_>,
    depth: Depth,
    visiting: &mut BTreeSet<String>,
) -> Result<ClassCore, PBmmReadError> {
    let persisted = entry.class;
    let name = persisted.name();
    let (properties, statics, functions, procedures) = match depth {
        Depth::Stub => (None, None, None, None),
        Depth::Full => (
            build_properties(builder, persisted, visiting)?,
            build_constants(builder, persisted, visiting)?,
            build_functions(builder, persisted, visiting)?,
            build_procedures(builder, persisted, visiting)?,
        ),
    };
    let features = collect_features(
        properties.as_ref(),
        statics.as_ref(),
        functions.as_ref(),
        procedures.as_ref(),
    );
    Ok(ClassCore {
        name: name.to_owned(),
        documentation: documentation_of(persisted.documentation()),
        // `BMM_MODULE.feature_groups` is "List of feature groups in this class"
        // (`org.openehr.lang.bmm3.bmm_module.adoc` §Attributes). P_BMM declares no
        // grouping, so every feature is in the one default group
        // ([`DEFAULT_FEATURE_GROUP_NAME`]), which the class carries populated.
        feature_groups: if features.is_empty() {
            Vec::new()
        } else {
            vec![BmmFeatureGroup {
                name: DEFAULT_FEATURE_GROUP_NAME.to_owned(),
                properties: BTreeMap::new(),
                features: present(features.clone()),
                visibility: None,
            }]
        },
        features,
        ancestors: build_ancestors(builder, persisted, depth, visiting)?,
        package: package_of(builder, name)?,
        properties,
        static_properties: statics,
        functions,
        procedures,
        source_schema_id: persisted
            .source_schema_id()
            .unwrap_or(builder.schema_id.as_str())
            .to_owned(),
        is_abstract: Some(persisted.is_abstract()),
        is_primitive: Some(entry.is_primitive_type),
        is_override: persisted.is_override(),
        // Filled by `build_class` once the feature maps a bare name resolves
        // against are complete.
        invariants: Vec::new(),
    })
}

/// The owning-package stub of the class named `name` — the v3 `BMM_PACKAGE`
/// counterpart of the v2.x one (`BMM_CLASS.package` is `1..1`,
/// `org.openehr.lang.bmm3.bmm_class.adoc` §Attributes).
fn package_of(builder: &Builder<'_>, name: &str) -> Result<BmmPackage, PBmmReadError> {
    let path = builder
        .owning_package
        .get(&name.to_uppercase())
        .ok_or_else(|| PBmmReadError::ClassNotInAnyPackage {
            class: name.to_owned(),
        })?;
    Ok(BmmPackage {
        name: path.clone(),
        documentation: None,
        extensions: None,
        packages: None,
        members: present(Vec::new()),
    })
}

/// Every feature of the class, as the union of the specific maps — "all features
/// are contained in the `_features_` attribute … features of each specific type
/// being referenced in a dedicated map"
/// (`LANG/docs/bmm3/master07-core-classes.adoc` §Overview).
fn collect_features(
    properties: Option<&BTreeMap<String, BmmProperty>>,
    statics: Option<&BTreeMap<String, BmmStatic>>,
    functions: Option<&BTreeMap<String, BmmFunction>>,
    procedures: Option<&BTreeMap<String, BmmProcedure>>,
) -> Vec<BmmFeature> {
    let mut out: Vec<BmmFeature> = Vec::new();
    for property in properties.into_iter().flatten().map(|(_, p)| p) {
        out.push(match property {
            BmmProperty::BmmContainerProperty(container) => {
                BmmFeature::BmmContainerProperty(container.clone())
            }
            BmmProperty::BmmUnitaryProperty(unitary) => {
                BmmFeature::BmmUnitaryProperty(unitary.clone())
            }
        });
    }
    for value in statics.into_iter().flatten().map(|(_, s)| s) {
        out.push(match value {
            BmmStatic::BmmConstant(constant) => BmmFeature::BmmConstant(constant.clone()),
            BmmStatic::BmmSingleton(singleton) => BmmFeature::BmmSingleton(singleton.clone()),
        });
    }
    for function in functions.into_iter().flatten().map(|(_, f)| f) {
        out.push(BmmFeature::BmmFunction(function.clone()));
    }
    for procedure in procedures.into_iter().flatten().map(|(_, p)| p) {
        out.push(BmmFeature::BmmProcedure(procedure.clone()));
    }
    out
}

/// Builds `BMM_CLASS.ancestors` as a map of TYPES, carrying a generic ancestor's
/// parameter substitutions (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes +
/// §Description; `LANG/docs/bmm_persistence/master04-syntax.adoc` §Inheritance).
fn build_ancestors(
    builder: &Builder<'_>,
    persisted: &PBmmClass,
    depth: Depth,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<BTreeMap<String, BmmModelType>>, PBmmReadError> {
    // The chain is only followed once per lineage: re-entering a class already on
    // the stack would not terminate on a schema whose inheritance is cyclic,
    // which the spec forbids ("results in an acyclic graph",
    // `LANG/docs/bmm3/master13-model_semantics.adoc` §Simple Inheritance) but
    // nothing in the persisted form enforces.
    if !visiting.insert(persisted.name().to_uppercase()) {
        return Ok(None);
    }
    let mut out: BTreeMap<String, BmmModelType> = BTreeMap::new();
    // The structured generic ancestors first: each carries the substitution the
    // plain name list cannot express.
    for def in persisted.ancestor_defs() {
        let context = format!("class `{}` ancestor `{}`", persisted.name(), def.root_type);
        let entry = builder.entry(&context, &def.root_type)?;
        out.insert(
            entry.class.name().to_owned(),
            BmmModelType::BmmGenericType(build_generic_type(
                builder, &context, def, persisted, visiting,
            )?),
        );
    }
    for parent in persisted.ancestors() {
        let entry = builder.classes.get(&parent.to_uppercase()).ok_or_else(|| {
            PBmmReadError::UnknownAncestor {
                class: persisted.name().to_owned(),
                ancestor: parent.clone(),
            }
        })?;
        // Keyed by the ancestor's OWN name, so a name read back out of the map
        // looks up in `BMM_MODEL.class_definitions`.
        let key = entry.class.name().to_owned();
        if out.contains_key(&key) {
            continue;
        }
        // An ancestor is embedded at the SAME depth as the class being built, so
        // a class definition carries its whole lineage and the class-level walks
        // are transitive; a stub's ancestors stay stubs.
        let ancestor = build_class(builder, entry, depth, visiting)?;
        out.insert(key, class_as_type(ancestor));
    }
    visiting.remove(&persisted.name().to_uppercase());
    Ok((!out.is_empty()).then_some(out))
}

/// Adds the generic-substituted properties a class inherits from a generic
/// ancestor whose formal parameters it binds.
///
/// `master13-model_semantics.adoc` §Generic Inheritance: where `DV_INTERVAL
/// <T:DV_ORDERED>` inherits `Interval<T:Ordered>`, "the resulting types of
/// `lower` and `upper` are now `T:DV_ORDERED` rather than `T:Ordered` from the
/// parent … these two properties are synthesised within `DV_INTERVAL<T>` with
/// their new concrete types. Their BMM meta-type objects (type
/// `BMM_UNITARY_PROPERTY`) will both have the meta-attribute
/// `_is_synthesised_generic_` set to `True`". The closed case is the same
/// operation with a concrete binding: `TIMER_WAIT` inheriting `WAIT<TIMER_
/// EVENT>` gets `event: TIMER_EVENT` "synthesised new … with the meta-attribute
/// `_is_synthesised_generic_` set `True`".
///
/// Both examples are ONE rule — replace the ancestor's formal parameter with
/// whatever the descendant bound it to — because a binding to another formal
/// parameter carries that parameter's own (narrowed) constraint in its type
/// object. Three decisions the section leaves to the implementation:
///
/// * A property the descendant DECLARES wins: the section synthesises the
///   ancestor's properties "within" the descendant, which cannot mean
///   overwriting one the descendant defines for itself.
/// * Propagation down a partially-closed chain is automatic rather than
///   special-cased: an embedded ancestor has already had its own synthesis
///   applied (this runs per class, before the class is embedded), so a
///   re-substituted property is just the next step of the same rule.
/// * Only a property whose type IS the parameter is synthesised. A parameter
///   nested inside a generic argument (`List<T>` where the container type is
///   the property's type) is substituted in place by the same walk.
fn synthesise_generic_properties(
    builder: &Builder<'_>,
    depth: Depth,
    visiting: &mut BTreeSet<String>,
    core: &mut ClassCore,
) -> Result<(), PBmmReadError> {
    let generic_ancestors: Vec<BmmGenericType> = core
        .ancestors
        .iter()
        .flat_map(|map| map.values())
        .filter_map(|ancestor| match ancestor {
            BmmModelType::BmmGenericType(generic) => Some(generic.clone()),
            BmmModelType::BmmSimpleType(_) => None,
        })
        .collect();
    let mut synthesised: BTreeMap<String, BmmProperty> = BTreeMap::new();
    for generic in generic_ancestors {
        let bindings = parameter_bindings(&generic);
        if bindings.is_empty() {
            continue;
        }
        // The embedded `base_class` is a STUB (no features — the
        // embedding-depth adjudication in the module docs), so the ancestor's
        // properties come from its own definition, rebuilt here. That build
        // applies this same synthesis to the ancestor, which is what carries a
        // substitution down a partially-closed chain.
        let name = generic.base_class.name.clone();
        let key = name.to_uppercase();
        if visiting.contains(&key) {
            continue;
        }
        let context = format!("generic ancestor `{name}`");
        let entry = builder.entry(&context, &name)?;
        visiting.insert(key.clone());
        let ancestor = build_class(builder, entry, depth, visiting);
        visiting.remove(&key);
        for (property_name, property) in ancestor?.properties().iter().copied().flatten() {
            if core
                .properties
                .as_ref()
                .is_some_and(|own| own.contains_key(property_name))
            {
                continue;
            }
            if let Some(substituted) = substitute_property(property, &bindings) {
                synthesised.insert(property_name.clone(), substituted);
            }
        }
    }
    if !synthesised.is_empty() {
        core.properties
            .get_or_insert_with(BTreeMap::new)
            .extend(synthesised);
    }
    Ok(())
}

/// Pairs a generic ancestor's FORMAL parameter names with the types the
/// descendant bound them to.
///
/// `BMM_GENERIC_TYPE.generic_parameters` is "the actual generic parameter
/// types" as an ordered list while `BMM_GENERIC_CLASS.generic_parameters` is
/// the formal declarations keyed by name
/// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` +
/// `…bmm3.bmm_generic_class.adoc` §Attributes), so the two zip positionally.
/// A binding that merely restates the formal parameter unchanged is dropped:
/// substituting a parameter for itself synthesises nothing.
fn parameter_bindings(generic: &BmmGenericType) -> BTreeMap<String, BmmUnitaryType> {
    generic
        .base_class
        .generic_parameters
        .iter()
        .zip(generic.generic_parameters.iter())
        .filter(|((name, formal), actual)| !is_unsubstituted(name, formal, actual))
        .map(|((name, _), actual)| (name.clone(), actual.clone()))
        .collect()
}

/// Whether `actual` restates the formal parameter unchanged — the same
/// parameter name under the same conformance constraint.
///
/// A same-named parameter under a NARROWER constraint is a substitution, which
/// is what makes `DV_INTERVAL<T:DV_ORDERED>` inheriting `Interval<T:Ordered>`
/// synthesise (`master13-model_semantics.adoc` §Generic Inheritance: "the
/// formal parameters of the inheriting class may further constrain any of the
/// ancestor type's formal parameters"). The constraint is read through
/// `BMM_PARAMETER_TYPE.flattened_conforms_to_type`
/// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions), so an
/// inherited precursor counts.
fn is_unsubstituted(name: &str, formal: &BmmParameterType, actual: &BmmUnitaryType) -> bool {
    let BmmUnitaryType::BmmParameterType(actual) = actual else {
        return false;
    };
    actual.name.eq_ignore_ascii_case(name) && conformance_name(actual) == conformance_name(formal)
}

/// A formal parameter's flattened conformance-type name, if it declares one.
fn conformance_name(parameter: &BmmParameterType) -> Option<String> {
    parameter
        .flattened_conforms_to_type()
        .map(BmmEffectiveType::type_name)
}

/// One ancestor property re-typed under `bindings`, or `None` when its type
/// mentions no bound parameter (nothing to synthesise).
fn substitute_property(
    property: &BmmProperty,
    bindings: &BTreeMap<String, BmmUnitaryType>,
) -> Option<BmmProperty> {
    match property {
        BmmProperty::BmmUnitaryProperty(unitary) => {
            let substituted = substitute_unitary(&unitary.r#type, bindings)?;
            let mut out = unitary.clone();
            out.r#type = substituted;
            out.is_synthesised_generic = Some(true);
            Some(BmmProperty::BmmUnitaryProperty(out))
        }
        BmmProperty::BmmContainerProperty(BmmContainerProperty::BmmContainerProperty(data)) => {
            let substituted = substitute_container(&data.r#type, bindings)?;
            let mut out = data.clone();
            out.r#type = substituted;
            out.is_synthesised_generic = Some(true);
            Some(BmmProperty::BmmContainerProperty(
                BmmContainerProperty::BmmContainerProperty(out),
            ))
        }
        BmmProperty::BmmContainerProperty(BmmContainerProperty::BmmIndexedContainerProperty(
            indexed,
        )) => {
            let substituted = substitute_indexed_container(&indexed.r#type, bindings)?;
            let mut out = indexed.clone();
            out.r#type = substituted;
            out.is_synthesised_generic = Some(true);
            Some(BmmProperty::BmmContainerProperty(
                BmmContainerProperty::BmmIndexedContainerProperty(out),
            ))
        }
    }
}

/// A unitary type with every bound formal parameter replaced, or `None` when it
/// mentions none. Recurses through a generic type's own arguments, so
/// `List<T>`-shaped property types substitute in place.
fn substitute_unitary(
    r#type: &BmmUnitaryType,
    bindings: &BTreeMap<String, BmmUnitaryType>,
) -> Option<BmmUnitaryType> {
    match r#type {
        BmmUnitaryType::BmmParameterType(parameter) => bindings
            .iter()
            .find(|(formal, _)| formal.eq_ignore_ascii_case(&parameter.name))
            .map(|(_, bound)| bound.clone()),
        BmmUnitaryType::BmmGenericType(generic) => {
            let mut arguments = generic.generic_parameters.clone();
            let mut touched = false;
            for argument in &mut arguments {
                if let Some(substituted) = substitute_unitary(argument, bindings) {
                    *argument = substituted;
                    touched = true;
                }
            }
            touched.then(|| {
                let mut out = generic.clone();
                out.generic_parameters = arguments;
                BmmUnitaryType::BmmGenericType(out)
            })
        }
        _ => None,
    }
}

/// A container type whose ITEM type mentions a bound parameter, re-typed
/// (`BMM_CONTAINER_TYPE.item_type`, `…bmm3.bmm_container_type.adoc`
/// §Attributes).
fn substitute_container(
    r#type: &BmmContainerType,
    bindings: &BTreeMap<String, BmmUnitaryType>,
) -> Option<BmmContainerType> {
    match r#type {
        BmmContainerType::BmmContainerType(data) => {
            let item = substitute_unitary(&data.item_type, bindings)?;
            let mut out = data.clone();
            out.item_type = Box::new(item);
            Some(BmmContainerType::BmmContainerType(out))
        }
        BmmContainerType::BmmIndexedContainerType(indexed) => {
            substitute_indexed_container(indexed, bindings)
                .map(|out| BmmContainerType::BmmIndexedContainerType(Box::new(out)))
        }
    }
}

/// The indexed-container arm of [`substitute_container`], reached directly by
/// a `BMM_INDEXED_CONTAINER_PROPERTY`, whose `type` is the indexed type itself.
fn substitute_indexed_container(
    r#type: &BmmIndexedContainerType,
    bindings: &BTreeMap<String, BmmUnitaryType>,
) -> Option<BmmIndexedContainerType> {
    let item = substitute_unitary(&r#type.item_type, bindings)?;
    let mut out = r#type.clone();
    out.item_type = item;
    Some(out)
}

/// The type a class generates — `BMM_CLASS.type` ("Generate a type object that
/// represents the type for which this class is the definer",
/// `org.openehr.lang.bmm3.bmm_class.adoc` §Functions), applied to a stub class.
fn class_as_type(class: BmmClass) -> BmmModelType {
    match class {
        BmmClass::BmmGenericClass(generic) => BmmModelType::BmmGenericType(generic.r#type()),
        BmmClass::BmmSimpleClass(simple) => BmmModelType::BmmSimpleType(simple.r#type()),
    }
}

/// Builds a generic class's formal parameter declarations as
/// `BMM_PARAMETER_TYPE`s (`org.openehr.lang.bmm3.bmm_generic_class.adoc`
/// §Attributes — "List of formal generic parameters, keyed by name").
fn build_generic_parameters(
    builder: &Builder<'_>,
    persisted: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BTreeMap<String, BmmParameterType>, PBmmReadError> {
    // Built at BOTH depths (no `Depth` parameter): a `BMM_GENERIC_CLASS`
    // without its formal parameters is not a valid instance of the model (the
    // parameters are what make it generic,
    // `org.openehr.lang.bmm3.bmm_generic_class.adoc` §Attributes), and
    // `class_as_type` must generate the fully open type of a stub. The
    // parameter NAMES recurse nowhere; the constraint chain is what
    // [`build_parameter_type`] cycle-cuts.
    let mut out = BTreeMap::new();
    for (key, parameter) in persisted.generic_parameter_defs().into_iter().flatten() {
        out.insert(
            key.clone(),
            build_parameter_type(
                builder,
                persisted.name(),
                &parameter.name,
                parameter.conforms_to_type.as_deref(),
                visiting,
            )?,
        );
    }
    Ok(out)
}

/// Builds one `BMM_PARAMETER_TYPE`: the formal parameter `name` with its optional
/// conformance constraint (`org.openehr.lang.bmm3.bmm_parameter_type.adoc`
/// §Attributes — `type_constraint` is an "Optional conformance constraint that
/// must be the name of a defined type").
fn build_parameter_type(
    builder: &Builder<'_>,
    owner: &str,
    name: &str,
    constraint: Option<&str>,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmParameterType, PBmmReadError> {
    let type_constraint = match constraint {
        None => None,
        Some(constrainer) => {
            // Constraint chains recurse: the constrainer builds as a stub whose
            // own formal parameters may be constrained back onto a class already
            // being built (`master13-model_semantics.adoc` §Simple Inheritance
            // states acyclicity for INHERITANCE only — nothing forbids `A<T: B>`
            // with `B<U: A>`), so the repeated edge is cut by omitting the
            // OPTIONAL constraint ("Optional conformance constraint",
            // `…bmm3.bmm_parameter_type.adoc` §Attributes) under a namespaced key.
            let edge = format!("parameter-constraint {owner}::{name}");
            if visiting.insert(edge.clone()) {
                let context = format!("class `{owner}` generic parameter `{name}`");
                let stub = build_class(
                    builder,
                    builder.entry(&context, constrainer)?,
                    Depth::Stub,
                    visiting,
                )?;
                visiting.remove(&edge);
                Some(Box::new(BmmEffectiveType::from(class_as_type(stub))))
            } else {
                None
            }
        }
    };
    Ok(BmmParameterType {
        name: name.to_owned(),
        type_constraint,
        // `inheritance_precursor` is "the corresponding generic parameter
        // definition in an ancestor class" (same §Attributes) — P_BMM declares no
        // such link, and computing one would guess which same-named ancestor
        // parameter is meant.
        inheritance_precursor: None,
    })
}

/// Builds a class's `BMM_CLASS.properties` map.
fn build_properties(
    builder: &Builder<'_>,
    persisted: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<BTreeMap<String, BmmProperty>>, PBmmReadError> {
    let Some(properties) = persisted.properties() else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, property) in properties {
        out.insert(
            key.clone(),
            build_property(builder, persisted, property, visiting)?,
        );
    }
    Ok((!out.is_empty()).then_some(out))
}

/// Builds one v3 `BMM_PROPERTY`.
///
/// A single/open/generic property is a `BMM_UNITARY_PROPERTY` whose type is
/// narrowed to `BMM_UNITARY_TYPE`
/// (`org.openehr.lang.bmm3.bmm_unitary_property.adoc` §Attributes) — in v3 that
/// enum DOES contain the model types, so an ordinary class-typed property is a
/// unitary property. `is_nullable` is the v3 spelling of optionality ("True if
/// this element can be null (Void) at execution time",
/// `…bmm3.bmm_formal_element.adoc` §Attributes), which is the negation of P_BMM's
/// `is_mandatory` (`…bmm_persistence.p_bmm_property.adoc` §Attributes).
#[expect(
    clippy::too_many_lines,
    reason = "one arm per P_BMM_PROPERTY subtype; splitting the dispatch would hide the five-way persisted-property → v3 BMM_PROPERTY mapping"
)]
fn build_property(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    property: &PBmmProperty,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmProperty, PBmmReadError> {
    let class_name = owner.name();
    match property {
        PBmmProperty::PBmmSingleProperty(single) => {
            let context = property_context(class_name, &single.name);
            let (r#type, constraint) = match (
                single.type_def.as_ref(),
                single.type_ref.as_ref(),
                single.r#type.as_deref(),
            ) {
                (Some(type_def), _, _) => (
                    build_unitary_type(builder, &context, type_def, owner, visiting)?,
                    None,
                ),
                (None, Some(type_ref), _) => (
                    build_named_unitary_type(builder, &context, &type_ref.r#type, owner, visiting)?,
                    type_ref.value_constraint.as_deref(),
                ),
                (None, None, Some(name)) => (
                    build_named_unitary_type(builder, &context, name, owner, visiting)?,
                    None,
                ),
                (None, None, None) => {
                    return Err(PBmmReadError::TypeDefinitionMissing { context });
                }
            };
            Ok(BmmProperty::BmmUnitaryProperty(unitary_property(
                &single.name,
                single.documentation.as_deref(),
                with_value_constraint(r#type, constraint),
                single.is_mandatory,
                single.is_im_runtime,
                single.is_im_infrastructure,
            )))
        }
        PBmmProperty::PBmmSinglePropertyOpen(open) => {
            let context = property_context(class_name, &open.name);
            let parameter = match (open.type_ref.as_ref(), open.r#type.as_deref()) {
                (Some(type_ref), _) => type_ref.r#type.as_str(),
                (None, Some(name)) => name,
                (None, None) => {
                    return Err(PBmmReadError::TypeDefinitionMissing { context });
                }
            };
            let r#type = BmmUnitaryType::BmmParameterType(Box::new(open_parameter_type(
                builder, owner, &open.name, parameter, visiting,
            )?));
            Ok(BmmProperty::BmmUnitaryProperty(unitary_property(
                &open.name,
                open.documentation.as_deref(),
                r#type,
                open.is_mandatory,
                open.is_im_runtime,
                open.is_im_infrastructure,
            )))
        }
        PBmmProperty::PBmmGenericProperty(generic) => {
            let context = property_context(class_name, &generic.name);
            let Some(type_def) = generic.type_def.as_ref() else {
                return Err(PBmmReadError::TypeDefinitionMissing { context });
            };
            let r#type = BmmUnitaryType::BmmGenericType(build_generic_type(
                builder, &context, type_def, owner, visiting,
            )?);
            Ok(BmmProperty::BmmUnitaryProperty(unitary_property(
                &generic.name,
                generic.documentation.as_deref(),
                r#type,
                generic.is_mandatory,
                generic.is_im_runtime,
                generic.is_im_infrastructure,
            )))
        }
        PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
            container,
        )) => {
            let context = property_context(class_name, &container.name);
            let Some(type_def) = container.type_def.as_ref() else {
                return Err(PBmmReadError::TypeDefinitionMissing { context });
            };
            let r#type = build_container_type(builder, &context, type_def, owner, visiting)?;
            Ok(BmmProperty::BmmContainerProperty(
                BmmContainerProperty::BmmContainerProperty(BmmContainerPropertyData {
                    name: container.name.clone(),
                    documentation: documentation_of(container.documentation.as_deref()),
                    extensions: None,
                    r#type,
                    is_nullable: is_nullable(container.is_mandatory),
                    is_synthesised_generic: None,
                    feature_extensions: present(Vec::new()),
                    group: group_back_reference(),
                    is_im_runtime: container.is_im_runtime,
                    is_im_infrastructure: container.is_im_infrastructure,
                    // P_BMM declares no composition flag
                    // (`…bmm_persistence.p_bmm_property.adoc` §Attributes), so
                    // v3's `is_composition` ("True if this property instance is a
                    // compositional sub-part of the owning class instance",
                    // `…bmm3.bmm_property.adoc` §Attributes) has no source.
                    is_composition: None,
                    cardinality: container.cardinality.as_ref().map(multiplicity_of),
                }),
            ))
        }
        PBmmProperty::PBmmContainerProperty(
            PBmmContainerProperty::PBmmIndexedContainerProperty(indexed),
        ) => {
            let context = property_context(class_name, &indexed.name);
            let Some(type_def) = indexed.type_def.as_ref() else {
                return Err(PBmmReadError::TypeDefinitionMissing { context });
            };
            let r#type =
                build_indexed_container_type(builder, &context, type_def, owner, visiting)?;
            Ok(BmmProperty::BmmContainerProperty(
                BmmContainerProperty::BmmIndexedContainerProperty(BmmIndexedContainerProperty {
                    name: indexed.name.clone(),
                    documentation: documentation_of(indexed.documentation.as_deref()),
                    extensions: None,
                    r#type,
                    is_nullable: is_nullable(indexed.is_mandatory),
                    is_synthesised_generic: None,
                    feature_extensions: present(Vec::new()),
                    group: group_back_reference(),
                    is_im_runtime: indexed.is_im_runtime,
                    is_im_infrastructure: indexed.is_im_infrastructure,
                    is_composition: None,
                    cardinality: indexed.cardinality.as_ref().map(multiplicity_of),
                }),
            ))
        }
    }
}

/// The v3 `is_nullable` of a persisted `is_mandatory`: an element that must be
/// present cannot be null, and vice versa
/// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Attributes vs
/// `…bmm_persistence.p_bmm_property.adoc` §Attributes). An unstated
/// `is_mandatory` leaves `is_nullable` unstated too, so its `{default = false}`
/// applies rather than a guess.
fn is_nullable(is_mandatory: Option<bool>) -> Option<bool> {
    is_mandatory.map(|mandatory| !mandatory)
}

/// Assembles one `BMM_UNITARY_PROPERTY`.
fn unitary_property(
    name: &str,
    documentation: Option<&str>,
    r#type: BmmUnitaryType,
    is_mandatory: Option<bool>,
    is_im_runtime: Option<bool>,
    is_im_infrastructure: Option<bool>,
) -> crate::v1_1::bmm3::core::feature::bmm_unitary_property::BmmUnitaryProperty {
    crate::v1_1::bmm3::core::feature::bmm_unitary_property::BmmUnitaryProperty {
        name: name.to_owned(),
        documentation: documentation_of(documentation),
        extensions: None,
        r#type,
        is_nullable: is_nullable(is_mandatory),
        is_synthesised_generic: None,
        feature_extensions: present(Vec::new()),
        group: group_back_reference(),
        is_im_runtime,
        is_im_infrastructure,
        is_composition: None,
    }
}

/// Attaches a value-set constraint to a model type
/// (`BMM_MODEL_TYPE.value_constraint`,
/// `org.openehr.lang.bmm3.bmm_model_type.adoc` §Attributes). A constraint on a
/// non-model type (a formal parameter, a built-in) has nowhere to attach — only
/// `BMM_MODEL_TYPE` declares the attribute — and P_BMM only ever states one on a
/// `type_ref` naming a class, so the other forms pass through unchanged.
fn with_value_constraint(r#type: BmmUnitaryType, constraint: Option<&str>) -> BmmUnitaryType {
    let Some(spec) = value_set_spec(constraint) else {
        return r#type;
    };
    match r#type {
        BmmUnitaryType::BmmSimpleType(mut simple) => {
            simple.value_constraint = Some(spec);
            BmmUnitaryType::BmmSimpleType(simple)
        }
        BmmUnitaryType::BmmGenericType(mut generic) => {
            generic.value_constraint = Some(spec);
            BmmUnitaryType::BmmGenericType(generic)
        }
        other => other,
    }
}

/// Builds a `BMM_UNITARY_TYPE` from a persisted `P_BMM_TYPE`.
///
/// A container type is not unitary (`org.openehr.lang.bmm3.bmm_unitary_type.adoc`
/// §Description — "the type of any instantiated object that is **not** a
/// container object"), so a persisted container in a single-property slot is
/// reduced to its item type, which is what `unitary_type()` returns for it
/// (`…bmm3.bmm_container_type.adoc` §Functions).
fn build_unitary_type(
    builder: &Builder<'_>,
    context: &str,
    r#type: &PBmmType,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmUnitaryType, PBmmReadError> {
    match r#type {
        PBmmType::PBmmSimpleType(simple) => {
            let built =
                build_named_unitary_type(builder, context, &simple.r#type, owner, visiting)?;
            Ok(with_value_constraint(
                built,
                simple.value_constraint.as_deref(),
            ))
        }
        PBmmType::PBmmOpenType(open) => Ok(BmmUnitaryType::BmmParameterType(Box::new(
            open_parameter_type(builder, owner, context, &open.r#type, visiting)?,
        ))),
        PBmmType::PBmmGenericType(generic) => Ok(BmmUnitaryType::BmmGenericType(
            build_generic_type(builder, context, generic, owner, visiting)?,
        )),
        PBmmType::PBmmContainerType(container) => {
            Ok(build_container_type(builder, context, container, owner, visiting)?.unitary_type())
        }
    }
}

/// Builds a `BMM_UNITARY_TYPE` from a persisted `P_BMM_BASE_TYPE`.
fn build_base_unitary_type(
    builder: &Builder<'_>,
    context: &str,
    r#type: &PBmmBaseType,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmUnitaryType, PBmmReadError> {
    match r#type {
        PBmmBaseType::PBmmSimpleType(simple) => {
            let built =
                build_named_unitary_type(builder, context, &simple.r#type, owner, visiting)?;
            Ok(with_value_constraint(
                built,
                simple.value_constraint.as_deref(),
            ))
        }
        PBmmBaseType::PBmmOpenType(open) => Ok(BmmUnitaryType::BmmParameterType(Box::new(
            open_parameter_type(builder, owner, context, &open.r#type, visiting)?,
        ))),
        PBmmBaseType::PBmmGenericType(generic) => Ok(BmmUnitaryType::BmmGenericType(
            build_generic_type(builder, context, generic, owner, visiting)?,
        )),
    }
}

/// Builds the `BMM_UNITARY_TYPE` a bare type NAME denotes: a formal generic
/// parameter of the owning class if it declares one, else the simple type of the
/// named class.
pub(super) fn build_named_unitary_type(
    builder: &Builder<'_>,
    context: &str,
    name: &str,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmUnitaryType, PBmmReadError> {
    if builder.find_generic_parameter(owner, name).is_some() {
        return Ok(BmmUnitaryType::BmmParameterType(Box::new(
            open_parameter_type(builder, owner, context, name, visiting)?,
        )));
    }
    let stub = build_class(
        builder,
        builder.entry(context, name)?,
        Depth::Stub,
        visiting,
    )?;
    Ok(match class_as_type(stub) {
        BmmModelType::BmmGenericType(generic) => BmmUnitaryType::BmmGenericType(generic),
        BmmModelType::BmmSimpleType(simple) => BmmUnitaryType::BmmSimpleType(simple),
    })
}

/// Builds the `BMM_PARAMETER_TYPE` of the formal parameter `parameter`, refusing
/// a parameter the owning class does not declare — "The parameter must be in the
/// type declaration of the owning `BMM_CLASS`"
/// (`org.openehr.lang.bmm.bmm_open_type.adoc` §Description, the persisted form's
/// own rule).
fn open_parameter_type(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    property: &str,
    parameter: &str,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmParameterType, PBmmReadError> {
    let declared = builder
        .find_generic_parameter(owner, parameter)
        .ok_or_else(|| PBmmReadError::UndeclaredGenericParameter {
            class: owner.name().to_owned(),
            property: property.to_owned(),
            parameter: parameter.to_owned(),
        })?;
    build_parameter_type(
        builder,
        owner.name(),
        &declared.name,
        declared.conforms_to_type.as_deref(),
        visiting,
    )
}

/// Builds a `BMM_GENERIC_TYPE`, carrying its actual parameters — "the string
/// types … then the complex type references"
/// (`LANG/docs/bmm_persistence/master04-syntax.adoc` §Generic Classes), each of
/// which is a `BMM_UNITARY_TYPE` (`org.openehr.lang.bmm3.bmm_generic_type.adoc`
/// §Attributes).
fn build_generic_type(
    builder: &Builder<'_>,
    context: &str,
    generic: &PBmmGenericType,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmGenericType, PBmmReadError> {
    let entry = builder.entry(context, &generic.root_type)?;
    let BmmClass::BmmGenericClass(base_class) = build_class(builder, entry, Depth::Stub, visiting)?
    else {
        return Err(PBmmReadError::NotAGenericClass {
            context: context.to_owned(),
            type_name: generic.root_type.clone(),
        });
    };
    let mut generic_parameters: Vec<BmmUnitaryType> = Vec::new();
    for name in generic.generic_parameters.iter().flatten() {
        generic_parameters.push(build_named_unitary_type(
            builder, context, name, owner, visiting,
        )?);
    }
    for parameter in &generic.generic_parameter_defs {
        generic_parameters.push(build_unitary_type(
            builder, context, parameter, owner, visiting,
        )?);
    }
    Ok(BmmGenericType {
        value_constraint: value_set_spec(generic.value_constraint.as_deref()),
        base_class,
        // `BMM_GENERIC_TYPE.generic_parameters` is `1..*`
        // (`docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm3.bmm_generic_type.adoc`
        // §Attributes); a source type specifier that supplied none is refused
        // here rather than carried as an empty list.
        generic_parameters: openehr_base::containers::NonEmptyVec::new(generic_parameters)
            .map_err(|empty| PBmmReadError::TypeDefinitionMissing {
                context: format!("generic type `{}`: {empty}", generic.root_type),
            })?,
    })
}

/// Builds a `BMM_CONTAINER_TYPE`.
///
/// `is_ordered`/`is_unique` carry the List/Set/Bag semantics
/// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Attributes); P_BMM states
/// only the container CLASS name (`master04-syntax.adoc` §Container Properties),
/// so both are left unstated and their declared defaults (`is_ordered = true`,
/// `is_unique = false`) apply — the container class name is the discriminator the
/// persisted form actually carries.
fn build_container_type(
    builder: &Builder<'_>,
    context: &str,
    container: &PBmmContainerType,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmContainerType, PBmmReadError> {
    match container {
        PBmmContainerType::PBmmIndexedContainerType(indexed) => {
            Ok(BmmContainerType::BmmIndexedContainerType(Box::new(
                build_indexed_container_type(builder, context, indexed, owner, visiting)?,
            )))
        }
        PBmmContainerType::PBmmContainerType(data) => {
            Ok(BmmContainerType::BmmContainerType(BmmContainerTypeData {
                container_class: generic_container_class(
                    builder,
                    context,
                    &data.container_type,
                    visiting,
                )?,
                item_type: Box::new(build_container_target(
                    builder,
                    context,
                    data.r#type.as_deref(),
                    data.type_def.as_ref(),
                    owner,
                    visiting,
                )?),
                is_ordered: None,
                is_unique: None,
            }))
        }
    }
}

/// Builds a `BMM_INDEXED_CONTAINER_TYPE` (`index_type` is a `BMM_SIMPLE_TYPE`,
/// `org.openehr.lang.bmm3.bmm_indexed_container_type.adoc` §Attributes).
fn build_indexed_container_type(
    builder: &Builder<'_>,
    context: &str,
    indexed: &PBmmIndexedContainerType,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmIndexedContainerType, PBmmReadError> {
    let index_stub = build_class(
        builder,
        builder.entry(context, &indexed.index_type)?,
        Depth::Stub,
        visiting,
    )?;
    let BmmModelType::BmmSimpleType(index_type) = class_as_type(index_stub) else {
        return Err(PBmmReadError::NotAGenericClass {
            context: context.to_owned(),
            type_name: indexed.index_type.clone(),
        });
    };
    Ok(BmmIndexedContainerType {
        container_class: generic_container_class(
            builder,
            context,
            &indexed.container_type,
            visiting,
        )?,
        item_type: build_container_target(
            builder,
            context,
            indexed.r#type.as_deref(),
            indexed.type_def.as_ref(),
            owner,
            visiting,
        )?,
        is_ordered: None,
        is_unique: None,
        index_type,
    })
}

/// The container's own class, which v3 requires to be a `BMM_GENERIC_CLASS`
/// ("The type of the container. This converts to the `_root_type_` in
/// `BMM_GENERIC_TYPE`", `org.openehr.lang.bmm3.bmm_container_type.adoc`
/// §Attributes) — `List<T>`, `Hash<K,V>` etc.
fn generic_container_class(
    builder: &Builder<'_>,
    context: &str,
    name: &str,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmGenericClass, PBmmReadError> {
    let stub = build_class(
        builder,
        builder.entry(context, name)?,
        Depth::Stub,
        visiting,
    )?;
    match stub {
        BmmClass::BmmGenericClass(generic) => Ok(generic),
        BmmClass::BmmSimpleClass(_) => Err(PBmmReadError::NotAGenericClass {
            context: context.to_owned(),
            type_name: name.to_owned(),
        }),
    }
}

/// Builds a container's item type from its `type` name or nested `type_def`.
fn build_container_target(
    builder: &Builder<'_>,
    context: &str,
    name: Option<&str>,
    type_def: Option<&PBmmBaseType>,
    owner: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmUnitaryType, PBmmReadError> {
    match (type_def, name) {
        (Some(nested), _) => build_base_unitary_type(builder, context, nested, owner, visiting),
        (None, Some(name)) => build_named_unitary_type(builder, context, name, owner, visiting),
        (None, None) => Err(PBmmReadError::ContainerTargetTypeMissing {
            context: context.to_owned(),
        }),
    }
}

/// Builds a class's `BMM_CLASS.functions` map — the persisted functions that
/// state a result ("Type definition of the function result, if any (absent for
/// procedures)", `…bmm_persistence.p_bmm_function.adoc` §Attributes).
fn build_functions(
    builder: &Builder<'_>,
    persisted: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<BTreeMap<String, BmmFunction>>, PBmmReadError> {
    let Some(functions) = persisted.functions() else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, function) in functions {
        if function.result.is_none() {
            continue;
        }
        out.insert(
            key.clone(),
            build_function(builder, persisted, function, visiting)?,
        );
    }
    Ok((!out.is_empty()).then_some(out))
}

/// Builds a class's `BMM_CLASS.procedures` map — the persisted functions with no
/// result (same §Attributes: a result is "absent for procedures").
fn build_procedures(
    builder: &Builder<'_>,
    persisted: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<BTreeMap<String, BmmProcedure>>, PBmmReadError> {
    let Some(functions) = persisted.functions() else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, function) in functions {
        if function.result.is_some() {
            continue;
        }
        out.insert(
            key.clone(),
            build_procedure(builder, persisted, function, visiting)?,
        );
    }
    Ok((!out.is_empty()).then_some(out))
}

/// Builds one `BMM_FUNCTION`.
///
/// `BMM_FUNCTION.result` is `1..1` — the "Automatically created Result variable"
/// (`org.openehr.lang.bmm3.bmm_function.adoc` §Attributes) — and
/// `Inv_result_type` states `type = Result.type`, so both carry the persisted
/// result type. `pre_conditions`/`post_conditions` stay empty until the
/// assertion work lands. `operator_definition` has no persisted
/// source (P_BMM declares no operator meta-data).
fn build_function(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    function: &PBmmFunction,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmFunction, PBmmReadError> {
    let context = format!("class `{}` function `{}`", owner.name(), function.name);
    let result = function
        .result
        .as_ref()
        .ok_or_else(|| PBmmReadError::TypeDefinitionMissing {
            context: context.clone(),
        })?;
    let result_type = BmmType::from(build_unitary_type(
        builder, &context, result, owner, visiting,
    )?);
    Ok(BmmFunction {
        name: function.name.clone(),
        documentation: documentation_of(function.documentation.as_deref()),
        extensions: None,
        r#type: result_type.clone(),
        is_nullable: function.is_nullable,
        is_synthesised_generic: None,
        feature_extensions: present(Vec::new()),
        group: group_back_reference(),
        parameters: present(build_parameters(builder, owner, function, visiting)?),
        pre_conditions: present(routine_conditions(
            builder,
            owner,
            function,
            AssertionKind::PreCondition,
            Some(&result_type),
        )),
        post_conditions: present(routine_conditions(
            builder,
            owner,
            function,
            AssertionKind::PostCondition,
            Some(&result_type),
        )),
        // `BMM_ROUTINE.definition` is the routine BODY
        // (`org.openehr.lang.bmm3.bmm_routine.adoc` §Attributes); P_BMM persists
        // no bodies, only signatures.
        definition: None,
        operator_definition: None,
        result: Box::new(BmmResult {
            // `BMM_RESULT` "redefines" its name to the pre-defined `Result`
            // variable (`…bmm3.bmm_result.adoc` §Description — "Automatically
            // declared variable representing result of a Function call").
            name: RESULT_VARIABLE_NAME.to_owned(),
            documentation: None,
            extensions: None,
            r#type: result_type,
            is_nullable: function.is_nullable,
        }),
    })
}

/// The name of a function's automatically declared result variable.
///
/// Spec: `org.openehr.lang.bmm3.bmm_result.adoc` §Description; the `Result`
/// keyword of `LANG/docs/bmm3/master08-core-features.adoc` §Variables — "the
/// pre-defined `Result`".
pub const RESULT_VARIABLE_NAME: &str = "Result";

/// Builds one `BMM_PROCEDURE` — a routine whose result meta-type is the built-in
/// Status type (`org.openehr.lang.bmm3.bmm_procedure.adoc` §Attributes: `type` is
/// redefined to `BMM_STATUS_TYPE`).
fn build_procedure(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    function: &PBmmFunction,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmProcedure, PBmmReadError> {
    Ok(BmmProcedure {
        name: function.name.clone(),
        documentation: documentation_of(function.documentation.as_deref()),
        extensions: None,
        r#type: crate::v1_1::bmm3::core::entity::bmm_status_type::BmmStatusType {},
        is_nullable: function.is_nullable,
        is_synthesised_generic: None,
        feature_extensions: present(Vec::new()),
        group: group_back_reference(),
        parameters: present(build_parameters(builder, owner, function, visiting)?),
        pre_conditions: present(routine_conditions(
            builder,
            owner,
            function,
            AssertionKind::PreCondition,
            None,
        )),
        post_conditions: present(routine_conditions(
            builder,
            owner,
            function,
            AssertionKind::PostCondition,
            None,
        )),
        definition: None,
    })
}

/// Materialises one routine's persisted pre- or post-conditions, recording a
/// finding for each string that does not become a `BMM_ASSERTION`.
///
/// The owning class's feature maps are still under construction at this point
/// (a routine is built BY `build_core`), so a bare lower-case name here takes
/// `elBareRef`'s `elFunctionCall` reading rather than resolving to a declared
/// property — the class-invariant pass is where the declared features are
/// available.
fn routine_conditions(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    function: &PBmmFunction,
    kind: AssertionKind,
    result_type: Option<&BmmType>,
) -> Vec<BmmAssertion> {
    let source = match kind {
        AssertionKind::PreCondition => function.pre_conditions.as_ref(),
        AssertionKind::PostCondition => function.post_conditions.as_ref(),
        AssertionKind::Invariant => None,
    };
    let scope = AssertionScope {
        builder,
        owner,
        properties: None,
        statics: None,
        functions: None,
        result_type,
    };
    let mut findings: Vec<PBmmValidityFinding> = Vec::new();
    let built = build_assertions(
        &scope,
        kind,
        Some(function.name.as_str()),
        source,
        &mut findings,
    );
    builder.findings.borrow_mut().extend(findings);
    built
}

/// Builds a routine's ordered parameter list — "Formal parameters of the routine"
/// (`org.openehr.lang.bmm3.bmm_routine.adoc` §Attributes) from the persisted map
/// keyed by parameter name (`…bmm_persistence.p_bmm_function.adoc` §Attributes).
///
/// NOTE (recorded deviation, the same one the v2.x transform records for generic
/// parameters): `BMM_ROUTINE.parameters` is an ORDERED list while P_BMM keys them
/// by name, so declaration order is not preserved and sorted-key order is used.
fn build_parameters(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    function: &PBmmFunction,
    visiting: &mut BTreeSet<String>,
) -> Result<Vec<BmmParameter>, PBmmReadError> {
    let mut out: Vec<BmmParameter> = Vec::new();
    for parameter in function.parameters.iter().flatten().map(|(_, p)| p) {
        let (name, documentation, r#type, is_nullable) = match parameter {
            PBmmFunctionParameter::PBmmSingleFunctionParameter(single) => {
                let context = parameter_context(owner.name(), &function.name, &single.name);
                let r#type = build_named_unitary_type(
                    builder,
                    &context,
                    single.r#type.as_str(),
                    owner,
                    visiting,
                )?;
                (
                    single.name.clone(),
                    single.documentation.clone(),
                    r#type,
                    single.is_nullable,
                )
            }
            PBmmFunctionParameter::PBmmSingleFunctionParameterOpen(open) => {
                let context = parameter_context(owner.name(), &function.name, &open.name);
                let r#type = BmmUnitaryType::BmmParameterType(Box::new(open_parameter_type(
                    builder,
                    owner,
                    &context,
                    open.r#type.as_str(),
                    visiting,
                )?));
                (
                    open.name.clone(),
                    open.documentation.clone(),
                    r#type,
                    open.is_nullable,
                )
            }
            PBmmFunctionParameter::PBmmGenericFunctionParameter(generic) => {
                let context = parameter_context(owner.name(), &function.name, &generic.name);
                (
                    generic.name.clone(),
                    generic.documentation.clone(),
                    BmmUnitaryType::BmmGenericType(build_generic_type(
                        builder,
                        &context,
                        &generic.type_def,
                        owner,
                        visiting,
                    )?),
                    generic.is_nullable,
                )
            }
            PBmmFunctionParameter::PBmmContainerFunctionParameter(container) => {
                let context = parameter_context(owner.name(), &function.name, &container.name);
                (
                    container.name.clone(),
                    container.documentation.clone(),
                    build_container_type(builder, &context, &container.type_def, owner, visiting)?
                        .unitary_type(),
                    container.is_nullable,
                )
            }
        };
        out.push(BmmParameter {
            name,
            documentation: documentation_of(documentation.as_deref()),
            extensions: None,
            r#type: BmmType::from(r#type),
            is_nullable,
            // `BMM_PARAMETER.direction` — "If none-supplied, the parameter is
            // treated as `in`" (`org.openehr.lang.bmm3.bmm_parameter.adoc`
            // §Attributes); P_BMM states no direction.
            direction: None,
        });
    }
    Ok(out)
}

/// The error context naming one routine parameter.
fn parameter_context(class: &str, routine: &str, parameter: &str) -> String {
    format!("class `{class}` routine `{routine}` parameter `{parameter}`")
}

/// Builds a class's `BMM_CLASS.static_properties` map from its persisted
/// constants — "Static properties defined in this class"
/// (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes), of which `BMM_CONSTANT`
/// is the literal-valued form (`…bmm3.bmm_static.adoc`).
///
/// A constant stating no value carries no `value_literal`, so it is COLLECTED
/// as a
/// [`crate::v1_1::bmm_persistence::validate::PBmmValidityFinding::ConstantNotMaterialised`]
/// and omitted — the same boundary the assertions take, and for the same
/// reason: the persisted form is legal, so the schema is not refused.
fn build_constants(
    builder: &Builder<'_>,
    persisted: &PBmmClass,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<BTreeMap<String, BmmStatic>>, PBmmReadError> {
    let Some(constants) = persisted.constants() else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, constant) in constants {
        let Some(value_literal) = constant.value.clone() else {
            builder
                .findings
                .borrow_mut()
                .push(PBmmValidityFinding::ConstantNotMaterialised {
                    class: persisted.name().to_owned(),
                    constant: constant.name.clone(),
                });
            continue;
        };
        out.insert(
            key.clone(),
            BmmStatic::BmmConstant(build_constant(
                builder,
                persisted,
                constant,
                value_literal,
                visiting,
            )?),
        );
    }
    Ok((!out.is_empty()).then_some(out))
}

/// Builds one `BMM_CONSTANT`.
///
/// `BMM_CONSTANT.generator` is `1..1` — "Literal value of the constant"
/// (`org.openehr.lang.bmm3.bmm_constant.adoc` §Attributes) — and P_BMM persists
/// the value as a serialised string (`…bmm_persistence.p_bmm_constant.adoc`
/// §Attributes), which is exactly `BMM_LITERAL_VALUE.value_literal` ("A serial
/// representation of the value", `…bmm3.bmm_literal_value.adoc` §Attributes). The
/// native `value` is left unset, per the literal-evaluation boundary recorded in
/// [`crate::v1_1::bmm3::core::literal_value::bmm_literal_value_impl`]. `Inv_not_nullable`
/// makes a constant non-nullable (`…bmm3.bmm_constant.adoc` §Invariants).
///
/// `value_literal` is `1..1`, so the persisted value arrives as a parameter its
/// caller has already resolved: a constant that states none never reaches here.
fn build_constant(
    builder: &Builder<'_>,
    owner: &PBmmClass,
    constant: &PBmmConstant,
    value_literal: String,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmConstant, PBmmReadError> {
    let context = format!("class `{}` constant `{}`", owner.name(), constant.name);
    let unitary = build_named_unitary_type(builder, &context, &constant.r#type, owner, visiting)?;
    // `BMM_PRIMITIVE_VALUE.type` is a `BMM_SIMPLE_TYPE`
    // (`org.openehr.lang.bmm3.bmm_primitive_value.adoc` §Attributes), so a
    // constant of a non-simple type carries its literal under the least-rich
    // literal form's simple type; only a simple type can be stated there.
    let BmmUnitaryType::BmmSimpleType(simple) = unitary.clone() else {
        return Err(PBmmReadError::UnknownType {
            context,
            type_name: constant.r#type.clone(),
        });
    };
    Ok(BmmConstant {
        name: constant.name.clone(),
        documentation: documentation_of(constant.documentation.as_deref()),
        extensions: None,
        r#type: BmmType::from(unitary),
        is_nullable: Some(false),
        is_synthesised_generic: None,
        feature_extensions: present(Vec::new()),
        group: group_back_reference(),
        generator: BmmLiteralValue::BmmPrimitiveValue(BmmPrimitiveValue::BmmPrimitiveValue(
            BmmPrimitiveValueData {
                value_literal,
                value: None,
                // `_syntax_` unset means the `json` default applies
                // (`…bmm3.bmm_literal_value.adoc` §Attributes); P_BMM states no
                // syntax for a constant value.
                syntax: None,
                r#type: simple,
            },
        )),
    })
}

/// Builds one `BMM_ENUMERATION` form, with `item_values` as typed literal values.
///
/// v3 types `item_values` as `List<BMM_PRIMITIVE_VALUE>` and its two degenerate
/// subtypes redefine it to `List<BMM_INTEGER_VALUE>` / `List<BMM_STRING_VALUE>`
/// (`org.openehr.lang.bmm3.bmm_enumeration.adoc`,
/// `…bmm3.bmm_enumeration_integer.adoc`, `…bmm3.bmm_enumeration_string.adoc`
/// §Attributes), where P_BMM persists them as `List<Any>` (JSON scalars). Each
/// persisted scalar therefore becomes the matching literal-value object over the
/// enumeration's underlying simple type, with `value_literal` carrying its serial
/// form.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per BMM_ENUMERATION form, each assembling the same 20-attribute class shape with its own redefined item_values type"
)]
fn build_enumeration(
    builder: &Builder<'_>,
    core: ClassCore,
    persisted: &PBmmEnumeration,
    visiting: &mut BTreeSet<String>,
) -> Result<BmmEnumeration, PBmmReadError> {
    let context = format!("enumeration class `{}`", core.name);
    let underlying = build_named_unitary_type(
        builder,
        &context,
        persisted.underlying_type_name(),
        &PBmmClass::PBmmEnumeration(persisted.clone()),
        visiting,
    )?;
    let BmmUnitaryType::BmmSimpleType(underlying) = underlying else {
        return Err(PBmmReadError::UnknownType {
            context,
            type_name: persisted.underlying_type_name().to_owned(),
        });
    };
    let item_names = persisted.item_names().to_vec();
    match persisted {
        PBmmEnumeration::PBmmEnumerationInteger(_) => {
            let item_values = integer_item_values(&core.name, persisted, &underlying)?;
            Ok(BmmEnumeration::BmmEnumerationInteger(
                BmmEnumerationInteger {
                    name: core.name,
                    documentation: core.documentation,
                    extensions: None,
                    feature_groups: present(core.feature_groups),
                    features: present(core.features),
                    ancestors: core.ancestors,
                    package: core.package,
                    properties: core.properties,
                    source_schema_id: core.source_schema_id,
                    immediate_descendants: present(Vec::new()),
                    is_override: core.is_override,
                    static_properties: core.static_properties,
                    functions: core.functions,
                    procedures: core.procedures,
                    is_primitive: core.is_primitive,
                    is_abstract: core.is_abstract,
                    invariants: present(core.invariants.clone()),
                    creators: None,
                    converters: None,
                    item_names: present(item_names),
                    item_values: present(item_values),
                },
            ))
        }
        PBmmEnumeration::PBmmEnumerationString(_) => {
            Ok(BmmEnumeration::BmmEnumerationString(BmmEnumerationString {
                name: core.name,
                documentation: core.documentation,
                extensions: None,
                feature_groups: present(core.feature_groups),
                features: present(core.features),
                ancestors: core.ancestors,
                package: core.package,
                properties: core.properties,
                source_schema_id: core.source_schema_id,
                immediate_descendants: present(Vec::new()),
                is_override: core.is_override,
                static_properties: core.static_properties,
                functions: core.functions,
                procedures: core.procedures,
                is_primitive: core.is_primitive,
                is_abstract: core.is_abstract,
                invariants: present(core.invariants.clone()),
                creators: None,
                converters: None,
                item_names: present(item_names),
                item_values: present(
                    persisted
                        .item_values()
                        .iter()
                        .map(|value| {
                            crate::v1_1::bmm3::core::literal_value::bmm_string_value::BmmStringValue {
                                value_literal: literal_form(value),
                                value: value
                                    .as_str()
                                    .map_or_else(|| literal_form(value), str::to_owned),
                                syntax: None,
                                r#type: underlying.clone(),
                            }
                        })
                        .collect(),
                ),
            }))
        }
        PBmmEnumeration::PBmmEnumeration(_) => {
            Ok(BmmEnumeration::BmmEnumeration(BmmEnumerationData {
                name: core.name,
                documentation: core.documentation,
                extensions: None,
                feature_groups: present(core.feature_groups),
                features: present(core.features),
                ancestors: core.ancestors,
                package: core.package,
                properties: core.properties,
                source_schema_id: core.source_schema_id,
                immediate_descendants: present(Vec::new()),
                is_override: core.is_override,
                static_properties: core.static_properties,
                functions: core.functions,
                procedures: core.procedures,
                is_primitive: core.is_primitive,
                is_abstract: core.is_abstract,
                invariants: present(core.invariants.clone()),
                creators: None,
                converters: None,
                item_names: present(item_names),
                item_values: present(
                    persisted
                        .item_values()
                        .iter()
                        .map(|value| {
                            BmmPrimitiveValue::BmmPrimitiveValue(BmmPrimitiveValueData {
                                value_literal: literal_form(value),
                                value: Some(value.clone()),
                                syntax: None,
                                r#type: underlying.clone(),
                            })
                        })
                        .collect(),
                ),
            }))
        }
    }
}

/// Builds the `BMM_ENUMERATION_INTEGER` item values, one `BMM_INTEGER_VALUE`
/// per persisted scalar.
///
/// `BMM_ENUMERATION_INTEGER` redefines `item_values` to
/// `List<BMM_INTEGER_VALUE>`
/// (`org.openehr.lang.bmm3.bmm_enumeration_integer.adoc` §Attributes) whose
/// `value` is a "Native Integer value"
/// (`org.openehr.lang.bmm3.bmm_integer_value.adoc` §Attributes) — an `Integer`
/// being a 32-bit integer
/// (`BASE/docs/foundation_types/master03-primitive_types.adoc` §Overview) — so
/// a persisted scalar of another kind, or outside that range, states no native
/// value and is refused rather than substituted.
///
/// The item is named from `item_names` at the same position, which
/// [`check_enumeration_validity`] has already proven 1:1 with the values
/// whenever any value is stated.
///
/// # Errors
/// [`PBmmReadError::EnumerationItemValueNotAnInteger`].
fn integer_item_values(
    class: &str,
    persisted: &PBmmEnumeration,
    underlying: &BmmSimpleType,
) -> Result<Vec<BmmIntegerValue>, PBmmReadError> {
    let names = persisted.item_names();
    persisted
        .item_values()
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let native = value
                .as_i64()
                .and_then(|wide| i32::try_from(wide).ok())
                .ok_or_else(|| PBmmReadError::EnumerationItemValueNotAnInteger {
                    class: class.to_owned(),
                    index,
                    item: names.get(index).cloned(),
                    value: literal_form(value),
                })?;
            Ok(BmmIntegerValue {
                value_literal: literal_form(value),
                value: native,
                syntax: None,
                r#type: underlying.clone(),
            })
        })
        .collect()
}

/// The serial form of a persisted enumeration value — `BMM_LITERAL_VALUE.value_literal`,
/// "A serial representation of the value"
/// (`org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes). A JSON string
/// contributes its text (a quoted literal would double-quote it); every other
/// scalar contributes its JSON rendering.
fn literal_form(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Builds the v3 `BMM_PACKAGE` tree, keyed in upper case
/// (`BMM_PACKAGE_CONTAINER.packages` — "Child packages; keys all in upper case
/// for guaranteed matching", `org.openehr.lang.bmm3.bmm_package_container.adoc`
/// §Attributes). A package's `members` are the modules (classes) it lists
/// (`…bmm3.bmm_package.adoc` §Attributes).
fn build_packages(
    builder: &Builder<'_>,
    packages: &BTreeMap<String, PBmmPackage>,
    prefix: &str,
) -> Result<BTreeMap<String, BmmPackage>, PBmmReadError> {
    let mut out = BTreeMap::new();
    for package in packages.values() {
        let path = qualify(prefix, &package.name);
        let mut members: Vec<BmmModule> = Vec::new();
        for class in package.classes.iter().flatten() {
            let entry = builder.classes.get(&class.to_uppercase()).ok_or_else(|| {
                PBmmReadError::ClassNotDefined {
                    package: package.name.clone(),
                    class: class.clone(),
                }
            })?;
            members.push(as_module(&build_class(
                builder,
                entry,
                Depth::Stub,
                &mut BTreeSet::new(),
            )?));
        }
        let children = build_packages(builder, &package.packages, &path)?;
        out.insert(
            package.name.to_uppercase(),
            BmmPackage {
                name: path,
                documentation: None,
                extensions: None,
                packages: (!children.is_empty()).then_some(children),
                members: present(members),
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "the Book ch11 test shape: `?` propagates the read/model plumbing while a let-else over the materialised class shape IS the assertion"
    )]

    use crate::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model;
    use crate::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model_reporting;
    use crate::v1_1::bmm_persistence::error::PBmmReadError;
    use crate::v1_1::bmm_persistence::reader::read_schema;
    use crate::v1_1::bmm_persistence::validate::PBmmValidityFinding;
    use crate::v1_1::bmm3::core::entity::bmm_class::BmmClass;
    use crate::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
    use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration::BmmEnumeration;
    use crate::v1_1::bmm3::core::feature::bmm_static::BmmStatic;
    use crate::v1_1::bmm3::core::literal_value::bmm_literal_value::BmmLiteralValue;
    use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValue;
    use crate::v1_1::bmm3::core::model::bmm_model::BmmModel;

    /// A schema whose single package lists the primitive base classes plus the
    /// class under test, in the `master04-syntax.adoc` §Header Items shape.
    fn schema_src(definitions: &str) -> String {
        format!(
            r#"
            bmm_version = <"2.4">
            rm_publisher = <"openehr">
            schema_name = <"bmm3_literal_values">
            rm_release = <"1.0.2">
            packages = <
                ["test"] = <
                    name = <"test">
                    classes = <"Integer", "String", "SUBJECT">
                >
            >
            class_definitions = <
                ["Integer"] = < name = <"Integer"> >
                ["String"] = < name = <"String"> >
                {definitions}
            >
            "#
        )
    }

    /// The v3 model of a schema built by [`schema_src`].
    fn model_of(definitions: &str) -> Result<BmmModel, PBmmReadError> {
        create_bmm3_model(&read_schema(&schema_src(definitions))?)
    }

    /// The `BMM_ENUMERATION_INTEGER` item values of the class `SUBJECT` in a
    /// model built from `definitions`.
    fn integer_items(definitions: &str) -> Result<Vec<i32>, PBmmReadError> {
        let model = model_of(definitions)?;
        let classes = model
            .class_definitions
            .as_ref()
            .expect("the model defines classes");
        let subject = classes.get("SUBJECT").expect("SUBJECT materialised");
        let BmmClass::BmmSimpleClass(BmmSimpleClass::BmmEnumeration(
            BmmEnumeration::BmmEnumerationInteger(enumeration),
        )) = subject
        else {
            panic!("SUBJECT is not an integer enumeration");
        };
        Ok(enumeration
            .item_values
            .iter()
            .flatten()
            .map(|item| item.value)
            .collect())
    }

    /// `org.openehr.lang.bmm3.bmm_enumeration_integer.adoc` §Attributes: the
    /// item values are `BMM_INTEGER_VALUE`s, so distinct persisted scalars stay
    /// distinct native values.
    #[test]
    fn integer_enumeration_values_reach_the_model_unchanged() {
        let items = integer_items(
            r#"["SUBJECT"] = (P_BMM_ENUMERATION_INTEGER) <
                    name = <"SUBJECT">
                    ancestors = <"Integer">
                    item_names = <"first", "second">
                    item_values = <1001, 1002>
                >"#,
        )
        .expect("the enumeration materialises");
        assert_eq!(items, vec![1001, 1002]);
    }

    /// An item value outside `Integer`'s 32-bit range
    /// (`BASE/docs/foundation_types/master03-primitive_types.adoc` §Overview)
    /// has no `BMM_INTEGER_VALUE.value`, so it is refused with the item named
    /// rather than collapsed onto another item's value.
    #[test]
    fn an_out_of_range_integer_enumeration_value_is_refused() {
        let refusal = integer_items(
            r#"["SUBJECT"] = (P_BMM_ENUMERATION_INTEGER) <
                    name = <"SUBJECT">
                    ancestors = <"Integer">
                    item_names = <"first", "second">
                    item_values = <3000000000, 4000000000>
                >"#,
        )
        .expect_err("an out-of-range item value is refused");
        assert_eq!(
            refusal,
            PBmmReadError::EnumerationItemValueNotAnInteger {
                class: "SUBJECT".to_owned(),
                index: 0,
                item: Some("first".to_owned()),
                value: "3000000000".to_owned(),
            },
        );
    }

    /// A persisted item value of another JSON kind names no `Integer` either.
    #[test]
    fn a_non_numeric_integer_enumeration_value_is_refused() {
        let refusal = integer_items(
            r#"["SUBJECT"] = (P_BMM_ENUMERATION_INTEGER) <
                    name = <"SUBJECT">
                    ancestors = <"Integer">
                    item_names = <"first", "second">
                    item_values = <"one", "two">
                >"#,
        )
        .expect_err("a non-integer item value is refused");
        assert_eq!(
            refusal,
            PBmmReadError::EnumerationItemValueNotAnInteger {
                class: "SUBJECT".to_owned(),
                index: 0,
                item: Some("first".to_owned()),
                value: "one".to_owned(),
            },
        );
    }

    /// `org.openehr.lang.bmm3.bmm_constant.adoc` §Attributes: `generator` is
    /// `1..1`, and its `value_literal` carries the persisted serial form.
    #[test]
    fn a_persisted_constant_value_becomes_the_generator_literal() {
        let model = model_of(
            r#"["SUBJECT"] = <
                    name = <"SUBJECT">
                    constants = <
                        ["Max_retries"] = < name = <"Max_retries"> type = <"Integer"> value = <"3"> >
                    >
                >"#,
        )
        .expect("the class materialises");
        let classes = model
            .class_definitions
            .as_ref()
            .expect("the model defines classes");
        let subject = classes.get("SUBJECT").expect("SUBJECT materialised");
        let statics = subject.static_properties().expect("SUBJECT has constants");
        let BmmStatic::BmmConstant(constant) =
            statics.get("Max_retries").expect("the constant is keyed")
        else {
            panic!("Max_retries is not a constant");
        };
        let BmmLiteralValue::BmmPrimitiveValue(BmmPrimitiveValue::BmmPrimitiveValue(literal)) =
            &constant.generator
        else {
            panic!("the generator is not a primitive value");
        };
        assert_eq!(literal.value_literal, "3");
    }

    /// `P_BMM_CONSTANT.value` is `0..1` while `BMM_LITERAL_VALUE.value_literal`
    /// is `1..1`, so a constant stating no value is omitted with a finding —
    /// never materialised with an empty serial form, and never a refusal of a
    /// persisted form the P_BMM spec admits.
    #[test]
    fn a_constant_without_a_persisted_value_is_omitted_with_a_finding() {
        let (model, findings) = create_bmm3_model_reporting(
            &read_schema(&schema_src(
                r#"["SUBJECT"] = <
                    name = <"SUBJECT">
                    constants = <
                        ["Max_retries"] = < name = <"Max_retries"> type = <"Integer"> >
                        ["Min_retries"] = < name = <"Min_retries"> type = <"Integer"> value = <"1"> >
                    >
                >"#,
            ))
            .expect("the fixture reads"),
        )
        .expect("the class materialises");
        assert_eq!(
            findings,
            vec![PBmmValidityFinding::ConstantNotMaterialised {
                class: "SUBJECT".to_owned(),
                constant: "Max_retries".to_owned(),
            }],
        );
        let classes = model
            .class_definitions
            .as_ref()
            .expect("the model defines classes");
        let subject = classes.get("SUBJECT").expect("SUBJECT materialised");
        let statics = subject.static_properties().expect("SUBJECT has constants");
        assert!(!statics.contains_key("Max_retries"));
        assert!(statics.contains_key("Min_retries"));
        // The omission reaches the feature list too, which is derived from the
        // same map (`master07-core-classes.adoc` §Overview).
        assert!(!subject.features().iter().any(|f| f.name() == "Max_retries"));
    }
}
