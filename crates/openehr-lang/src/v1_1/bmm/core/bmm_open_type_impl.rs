// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_OPEN_TYPE` — the use of a formal generic
//! parameter as the type of a feature.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_open_type.adoc`
//! (§Description: "Open type reference to a single type parameter, i.e.
//! typically 'T', 'V', 'K' etc. The parameter must be in the type declaration
//! of the owning BMM_CLASS"; §Attributes `generic_constraint`: "The generic
//! constraint, which will be 'Any' if nothing set in original model";
//! §Functions `conformance_type_name`: "Return
//! generic_constraint.conformance_type_name") and
//! `LANG/docs/bmm/master05-core.adoc` §Semantics — §Basics ("for feature types
//! it will be the declared type, i.e. a simple name, an open type name (e.g.
//! `T`) or a generic type name (e.g. `Interval<Time>`)") and §Classes and Types
//! ("a `BMM_OPEN_TYPE` — corresponds to a generic parameter type from the class
//! type definition, e.g. `T`, `U` etc").

use crate::v1_1::bmm::core::bmm_open_type::BmmOpenType;

impl BmmOpenType {
    /// `BMM_CLASSIFIER.type_name` for an open type: the open type name, i.e.
    /// the name of the constrained generic parameter — `T`, `U`, `K`
    /// (`master05-core.adoc` §Semantics §Basics).
    #[must_use]
    pub fn type_name(&self) -> &str {
        self.generic_constraint.type_name()
    }

    /// `BMM_OPEN_TYPE.conformance_type_name`: "Return
    /// generic_constraint.conformance_type_name" (class doc §Functions), which
    /// for a generic parameter is its effective constraint — the parameter's
    /// `conforms_to_type` (own or inherited), else `Any` (class doc
    /// §Attributes: the constraint "will be 'Any' if nothing set in original
    /// model").
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.generic_constraint.conformance_type_name()
    }

    /// `BMM_CLASSIFIER.type_signature` for an open type: the parameter's
    /// signature form, e.g. `T:Ordered`
    /// (`org.openehr.lang.bmm.bmm_generic_parameter.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.generic_constraint.type_signature()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for an open type: the effective
    /// constraint of the parameter, or the `Any` type
    /// (`org.openehr.lang.bmm3.bmm_parameter_type.adoc` §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        self.generic_constraint.flattened_type_list()
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
    use crate::v1_1::bmm::core::bmm_open_type::BmmOpenType;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;

    /// An open type over a parameter named `name`, optionally constrained.
    fn open_type(name: &str, conforms_to: Option<&str>) -> BmmOpenType {
        BmmOpenType {
            documentation: None,
            generic_constraint: BmmGenericParameter {
                documentation: None,
                name: name.to_owned(),
                conforms_to_type: conforms_to.map(|constraint| {
                    BmmClass::BmmClass(BmmClassData {
                        documentation: None,
                        name: constraint.to_owned(),
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
                }),
                inheritance_precursor: None,
            },
        }
    }

    #[test]
    fn a_constrained_open_type_names_its_parameter_and_constraint() {
        let open = open_type("T", Some("Ordered"));
        assert_eq!(open.type_name(), "T");
        assert_eq!(open.type_signature(), "T:Ordered");
        assert_eq!(open.conformance_type_name(), "Ordered");
        assert_eq!(open.flattened_type_list(), ["Ordered".to_owned()]);
    }

    #[test]
    fn an_unconstrained_open_type_conforms_to_any() {
        let open = open_type("T", None);
        assert_eq!(open.type_name(), "T");
        assert_eq!(open.type_signature(), "T");
        assert_eq!(open.conformance_type_name(), "Any");
    }
}
