// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of the BMM **v3** class-feature family
//! (`BMM_FORMAL_ELEMENT` / `BMM_FEATURE` / `BMM_ROUTINE` and their leaves) — and
//! the home of that family's invariant boundary.
//!
//! Spec: `LANG/docs/bmm3/master08-core-features.adoc` plus the class definitions
//! under `LANG/docs/UML/classes/`: `org.openehr.lang.bmm3.bmm_formal_element.adoc`
//! §Functions (`signature`, `is_boolean`), `…bmm3.bmm_routine.adoc` §Functions
//! (`arity`), `…bmm3.bmm_function.adoc`, `…bmm3.bmm_procedure.adoc`,
//! `…bmm3.bmm_property.adoc`, `…bmm3.bmm_constant.adoc`, all §Functions and
//! §Invariants.
//!
//! NOTE: the declared feature invariants (`Operator_validity`,
//! `Inv_signature_has_result`, `Inv_result_type`, and peers) are NOT enforced
//! — this workspace only CONSTRUCTS v3 models from vendored schemas (P_BMM is
//! the v2.x persistence form, `LANG/docs/bmm/master06-persistence.adoc`, its
//! routine/constant attributes opaque strings), so no materialisation source
//! can produce a violating instance; the functions they are written against
//! (`signature`, `arity`) ARE implemented, so an invariant pass is ordinary
//! work the day a v3 model is built from an editable source.

use crate::v1_1::bmm3::core::bmm_formal_element::BmmFormalElement;
use crate::v1_1::bmm3::core::entity::bmm_container_type::BmmContainerType;
use crate::v1_1::bmm3::core::entity::bmm_function_type::BmmFunctionType;
use crate::v1_1::bmm3::core::entity::bmm_procedure_type::BmmProcedureType;
use crate::v1_1::bmm3::core::entity::bmm_property_type::BmmPropertyType;
use crate::v1_1::bmm3::core::entity::bmm_routine_type::BmmRoutineType;
use crate::v1_1::bmm3::core::entity::bmm_signature::BmmSignature;
use crate::v1_1::bmm3::core::entity::bmm_tuple_type::BmmTupleType;
use crate::v1_1::bmm3::core::entity::bmm_type::BmmType;
use crate::v1_1::bmm3::core::feature::bmm_constant::BmmConstant;
use crate::v1_1::bmm3::core::feature::bmm_container_property::BmmContainerProperty;
use crate::v1_1::bmm3::core::feature::bmm_feature::BmmFeature;
use crate::v1_1::bmm3::core::feature::bmm_function::BmmFunction;
use crate::v1_1::bmm3::core::feature::bmm_parameter::BmmParameter;
use crate::v1_1::bmm3::core::feature::bmm_procedure::BmmProcedure;
use crate::v1_1::bmm3::core::feature::bmm_property::BmmProperty;
use crate::v1_1::bmm3::core::feature::bmm_routine::BmmRoutine;
use crate::v1_1::bmm3::core::feature::bmm_singleton::BmmSingleton;
use crate::v1_1::bmm3::core::feature::bmm_unitary_property::BmmUnitaryProperty;

/// The type name a notionally-Boolean element's type carries
/// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions `is_boolean`:
/// "a `BMM_SIMPLE_TYPE` with `_type_name()_` = `'Boolean'`").
pub const BOOLEAN_TYPE_NAME: &str = "Boolean";

/// The `_argument_types_` tuple of a routine signature: each parameter's type,
/// keyed by the parameter name — "represented as a type-tuple (list of arbitrary
/// types)" whose items are "keyed by purpose in the tuple"
/// (`org.openehr.lang.bmm3.bmm_routine_type.adoc` §Attributes,
/// `…bmm3.bmm_tuple_type.adoc` §Attributes). `None` for a routine with no
/// parameters, since `_argument_types_` is `0..1` ("Type of arguments in the
/// signature, **if any**").
fn argument_types(parameters: &[BmmParameter]) -> Option<BmmTupleType> {
    if parameters.is_empty() {
        return None;
    }
    Some(BmmTupleType {
        item_types: parameters
            .iter()
            .map(|p| (p.name.clone(), p.r#type.clone()))
            .collect(),
    })
}

/// Is `t` the notionally-Boolean type — "a `BMM_SIMPLE_TYPE` with
/// `_type_name()_` = `'Boolean'`" (`…bmm3.bmm_formal_element.adoc` §Functions)?
///
/// The name comparison is case-insensitive per the BMM naming convention
/// ("it is assumed that case-insensitive matching is used",
/// `LANG/docs/bmm3/master05-core-model.adoc` §Naming Convention).
fn type_is_boolean(t: &BmmType) -> bool {
    matches!(t, BmmType::BmmSimpleType(simple)
        if simple.type_name().eq_ignore_ascii_case(BOOLEAN_TYPE_NAME))
}

impl BmmContainerProperty {
    /// `BMM_CONTAINER_PROPERTY.type` (redefined): the container type of this
    /// property (`org.openehr.lang.bmm3.bmm_container_property.adoc` §Attributes),
    /// for either container-property form.
    #[must_use]
    pub fn r#type(&self) -> BmmContainerType {
        match self {
            Self::BmmIndexedContainerProperty(indexed) => {
                BmmContainerType::BmmIndexedContainerType(Box::new(indexed.r#type.clone()))
            }
            Self::BmmContainerProperty(data) => data.r#type.clone(),
        }
    }

    /// `BMM_MODEL_ELEMENT.name`: "Name of this model element"
    /// (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BmmIndexedContainerProperty(indexed) => indexed.name.as_str(),
            Self::BmmContainerProperty(data) => data.name.as_str(),
        }
    }
}

impl BmmProperty {
    /// `BMM_MODEL_ELEMENT.name`: "Name of this model element"
    /// (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BmmContainerProperty(property) => property.name(),
            Self::BmmUnitaryProperty(property) => property.name.as_str(),
        }
    }
}

