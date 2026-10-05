// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of the BMM **v3** `BMM_TYPE` family — the type
//! naming surface of the v3 (`org.openehr.lang.bmm3`) generation.
//!
//! Spec: `LANG/docs/bmm3/master06-core-types.adoc` (the v3 type meta-model) plus
//! the class definitions under `LANG/docs/UML/classes/`:
//! `org.openehr.lang.bmm3.bmm_type.adoc` §Functions (`type_name` — "Formal
//! string form of the type as per UML"; `type_signature` — "Signature form of
//! the type name, which for generics includes generic parameter constrainer
//! types E.g. `Interval<T:Ordered>`. Defaults to the value of `_type_name()_`";
//! `flattened_type_list` — "Completely flattened list of type names, flattening
//! out all generic parameters"), `…bmm3.bmm_simple_type.adoc`,
//! `…bmm3.bmm_generic_type.adoc`, `…bmm3.bmm_container_type.adoc`,
//! `…bmm3.bmm_indexed_container_type.adoc`, `…bmm3.bmm_parameter_type.adoc`,
//! `…bmm3.bmm_signature.adoc`, `…bmm3.bmm_status_type.adoc`,
//! `…bmm3.bmm_tuple_type.adoc`, `…bmm3.bmm_effective_type.adoc`,
//! `…bmm3.bmm_unitary_type.adoc`, all §Functions.
//!
//! This is the v3 generation's OWN surface. The v2.x generation
//! (`LANG/docs/bmm/master01-preface.adoc` §History — "the normative,
//! tool-implemented version") carries its own, structurally different one at
//! [`crate::v1_1::bmm::core::bmm_type_impl`]; the two never share an impl, because
//! `BMM_TYPE`, `BMM_SIMPLE_TYPE`, `BMM_GENERIC_TYPE` and `BMM_CONTAINER_TYPE`
//! are different classes in the two generations
//! (`LANG/docs/bmm3/master00-amendment_record.adoc` SPECLANG-14, "Formalise the
//! BMM v2/v3 split").
//!
//! The naming/flattening surface and the meta-type LATTICE
//! (`is_abstract`, `is_primitive`, `type_base_name`, `unitary_type`,
//! `effective_type`, `effective_base_class`, `is_open`/`is_closed`/
//! `is_partially_closed`) both live here; the class and feature surfaces are the
//! siblings [`crate::v1_1::bmm3::core::entity::bmm_class_impl`] and
//! [`crate::v1_1::bmm3::core::feature::bmm_feature_impl`].
//!
//! The MODEL-level navigation this lattice is the precondition for
//! (`type_conforms_to` / `all_ancestor_classes` / `property_definition`) lives
//! at [`crate::v1_1::bmm3::core::model::bmm_model_impl`].

use crate::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use crate::v1_1::bmm3::core::entity::bmm_container_type::BmmContainerType;
use crate::v1_1::bmm3::core::entity::bmm_effective_type::BmmEffectiveType;
use crate::v1_1::bmm3::core::entity::bmm_function_type::BmmFunctionType;
use crate::v1_1::bmm3::core::entity::bmm_generic_class::BmmGenericClass;
use crate::v1_1::bmm3::core::entity::bmm_generic_type::BmmGenericType;
use crate::v1_1::bmm3::core::entity::bmm_indexed_container_type::BmmIndexedContainerType;
use crate::v1_1::bmm3::core::entity::bmm_model_type::BmmModelType;
use crate::v1_1::bmm3::core::entity::bmm_parameter_type::BmmParameterType;
use crate::v1_1::bmm3::core::entity::bmm_procedure_type::BmmProcedureType;
use crate::v1_1::bmm3::core::entity::bmm_routine_type::BmmRoutineType;
use crate::v1_1::bmm3::core::entity::bmm_routine_type::BmmRoutineTypeData;
use crate::v1_1::bmm3::core::entity::bmm_signature::BmmSignature;
use crate::v1_1::bmm3::core::entity::bmm_signature::BmmSignatureData;
use crate::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
use crate::v1_1::bmm3::core::entity::bmm_simple_type::BmmSimpleType;
use crate::v1_1::bmm3::core::entity::bmm_status_type::BmmStatusType;
use crate::v1_1::bmm3::core::entity::bmm_tuple_type::BmmTupleType;
use crate::v1_1::bmm3::core::entity::bmm_type::BmmType;
use crate::v1_1::bmm3::core::entity::bmm_unitary_type::BmmUnitaryType;

/// The name of the top `Any` type.
///
/// Used wherever an unconstrained generic
/// parameter has to be reduced to a concrete conformance name
/// (`org.openehr.lang.bmm3.bmm_definitions.adoc` §Functions `Any_class`:
/// "built-in class definition corresponding to the top `Any` class").
pub const ANY_TYPE_NAME: &str = "Any";

/// Collapses `names` to its first-occurrence-ordered set — the "logical set
/// (i.e. unique items)" the v3 composite meta-types state for their flattened
/// type lists (`org.openehr.lang.bmm3.bmm_signature.adoc` §Functions).
fn unique(names: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

impl BmmSimpleType {
    /// `BMM_SIMPLE_TYPE.type_name` (effected): "Result is `_base_class.name_`"
    /// (`org.openehr.lang.bmm3.bmm_simple_type.adoc` §Functions).
    #[must_use]
    pub fn type_name(&self) -> String {
        self.base_class.name().to_owned()
    }

    /// `BMM_TYPE.type_signature`: "Defaults to the value of `_type_name()_`" for
    /// a meta-type with no constrainer (`org.openehr.lang.bmm3.bmm_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.type_name()
    }

    /// The reduced conformance form of a simple type: the defining class name
    /// (`master06-core-types.adoc` §Simple Type — a simple type "is just a
    /// reference to a class").
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.base_class.name().to_owned()
    }

    /// `BMM_SIMPLE_TYPE.flattened_type_list` (effected): "Result is
    /// `_base_class.name_`" (`org.openehr.lang.bmm3.bmm_simple_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        vec![self.base_class.name().to_owned()]
    }
}

