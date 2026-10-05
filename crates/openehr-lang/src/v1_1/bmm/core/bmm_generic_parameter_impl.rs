// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_GENERIC_PARAMETER` — the formal generic
//! parameter of a generic class definition.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_generic_parameter.adoc`
//! §Functions (`flattened_conforms_to_type`, `effective_conforms_to_type`,
//! `type_signature`) and §Invariants (`Inv_generic_name`:
//! `name.count = 1 and name.is_upper`), read together with
//! `LANG/docs/bmm/master05-core.adoc` §Semantics §Basics (a generic parameter is
//! one of the things that "functions as the type of a property") and §Classes
//! and Types (`BMM_OPEN_TYPE` "corresponds to a generic parameter type from the
//! class type definition, e.g. `T`, `U` etc"). The v3 counterpart of this class
//! is `BMM_PARAMETER_TYPE` (`…bmm3.bmm_parameter_type.adoc`), whose §Functions
//! state the same three functions with sharper post-conditions and are cited
//! where they settle the v2 wording.

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
use crate::v1_1::bmm::core::bmm_type_impl::ANY_TYPE_NAME;

impl BmmGenericParameter {
    /// `BMM_GENERIC_PARAMETER.flattened_conforms_to_type`: "Get any ultimate
    /// type conformance constraint on this generic parameter due to
    /// inheritance" (class doc §Functions) — i.e. this parameter's own
    /// `conforms_to_type` when set, else the constraint of its
    /// `inheritance_precursor`, recursively
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions states the
    /// rule explicitly: "Result is either `_conforms_to_type_` or
    /// `_inheritance_precursor.flattened_conforms_to_type_`").
    ///
    /// `None` means unconstrained; see
    /// [`Self::effective_conforms_to_type_name`] for the `Any` projection.
    #[must_use]
    pub fn flattened_conforms_to_type(&self) -> Option<&str> {
        match &self.conforms_to_type {
            Some(class) => Some(class.name()),
            None => self
                .inheritance_precursor
                .as_ref()
                .and_then(|precursor| precursor.flattened_conforms_to_type()),
        }
    }

    /// `BMM_GENERIC_PARAMETER.conforms_to_type`, resolved to the constraining
    /// class: this parameter's own constraint when set, else the inherited one
    /// (class doc §Attributes: "If set, is the corresponding generic parameter
    /// definition in an ancestor class").
    #[must_use]
    pub fn flattened_conforms_to_class(&self) -> Option<&BmmClass> {
        match &self.conforms_to_type {
            Some(class) => Some(class),
            None => self
                .inheritance_precursor
                .as_ref()
                .and_then(|precursor| precursor.flattened_conforms_to_class()),
        }
    }

    /// `BMM_GENERIC_PARAMETER.effective_conforms_to_type`: "Generate ultimate
    /// conformance type, which is either from `conforms_to_type` or if not set,
    /// 'Any'" (class doc §Functions).
    ///
    /// NOTE: the class doc's function returns a `BMM_CLASS` with an `Any`
    /// fallback the schema need not define
    /// (`org.openehr.lang.bmm3.bmm_definitions.adoc` §Functions `Any_class`);
    /// the generated model carries no such singleton and synthesising one
    /// would be a shadow type, so this surface returns the type NAME, the
    /// typed constraint staying reachable via
    /// [`Self::flattened_conforms_to_class`].
    #[must_use]
    pub fn effective_conforms_to_type_name(&self) -> String {
        self.flattened_conforms_to_type()
            .unwrap_or(ANY_TYPE_NAME)
            .to_owned()
    }

    /// `BMM_CLASSIFIER.type_name` for a generic parameter: the parameter name
    /// (`master05-core.adoc` §Semantics §Basics: "for feature types it will be
    /// the declared type, i.e. a simple name, an open type name (e.g. `T`) …";
    /// `org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions: `type_name`
    /// "Return `_name_`").
    #[must_use]
    pub fn type_name(&self) -> &str {
        self.name.as_str()
    }

