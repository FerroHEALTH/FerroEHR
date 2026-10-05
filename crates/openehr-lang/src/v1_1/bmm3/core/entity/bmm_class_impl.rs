// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of the BMM **v3** `BMM_CLASS` family.
//!
//! This is the class-side surface of the v3 (`org.openehr.lang.bmm3`)
//! generation: the shared-attribute accessors, the inheritance walks, the
//! class→type generators and the enumeration name map.
//!
//! Spec: `LANG/docs/bmm3/master07-core-classes.adoc` (§Overview, §Simple
//! Classes, §Generic Classes, §Range-Constrained Classes, §Inheritance) plus the
//! class definitions under `LANG/docs/UML/classes/`:
//! `org.openehr.lang.bmm3.bmm_class.adoc` §Attributes + §Functions,
//! `…bmm3.bmm_simple_class.adoc` §Functions (`type`),
//! `…bmm3.bmm_generic_class.adoc` §Functions (`type`,
//! `generic_parameter_conformance_type`), `…bmm3.bmm_enumeration.adoc`
//! §Functions (`name_map`).
//!
//! This is the v3 generation's OWN surface: in v3 a class inherits from
//! **types** ("the `_ancestors_` attribute … contains a list of _types_ rather
//! than classes", `…bmm3.bmm_class.adoc` §Description), where the v2.x
//! generation's `BMM_CLASS.ancestors` is a map of CLASSES
//! (`org.openehr.lang.bmm.bmm_class.adoc` §Attributes). The two surfaces are
//! therefore never shared; the v2.x one is
//! [`crate::v1_1::bmm::core::bmm_class_impl`].
//!
//! NOTE (adjudicated): the walks here read the ancestor graph the way the v3
//! model states it — each `BMM_MODEL_TYPE` ancestor carries its `base_class`
//! whole — so they need no `BMM_MODEL` argument, unlike their v2.x counterparts
//! (whose `immediate_descendants` are names only). Downward navigation
//! (`all_descendants`) is not reimplemented here: v3's
//! `immediate_descendants: List<BMM_CLASS>` (`…bmm3.bmm_class.adoc`
//! §Attributes) is a BMM-mandatory back-reference the emitter cannot own as
//! forward data, so no v3 instance carries one to follow.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use crate::v1_1::bmm3::core::entity::bmm_effective_type::BmmEffectiveType;
use crate::v1_1::bmm3::core::entity::bmm_generic_class::BmmGenericClass;
use crate::v1_1::bmm3::core::entity::bmm_generic_type::BmmGenericType;
use crate::v1_1::bmm3::core::entity::bmm_model_type::BmmModelType;
use crate::v1_1::bmm3::core::entity::bmm_parameter_type::BmmParameterType;
use crate::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
use crate::v1_1::bmm3::core::entity::bmm_simple_type::BmmSimpleType;
use crate::v1_1::bmm3::core::entity::bmm_type_impl::ANY_TYPE_NAME;
use crate::v1_1::bmm3::core::entity::bmm_unitary_type::BmmUnitaryType;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration::BmmEnumeration;
use crate::v1_1::bmm3::core::feature::bmm_feature::BmmFeature;
use crate::v1_1::bmm3::core::feature::bmm_function::BmmFunction;
use crate::v1_1::bmm3::core::feature::bmm_procedure::BmmProcedure;
use crate::v1_1::bmm3::core::feature::bmm_property::BmmProperty;
use crate::v1_1::bmm3::core::feature::bmm_static::BmmStatic;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValue;
use crate::v1_1::bmm3::core::model::bmm_package::BmmPackage;
use crate::v1_1::bmm3::statement::bmm_assertion::BmmAssertion;