impl BmmGenericType {
    /// `BMM_GENERIC_TYPE.type_name` (effected): "Return the full name of the
    /// type including generic parameters, e.g. `DV_INTERVAL<T>`,
    /// `TABLE<List<THING>,String>`" (`org.openehr.lang.bmm3.bmm_generic_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn type_name(&self) -> String {
        let root = &self.base_class.name;
        let parameters: Vec<String> = (&self.generic_parameters)
            .into_iter()
            .map(BmmUnitaryType::type_name)
            .collect();
        format!("{root}<{}>", parameters.join(","))
    }

    /// `BMM_GENERIC_TYPE.type_signature` (redefined): "Signature form of the
    /// type, which for generics includes generic parameter constrainer types
    /// E.g. `Interval<T:Ordered>`"
    /// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        let root = &self.base_class.name;
        let parameters: Vec<String> = self
            .generic_parameters
            .iter()
            .map(BmmUnitaryType::type_signature)
            .collect();
        format!("{root}<{}>", parameters.join(","))
    }

    /// The reduced conformance form of a generic type: "the _root_ type from a
    /// generic type (e.g. `Interval` from `Interval<T>`)"
    /// (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics — the v3 chapters
    /// keep the same reduction, `master06-core-types.adoc` §Generic Type).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.base_class.name.clone()
    }

    /// `BMM_GENERIC_TYPE.flattened_type_list` (effected): "Result is
    /// `_base_class.name_` followed by names of all generic parameter type
    /// names, which may be open or closed"
    /// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        let mut out = vec![self.base_class.name.clone()];
        for parameter in &self.generic_parameters {
            out.extend(parameter.flattened_type_list());
        }
        unique(out)
    }
}

impl BmmContainerType {
    /// `BMM_CONTAINER_TYPE.type_name` (effected): "Return full type name, e.g.
    /// `List<ELEMENT>`" (`org.openehr.lang.bmm3.bmm_container_type.adoc`
    /// §Functions); an indexed container renders both parameters, e.g.
    /// `HashMap<String, ELEMENT>`
    /// (`…bmm3.bmm_indexed_container_type.adoc` §Functions).
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmIndexedContainerType(indexed) => indexed.type_name(),
            Self::BmmContainerType(data) => {
                let container = &data.container_class.name;
                let item = data.item_type.type_name();
                format!("{container}<{item}>")
            }
        }
    }

    /// `BMM_TYPE.type_signature`: the container form with each parameter in
    /// signature form (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmIndexedContainerType(indexed) => indexed.type_signature(),
            Self::BmmContainerType(data) => {
                let container = &data.container_class.name;
                let item = data.item_type.type_signature();
                format!("{container}<{item}>")
            }
        }
    }

    /// The reduced conformance form of a container type: "the _contained_ type
    /// for a container type (e.g. `ELEMENT` from the type `List<ELEMENT>`)"
    /// (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics; the v3 container
    /// meta-type keeps `_item_type_` as the contained type,
    /// `org.openehr.lang.bmm3.bmm_container_type.adoc` §Attributes).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmIndexedContainerType(indexed) => indexed.item_type.conformance_type_name(),
            Self::BmmContainerType(data) => data.item_type.conformance_type_name(),
        }
    }

    /// `BMM_CONTAINER_TYPE.flattened_type_list` (effected):
    /// `Post_result: Result = item_type.flattened_type_list`
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Functions) — the v3
    /// reading narrows to the item type, unlike the v2 one which also carries
    /// the container class (see [`crate::v1_1::bmm::core::bmm_type_impl`]).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmIndexedContainerType(indexed) => indexed.flattened_type_list(),
            Self::BmmContainerType(data) => unique(data.item_type.flattened_type_list()),
        }
    }
}

impl BmmIndexedContainerType {
    /// `BMM_INDEXED_CONTAINER_TYPE.type_name` (effected): "Return full type
    /// name, e.g. `HashMap<String, ELEMENT>`"
    /// (`org.openehr.lang.bmm3.bmm_indexed_container_type.adoc` §Functions).
    #[must_use]
    pub fn type_name(&self) -> String {
        let container = &self.container_class.name;
        let index = self.index_type.type_name();
        let item = self.item_type.type_name();
        format!("{container}<{index},{item}>")
    }

    /// `BMM_TYPE.type_signature` for an indexed container: the same form with
    /// each parameter in signature form
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        let container = &self.container_class.name;
        let index = self.index_type.type_signature();
        let item = self.item_type.type_signature();
        format!("{container}<{index},{item}>")
    }

    /// `BMM_CONTAINER_TYPE.flattened_type_list` for an indexed container: the
    /// item type's flattened list plus the index type, which is itself a
    /// declared `BMM_SIMPLE_TYPE` of the model
    /// (`org.openehr.lang.bmm3.bmm_indexed_container_type.adoc` §Attributes:
    /// `index_type: BMM_SIMPLE_TYPE`).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        let mut out = self.index_type.flattened_type_list();
        out.extend(self.item_type.flattened_type_list());
        unique(out)
    }
}

impl BmmModelType {
    /// `BMM_TYPE.type_name` for a model type (a type defined by a class of the
    /// model — `org.openehr.lang.bmm3.bmm_model_type.adoc` §Description).
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.type_name(),
            Self::BmmSimpleType(simple) => simple.type_name(),
        }
    }

    /// `BMM_TYPE.type_signature` for a model type.
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.type_signature(),
            Self::BmmSimpleType(simple) => simple.type_signature(),
        }
    }

    /// The reduced conformance form of a model type.
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.conformance_type_name(),
        }
    }

    /// `BMM_TYPE.flattened_type_list` for a model type.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmGenericType(generic) => generic.flattened_type_list(),
            Self::BmmSimpleType(simple) => simple.flattened_type_list(),
        }
    }
}

