// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_CLASSIFIER` — the abstract parent of
//! "anything that functions as the type of a property (viz: classes, types and
//! generic parameters)".
//!
//! Spec: `LANG/docs/bmm/master05-core.adoc` §Semantics §Basics (which defines
//! the naming trio `type_name` / `type_signature` / `conformance_type_name`
//! normatively — quoted in full on
//! [`crate::v1_1::bmm::core::bmm_type_impl`]) and
//! `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_classifier.adoc` §Functions
//! (`type_name`, `type_category`, `type_signature`, `base_class`,
//! `flattened_type_list`).
//!
//! Every function here is a pure dispatcher: the per-class behaviour lives with
//! the class the class definitions declare it on
//! ([`crate::v1_1::bmm::core::bmm_class_impl`],
//! [`crate::v1_1::bmm::core::bmm_type_impl`],
//! [`crate::v1_1::bmm::core::bmm_generic_parameter_impl`],
//! [`crate::v1_1::bmm::core::bmm_open_type_impl`]).

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_classifier::BmmClassifier;

impl BmmClassifier {
    /// `BMM_CLASSIFIER.type_name`: "Formal string form of the type as per UML"
    /// (class doc §Functions).
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmClass(class) => class.type_name(),
            Self::BmmContainerType(container) => container.type_name(),
            Self::BmmGenericParameter(parameter) => parameter.type_name().to_owned(),
            Self::BmmGenericType(generic) => generic.type_name(),
            Self::BmmOpenType(open) => open.type_name().to_owned(),
            Self::BmmSimpleType(simple) => simple.type_name(),
        }
    }

    /// `BMM_CLASSIFIER.type_signature`: "Signature form of the type, which for
    /// generics includes generic parameter constrainer types e.g.
    /// Interval<T:Ordered>" (class doc §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmClass(class) => class.type_signature(),
            Self::BmmContainerType(container) => container.type_signature(),
            Self::BmmGenericParameter(parameter) => parameter.type_signature(),
            Self::BmmGenericType(generic) => generic.type_signature(),
            Self::BmmOpenType(open) => open.type_signature(),
            Self::BmmSimpleType(simple) => simple.type_signature(),
        }
    }

    /// `conformance_type_name`: "a reduced form of the type useful in some
    /// circumstances that is either a simple class name, the _contained_ type
    /// for a container type (e.g. `ELEMENT` from the type `List<ELEMENT>`), and
    /// the _root_ type from a generic type (e.g. `Interval` from
    /// `Interval<T>`)" (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics —
    /// the function is defined there in prose; only
    /// `BMM_OPEN_TYPE` carries it in a §Functions table — the only class doc in
    /// the vendored LANG set that names it).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmClass(class) => class.name().to_owned(),
            Self::BmmContainerType(container) => container.conformance_type_name(),
            Self::BmmGenericParameter(parameter) => parameter.conformance_type_name(),
            Self::BmmGenericType(generic) => generic.conformance_type_name(),
            Self::BmmOpenType(open) => open.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.conformance_type_name(),
        }
    }

    /// `BMM_CLASSIFIER.flattened_type_list`: "Completely flattened list of type
    /// names, flattening out all generic parameters" (class doc §Functions).
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmClass(class) => class.suppliers(),
            Self::BmmContainerType(container) => container.flattened_type_list(),
            Self::BmmGenericParameter(parameter) => parameter.flattened_type_list(),
            Self::BmmGenericType(generic) => generic.flattened_type_list(),
            Self::BmmOpenType(open) => open.flattened_type_list(),
            Self::BmmSimpleType(simple) => simple.flattened_type_list(),
        }
    }

    /// `BMM_CLASSIFIER.base_class`: "Main design class for this type, from which
    /// properties etc can be extracted" (class doc §Functions), projected to the
    /// class NAME.
    ///
    /// NOTE: the class doc's signature returns a `BMM_CLASS`, which the
    /// built-in meta-types have none of
    /// (`org.openehr.lang.bmm3.bmm_builtin_type.adoc` §Description: built-in
    /// types "are treated as being primitive and non-abstract") — synthesising
    /// one would be a shadow type, so the enum-level surface is the NAME and
    /// the typed `base_class` fields stay reachable on the variants that have
    /// one.
    #[must_use]
    pub fn base_class_name(&self) -> String {
        match self {
            Self::BmmClass(class) => class.name().to_owned(),
            Self::BmmContainerType(container) => container.container_type().name().to_owned(),
            Self::BmmGenericParameter(parameter) => parameter.effective_conforms_to_type_name(),
            Self::BmmGenericType(generic) => generic.base_class.name.clone(),
            Self::BmmOpenType(open) => open.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.base_class.name().to_owned(),
        }
    }

    /// `BMM_CLASSIFIER.type_category`: "Generate a type category of main target
    /// type from Type_category_xx values" (class doc §Functions).
    ///
    /// NOTE: the v2 `Type_category_xx` constant set is DANGLING (no v2 class
    /// declares a single value), so this function returns the declared
    /// successor vocabulary —
    /// `org.openehr.lang.bmm3.bmm_entity_metatype.adoc` §Constants, generated
    /// as
    /// [`BmmEntityMetatype`](crate::v1_1::bmm3::core::entity::bmm_entity_metatype::BmmEntityMetatype)
    /// — with the literal/vocabulary agreement pinned by a unit test in this
    /// module.
    ///
    /// The mapping: a generic class or generic type is `…_generic`; a generic
    /// parameter, open type or parameter type is `…_generic_parameter`; an
    /// enumeration class is `…_enumeration`; a container type is
    /// `…_container`; a simple class, simple type and the built-in
    /// signature/status/tuple meta-types are `…_simple`.
    #[must_use]
    pub fn type_category(&self) -> &'static str {
        match self {
            Self::BmmClass(BmmClass::BmmEnumeration(_)) => "Entity_metatype_enumeration",
            Self::BmmClass(BmmClass::BmmGenericClass(_)) | Self::BmmGenericType(_) => {
                "Entity_metatype_generic"
            }
            Self::BmmGenericParameter(_) | Self::BmmOpenType(_) => {
                "Entity_metatype_generic_parameter"
            }
            Self::BmmContainerType(_) => "Entity_metatype_container",
            Self::BmmClass(BmmClass::BmmClass(_)) | Self::BmmSimpleType(_) => {
                "Entity_metatype_simple"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_classifier::BmmClassifier;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerType;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerTypeData;
    use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumeration;
    use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumerationData;
    use crate::v1_1::bmm::core::bmm_generic_class::BmmGenericClass;
    use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
    use crate::v1_1::bmm::core::bmm_generic_type::BmmGenericType;
    use crate::v1_1::bmm::core::bmm_open_type::BmmOpenType;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;
    use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
    use crate::v1_1::bmm::core::bmm_type::BmmType;
    use crate::v1_1::bmm3::core::entity::bmm_entity_metatype::BmmEntityMetatype;

    /// An empty package node.
    fn package() -> BmmPackage {
        BmmPackage {
            documentation: None,
            packages: None,
            name: "org.openehr.base.foundation_types".to_owned(),
            classes: openehr_base::containers::present(Vec::new()),
        }
    }

    /// A simple class named `name`.
    fn simple_class(name: &str) -> BmmClass {
        BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: package(),
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
        })
    }

    /// A generic class named `name` with one parameter `T`, constrained to
    /// `constraint`.
    fn generic_class(name: &str, constraint: Option<&str>) -> BmmGenericClass {
        BmmGenericClass {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: package(),
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
            generic_parameters: [("T".to_owned(), generic_parameter(constraint))]
                .into_iter()
                .collect(),
        }
    }

    /// A generic parameter `T`, optionally constrained.
    fn generic_parameter(constraint: Option<&str>) -> BmmGenericParameter {
        BmmGenericParameter {
            documentation: None,
            name: "T".to_owned(),
            conforms_to_type: constraint.map(simple_class),
            inheritance_precursor: None,
        }
    }

    /// An enumeration class named `name`.
    fn enumeration_class(name: &str) -> BmmClass {
        BmmClass::BmmEnumeration(BmmEnumeration::BmmEnumeration(BmmEnumerationData {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: package(),
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
            item_names: Some(vec!["equal".to_owned()]),
            item_values: openehr_base::containers::present(Vec::new()),
            underlying_type_name: "Integer".to_owned(),
        }))
    }

    #[test]
    fn the_naming_trio_dispatches_over_every_variant() {
        let simple = BmmClassifier::BmmSimpleType(BmmSimpleType {
            documentation: None,
            base_class: simple_class("ELEMENT"),
        });
        assert_eq!(simple.type_name(), "ELEMENT");
        assert_eq!(simple.type_signature(), "ELEMENT");
        assert_eq!(simple.conformance_type_name(), "ELEMENT");
        assert_eq!(simple.base_class_name(), "ELEMENT");
        assert_eq!(simple.flattened_type_list(), ["ELEMENT".to_owned()]);

        let class = BmmClassifier::BmmClass(BmmClass::BmmGenericClass(generic_class(
            "Interval",
            Some("Ordered"),
        )));
        assert_eq!(class.type_name(), "Interval<T>");
        assert_eq!(class.type_signature(), "Interval<T:Ordered>");
        assert_eq!(class.conformance_type_name(), "Interval");
        assert_eq!(class.base_class_name(), "Interval");

        let generic = BmmClassifier::BmmGenericType(BmmGenericType {
            documentation: None,
            generic_parameters: vec![BmmType::BmmSimpleType(BmmSimpleType {
                documentation: None,
                base_class: simple_class("Time"),
            })],
            base_class: generic_class("Interval", Some("Ordered")),
        });
        assert_eq!(generic.type_name(), "Interval<Time>");
        assert_eq!(generic.conformance_type_name(), "Interval");
        assert_eq!(
            generic.flattened_type_list(),
            ["Interval".to_owned(), "Time".to_owned()]
        );

        let container = BmmClassifier::BmmContainerType(BmmContainerType::BmmContainerType(
            BmmContainerTypeData {
                documentation: None,
                container_type: simple_class("List"),
                base_type: Box::new(BmmType::BmmSimpleType(BmmSimpleType {
                    documentation: None,
                    base_class: simple_class("ELEMENT"),
                })),
            },
        ));
        assert_eq!(container.type_name(), "List<ELEMENT>");
        assert_eq!(container.conformance_type_name(), "ELEMENT");
        assert_eq!(container.base_class_name(), "List");

        let parameter = BmmClassifier::BmmGenericParameter(generic_parameter(Some("Ordered")));
        assert_eq!(parameter.type_name(), "T");
        assert_eq!(parameter.type_signature(), "T:Ordered");
        assert_eq!(parameter.conformance_type_name(), "Ordered");

        let open = BmmClassifier::BmmOpenType(BmmOpenType {
            documentation: None,
            generic_constraint: generic_parameter(None),
        });
        assert_eq!(open.type_name(), "T");
        assert_eq!(open.conformance_type_name(), "Any");
    }

    #[test]
    fn type_category_matches_the_generated_metatype_vocabulary() {
        // The literals this function returns ARE the BMM_ENTITY_METATYPE wire
        // strings (bmm3.bmm_entity_metatype.adoc §Constants) — the v3
        // generation's vocabulary, adopted because the v2 `Type_category_xx`
        // set is dangling (see the NOTE on `type_category`); this pins the
        // agreement so the two cannot drift apart.
        let cases: [(BmmClassifier, BmmEntityMetatype); 5] = [
            (
                BmmClassifier::BmmSimpleType(BmmSimpleType {
                    documentation: None,
                    base_class: simple_class("ELEMENT"),
                }),
                BmmEntityMetatype::EntityMetatypeSimple,
            ),
            (
                BmmClassifier::BmmClass(BmmClass::BmmGenericClass(generic_class("Interval", None))),
                BmmEntityMetatype::EntityMetatypeGeneric,
            ),
            (
                BmmClassifier::BmmGenericParameter(generic_parameter(None)),
                BmmEntityMetatype::EntityMetatypeGenericParameter,
            ),
            (
                BmmClassifier::BmmClass(enumeration_class("MATCH_KIND")),
                BmmEntityMetatype::EntityMetatypeEnumeration,
            ),
            (
                BmmClassifier::BmmContainerType(BmmContainerType::BmmContainerType(
                    BmmContainerTypeData {
                        documentation: None,
                        container_type: simple_class("List"),
                        base_type: Box::new(BmmType::BmmSimpleType(BmmSimpleType {
                            documentation: None,
                            base_class: simple_class("ELEMENT"),
                        })),
                    },
                )),
                BmmEntityMetatype::EntityMetatypeContainer,
            ),
        ];
        for (classifier, expected) in cases {
            assert_eq!(classifier.type_category(), expected.as_str());
        }
    }
}