/// The `BMM_CLASS` attributes every generated v3 variant carries, projected out
/// of the variant's own struct so the computed features are written once
/// (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes).
struct ClassCommon<'a> {
    /// `BMM_CLASS.name` (from `BMM_MODEL_ELEMENT`).
    name: &'a str,
    /// `BMM_CLASS.ancestors` — immediate inheritance parents, as TYPES.
    ancestors: Option<&'a BTreeMap<String, BmmModelType>>,
    /// `BMM_CLASS.package`.
    package: &'a BmmPackage,
    /// `BMM_CLASS.features` — "Features of this module".
    features: &'a [BmmFeature],
    /// `BMM_CLASS.properties` — the *differential* property set.
    properties: Option<&'a BTreeMap<String, BmmProperty>>,
    /// `BMM_CLASS.static_properties`.
    static_properties: Option<&'a BTreeMap<String, BmmStatic>>,
    /// `BMM_CLASS.functions`.
    functions: Option<&'a BTreeMap<String, BmmFunction>>,
    /// `BMM_CLASS.procedures`.
    procedures: Option<&'a BTreeMap<String, BmmProcedure>>,
    /// `BMM_CLASS.creators`.
    creators: Option<&'a BTreeMap<String, BmmProcedure>>,
    /// `BMM_CLASS.converters`.
    converters: Option<&'a BTreeMap<String, BmmProcedure>>,
    /// `BMM_CLASS.invariants`.
    invariants: &'a [BmmAssertion],
    /// `BMM_CLASS.is_abstract` (`{default = false}`).
    is_abstract: Option<bool>,
    /// `BMM_CLASS.is_primitive` (`{default = false}`).
    is_primitive: Option<bool>,
    /// `BMM_CLASS.source_schema_id`.
    source_schema_id: &'a str,
}

/// Projects one generated v3 `BMM_CLASS`-family struct onto [`ClassCommon`].
macro_rules! class_common {
    ($c:expr) => {
        ClassCommon {
            name: $c.name.as_str(),
            ancestors: $c.ancestors.as_ref(),
            package: &$c.package,
            features: $c.features.as_deref().unwrap_or_default(),
            properties: $c.properties.as_ref(),
            static_properties: $c.static_properties.as_ref(),
            functions: $c.functions.as_ref(),
            procedures: $c.procedures.as_ref(),
            creators: $c.creators.as_ref(),
            converters: $c.converters.as_ref(),
            invariants: $c.invariants.as_deref().unwrap_or_default(),
            is_abstract: $c.is_abstract,
            is_primitive: $c.is_primitive,
            source_schema_id: $c.source_schema_id.as_str(),
        }
    };
}

/// The shared `BMM_CLASS` attributes of an enumeration class.
fn enumeration_common(enumeration: &BmmEnumeration) -> ClassCommon<'_> {
    match enumeration {
        BmmEnumeration::BmmEnumerationInteger(c) => class_common!(c),
        BmmEnumeration::BmmEnumerationString(c) => class_common!(c),
        BmmEnumeration::BmmEnumeration(c) => class_common!(c),
    }
}

/// The shared `BMM_CLASS` attributes of a simple class.
fn simple_class_common(class: &BmmSimpleClass) -> ClassCommon<'_> {
    match class {
        BmmSimpleClass::BmmEnumeration(e) => enumeration_common(e),
        BmmSimpleClass::BmmSimpleClass(c) => class_common!(c),
    }
}

/// The shared `BMM_CLASS` attributes of the class generating a model type — the
/// `_base_class_` every `BMM_MODEL_TYPE` carries
/// (`org.openehr.lang.bmm3.bmm_model_type.adoc` §Attributes).
fn model_type_base_class_common(model_type: &BmmModelType) -> ClassCommon<'_> {
    match model_type {
        BmmModelType::BmmGenericType(generic) => class_common!(generic.base_class),
        BmmModelType::BmmSimpleType(simple) => simple_class_common(&simple.base_class),
    }
}

/// Does the class described by `class` inherit, at any depth, from a class named
/// `name`? The recursive half of [`BmmClass::has_ancestor_class`].
fn walk_has_ancestor(class: &ClassCommon<'_>, name: &str, seen: &mut BTreeSet<String>) -> bool {
    let Some(ancestors) = class.ancestors else {
        return false;
    };
    for ancestor in ancestors.values() {
        let base = model_type_base_class_common(ancestor);
        if base.name.eq_ignore_ascii_case(name) {
            return true;
        }
        if !seen.insert(base.name.to_uppercase()) {
            continue;
        }
        if walk_has_ancestor(&base, name, seen) {
            return true;
        }
    }
    false
}