impl BmmParameterType {
    /// `BMM_PARAMETER_TYPE.type_name` (effected): "Return `_name_`"
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions) — the open
    /// type name `T`, `U` etc. of `master05-core.adoc` §Basics.
    #[must_use]
    pub fn type_name(&self) -> &str {
        self.name.as_str()
    }

    /// `BMM_PARAMETER_TYPE.type_signature` (redefined): "Signature form of the
    /// open type, including constrainer type if there is one, e.g. `T:Ordered`"
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    ///
    /// Delimiter per `org.openehr.lang.bmm3.bmm_definitions.adoc` §Constants
    /// (`Generic_constraint_delimiter` `':'`).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self.flattened_conforms_to_type() {
            Some(constraint) => {
                let name = &self.name;
                let constraint = constraint.type_name();
                format!("{name}:{constraint}")
            }
            None => self.name.clone(),
        }
    }

    /// `BMM_PARAMETER_TYPE.flattened_conforms_to_type`: "Result is either
    /// `_conforms_to_type_` or `_inheritance_precursor.flattened_conforms_to_type_`"
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_conforms_to_type(&self) -> Option<&BmmEffectiveType> {
        match &self.type_constraint {
            Some(constraint) => Some(constraint),
            None => self
                .inheritance_precursor
                .as_ref()
                .and_then(|precursor| precursor.flattened_conforms_to_type()),
        }
    }

    /// `BMM_PARAMETER_TYPE.effective_type` (effected): "Generate ultimate
    /// conformance type, which is either `_flattened_conforms_to_type_` or if
    /// not set, `'Any'`" (`org.openehr.lang.bmm3.bmm_parameter_type.adoc`
    /// §Functions), projected to the type NAME — see the projection NOTE on
    /// [`crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter::effective_conforms_to_type_name`].
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self.flattened_conforms_to_type() {
            Some(constraint) => constraint.conformance_type_name(),
            None => ANY_TYPE_NAME.to_owned(),
        }
    }

    /// `BMM_PARAMETER_TYPE.flattened_type_list` (effected): "Result is either
    /// `_flattened_conforms_to_type.flattened_type_list_` or the `Any` type"
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self.flattened_conforms_to_type() {
            Some(constraint) => constraint.flattened_type_list(),
            None => vec![ANY_TYPE_NAME.to_owned()],
        }
    }
}

impl BmmStatusType {
    /// `BMM_STATUS_TYPE.type_name`: the built-in `base_name` `"Status"`
    /// (`org.openehr.lang.bmm3.bmm_status_type.adoc` §Constants).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare this as an instance function on the meta-type (BMM_CLASSIFIER §Functions); a built-in meta-type's name is simply constant"
    )]
    #[must_use]
    pub fn type_name(&self) -> String {
        Self::BASE_NAME.to_owned()
    }

    /// `conformance_type_name` for the status meta-type: its
    /// built-in base name (`org.openehr.lang.bmm3.bmm_status_type.adoc`
    /// §Constants).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare this as an instance function on the meta-type (BMM_CLASSIFIER §Functions); a built-in meta-type's name is simply constant"
    )]
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        Self::BASE_NAME.to_owned()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for the status meta-type: its
    /// built-in base name (`org.openehr.lang.bmm3.bmm_status_type.adoc`
    /// §Constants).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare this as an instance function on the meta-type (BMM_CLASSIFIER §Functions); a built-in meta-type's name is simply constant"
    )]
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        vec![Self::BASE_NAME.to_owned()]
    }
}

impl BmmTupleType {
    /// `BMM_TUPLE_TYPE.type_name`: the built-in `base_name` `"Tuple"`
    /// (`org.openehr.lang.bmm3.bmm_tuple_type.adoc` §Constants).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare this as an instance function on the meta-type (BMM_CLASSIFIER §Functions); a built-in meta-type's name is simply constant"
    )]
    #[must_use]
    pub fn type_name(&self) -> String {
        Self::BASE_NAME.to_owned()
    }

    /// `conformance_type_name` for the tuple meta-type: its
    /// built-in base name (`org.openehr.lang.bmm3.bmm_tuple_type.adoc`
    /// §Constants).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare this as an instance function on the meta-type (BMM_CLASSIFIER §Functions); a built-in meta-type's name is simply constant"
    )]
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        Self::BASE_NAME.to_owned()
    }

    /// `BMM_TUPLE_TYPE.flattened_type_list` (effected): "Return the logical set
    /// (i.e. unique types) from the merge of `_flattened_type_list_()` called on
    /// each member of `_item_types_`"
    /// (`org.openehr.lang.bmm3.bmm_tuple_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        let mut out = Vec::new();
        for item in self.item_types.values() {
            out.extend(item.flattened_type_list());
        }
        unique(out)
    }
}

impl BmmSignature {
    /// `BMM_SIGNATURE.type_name`: the built-in `base_name` of the effective
    /// signature meta-type — `"Signature"`, `"Routine"`, `"Function"` or
    /// `"Procedure"` (`org.openehr.lang.bmm3.bmm_signature.adoc`,
    /// `…bmm3.bmm_routine_type.adoc`, `…bmm3.bmm_function_type.adoc`,
    /// `…bmm3.bmm_procedure_type.adoc`, all §Constants).
    ///
    /// `BMM_PROPERTY_TYPE` declares no `base_name` of its own
    /// (`org.openehr.lang.bmm3.bmm_property_type.adoc`), so it takes the
    /// inherited `BMM_SIGNATURE` one.
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmPropertyType(_) | Self::BmmSignature(_) => {
                BmmSignatureData::BASE_NAME.to_owned()
            }
            Self::BmmRoutineType(routine) => match routine.as_ref() {
                BmmRoutineType::BmmFunctionType(_) => BmmFunctionType::BASE_NAME.to_owned(),
                BmmRoutineType::BmmProcedureType(_) => BmmProcedureType::BASE_NAME.to_owned(),
                BmmRoutineType::BmmRoutineType(_) => BmmRoutineTypeData::BASE_NAME.to_owned(),
            },
        }
    }

    /// `conformance_type_name` for a signature meta-type: its
    /// built-in base name — a signature is not a model class, so there is no
    /// reduced form beyond the name itself (`master05-core.adoc` §Basics).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.type_name()
    }

    /// `BMM_SIGNATURE.flattened_type_list` (effected): "Return the logical set
    /// (i.e. unique items) consisting of `_argument_types.flattened_type_list_()`
    /// and `_result_type.flattened_type_list_()`"
    /// (`org.openehr.lang.bmm3.bmm_signature.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self {
            Self::BmmPropertyType(property) => {
                out.extend(property.result_type.flattened_type_list());
            }
            Self::BmmSignature(data) => out.extend(data.result_type.flattened_type_list()),
            Self::BmmRoutineType(routine) => match routine.as_ref() {
                BmmRoutineType::BmmFunctionType(function) => {
                    out.extend(function.result_type.flattened_type_list());
                    if let Some(arguments) = &function.argument_types {
                        out.extend(arguments.flattened_type_list());
                    }
                }
                BmmRoutineType::BmmProcedureType(procedure) => {
                    if let Some(result) = &procedure.result_type {
                        out.extend(result.flattened_type_list());
                    }
                    if let Some(arguments) = &procedure.argument_types {
                        out.extend(arguments.flattened_type_list());
                    }
                }
                BmmRoutineType::BmmRoutineType(data) => {
                    out.extend(data.result_type.flattened_type_list());
                    if let Some(arguments) = &data.argument_types {
                        out.extend(arguments.flattened_type_list());
                    }
                }
            },
        }
        unique(out)
    }
}

