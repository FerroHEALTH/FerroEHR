// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Binding actual generic parameters to the formal parameters of their root
//! class, by name and in declaration order.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_generic_type.adoc`
//! §Attributes (SPECLANG-16): `generic_parameter_defs` is "keyed by formal
//! generic parameter name", and both it and `generic_parameters` follow "the
//! order of the owning class's formal generic parameter declarations".

use std::collections::BTreeMap;

use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;
use crate::v1_1::bmm3::core::entity::bmm_effective_type::BmmEffectiveType;
use crate::v1_1::bmm3::core::entity::bmm_generic_type::BmmGenericType;
use crate::v1_1::bmm3::core::entity::bmm_parameter_type::BmmParameterType;
use crate::v1_1::bmm3::core::entity::bmm_unitary_type::BmmUnitaryType;

/// One actual generic parameter of a `P_BMM_GENERIC_TYPE`, as persisted: a
/// name from `generic_parameters` or a type from `generic_parameter_defs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActualParameter<'a> {
    /// A simple type named in `generic_parameters`.
    Named(&'a str),
    /// A type from `generic_parameter_defs`.
    Structured(&'a PBmmType),
}

impl PBmmGenericType {
    /// The actual parameters bound to `formals`, the root class's formal
    /// generic parameter names in declaration order.
    ///
    /// `generic_parameter_defs` is "keyed by formal generic parameter name" and
    /// both lists follow "the order of the owning class's formal generic
    /// parameter declarations" (`…p_bmm_generic_type.adoc` §Attributes), so a
    /// keyed entry binds the formal of its name and each formal left unbound
    /// takes the next `generic_parameters` name. Names beyond the formals are
    /// kept at the end, for validation to report.
    ///
    /// # Errors
    /// [`PBmmReadError::UnknownGenericParameterKey`] for a key that names no
    /// formal, and [`PBmmReadError::GenericParameterBoundTwice`] for two keys
    /// that bind one formal.
    pub fn bind_parameters<'a>(
        &'a self,
        context: &str,
        formals: &[&str],
    ) -> Result<Vec<ActualParameter<'a>>, PBmmReadError> {
        let mut bound: Vec<Option<&PBmmType>> = vec![None; formals.len()];
        for (key, def) in self.generic_parameter_defs.iter().flatten() {
            let (slot, formal) = formals
                .iter()
                .position(|formal| formal == key)
                .or_else(|| positional_key(key, formals.len()))
                .and_then(|position| bound.get_mut(position).zip(formals.get(position)))
                .ok_or_else(|| PBmmReadError::UnknownGenericParameterKey {
                    context: context.to_owned(),
                    root_type: self.root_type.clone(),
                    key: key.clone(),
                })?;
            if slot.replace(def).is_some() {
                return Err(PBmmReadError::GenericParameterBoundTwice {
                    context: context.to_owned(),
                    root_type: self.root_type.clone(),
                    parameter: (*formal).to_owned(),
                });
            }
        }
        let mut named = self.generic_parameters.iter().flatten();
        let mut out = Vec::new();
        for slot in bound {
            match slot {
                Some(def) => out.push(ActualParameter::Structured(def)),
                None => out.extend(named.next().map(|name| ActualParameter::Named(name))),
            }
        }
        out.extend(named.map(|name| ActualParameter::Named(name)));
        Ok(out)
    }
}

/// The 0-based formal position a 1-based integer key names, when in range.
///
/// NOTE: no openEHR spec governs the integer-keyed `generic_parameters` list
/// the vendored AM schemas write; the reader keeps its keys, read as positions.
fn positional_key(key: &str, formals: usize) -> Option<usize> {
    key.parse::<usize>()
        .ok()
        .and_then(|position| position.checked_sub(1))
        .filter(|position| *position < formals)
}

/// Pairs a generic ancestor's FORMAL parameter names with the types the
/// descendant bound them to.
///
/// `BMM_GENERIC_TYPE.generic_parameters` is "the actual generic parameter
/// types" in "the order of the owning class's formal generic parameter
/// declarations", and `BMM_GENERIC_CLASS.generic_parameters` keeps those
/// declarations in that order (`org.openehr.lang.bmm3.bmm_generic_type.adoc` +
/// `…bmm3.bmm_generic_class.adoc` §Attributes), so the two zip positionally.
/// A binding that merely restates the formal parameter unchanged is dropped:
/// substituting a parameter for itself synthesises nothing.
pub(super) fn parameter_bindings(generic: &BmmGenericType) -> BTreeMap<String, BmmUnitaryType> {
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