/// Collects every ancestor class name reachable from `class`. The recursive half
/// of [`BmmClass::all_ancestors`].
fn walk_ancestor_names(
    class: &ClassCommon<'_>,
    out: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    let Some(ancestors) = class.ancestors else {
        return;
    };
    for ancestor in ancestors.values() {
        let base = model_type_base_class_common(ancestor);
        if !seen.insert(base.name.to_uppercase()) {
            continue;
        }
        out.push(base.name.to_owned());
        walk_ancestor_names(&base, out, seen);
    }
}

/// Merges the flattened feature set of `class`: ancestors first, own features
/// last so the nearer definition wins. The recursive half of
/// [`BmmClass::flat_features`].
fn walk_flat_features(
    class: &ClassCommon<'_>,
    by_name: &mut BTreeMap<String, BmmFeature>,
    seen: &mut BTreeSet<String>,
) {
    if let Some(ancestors) = class.ancestors {
        for ancestor in ancestors.values() {
            let base = model_type_base_class_common(ancestor);
            if !seen.insert(base.name.to_uppercase()) {
                continue;
            }
            walk_flat_features(&base, by_name, seen);
        }
    }
    for feature in class.features {
        by_name.insert(feature.name().to_owned(), feature.clone());
    }
}

impl BmmEnumeration {
    /// `BMM_MODEL_ELEMENT.name`: "Name of this model element"
    /// (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BmmEnumerationInteger(e) => e.name.as_str(),
            Self::BmmEnumerationString(e) => e.name.as_str(),
            Self::BmmEnumeration(e) => e.name.as_str(),
        }
    }

    /// `BMM_ENUMERATION.item_names`: "The list of names of the enumeration"
    /// (`org.openehr.lang.bmm3.bmm_enumeration.adoc` §Attributes).
    #[must_use]
    pub fn item_names(&self) -> &[String] {
        match self {
            Self::BmmEnumerationInteger(e) => e.item_names.as_deref().unwrap_or_default(),
            Self::BmmEnumerationString(e) => e.item_names.as_deref().unwrap_or_default(),
            Self::BmmEnumeration(e) => e.item_names.as_deref().unwrap_or_default(),
        }
    }

    /// The enumeration's item values in their serialised (`_value_literal_`)
    /// form, empty when the enumeration states names only.
    ///
    /// Each `item_values` member is a literal-value meta-object whose
    /// `_value_literal_` is "A serial representation of the value"
    /// (`org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes) — which is
    /// exactly the "(stringified)" form [`BmmEnumeration::name_map`] states. The
    /// two degenerate leaves redefine the list's item type
    /// (`…bmm3.bmm_enumeration_integer.adoc`,
    /// `…bmm3.bmm_enumeration_string.adoc` §Attributes), so the projection is
    /// per leaf.
    #[must_use]
    pub fn item_value_literals(&self) -> Vec<&str> {
        match self {
            Self::BmmEnumerationInteger(e) => e
                .item_values
                .iter()
                .flatten()
                .map(|v| v.value_literal.as_str())
                .collect(),
            Self::BmmEnumerationString(e) => e
                .item_values
                .iter()
                .flatten()
                .map(|v| v.value_literal.as_str())
                .collect(),
            Self::BmmEnumeration(e) => e
                .item_values
                .iter()
                .flatten()
                .map(BmmPrimitiveValue::value_literal)
                .collect(),
        }
    }

    /// `BMM_ENUMERATION.name_map`: "Map of `_item_names_` to `_item_values_`
    /// (stringified)" (`org.openehr.lang.bmm3.bmm_enumeration.adoc` §Functions).
    ///
    /// When no values are stated the spec supplies them: "If no values are
    /// supplied, the integer values 0, 1, 2, ... are assumed" (same
    /// §Attributes), so each name maps to its ordinal. A stated value list is
    /// "1:1 with `_item_names_`" (same §Attributes) — a name beyond the value
    /// list is therefore a malformed enumeration, and mapping it to its ordinal
    /// keeps the map total over `_item_names_` rather than silently dropping the
    /// name; the P_BMM pipeline refuses that shape up front
    /// ([`crate::v1_1::bmm_persistence::error::PBmmReadError::EnumerationItemListsNotOneToOne`]).
    #[must_use]
    pub fn name_map(&self) -> BTreeMap<String, String> {
        let values = self.item_value_literals();
        self.item_names()
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let value = values
                    .get(index)
                    .map_or_else(|| index.to_string(), |literal| (*literal).to_owned());
                (name.clone(), value)
            })
            .collect()
    }
}