impl BmmEffectiveType {
    /// `BMM_CLASSIFIER.type_name` for a concrete unitary type usable as an
    /// actual generic parameter (`org.openehr.lang.bmm3.bmm_effective_type.adoc`
    /// §Description).
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.type_name(),
            Self::BmmSignature(signature) => signature.type_name(),
            Self::BmmSimpleType(simple) => simple.type_name(),
            Self::BmmStatusType(status) => status.type_name(),
            Self::BmmTupleType(tuple) => tuple.type_name(),
        }
    }

    /// `BMM_TYPE.type_signature` for an effective type
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.type_signature(),
            Self::BmmSignature(signature) => signature.type_name(),
            Self::BmmSimpleType(simple) => simple.type_signature(),
            Self::BmmStatusType(status) => status.type_name(),
            Self::BmmTupleType(tuple) => tuple.type_name(),
        }
    }

    /// `conformance_type_name` for an effective type.
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.conformance_type_name(),
            Self::BmmSignature(signature) => signature.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.conformance_type_name(),
            Self::BmmStatusType(status) => status.conformance_type_name(),
            Self::BmmTupleType(tuple) => tuple.conformance_type_name(),
        }
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for an effective type.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmGenericType(generic) => generic.flattened_type_list(),
            Self::BmmSignature(signature) => signature.flattened_type_list(),
            Self::BmmSimpleType(simple) => simple.flattened_type_list(),
            Self::BmmStatusType(status) => status.flattened_type_list(),
            Self::BmmTupleType(tuple) => tuple.flattened_type_list(),
        }
    }
}

impl BmmUnitaryType {
    /// `BMM_CLASSIFIER.type_name` for a unitary type, i.e. "the type of any
    /// instantiated object that is not a container object"
    /// (`org.openehr.lang.bmm3.bmm_unitary_type.adoc` §Description).
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.type_name(),
            Self::BmmParameterType(parameter) => parameter.type_name().to_owned(),
            Self::BmmSignature(signature) => signature.type_name(),
            Self::BmmSimpleType(simple) => simple.type_name(),
            Self::BmmStatusType(status) => status.type_name(),
            Self::BmmTupleType(tuple) => tuple.type_name(),
        }
    }

    /// `BMM_TYPE.type_signature` for a unitary type
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.type_signature(),
            Self::BmmParameterType(parameter) => parameter.type_signature(),
            Self::BmmSignature(signature) => signature.type_name(),
            Self::BmmSimpleType(simple) => simple.type_signature(),
            Self::BmmStatusType(status) => status.type_name(),
            Self::BmmTupleType(tuple) => tuple.type_name(),
        }
    }

    /// `conformance_type_name` for a unitary type.
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmGenericType(generic) => generic.conformance_type_name(),
            Self::BmmParameterType(parameter) => parameter.conformance_type_name(),
            Self::BmmSignature(signature) => signature.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.conformance_type_name(),
            Self::BmmStatusType(status) => status.conformance_type_name(),
            Self::BmmTupleType(tuple) => tuple.conformance_type_name(),
        }
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for a unitary type.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmGenericType(generic) => generic.flattened_type_list(),
            Self::BmmParameterType(parameter) => parameter.flattened_type_list(),
            Self::BmmSignature(signature) => signature.flattened_type_list(),
            Self::BmmSimpleType(simple) => simple.flattened_type_list(),
            Self::BmmStatusType(status) => status.flattened_type_list(),
            Self::BmmTupleType(tuple) => tuple.flattened_type_list(),
        }
    }
}
impl BmmType {
    /// `BMM_TYPE.type_name`: "Formal string form of the type as per UML"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions), dispatched to the
    /// effective meta-type.
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmContainerType(container) => container.type_name(),
            Self::BmmGenericType(generic) => generic.type_name(),
            Self::BmmParameterType(parameter) => parameter.type_name().to_owned(),
            Self::BmmSignature(signature) => signature.type_name(),
            Self::BmmSimpleType(simple) => simple.type_name(),
            Self::BmmStatusType(status) => status.type_name(),
            Self::BmmTupleType(tuple) => tuple.type_name(),
        }
    }

    /// `BMM_TYPE.type_signature`: "Signature form of the type name, which for
    /// generics includes generic parameter constrainer types E.g.
    /// `Interval<T:Ordered>`. Defaults to the value of `_type_name()_`"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmContainerType(container) => container.type_signature(),
            Self::BmmGenericType(generic) => generic.type_signature(),
            Self::BmmParameterType(parameter) => parameter.type_signature(),
            Self::BmmSignature(signature) => signature.type_name(),
            Self::BmmSimpleType(simple) => simple.type_signature(),
            Self::BmmStatusType(status) => status.type_name(),
            Self::BmmTupleType(tuple) => tuple.type_name(),
        }
    }

    /// The reduced conformance form of a v3 type
    /// (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics, whose reduction
    /// the v3 chapters keep — `master06-core-types.adoc` §Type Conformance).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmContainerType(container) => container.conformance_type_name(),
            Self::BmmGenericType(generic) => generic.conformance_type_name(),
            Self::BmmParameterType(parameter) => parameter.conformance_type_name(),
            Self::BmmSignature(signature) => signature.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.conformance_type_name(),
            Self::BmmStatusType(status) => status.conformance_type_name(),
            Self::BmmTupleType(tuple) => tuple.conformance_type_name(),
        }
    }

    /// `BMM_TYPE.flattened_type_list`: "Completely flattened list of type names,
    /// flattening out all generic parameters"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmContainerType(container) => container.flattened_type_list(),
            Self::BmmGenericType(generic) => generic.flattened_type_list(),
            Self::BmmParameterType(parameter) => parameter.flattened_type_list(),
            Self::BmmSignature(signature) => signature.flattened_type_list(),
            Self::BmmSimpleType(simple) => simple.flattened_type_list(),
            Self::BmmStatusType(status) => status.flattened_type_list(),
            Self::BmmTupleType(tuple) => tuple.flattened_type_list(),
        }
    }
}

