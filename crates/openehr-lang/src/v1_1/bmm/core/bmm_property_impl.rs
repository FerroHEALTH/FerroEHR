// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_PROPERTY<T>` — the property definitions
//! whose differential and flat sets `BMM_CLASS` exposes.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_property.adoc`
//! §Attributes + §Functions (`existence`: "Interval form of `0..1`, `1..1` etc,
//! generated from `_is_mandatory_`"; `display_name`: "Name of this attribute to
//! display in UI"), `…bmm.bmm_container_property.adoc` (the container variant,
//! whose `type` is "Redefined to BMM_CONTAINER_TYPE") and
//! `LANG/docs/bmm/master05-core.adoc` §Semantics §Classes and Properties
//! ("Class properties are defined using the generic class
//! `BMM_PROPERTY <T: BMM_TYPE>`").

use openehr_base::v1_3::prelude::MultiplicityInterval;

use crate::v1_1::bmm::core::bmm_container_property::BmmContainerProperty;
use crate::v1_1::bmm::core::bmm_property::BmmProperty;
use crate::v1_1::bmm::core::bmm_type::BmmType;

impl<T> BmmProperty<T> {
    /// `BMM_PROPERTY.name`: "Name of this property in the model" (class doc
    /// §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BmmContainerProperty(container) => container.name.as_str(),
            Self::BmmProperty(property) => property.name.as_str(),
        }
    }

    /// `BMM_PROPERTY.is_mandatory`: "True if this property is mandatory in its
    /// class" (class doc §Attributes).
    ///
    /// The attribute is optional (`0..1`) in the class definition, so an absent
    /// value is reported as *not mandatory* — the only reading that keeps
    /// [`Self::existence`] total, and the same reading the class doc's
    /// `0..1`/`1..1` example pair implies (absent = the weaker `0..1` bound).
    #[must_use]
    pub fn is_mandatory(&self) -> bool {
        let flag = match self {
            Self::BmmContainerProperty(container) => container.is_mandatory,
            Self::BmmProperty(property) => property.is_mandatory,
        };
        flag == Some(true)
    }

    /// `BMM_PROPERTY.is_computed`: "True if this property is computed rather
    /// than stored in objects of this class" (class doc §Attributes); an absent
    /// optional value reads as *stored*.
    #[must_use]
    pub fn is_computed(&self) -> bool {
        let flag = match self {
            Self::BmmContainerProperty(container) => container.is_computed,
            Self::BmmProperty(property) => property.is_computed,
        };
        flag == Some(true)
    }

    /// `BMM_PROPERTY.existence`: "Interval form of `0..1`, `1..1` etc, generated
    /// from `_is_mandatory_`" (class doc §Functions) — a closed, fully bounded
    /// interval with both limits included, lower `1` when mandatory and `0`
    /// otherwise, upper always `1` (existence constrains presence of the single
    /// property slot; a container's item count is `cardinality`,
    /// `org.openehr.lang.bmm.bmm_container_property.adoc` §Attributes).
    #[must_use]
    pub fn existence(&self) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(i32::from(self.is_mandatory())),
            upper: Some(1),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    /// `BMM_PROPERTY.display_name`: "Name of this attribute to display in UI"
    /// (class doc §Functions) — the property name; `BMM_CONTAINER_PROPERTY`
    /// redefines the function but restates the same meaning
    /// (`org.openehr.lang.bmm.bmm_container_property.adoc` §Functions), and
    /// neither class definition prescribes any transformation of the name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name()
    }
}

impl BmmContainerProperty {
    /// `BMM_CONTAINER_PROPERTY.display_name` (redefined): "Name of this
    /// attribute to display in UI"
    /// (`org.openehr.lang.bmm.bmm_container_property.adoc` §Functions).
    ///
    /// NOTE: the redefinition restates its parent's meaning verbatim
    /// (`…bmm.bmm_property.adoc` §Functions) and neither class definition
    /// prescribes a transformation, so the container form is the property name
    /// too — not the container type's rendering.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name.as_str()
    }
}