impl BmmSimpleClass {
    /// `BMM_MODEL_ELEMENT.name`: "Name of this model element"
    /// (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BmmEnumeration(e) => e.name(),
            Self::BmmSimpleClass(c) => c.name.as_str(),
        }
    }

    /// `BMM_SIMPLE_CLASS.type` (effected): "Generate a type object that
    /// represents the type of this class. Can only be an instance of
    /// `BMM_SIMPLE_TYPE` or a descendant"
    /// (`org.openehr.lang.bmm3.bmm_simple_class.adoc` §Functions).
    ///
    /// The generated type carries no `_value_constraint_`: a value-set
    /// constraint belongs to a type's USE, not to the generating class
    /// (`LANG/docs/bmm3/master07-core-classes.adoc` §Range-Constrained Classes —
    /// the value-set mechanism "applied to the use of a type").
    #[must_use]
    pub fn r#type(&self) -> BmmSimpleType {
        BmmSimpleType {
            value_constraint: None,
            base_class: self.clone(),
        }
    }
}

impl BmmGenericClass {
    /// `BMM_GENERIC_CLASS.type` (effected): "Generate a fully open
    /// `BMM_GENERIC_TYPE` instance that corresponds to this class definition"
    /// (`org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions).
    ///
    /// "Fully open" means every generic parameter is the formal parameter itself
    /// (`BMM_PARAMETER_TYPE`), not a substitution — the `Interval<T>` form of
    /// `master06-core-types.adoc` §Generic Type.
    ///
    /// NOTE: parameter order follows the keyed map's sorted keys — the
    /// declaration order `…bmm3.bmm_generic_type.adoc` §Attributes mandates
    /// is not recoverable from a name-keyed map, and for the
    /// single-upper-case-letter names `Inv_generic_name` mandates, sorted
    /// order IS declaration order for the conventional `T`/`U`/`V`.
    /// # Panics
    /// Never in practice: see the `expect` reason below — a `BMM_GENERIC_CLASS`
    /// with no formal generic parameter cannot arise from a valid schema.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "unreachable by the model: this method exists only on BMM_GENERIC_CLASS, whose `generic_parameters` is a mandatory Hash the spec describes as the class's formal generic parameters (`docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm3.bmm_generic_class.adoc` §Attributes) — a generic class with none is not a generic class, so the empty map cannot arise from a valid schema"
    )]
    pub fn r#type(&self) -> BmmGenericType {
        BmmGenericType {
            value_constraint: None,
            base_class: self.clone(),
            generic_parameters: openehr_base::containers::NonEmptyVec::new(
                self.generic_parameters
                    .values()
                    .map(|p| BmmUnitaryType::BmmParameterType(Box::new(p.clone())))
                    .collect(),
            )
            .expect("a BMM_GENERIC_CLASS should declare at least one formal generic parameter"),
        }
    }

    /// The formal generic parameter named `name`, matched case-insensitively
    /// ("it is assumed that case-insensitive matching is used",
    /// `LANG/docs/bmm3/master05-core-model.adoc` §Naming Convention).
    #[must_use]
    pub fn generic_parameter(&self, name: &str) -> Option<&BmmParameterType> {
        self.generic_parameters.get(name).or_else(|| {
            self.generic_parameters
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, parameter)| parameter)
        })
    }

    /// `BMM_GENERIC_CLASS.generic_parameter_conformance_type`: "For a generic
    /// class, type to which generic parameter `a_name` conforms e.g. if this
    /// class is `Interval <T:Comparable>` then the Result will be the single type
    /// `Comparable`. For an unconstrained type `T`, the Result will be `Any`"
    /// (`org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions).
    ///
    /// `None` when this class declares no such parameter — the class definition
    /// states no result for that case, and answering `Any` would claim a
    /// conformance type for a parameter that does not exist. The constraint
    /// itself resolves through the parameter's inheritance precursor
    /// (`…bmm3.bmm_parameter_type.adoc` §Functions
    /// `flattened_conforms_to_type`).
    #[must_use]
    pub fn generic_parameter_conformance_type(&self, name: &str) -> Option<String> {
        self.generic_parameter(name).map(|parameter| {
            parameter
                .flattened_conforms_to_type()
                .map_or_else(|| ANY_TYPE_NAME.to_owned(), BmmEffectiveType::type_name)
        })
    }
}