// ── the meta-type lattice (master06-core-types.adoc §Overview) ───────────────
// `BMM_TYPE` splits into the abstract `BMM_UNITARY_TYPE` and the concrete
// container meta-types; unitary splits into `BMM_PARAMETER_TYPE` and
// `BMM_EFFECTIVE_TYPE`; effective splits into `BMM_MODEL_TYPE`,
// `BMM_TUPLE_TYPE` and `BMM_SIGNATURE` (L39-45). Rust models each level as its
// own closed enum, so moving UP a level is a total conversion — these `From`
// impls are that lattice, and they are what makes `unitary_type()` /
// `effective_type()` typeable.

impl From<BmmUnitaryType> for BmmType {
    fn from(unitary: BmmUnitaryType) -> Self {
        match unitary {
            BmmUnitaryType::BmmGenericType(generic) => Self::BmmGenericType(generic),
            BmmUnitaryType::BmmParameterType(parameter) => Self::BmmParameterType(parameter),
            BmmUnitaryType::BmmSignature(signature) => Self::BmmSignature(signature),
            BmmUnitaryType::BmmSimpleType(simple) => Self::BmmSimpleType(simple),
            BmmUnitaryType::BmmStatusType(status) => Self::BmmStatusType(status),
            BmmUnitaryType::BmmTupleType(tuple) => Self::BmmTupleType(tuple),
        }
    }
}

impl From<BmmEffectiveType> for BmmUnitaryType {
    fn from(effective: BmmEffectiveType) -> Self {
        match effective {
            BmmEffectiveType::BmmGenericType(generic) => Self::BmmGenericType(generic),
            BmmEffectiveType::BmmSignature(signature) => Self::BmmSignature(signature),
            BmmEffectiveType::BmmSimpleType(simple) => Self::BmmSimpleType(simple),
            BmmEffectiveType::BmmStatusType(status) => Self::BmmStatusType(status),
            BmmEffectiveType::BmmTupleType(tuple) => Self::BmmTupleType(tuple),
        }
    }
}

impl From<BmmEffectiveType> for BmmType {
    fn from(effective: BmmEffectiveType) -> Self {
        Self::from(BmmUnitaryType::from(effective))
    }
}

impl From<BmmModelType> for BmmEffectiveType {
    fn from(model_type: BmmModelType) -> Self {
        match model_type {
            BmmModelType::BmmGenericType(generic) => Self::BmmGenericType(generic),
            BmmModelType::BmmSimpleType(simple) => Self::BmmSimpleType(simple),
        }
    }
}

impl From<BmmModelType> for BmmUnitaryType {
    fn from(model_type: BmmModelType) -> Self {
        Self::from(BmmEffectiveType::from(model_type))
    }
}

impl From<BmmModelType> for BmmType {
    fn from(model_type: BmmModelType) -> Self {
        Self::from(BmmEffectiveType::from(model_type))
    }
}

impl BmmSimpleType {
    /// `BMM_SIMPLE_TYPE.is_abstract` (effected): "Result is
    /// `_base_class.is_abstract_`" (`org.openehr.lang.bmm3.bmm_simple_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        BmmClass::BmmSimpleClass(self.base_class.clone()).is_abstract()
    }

    /// `BMM_MODEL_TYPE.is_primitive` (effected): "Result =
    /// `_base_class.is_primitive_`" (`org.openehr.lang.bmm3.bmm_model_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        BmmClass::BmmSimpleClass(self.base_class.clone()).is_primitive()
    }

    /// `BMM_MODEL_TYPE.type_base_name` (effected): "Result = `_base_class.name_`"
    /// (`org.openehr.lang.bmm3.bmm_model_type.adoc` §Functions).
    #[must_use]
    pub fn type_base_name(&self) -> &str {
        self.base_class.name()
    }

    /// `BMM_SIMPLE_TYPE.effective_base_class`: "Main design class for this type,
    /// from which properties etc can be extracted"
    /// (`org.openehr.lang.bmm3.bmm_simple_type.adoc` §Functions).
    ///
    /// A simple type is 1:1 with its generating class
    /// (`…bmm3.bmm_simple_class.adoc` §Description), so the effective base class
    /// IS `_base_class_`; the function exists because the generic form abstracts
    /// a container away ([`BmmGenericType::effective_base_class`]).
    #[must_use]
    pub fn effective_base_class(&self) -> &BmmSimpleClass {
        &self.base_class
    }
}