impl BmmProperty<BmmType> {
    /// `BMM_CLASSIFIER.type_name` of this property's type — "the declared type"
    /// of the feature (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics).
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmContainerProperty(container) => container.r#type.type_name(),
            Self::BmmProperty(property) => property.r#type.type_name(),
        }
    }

    /// `conformance_type_name` of this property's type: the
    /// reduced form that abstracts a container away — "the _contained_ type for
    /// a container type (e.g. `ELEMENT` from the type `List<ELEMENT>`)"
    /// (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics). This is the
    /// projection `BMM_MODEL.property_definition_at_path` and
    /// `BMM_MODEL.ms_conformant_property_type` navigate on.
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmContainerProperty(container) => container.r#type.conformance_type_name(),
            Self::BmmProperty(property) => property.r#type.conformance_type_name(),
        }
    }

    /// `BMM_CLASSIFIER.flattened_type_list` of this property's type — the
    /// supplier names this property contributes to
    /// [`BmmClass::suppliers`](crate::v1_1::bmm::core::bmm_class::BmmClass::suppliers).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmContainerProperty(container) => container.r#type.flattened_type_list(),
            Self::BmmProperty(property) => property.r#type.flattened_type_list(),
        }
    }
}

#[cfg(test)]
mod tests {
    use openehr_base::v1_3::prelude::MultiplicityInterval;

    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_container_property::BmmContainerProperty;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerType;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerTypeData;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;
    use crate::v1_1::bmm::core::bmm_property::BmmProperty;
    use crate::v1_1::bmm::core::bmm_property::BmmPropertyData;
    use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
    use crate::v1_1::bmm::core::bmm_type::BmmType;

    /// A simple class named `name`.
    fn simple_class(name: &str) -> BmmClass {
        BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: BmmPackage {
                documentation: None,
                packages: None,
                name: "org.openehr.rm.data_structures".to_owned(),
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

    /// A unitary property of the given name, type and mandatory flag.
    fn property(name: &str, type_name: &str, is_mandatory: Option<bool>) -> BmmProperty<BmmType> {
        BmmProperty::BmmProperty(BmmPropertyData {
            documentation: None,
            name: name.to_owned(),
            is_mandatory,
            is_computed: None,
            r#type: BmmType::BmmSimpleType(BmmSimpleType {
                documentation: None,
                base_class: simple_class(type_name),
            }),
            is_im_runtime: None,
            is_im_infrastructure: None,
        })
    }

    /// A `List<item>` container property.
    fn container_property(name: &str, item: &str) -> BmmProperty<BmmType> {
        BmmProperty::BmmContainerProperty(BmmContainerProperty {
            documentation: None,
            name: name.to_owned(),
            is_mandatory: Some(true),
            is_computed: None,
            r#type: BmmContainerType::BmmContainerType(BmmContainerTypeData {
                documentation: None,
                container_type: simple_class("List"),
                base_type: Box::new(BmmType::BmmSimpleType(BmmSimpleType {
                    documentation: None,
                    base_class: simple_class(item),
                })),
            }),
            is_im_runtime: None,
            is_im_infrastructure: None,
            cardinality: None,
        })
    }

    #[test]
    fn existence_is_zero_one_or_one_one() {
        let optional = property("units", "String", None);
        assert!(!optional.is_mandatory());
        assert_eq!(
            optional.existence(),
            MultiplicityInterval {
                lower: Some(0),
                upper: Some(1),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: true,
            }
        );

        let mandatory = property("magnitude", "Real", Some(true));
        assert!(mandatory.is_mandatory());
        assert_eq!(
            mandatory.existence(),
            MultiplicityInterval {
                lower: Some(1),
                upper: Some(1),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: true,
            }
        );
    }

    /// The redefined `display_name` on the container class itself answers the
    /// property NAME, not the container type's rendering — the boundary the
    /// redefinition could plausibly have moved.
    #[test]
    fn the_container_redefinition_still_displays_the_property_name() {
        let BmmProperty::BmmContainerProperty(items) = container_property("items", "ELEMENT")
        else {
            panic!("container_property builds the container variant");
        };
        assert_eq!(items.display_name(), "items");
        assert_ne!(items.display_name(), items.r#type.type_name());
    }

    #[test]
    fn names_and_types_read_through_every_variant() {
        let unitary = property("magnitude", "Real", Some(true));
        assert_eq!(unitary.name(), "magnitude");
        assert_eq!(unitary.display_name(), "magnitude");
        assert!(!unitary.is_computed());
        assert_eq!(unitary.type_name(), "Real");
        assert_eq!(unitary.conformance_type_name(), "Real");
        assert_eq!(unitary.flattened_type_list(), ["Real".to_owned()]);

        let items = container_property("items", "ELEMENT");
        assert_eq!(items.name(), "items");
        assert!(items.is_mandatory());
        assert_eq!(items.type_name(), "List<ELEMENT>");
        // The container is abstracted away: master05-core.adoc §Basics.
        assert_eq!(items.conformance_type_name(), "ELEMENT");
        assert_eq!(
            items.flattened_type_list(),
            ["List".to_owned(), "ELEMENT".to_owned()]
        );
    }
}