impl BmmFeature {
    /// `BMM_MODEL_ELEMENT.name`: "Name of this model element"
    /// (`org.openehr.lang.bmm3.bmm_model_element.adoc` §Attributes) — the key
    /// every feature map of `BMM_CLASS` is keyed by (`…bmm3.bmm_class.adoc`
    /// §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BmmConstant(constant) => constant.name.as_str(),
            Self::BmmContainerProperty(property) => property.name(),
            Self::BmmFunction(function) => function.name.as_str(),
            Self::BmmProcedure(procedure) => procedure.name.as_str(),
            Self::BmmSingleton(singleton) => singleton.name.as_str(),
            Self::BmmUnitaryProperty(property) => property.name.as_str(),
        }
    }
}

impl BmmFunction {
    /// `BMM_ROUTINE.arity`: "Return number of arguments of this routine"
    /// (`org.openehr.lang.bmm3.bmm_routine.adoc` §Functions).
    #[must_use]
    pub fn arity(&self) -> i32 {
        i32::try_from(self.parameters.as_ref().map_or(0, Vec::len)).unwrap_or(i32::MAX)
    }

    /// `BMM_FORMAL_ELEMENT.signature` as effected for a function: the
    /// `BMM_FUNCTION_TYPE` of its parameters and result
    /// (`org.openehr.lang.bmm3.bmm_function_type.adoc` §Description "Meta-type
    /// for function object signatures"; `…bmm3.bmm_formal_element.adoc`
    /// §Functions "Formal signature of this element, in the form: `name
    /// [arg1_name: T_arg1, ...][:T_value]`").
    ///
    /// The result type is the function's own `_type_`, which
    /// `BMM_FUNCTION.Inv_result_type` states equals `Result.type`
    /// (`…bmm3.bmm_function.adoc` §Invariants).
    #[must_use]
    pub fn signature(&self) -> BmmFunctionType {
        BmmFunctionType {
            result_type: self.r#type.clone(),
            argument_types: argument_types(self.parameters.as_deref().unwrap_or_default()),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`: "True if `_type_` is notionally Boolean"
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        type_is_boolean(&self.r#type)
    }
}

impl BmmProcedure {
    /// `BMM_ROUTINE.arity`: "Return number of arguments of this routine"
    /// (`org.openehr.lang.bmm3.bmm_routine.adoc` §Functions).
    #[must_use]
    pub fn arity(&self) -> i32 {
        i32::try_from(self.parameters.as_ref().map_or(0, Vec::len)).unwrap_or(i32::MAX)
    }

    /// `BMM_PROCEDURE.signature` (effected): the `BMM_PROCEDURE_TYPE` whose
    /// `_result_type_` is the built-in Status type
    /// (`org.openehr.lang.bmm3.bmm_procedure.adoc` §Functions + §Attributes —
    /// `type` is redefined to `BMM_STATUS_TYPE`;
    /// `…bmm3.bmm_procedure_type.adoc` §Description "with `_result_type_` being
    /// the special Status meta-type").
    #[must_use]
    pub fn signature(&self) -> BmmProcedureType {
        BmmProcedureType {
            result_type: Some(self.r#type.clone()),
            argument_types: argument_types(self.parameters.as_deref().unwrap_or_default()),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean` — always false for a procedure, whose
    /// `_type_` is the Status meta-type, not a `BMM_SIMPLE_TYPE`
    /// (`org.openehr.lang.bmm3.bmm_procedure.adoc` §Attributes).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare is_boolean() on BMM_FORMAL_ELEMENT; for a procedure the redefined `type` makes the answer constant"
    )]
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }
}