impl BmmGenericType {
    /// `BMM_GENERIC_TYPE.is_abstract` (effected): "True if
    /// `_base_class.is_abstract_` or if any (non-open) parameter type is
    /// abstract" (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    ///
    /// "Non-open" excludes a formal parameter (`BMM_PARAMETER_TYPE`), which the
    /// class definition declares non-abstract by definition
    /// (`…bmm3.bmm_parameter_type.adoc` §Functions `is_abstract`: "Result =
    /// `False`").
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        BmmClass::BmmGenericClass(self.base_class.clone()).is_abstract()
            || self
                .generic_parameters
                .iter()
                .any(BmmUnitaryType::is_abstract)
    }

    /// `BMM_MODEL_TYPE.is_primitive` (effected): "Result =
    /// `_base_class.is_primitive_`" (`org.openehr.lang.bmm3.bmm_model_type.adoc`
    /// §Functions) — the generating class's own designation, not the parameters'.
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        BmmClass::BmmGenericClass(self.base_class.clone()).is_primitive()
    }

    /// `BMM_MODEL_TYPE.type_base_name` (effected): "Result = `_base_class.name_`"
    /// — "Name of base generator type, i.e. excluding any generic parts if
    /// present" (`org.openehr.lang.bmm3.bmm_model_type.adoc` §Functions,
    /// `…bmm3.bmm_effective_type.adoc` §Functions).
    #[must_use]
    pub fn type_base_name(&self) -> &str {
        self.base_class.name.as_str()
    }

    /// `BMM_GENERIC_TYPE.effective_base_class`: "Effective underlying class for
    /// this type, abstracting away any container type"
    /// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    #[must_use]
    pub fn effective_base_class(&self) -> &BmmGenericClass {
        &self.base_class
    }

    /// `BMM_GENERIC_TYPE.is_open`: "True if all generic parameters from ancestor
    /// generic types have been substituted in this type"
    /// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    ///
    /// NOTE: the function is NAMED `is_open` while its stated semantics are
    /// CLOSURE (the chapter calls the same property `is_closed` —
    /// `master06-core-types.adoc` §Generic Type L81 — a function no class
    /// declares); this follows the class definition's semantics under its own
    /// name, with [`BmmGenericType::is_closed`] as the same predicate under
    /// the chapter's name.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self
            .generic_parameters
            .iter()
            .any(|p| matches!(p, BmmUnitaryType::BmmParameterType(_)))
    }

    /// `master06-core-types.adoc` §Generic Type L81's name for the fully
    /// substituted state ("closure is detected via the function `_is_closed_`") —
    /// identical to [`BmmGenericType::is_open`], see its NOTE for the upstream
    /// naming slip.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.is_open()
    }

    /// `BMM_GENERIC_TYPE.is_partially_closed`: "Returns True if there is any
    /// substituted generic parameter"
    /// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    #[must_use]
    pub fn is_partially_closed(&self) -> bool {
        self.generic_parameters
            .iter()
            .any(|p| !matches!(p, BmmUnitaryType::BmmParameterType(_)))
    }
}

impl BmmModelType {
    /// `BMM_MODEL_TYPE.type_base_name` (effected): "Result = `_base_class.name_`"
    /// (`org.openehr.lang.bmm3.bmm_model_type.adoc` §Functions).
    #[must_use]
    pub fn type_base_name(&self) -> &str {
        match self {
            Self::BmmGenericType(generic) => generic.type_base_name(),
            Self::BmmSimpleType(simple) => simple.type_base_name(),
        }
    }

    /// `BMM_TYPE.is_abstract`: "indicates a type based on an abstract class,
    /// i.e. a type that cannot be directly instantiated"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        match self {
            Self::BmmGenericType(generic) => generic.is_abstract(),
            Self::BmmSimpleType(simple) => simple.is_abstract(),
        }
    }

    /// `BMM_MODEL_TYPE.is_primitive` (effected): "Result =
    /// `_base_class.is_primitive_`" (`org.openehr.lang.bmm3.bmm_model_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        match self {
            Self::BmmGenericType(generic) => generic.is_primitive(),
            Self::BmmSimpleType(simple) => simple.is_primitive(),
        }
    }

    /// The class that generates this type — the `_base_class_` every model type
    /// carries (`org.openehr.lang.bmm3.bmm_model_type.adoc` §Attributes).
    #[must_use]
    pub fn base_class(&self) -> BmmClass {
        match self {
            Self::BmmGenericType(generic) => BmmClass::BmmGenericClass(generic.base_class.clone()),
            Self::BmmSimpleType(simple) => BmmClass::BmmSimpleClass(simple.base_class.clone()),
        }
    }
}

impl BmmParameterType {
    /// `BMM_PARAMETER_TYPE.is_abstract` (effected): "Result = `False` - generic
    /// parameters are understood by definition to be non-abstract"
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant False"
    )]
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        false
    }

    /// `BMM_PARAMETER_TYPE.is_primitive` (effected): "Result = `False` - generic
    /// parameters are understood by definition to be non-primitive"
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    ///
    /// NOTE (upstream contradiction, adjudicated to the stated result): the same
    /// entry also carries `Post_validity: Result = base_class.is_primitive`, a
    /// post-condition naming an attribute `BMM_PARAMETER_TYPE` does not have
    /// (only `BMM_MODEL_TYPE` has `base_class`) — a copy-paste from that class.
    /// The prose result ("`False`") is the only consistent reading and is the one
    /// implemented.
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant False"
    )]
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        false
    }

    /// `BMM_PARAMETER_TYPE.effective_type` (effected): "Generate ultimate
    /// conformance type, which is either `_flattened_conforms_to_type_` or if not
    /// set, `'Any'`" (`org.openehr.lang.bmm3.bmm_parameter_type.adoc`
    /// §Functions).
    ///
    /// `None` is the `'Any'` case: `Any` is the model's top class
    /// (`…bmm3.bmm_definitions.adoc` §Functions `Any_class`), so returning it as
    /// a `BMM_EFFECTIVE_TYPE` would mean inventing a `BMM_SIMPLE_TYPE` over a
    /// class definition this parameter does not carry. The name is available as
    /// [`ANY_TYPE_NAME`], and [`BmmParameterType::conformance_type_name`] is the
    /// name-level answer that already applies it.
    #[must_use]
    pub fn effective_type(&self) -> Option<BmmEffectiveType> {
        self.flattened_conforms_to_type().cloned()
    }
}

impl BmmSignature {
    /// `BMM_BUILTIN_TYPE.is_abstract` (effected): "Return False" — built-in types
    /// "are treated as being primitive and non-abstract"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions + §Description).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant False"
    )]
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        false
    }

    /// `BMM_BUILTIN_TYPE.is_primitive` (effected): "Return True"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant True"
    )]
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        true
    }

    /// `BMM_BUILTIN_TYPE.type_base_name` (effected): "Return `_base_name_`"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions) — the
    /// `"Signature"` / `"Routine"` / `"Function"` / `"Procedure"` constants of
    /// the signature family.
    #[must_use]
    pub fn type_base_name(&self) -> &'static str {
        match self {
            // `BMM_PROPERTY_TYPE` declares no `base_name` of its own
            // (`org.openehr.lang.bmm3.bmm_property_type.adoc`), so it inherits
            // `BMM_SIGNATURE`'s — the same answer as the base form.
            Self::BmmPropertyType(_) | Self::BmmSignature(_) => BmmSignatureData::BASE_NAME,
            Self::BmmRoutineType(routine) => match routine.as_ref() {
                BmmRoutineType::BmmFunctionType(_) => BmmFunctionType::BASE_NAME,
                BmmRoutineType::BmmProcedureType(_) => BmmProcedureType::BASE_NAME,
                BmmRoutineType::BmmRoutineType(_) => BmmRoutineTypeData::BASE_NAME,
            },
        }
    }
}