impl BmmClass {
    /// The `BMM_CLASS` attributes shared by every v3 variant.
    fn common(&self) -> ClassCommon<'_> {
        match self {
            Self::BmmGenericClass(c) => class_common!(c),
            Self::BmmSimpleClass(c) => simple_class_common(c),
        }
    }

    /// `BMM_MODEL_ELEMENT.name`: "Name of this model element"
    /// (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes). Note that
    /// unlike UML this is "just the root name, even if the class is generic"
    /// (`org.openehr.lang.bmm3.bmm_class.adoc` §Description NOTE).
    #[must_use]
    pub fn name(&self) -> &str {
        self.common().name
    }

    /// `BMM_CLASS.ancestors`: "List of immediate inheritance parents", as TYPES
    /// (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes).
    #[must_use]
    pub fn ancestors(&self) -> Option<&BTreeMap<String, BmmModelType>> {
        self.common().ancestors
    }

    /// `BMM_CLASS.package`: "Package this class belongs to" (class doc
    /// §Attributes).
    #[must_use]
    pub fn package(&self) -> &BmmPackage {
        self.common().package
    }

    /// `BMM_CLASS.features` / `features()`: "Features of this module" — the
    /// differential set introduced by this class (class doc §Attributes +
    /// §Functions).
    #[must_use]
    pub fn features(&self) -> &[BmmFeature] {
        self.common().features
    }

    /// `BMM_CLASS.properties`: "Properties defined in this class (subset of
    /// `_features_`)" (class doc §Attributes) — the differential set.
    #[must_use]
    pub fn properties(&self) -> Option<&BTreeMap<String, BmmProperty>> {
        self.common().properties
    }

    /// `BMM_CLASS.static_properties`: "Static properties defined in this class
    /// (subset of `_features_`)" (class doc §Attributes).
    #[must_use]
    pub fn static_properties(&self) -> Option<&BTreeMap<String, BmmStatic>> {
        self.common().static_properties
    }

    /// `BMM_CLASS.functions`: "Functions defined in this class (subset of
    /// `_features_`)" (class doc §Attributes).
    #[must_use]
    pub fn functions(&self) -> Option<&BTreeMap<String, BmmFunction>> {
        self.common().functions
    }

    /// `BMM_CLASS.procedures`: "Procedures defined in this class (subset of
    /// `_features_`)" (class doc §Attributes).
    #[must_use]
    pub fn procedures(&self) -> Option<&BTreeMap<String, BmmProcedure>> {
        self.common().procedures
    }

    /// `BMM_CLASS.creators`: "Subset of `_procedures_` that may be used to
    /// initialise a new instance of an object" (class doc §Attributes).
    #[must_use]
    pub fn creators(&self) -> Option<&BTreeMap<String, BmmProcedure>> {
        self.common().creators
    }

    /// `BMM_CLASS.converters`: "Subset of `_creators_` that create a new
    /// instance from a single argument of another type" (class doc §Attributes).
    #[must_use]
    pub fn converters(&self) -> Option<&BTreeMap<String, BmmProcedure>> {
        self.common().converters
    }

    /// `BMM_CLASS.invariants`: the class's assertions, which are "always in the
    /// form of an `BMM_ASSERTION`" (`LANG/docs/bmm3/master10-expressions.adoc`
    /// §Usage in BMM Models; `org.openehr.lang.bmm3.bmm_class.adoc` §Attributes).
    #[must_use]
    pub fn invariants(&self) -> &[BmmAssertion] {
        self.common().invariants
    }

    /// `BMM_CLASS.source_schema_id`: "Reference to original source schema
    /// defining this class" (class doc §Attributes).
    #[must_use]
    pub fn source_schema_id(&self) -> &str {
        self.common().source_schema_id
    }

    /// `BMM_CLASS.is_abstract`: "True if this class is marked as abstract, i.e.
    /// direct instances cannot be created from its direct type" — `{default =
    /// false}` (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.common().is_abstract.unwrap_or(false)
    }

    /// `BMM_CLASS.is_primitive`: "True if this class represents a type
    /// considered to be primitive in the type system" — `{default = false}`
    /// (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes).
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        self.common().is_primitive.unwrap_or(false)
    }

    /// `BMM_CLASS.type` (abstract): "Generate a type object that represents the
    /// type for which this class is the definer"
    /// (`org.openehr.lang.bmm3.bmm_class.adoc` §Functions), effected per subtype
    /// ([`BmmSimpleClass::type`], [`BmmGenericClass::type`]).
    #[must_use]
    pub fn r#type(&self) -> BmmModelType {
        match self {
            Self::BmmGenericClass(c) => BmmModelType::BmmGenericType(c.r#type()),
            Self::BmmSimpleClass(c) => BmmModelType::BmmSimpleType(c.r#type()),
        }
    }

    /// `BMM_CLASS.has_ancestor_class(a_class_name)` — the test
    /// `master06-core-types.adoc` §Type Conformance calls in its base-class
    /// branch: does this class inherit, at any depth, from a class of that name?
    ///
    /// Names are matched case-insensitively ("`base_class.is_case_insensitive_equal
    /// (anc_base_class)`", same §Type Conformance). The walk carries its own
    /// visited set so a malformed (cyclic) ancestor graph cannot hang it — the
    /// spec states inheritance "results in an acyclic graph"
    /// (`LANG/docs/bmm3/master13-model_semantics.adoc` §Simple Inheritance) but
    /// nothing enforces that on a loaded model.
    #[must_use]
    pub fn has_ancestor_class(&self, name: &str) -> bool {
        walk_has_ancestor(&self.common(), name, &mut BTreeSet::new())
    }

    /// `BMM_CLASS.all_ancestors`: "List of all inheritance parent class names,
    /// recursively" (`org.openehr.lang.bmm3.bmm_class.adoc` §Functions).
    ///
    /// Deduplicated — multiple inheritance means the same ancestor can be
    /// reached by more than one path (`master13-model_semantics.adoc` §Multiple
    /// Inheritance) — and cycle-guarded, as in
    /// [`BmmClass::has_ancestor_class`].
    #[must_use]
    pub fn all_ancestors(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        walk_ancestor_names(&self.common(), &mut out, &mut BTreeSet::new());
        out
    }

    /// `BMM_CLASS.flat_features`: "Consolidated list of all feature definitions
    /// from this class and all inheritance ancestors"
    /// (`org.openehr.lang.bmm3.bmm_class.adoc` §Functions).
    ///
    /// Ancestor features are merged first and this class's own last, so a
    /// redefinition in the nearer class replaces the inherited definition of the
    /// same name — the differential-vs-flat distinction of
    /// `LANG/docs/bmm3/master08-core-features.adoc` §Overview ("Differential vs
    /// flat feature sets"). The result is name-ordered, so it is deterministic.
    ///
    /// NOTE (adjudicated): same-named features arriving from DIFFERENT ancestors
    /// are a clash the spec says needs resolution
    /// (`master07-core-classes.adoc` §Inheritance), and the model records no
    /// resolution a schema chose; the merge order here is a total function over
    /// the graph, not a clash verdict. Reporting clashes would need a
    /// validation-report shape openEHR does not define.
    #[must_use]
    pub fn flat_features(&self) -> Vec<BmmFeature> {
        let mut by_name: BTreeMap<String, BmmFeature> = BTreeMap::new();
        walk_flat_features(&self.common(), &mut by_name, &mut BTreeSet::new());
        by_name.into_values().collect()
    }
}