impl BmmRoutine {
    /// `BMM_ROUTINE.arity`: "Return number of arguments of this routine"
    /// (`org.openehr.lang.bmm3.bmm_routine.adoc` §Functions).
    #[must_use]
    pub fn arity(&self) -> i32 {
        match self {
            Self::BmmFunction(function) => function.arity(),
            Self::BmmProcedure(procedure) => procedure.arity(),
        }
    }

    /// `BMM_FORMAL_ELEMENT.signature` for either routine kind, as the
    /// `BMM_ROUTINE_TYPE` slot both effected forms belong to
    /// (`org.openehr.lang.bmm3.bmm_routine_type.adoc`).
    #[must_use]
    pub fn signature(&self) -> BmmRoutineType {
        match self {
            Self::BmmFunction(function) => BmmRoutineType::BmmFunctionType(function.signature()),
            Self::BmmProcedure(procedure) => {
                BmmRoutineType::BmmProcedureType(procedure.signature())
            }
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        match self {
            Self::BmmFunction(function) => function.is_boolean(),
            Self::BmmProcedure(procedure) => procedure.is_boolean(),
        }
    }
}

impl BmmUnitaryProperty {
    /// `BMM_FORMAL_ELEMENT.signature` for a property: a `BMM_PROPERTY_TYPE`
    /// carrying only the result type — "Meta-type for property and variable
    /// signatures" (`org.openehr.lang.bmm3.bmm_property_type.adoc`
    /// §Description), which is the degree-zero signature
    /// `BMM_PROPERTY.Inv_signature_no_args` states
    /// (`…bmm3.bmm_property.adoc` §Invariants).
    #[must_use]
    pub fn signature(&self) -> BmmPropertyType {
        BmmPropertyType {
            result_type: BmmType::from(self.r#type.clone()),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        type_is_boolean(&BmmType::from(self.r#type.clone()))
    }
}

impl BmmContainerProperty {
    /// `BMM_FORMAL_ELEMENT.signature` for a container property: the
    /// `BMM_PROPERTY_TYPE` whose result type is the container type itself
    /// (`org.openehr.lang.bmm3.bmm_container_property.adoc` §Attributes redefines
    /// `type` to `BMM_CONTAINER_TYPE`; `…bmm3.bmm_property_type.adoc`
    /// §Description).
    #[must_use]
    pub fn signature(&self) -> BmmPropertyType {
        BmmPropertyType {
            result_type: BmmType::BmmContainerType(Box::new(self.r#type())),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean` — always false for a container property:
    /// its `_type_` is a `BMM_CONTAINER_TYPE`, never a `BMM_SIMPLE_TYPE`
    /// (`org.openehr.lang.bmm3.bmm_container_property.adoc` §Attributes).
    #[expect(
        clippy::unused_self,
        reason = "the class definitions declare is_boolean() on BMM_FORMAL_ELEMENT; for a container property the redefined `type` makes the answer constant"
    )]
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }
}

impl BmmProperty {
    /// `BMM_FORMAL_ELEMENT.signature` for either property kind
    /// (`org.openehr.lang.bmm3.bmm_property_type.adoc`).
    #[must_use]
    pub fn signature(&self) -> BmmPropertyType {
        match self {
            Self::BmmContainerProperty(property) => property.signature(),
            Self::BmmUnitaryProperty(property) => property.signature(),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        match self {
            Self::BmmContainerProperty(property) => property.is_boolean(),
            Self::BmmUnitaryProperty(property) => property.is_boolean(),
        }
    }
}

impl BmmConstant {
    /// `BMM_FORMAL_ELEMENT.signature` for a constant — the argument-less
    /// `BMM_PROPERTY_TYPE` of its type
    /// (`org.openehr.lang.bmm3.bmm_property_type.adoc` §Description "Meta-type
    /// for property and variable signatures").
    #[must_use]
    pub fn signature(&self) -> BmmPropertyType {
        BmmPropertyType {
            result_type: self.r#type.clone(),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        type_is_boolean(&self.r#type)
    }
}

impl BmmSingleton {
    /// `BMM_FORMAL_ELEMENT.signature` for a singleton — the argument-less
    /// `BMM_PROPERTY_TYPE` of its type
    /// (`org.openehr.lang.bmm3.bmm_property_type.adoc` §Description).
    #[must_use]
    pub fn signature(&self) -> BmmPropertyType {
        BmmPropertyType {
            result_type: self.r#type.clone(),
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        type_is_boolean(&self.r#type)
    }
}

impl BmmFormalElement {
    /// `BMM_FORMAL_ELEMENT.signature` (abstract, "Specific implementations in
    /// descendants" — `org.openehr.lang.bmm3.bmm_formal_element.adoc`
    /// §Functions), dispatched to the effected form of each leaf.
    #[must_use]
    pub fn signature(&self) -> BmmSignature {
        match self {
            Self::BmmConstant(constant) => {
                BmmSignature::BmmPropertyType(Box::new(constant.signature()))
            }
            Self::BmmContainerProperty(property) => {
                BmmSignature::BmmPropertyType(Box::new(property.signature()))
            }
            Self::BmmFunction(function) => BmmSignature::BmmRoutineType(Box::new(
                BmmRoutineType::BmmFunctionType(function.signature()),
            )),
            Self::BmmProcedure(procedure) => BmmSignature::BmmRoutineType(Box::new(
                BmmRoutineType::BmmProcedureType(procedure.signature()),
            )),
            Self::BmmSingleton(singleton) => {
                BmmSignature::BmmPropertyType(Box::new(singleton.signature()))
            }
            Self::BmmUnitaryProperty(property) => {
                BmmSignature::BmmPropertyType(Box::new(property.signature()))
            }
            // The four variable leaves (`BMM_LOCAL`, `BMM_PARAMETER`,
            // `BMM_RESULT`, `BMM_SELF`) are `BMM_VARIABLE`s, i.e. formal
            // elements with no arguments, so their signature is the same
            // argument-less `BMM_PROPERTY_TYPE` — "Meta-type for property and
            // variable signatures" (`…bmm3.bmm_property_type.adoc`
            // §Description).
            Self::BmmLocal(local) => BmmSignature::BmmPropertyType(Box::new(BmmPropertyType {
                result_type: local.r#type.clone(),
            })),
            Self::BmmParameter(parameter) => {
                BmmSignature::BmmPropertyType(Box::new(BmmPropertyType {
                    result_type: parameter.r#type.clone(),
                }))
            }
            Self::BmmResult(result) => BmmSignature::BmmPropertyType(Box::new(BmmPropertyType {
                result_type: result.r#type.clone(),
            })),
            Self::BmmSelf(self_variable) => {
                BmmSignature::BmmPropertyType(Box::new(BmmPropertyType {
                    result_type: self_variable.r#type.clone(),
                }))
            }
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`: "True if `_type_` is notionally Boolean
    /// (i.e. a `BMM_SIMPLE_TYPE` with `_type_name()_` = `'Boolean'`)"
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        match self {
            Self::BmmConstant(constant) => constant.is_boolean(),
            Self::BmmContainerProperty(property) => property.is_boolean(),
            Self::BmmFunction(function) => function.is_boolean(),
            Self::BmmProcedure(procedure) => procedure.is_boolean(),
            Self::BmmSingleton(singleton) => singleton.is_boolean(),
            Self::BmmUnitaryProperty(property) => property.is_boolean(),
            Self::BmmLocal(local) => type_is_boolean(&local.r#type),
            Self::BmmParameter(parameter) => type_is_boolean(&parameter.r#type),
            Self::BmmResult(result) => type_is_boolean(&result.r#type),
            Self::BmmSelf(self_variable) => type_is_boolean(&self_variable.r#type),
        }
    }
}

impl BmmFeature {
    /// `BMM_FORMAL_ELEMENT.signature` for any feature of a class
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn signature(&self) -> BmmSignature {
        match self {
            Self::BmmConstant(constant) => {
                BmmSignature::BmmPropertyType(Box::new(constant.signature()))
            }
            Self::BmmContainerProperty(property) => {
                BmmSignature::BmmPropertyType(Box::new(property.signature()))
            }
            Self::BmmFunction(function) => BmmSignature::BmmRoutineType(Box::new(
                BmmRoutineType::BmmFunctionType(function.signature()),
            )),
            Self::BmmProcedure(procedure) => BmmSignature::BmmRoutineType(Box::new(
                BmmRoutineType::BmmProcedureType(procedure.signature()),
            )),
            Self::BmmSingleton(singleton) => {
                BmmSignature::BmmPropertyType(Box::new(singleton.signature()))
            }
            Self::BmmUnitaryProperty(property) => {
                BmmSignature::BmmPropertyType(Box::new(property.signature()))
            }
        }
    }

    /// `BMM_FORMAL_ELEMENT.is_boolean`
    /// (`org.openehr.lang.bmm3.bmm_formal_element.adoc` §Functions).
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        match self {
            Self::BmmConstant(constant) => constant.is_boolean(),
            Self::BmmContainerProperty(property) => property.is_boolean(),
            Self::BmmFunction(function) => function.is_boolean(),
            Self::BmmProcedure(procedure) => procedure.is_boolean(),
            Self::BmmSingleton(singleton) => singleton.is_boolean(),
            Self::BmmUnitaryProperty(property) => property.is_boolean(),
        }
    }
}