impl BmmStatusType {
    /// `BMM_BUILTIN_TYPE.is_abstract` (effected): "Return False"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant False"
    )]
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        false
    }

    /// `BMM_BUILTIN_TYPE.is_primitive` (effected): "Return True"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant True"
    )]
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        true
    }

    /// `BMM_BUILTIN_TYPE.type_base_name` (effected): "Return `_base_name_`"
    /// (`org.openehr.lang.bmm3.bmm_status_type.adoc` §Constants: `"Status"`).
    #[expect(
        clippy::unused_self,
        reason = "a built-in meta-type's base name is its declared constant"
    )]
    #[must_use]
    pub fn type_base_name(&self) -> &'static str {
        Self::BASE_NAME
    }
}

impl BmmTupleType {
    /// `BMM_BUILTIN_TYPE.is_abstract` (effected): "Return False"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant False"
    )]
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        false
    }

    /// `BMM_BUILTIN_TYPE.is_primitive` (effected): "Return True"
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Functions).
    #[expect(
        clippy::unused_self,
        reason = "the class definition declares this as an instance function whose result is the constant True"
    )]
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        true
    }

    /// `BMM_BUILTIN_TYPE.type_base_name` (effected): "Return `_base_name_`"
    /// (`org.openehr.lang.bmm3.bmm_tuple_type.adoc` §Constants: `"Tuple"`).
    #[expect(
        clippy::unused_self,
        reason = "a built-in meta-type's base name is its declared constant"
    )]
    #[must_use]
    pub fn type_base_name(&self) -> &'static str {
        Self::BASE_NAME
    }
}

impl BmmContainerType {
    /// `BMM_CONTAINER_TYPE.is_abstract` (effected): "True if the container class
    /// is abstract", `Post_is_abstract: Result = container_type.is_abstract`
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Functions — the
    /// post-condition's `container_type` is the attribute now spelled
    /// `_container_class_`, same §Attributes).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        BmmClass::BmmGenericClass(self.container_class().clone()).is_abstract()
    }

    /// `BMM_CONTAINER_TYPE.is_primitive` (effected): "True if `_item_type_` is
    /// primitive", `Post_result: Result = item_type.is_primitive`
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Functions).
    ///
    /// NOTE (upstream contradiction, adjudicated to `Post_result`): the same
    /// entry also carries `Post_validity: Result = base_class.is_primitive`,
    /// naming an attribute `BMM_CONTAINER_TYPE` does not have (it has
    /// `_container_class_` and `_item_type_`; `base_class` belongs to
    /// `BMM_MODEL_TYPE`). `Post_result` agrees with the prose description, so it
    /// is the implemented reading.
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        self.item_type().is_primitive()
    }

    /// `BMM_CONTAINER_TYPE.container_class`: "The type of the container"
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Attributes), for either
    /// container form.
    #[must_use]
    pub fn container_class(&self) -> &BmmGenericClass {
        match self {
            Self::BmmIndexedContainerType(indexed) => &indexed.container_class,
            Self::BmmContainerType(data) => &data.container_class,
        }
    }

    /// `BMM_CONTAINER_TYPE.item_type`: "The container item type"
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Attributes), for either
    /// container form.
    #[must_use]
    pub fn item_type(&self) -> &BmmUnitaryType {
        match self {
            Self::BmmIndexedContainerType(indexed) => &indexed.item_type,
            Self::BmmContainerType(data) => data.item_type.as_ref(),
        }
    }

    /// `BMM_CONTAINER_TYPE.unitary_type` (effected): "Return `_item_type_`"
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Functions) — the type
    /// "with any container abstracted away"
    /// (`…bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn unitary_type(&self) -> BmmUnitaryType {
        self.item_type().clone()
    }

    /// `BMM_CONTAINER_TYPE.effective_type` (effected): "Return
    /// `_item_type.effective_type_()`"
    /// (`org.openehr.lang.bmm3.bmm_container_type.adoc` §Functions).
    ///
    /// `None` carries the `'Any'` case through from
    /// [`BmmParameterType::effective_type`] — a container of an unconstrained
    /// formal parameter has no effective type object.
    #[must_use]
    pub fn effective_type(&self) -> Option<BmmEffectiveType> {
        self.item_type().effective_type()
    }
}

impl BmmUnitaryType {
    /// `BMM_TYPE.is_abstract` for a unitary type
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        match self {
            Self::BmmGenericType(generic) => generic.is_abstract(),
            Self::BmmParameterType(parameter) => parameter.is_abstract(),
            Self::BmmSignature(signature) => signature.is_abstract(),
            Self::BmmSimpleType(simple) => simple.is_abstract(),
            Self::BmmStatusType(status) => status.is_abstract(),
            Self::BmmTupleType(tuple) => tuple.is_abstract(),
        }
    }

    /// `BMM_TYPE.is_primitive` for a unitary type: "If True, indicates that a
    /// type based solely on primitive classes"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        match self {
            Self::BmmGenericType(generic) => generic.is_primitive(),
            Self::BmmParameterType(parameter) => parameter.is_primitive(),
            Self::BmmSignature(signature) => signature.is_primitive(),
            Self::BmmSimpleType(simple) => simple.is_primitive(),
            Self::BmmStatusType(status) => status.is_primitive(),
            Self::BmmTupleType(tuple) => tuple.is_primitive(),
        }
    }

    /// `BMM_UNITARY_TYPE.unitary_type` (effected): "Result = self"
    /// (`org.openehr.lang.bmm3.bmm_unitary_type.adoc` §Functions).
    #[must_use]
    pub fn unitary_type(&self) -> Self {
        self.clone()
    }

    /// `BMM_TYPE.effective_type`: "Type with any container abstracted away, and
    /// any formal parameter replaced by its effective constraint type"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions). For an effective type
    /// this is the identity ("Result = self",
    /// `…bmm3.bmm_effective_type.adoc` §Functions); for a formal parameter it is
    /// the parameter's constraint ([`BmmParameterType::effective_type`], whose
    /// `None` = the `'Any'` top).
    #[must_use]
    pub fn effective_type(&self) -> Option<BmmEffectiveType> {
        match self {
            Self::BmmGenericType(generic) => {
                Some(BmmEffectiveType::BmmGenericType(generic.clone()))
            }
            Self::BmmParameterType(parameter) => parameter.effective_type(),
            Self::BmmSignature(signature) => {
                Some(BmmEffectiveType::BmmSignature(signature.clone()))
            }
            Self::BmmSimpleType(simple) => Some(BmmEffectiveType::BmmSimpleType(simple.clone())),
            Self::BmmStatusType(status) => Some(BmmEffectiveType::BmmStatusType(status.clone())),
            Self::BmmTupleType(tuple) => Some(BmmEffectiveType::BmmTupleType(tuple.clone())),
        }
    }
}