    /// `BMM_GENERIC_PARAMETER.type_signature` (redefined): "Signature form of
    /// the open type, including constrainer type if there is one, e.g.
    /// 'T:Ordered'" (class doc §Functions).
    ///
    /// Delimiter per `org.openehr.lang.bmm3.bmm_definitions.adoc` §Constants
    /// (`Generic_constraint_delimiter` `':'`, "Delimiter between formal type
    /// parameter and constraint type, as used in `Sortable<T: Ordered>`"); the
    /// class doc's own example `T:Ordered` carries no whitespace, so neither
    /// does this rendering.
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self.flattened_conforms_to_type() {
            Some(constraint) => {
                let name = &self.name;
                format!("{name}:{constraint}")
            }
            None => self.name.clone(),
        }
    }

    /// `conformance_type_name` for a generic parameter: the
    /// effective constraint, i.e. the class the parameter is guaranteed to
    /// conform to — `Any` when unconstrained
    /// (`org.openehr.lang.bmm.bmm_open_type.adoc` §Functions:
    /// `conformance_type_name` "Return generic_constraint.conformance_type_name").
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.effective_conforms_to_type_name()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for a generic parameter: "Result is
    /// either `_flattened_conforms_to_type.flattened_type_list_` or the `Any`
    /// type" (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        vec![self.effective_conforms_to_type_name()]
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;

    /// A simple class named `name`.
    fn simple_class(name: &str) -> BmmClass {
        BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: BmmPackage {
                documentation: None,
                packages: None,
                name: "org.openehr.base.foundation_types".to_owned(),
                classes: openehr_base::containers::present(Vec::new()),
            },
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
        })
    }

    /// A generic parameter named `name`, optionally constrained, optionally
    /// with an inheritance precursor.
    fn parameter(
        name: &str,
        conforms_to: Option<&str>,
        precursor: Option<BmmGenericParameter>,
    ) -> BmmGenericParameter {
        BmmGenericParameter {
            documentation: None,
            name: name.to_owned(),
            conforms_to_type: conforms_to.map(simple_class),
            inheritance_precursor: precursor.map(Box::new),
        }
    }

    #[test]
    fn own_constraint_wins_over_the_inherited_one() {
        let precursor = parameter("T", Some("Ordered"), None);
        let own = parameter("T", Some("Comparable"), Some(precursor));
        assert_eq!(own.flattened_conforms_to_type(), Some("Comparable"));
        assert_eq!(own.effective_conforms_to_type_name(), "Comparable");
    }

    #[test]
    fn the_constraint_is_inherited_through_the_precursor_chain() {
        let root = parameter("T", Some("Ordered"), None);
        let middle = parameter("T", None, Some(root));
        let leaf = parameter("T", None, Some(middle));
        assert_eq!(leaf.flattened_conforms_to_type(), Some("Ordered"));
        assert_eq!(leaf.type_signature(), "T:Ordered");
    }

    #[test]
    fn an_unconstrained_parameter_is_any() {
        let unconstrained = parameter("T", None, None);
        assert_eq!(unconstrained.flattened_conforms_to_type(), None);
        assert_eq!(unconstrained.effective_conforms_to_type_name(), "Any");
        assert_eq!(unconstrained.conformance_type_name(), "Any");
        assert_eq!(unconstrained.flattened_type_list(), ["Any".to_owned()]);
        assert_eq!(unconstrained.type_signature(), "T");
        assert_eq!(unconstrained.type_name(), "T");
    }

    #[test]
    fn the_typed_constraint_stays_reachable() {
        let constrained = parameter("T", Some("Ordered"), None);
        let class = constrained
            .flattened_conforms_to_class()
            .expect("the constraint is set");
        assert_eq!(class.name(), "Ordered");
    }
}