impl BmmEffectiveType {
    /// `BMM_EFFECTIVE_TYPE.type_base_name` (abstract): "Name of base generator
    /// type, i.e. excluding any generic parts if present"
    /// (`org.openehr.lang.bmm3.bmm_effective_type.adoc` §Functions), effected by
    /// `BMM_MODEL_TYPE` ("`_base_class.name_`") and `BMM_BUILTIN_TYPE`
    /// ("`_base_name_`").
    #[must_use]
    pub fn type_base_name(&self) -> &str {
        match self {
            Self::BmmGenericType(generic) => generic.type_base_name(),
            Self::BmmSignature(signature) => signature.type_base_name(),
            Self::BmmSimpleType(simple) => simple.type_base_name(),
            Self::BmmStatusType(status) => status.type_base_name(),
            Self::BmmTupleType(tuple) => tuple.type_base_name(),
        }
    }

    /// `BMM_TYPE.is_abstract` for an effective type
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        match self {
            Self::BmmGenericType(generic) => generic.is_abstract(),
            Self::BmmSignature(signature) => signature.is_abstract(),
            Self::BmmSimpleType(simple) => simple.is_abstract(),
            Self::BmmStatusType(status) => status.is_abstract(),
            Self::BmmTupleType(tuple) => tuple.is_abstract(),
        }
    }

    /// `BMM_TYPE.is_primitive` for an effective type
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        match self {
            Self::BmmGenericType(generic) => generic.is_primitive(),
            Self::BmmSignature(signature) => signature.is_primitive(),
            Self::BmmSimpleType(simple) => simple.is_primitive(),
            Self::BmmStatusType(status) => status.is_primitive(),
            Self::BmmTupleType(tuple) => tuple.is_primitive(),
        }
    }

    /// `BMM_EFFECTIVE_TYPE.effective_type` (effected): "Result = self"
    /// (`org.openehr.lang.bmm3.bmm_effective_type.adoc` §Functions).
    #[must_use]
    pub fn effective_type(&self) -> Self {
        self.clone()
    }
}

impl BmmType {
    /// `BMM_TYPE.is_abstract` (abstract): "If true, indicates a type based on an
    /// abstract class, i.e. a type that cannot be directly instantiated"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        match self {
            Self::BmmContainerType(container) => container.is_abstract(),
            Self::BmmGenericType(generic) => generic.is_abstract(),
            Self::BmmParameterType(parameter) => parameter.is_abstract(),
            Self::BmmSignature(signature) => signature.is_abstract(),
            Self::BmmSimpleType(simple) => simple.is_abstract(),
            Self::BmmStatusType(status) => status.is_abstract(),
            Self::BmmTupleType(tuple) => tuple.is_abstract(),
        }
    }

    /// `BMM_TYPE.is_primitive` (abstract): "If True, indicates that a type based
    /// solely on primitive classes" (`org.openehr.lang.bmm3.bmm_type.adoc`
    /// §Functions).
    #[must_use]
    pub fn is_primitive(&self) -> bool {
        match self {
            Self::BmmContainerType(container) => container.is_primitive(),
            Self::BmmGenericType(generic) => generic.is_primitive(),
            Self::BmmParameterType(parameter) => parameter.is_primitive(),
            Self::BmmSignature(signature) => signature.is_primitive(),
            Self::BmmSimpleType(simple) => simple.is_primitive(),
            Self::BmmStatusType(status) => status.is_primitive(),
            Self::BmmTupleType(tuple) => tuple.is_primitive(),
        }
    }

    /// `BMM_TYPE.unitary_type` (abstract): "Type with any container abstracted
    /// away; may be a formal generic type"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions) — `_item_type_` for a
    /// container (`…bmm3.bmm_container_type.adoc` §Functions), self for every
    /// unitary meta-type (`…bmm3.bmm_unitary_type.adoc` §Functions).
    #[must_use]
    pub fn unitary_type(&self) -> BmmUnitaryType {
        match self {
            Self::BmmContainerType(container) => container.unitary_type(),
            Self::BmmGenericType(generic) => BmmUnitaryType::BmmGenericType(generic.clone()),
            Self::BmmParameterType(parameter) => {
                BmmUnitaryType::BmmParameterType(parameter.clone())
            }
            Self::BmmSignature(signature) => BmmUnitaryType::BmmSignature(signature.clone()),
            Self::BmmSimpleType(simple) => BmmUnitaryType::BmmSimpleType(simple.clone()),
            Self::BmmStatusType(status) => BmmUnitaryType::BmmStatusType(status.clone()),
            Self::BmmTupleType(tuple) => BmmUnitaryType::BmmTupleType(tuple.clone()),
        }
    }

    /// `BMM_TYPE.effective_type` (abstract): "Type with any container abstracted
    /// away, and any formal parameter replaced by its effective constraint type"
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    ///
    /// `None` is the `'Any'` case of an unconstrained formal parameter — see
    /// [`BmmParameterType::effective_type`].
    #[must_use]
    pub fn effective_type(&self) -> Option<BmmEffectiveType> {
        self.unitary_type().effective_type()
    }
}
