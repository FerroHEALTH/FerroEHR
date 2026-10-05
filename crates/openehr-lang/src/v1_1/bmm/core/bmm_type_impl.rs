// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of the `BMM_TYPE` family — the BMM v2 core
//! type-level computed features (the naming trio plus type substitutions).
//!
//! Spec: `LANG/docs/bmm/master05-core.adoc` §Semantics §Basics, which defines
//! the three naming functions normatively:
//!
//! * `type_name` — "the effective type of an entity; for simple classes, this
//!   will just be the class name (`BMM_CLASS._name_`); for generic and
//!   container classes it will be generic name such as `List<T>`, `Interval<T>`
//!   etc; for feature types it will be the declared type, i.e. a simple name,
//!   an open type name (e.g. `T`) or a generic type name (e.g.
//!   `Interval<Time>`)";
//! * `type_signature` — "a form of the type name that can be used as a
//!   fully-defined type signature, which for generic classes includes generic
//!   constrainer types, giving a signature such as `Interval<T:Ordered>`";
//! * `conformance_type_name` — "a reduced form of the type useful in some
//!   circumstances that is either a simple class name, the _contained_ type for
//!   a container type (e.g. `ELEMENT` from the type `List<ELEMENT>`), and the
//!   _root_ type from a generic type (e.g. `Interval` from `Interval<T>`)".
//!
//! plus §Classes and Types and the class definitions under
//! `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_{classifier,type,simple_type,
//! generic_type,container_type,indexed_container_type}.adoc`.
//!
//! This is the v2.x generation's surface only. The v3 line's own type lattice
//! lives beside its own generated types in
//! [`crate::v1_1::bmm3::core::entity::bmm_type_impl`], sharing no impl with this
//! one, because they are different classes
//! (`LANG/docs/bmm3/master00-amendment_record.adoc` SPECLANG-14).
//!
//! The per-class functions are implemented on the individual generated structs,
//! where the class definitions declare them, and
//! [`BmmType`]/[`crate::v1_1::bmm::core::bmm_classifier::BmmClassifier`] are
//! dispatchers over them.
//!
//! NOTE: v2's `flattened_type_list` flattens the WHOLE expression, container
//! class included (`org.openehr.lang.bmm.bmm_classifier.adoc` §Functions,
//! unredefined by any v2 subtype), where v3 narrows containers to the item
//! type (`…bmm3.bmm_container_type.adoc` §Functions) — this surface is the v2
//! core, so the v2 reading is implemented, with duplicates collapsed per v3's
//! stated logical-set intent (v2 is silent on duplicates,
//! `org.openehr.lang.bmm3.bmm_signature.adoc` §Functions).

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_container_type::BmmContainerType;
use crate::v1_1::bmm::core::bmm_generic_type::BmmGenericType;
use crate::v1_1::bmm::core::bmm_indexed_container_type::BmmIndexedContainerType;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
use crate::v1_1::bmm::core::bmm_type::BmmType;

/// The name of the top `Any` type.
///
/// Used wherever an unconstrained generic
/// parameter has to be reduced to a concrete conformance name
/// (`org.openehr.lang.bmm.bmm_open_type.adoc` §Attributes: the generic
/// constraint "will be 'Any' if nothing set in original model";
/// `org.openehr.lang.bmm3.bmm_definitions.adoc` §Functions `Any_class`:
/// "built-in class definition corresponding to the top `Any` class").
pub const ANY_TYPE_NAME: &str = "Any";

/// Collapses `names` to its first-occurrence-ordered set.
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
    /// `BMM_SIMPLE_TYPE.type_name` (redefined): "Return base_class.type_name"
    /// (`org.openehr.lang.bmm.bmm_simple_type.adoc` §Functions). For the simple
    /// classes this meta-type is defined over
    /// (`org.openehr.lang.bmm3.bmm_simple_type.adoc` §Functions sharpens it to
    /// "Result is `_base_class.name_`") the two coincide.
    #[must_use]
    pub fn type_name(&self) -> String {
        self.base_class.type_name()
    }

    /// `BMM_CLASSIFIER.type_signature` for a simple type: the type name — a
    /// simple type has no generic parameters to constrain
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions: `type_signature`
    /// "Defaults to the value of `_type_name()_`").
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.base_class.type_signature()
    }

    /// `conformance_type_name` for a simple type: "a simple class name"
    /// (`master05-core.adoc` §Semantics §Basics).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.base_class.name().to_owned()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for a simple type: the base class
    /// name (`org.openehr.lang.bmm3.bmm_simple_type.adoc` §Functions: "Result
    /// is `_base_class.name_`").
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        vec![self.base_class.name().to_owned()]
    }
}

impl BmmGenericType {
    /// `BMM_GENERIC_TYPE.type_name` (redefined): "Return the full name of the
    /// type including generic parameters, e.g. `DV_INTERVAL<T>`,
    /// `TABLE<List<THING>,String>`"
    /// (`org.openehr.lang.bmm.bmm_generic_type.adoc` §Functions).
    #[must_use]
    pub fn type_name(&self) -> String {
        self.rendered(BmmType::type_name)
    }

    /// `BMM_GENERIC_TYPE.type_signature` (redefined): the type name with each
    /// generic parameter rendered in signature form, e.g. `Interval<T:Ordered>`
    /// (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.rendered(BmmType::type_signature)
    }

    /// `conformance_type_name` for a generic type: "the _root_ type from a
    /// generic type (e.g. `Interval` from `Interval<T>`)"
    /// (`master05-core.adoc` §Semantics §Basics).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.base_class.name.clone()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for a generic type: "`_base_class.name_`
    /// followed by names of all generic parameter type names, which may be open
    /// or closed" (`org.openehr.lang.bmm3.bmm_generic_type.adoc` §Functions),
    /// each parameter flattened in turn.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        let mut out = vec![self.base_class.name.clone()];
        for parameter in &self.generic_parameters {
            out.extend(parameter.flattened_type_list());
        }
        unique(out)
    }

    /// `Root<p1,p2>` with each actual generic parameter rendered by `render`.
    fn rendered(&self, render: impl Fn(&BmmType) -> String) -> String {
        let root = &self.base_class.name;
        let parameters: Vec<String> = self.generic_parameters.iter().map(render).collect();
        let joined = parameters.join(",");
        format!("{root}<{joined}>")
    }
}

impl BmmContainerType {
    /// `BMM_CONTAINER_TYPE.type_name` (redefined): "Return full type name, e.g.
    /// `List<ELEMENT>`" (`org.openehr.lang.bmm.bmm_container_type.adoc`
    /// §Functions).
    ///
    /// NOTE: `BMM_INDEXED_CONTAINER_TYPE` does not redefine `type_name`
    /// (`org.openehr.lang.bmm.bmm_indexed_container_type.adoc` carries only the
    /// `index_type` attribute), so the two-parameter rendering
    /// `Hash<String,EVENT_ACTION>` is taken from that class's own §Description
    /// ("an indexed container such as `Hash<K,V>` … e.g. `String` in
    /// `Hash<String,EVENT_ACTION>`"), index type first.
    #[must_use]
    pub fn type_name(&self) -> String {
        self.rendered(BmmType::type_name)
    }

    /// `BMM_CLASSIFIER.type_signature` for a container type: the type name with
    /// the contained type in signature form
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions: `type_signature`
    /// "Defaults to the value of `_type_name()_`", refined only where a generic
    /// parameter carries a constrainer).
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.rendered(BmmType::type_signature)
    }

    /// `conformance_type_name` for a container type: "the _contained_ type for
    /// a container type (e.g. `ELEMENT` from the type `List<ELEMENT>`)"
    /// (`master05-core.adoc` §Semantics §Basics).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.base_type().conformance_type_name()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for a container type: the container
    /// class name, the index type (when the container is indexed) and the
    /// flattened contained type — see the module NOTE on the v2/v3 divergence.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmIndexedContainerType(indexed) => indexed.flattened_type_list(),
            Self::BmmContainerType(data) => {
                let mut out = vec![data.container_type.name().to_owned()];
                out.extend(data.base_type.flattened_type_list());
                unique(out)
            }
        }
    }

    /// `BMM_CONTAINER_TYPE.container_type`: "The type of the container"
    /// (`org.openehr.lang.bmm.bmm_container_type.adoc` §Attributes).
    #[must_use]
    pub fn container_type(&self) -> &BmmClass {
        match self {
            Self::BmmIndexedContainerType(indexed) => &indexed.container_type,
            Self::BmmContainerType(data) => &data.container_type,
        }
    }

    /// `BMM_CONTAINER_TYPE.base_type`: "The target type", i.e. the contained
    /// item type (`org.openehr.lang.bmm.bmm_container_type.adoc` §Attributes).
    #[must_use]
    pub fn base_type(&self) -> &BmmType {
        match self {
            Self::BmmIndexedContainerType(indexed) => &indexed.base_type,
            Self::BmmContainerType(data) => &data.base_type,
        }
    }

    /// `Container<item>` (or `Container<index,item>` when indexed) with each
    /// parameter rendered by `render`.
    fn rendered(&self, render: impl Fn(&BmmType) -> String) -> String {
        match self {
            Self::BmmIndexedContainerType(indexed) => indexed.rendered(render),
            Self::BmmContainerType(data) => {
                let container = data.container_type.name();
                let item = render(&data.base_type);
                format!("{container}<{item}>")
            }
        }
    }
}

impl BmmIndexedContainerType {
    /// `BMM_CONTAINER_TYPE.type_name` for an indexed container:
    /// `Hash<String,EVENT_ACTION>` — see the NOTE on
    /// [`BmmContainerType::type_name`] for why the index type comes first.
    #[must_use]
    pub fn type_name(&self) -> String {
        self.rendered(BmmType::type_name)
    }

    /// `BMM_CLASSIFIER.type_signature` for an indexed container: the type name
    /// with its parameters in signature form.
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.rendered(BmmType::type_signature)
    }

    /// `conformance_type_name` for an indexed container: "the _contained_
    /// type" (`master05-core.adoc` §Semantics §Basics), i.e. the VALUE type
    /// `base_type` — not the key type.
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        self.base_type.conformance_type_name()
    }

    /// `BMM_CLASSIFIER.flattened_type_list` for an indexed container: the
    /// container class name, then the flattened index and contained types — see
    /// the module NOTE on the v2/v3 divergence.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        let mut out = vec![self.container_type.name().to_owned()];
        out.extend(self.index_type.flattened_type_list());
        out.extend(self.base_type.flattened_type_list());
        unique(out)
    }

    /// `BMM_INDEXED_CONTAINER_TYPE.index_type`: "The key (index) type of the
    /// container, e.g. `String` in `Hash<String,EVENT_ACTION>`"
    /// (`org.openehr.lang.bmm.bmm_indexed_container_type.adoc` §Attributes).
    #[must_use]
    pub fn index_type_name(&self) -> String {
        self.index_type.type_name()
    }

    /// `Container<index,item>` with both parameters rendered by `render`.
    fn rendered(&self, render: impl Fn(&BmmType) -> String) -> String {
        let container = self.container_type.name();
        let index = self.index_type.type_name();
        let item = render(&self.base_type);
        format!("{container}<{index},{item}>")
    }
}

impl BmmType {
    /// `BMM_CLASSIFIER.type_name`: "Formal string form of the type as per UML"
    /// (`org.openehr.lang.bmm.bmm_classifier.adoc` §Functions), dispatched to
    /// the effective meta-type — see the module docs for the §Basics
    /// definition.
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::BmmContainerType(container) => container.type_name(),
            Self::BmmGenericType(generic) => generic.type_name(),
            Self::BmmOpenType(open) => open.type_name().to_owned(),
            Self::BmmSimpleType(simple) => simple.type_name(),
        }
    }

    /// `BMM_CLASSIFIER.type_signature`: "Signature form of the type, which for
    /// generics includes generic parameter constrainer types e.g.
    /// Interval<T:Ordered>" (`org.openehr.lang.bmm.bmm_classifier.adoc`
    /// §Functions); it "Defaults to the value of `_type_name()_`" for every
    /// meta-type that carries no constrainer
    /// (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn type_signature(&self) -> String {
        match self {
            Self::BmmContainerType(container) => container.type_signature(),
            Self::BmmGenericType(generic) => generic.type_signature(),
            Self::BmmOpenType(open) => open.type_signature(),
            Self::BmmSimpleType(simple) => simple.type_signature(),
        }
    }

    /// `conformance_type_name`: "a reduced form of the type … either a simple
    /// class name, the _contained_ type for a container type (e.g. `ELEMENT`
    /// from the type `List<ELEMENT>`), and the _root_ type from a generic type
    /// (e.g. `Interval` from `Interval<T>`)"
    /// (`LANG/docs/bmm/master05-core.adoc` §Semantics §Basics — the function is
    /// defined there in prose; of the class docs only
    /// `org.openehr.lang.bmm.bmm_open_type.adoc` carries it in a §Functions
    /// table).
    #[must_use]
    pub fn conformance_type_name(&self) -> String {
        match self {
            Self::BmmContainerType(container) => container.conformance_type_name(),
            Self::BmmGenericType(generic) => generic.conformance_type_name(),
            Self::BmmOpenType(open) => open.conformance_type_name(),
            Self::BmmSimpleType(simple) => simple.conformance_type_name(),
        }
    }

    /// `BMM_CLASSIFIER.flattened_type_list`: "Completely flattened list of type
    /// names, flattening out all generic parameters"
    /// (`org.openehr.lang.bmm.bmm_classifier.adoc` §Functions) — e.g.
    /// `Hash<String,Interval<Time>>` flattens to `Hash`, `String`, `Interval`,
    /// `Time`. See the module NOTE for the v2/v3 container divergence.
    #[must_use]
    pub fn flattened_type_list(&self) -> Vec<String> {
        match self {
            Self::BmmContainerType(container) => container.flattened_type_list(),
            Self::BmmGenericType(generic) => generic.flattened_type_list(),
            Self::BmmOpenType(open) => open.flattened_type_list(),
            Self::BmmSimpleType(simple) => simple.flattened_type_list(),
        }
    }

    /// `BMM_TYPE.has_type_substitutions`: "Determine if there are any type
    /// substitutions" (`org.openehr.lang.bmm.bmm_type.adoc` §Functions).
    #[must_use]
    pub fn has_type_substitutions(&self, model: &BmmModel) -> bool {
        !self.type_substitutions(model).is_empty()
    }

    /// `BMM_TYPE.type_substitutions`: "List of type substitutions if any
    /// available for this type within the current BMM model"
    /// (`org.openehr.lang.bmm.bmm_type.adoc` §Functions) — the descendants of
    /// this type's conformance root class, resolved through `model`
    /// ("`_a_desc_type_` has `_an_anc_type_` in its ancestors" is the
    /// conformance relation the substitution set realises,
    /// `org.openehr.lang.bmm.bmm_model.adoc` §Functions `type_conforms_to`).
    ///
    /// NOTE: the class doc's signature takes no argument because it assumes a
    /// live model with back-references; the generated persistence-shaped types
    /// record descendants as names only, so the model is passed in (see
    /// [`BmmClass::all_descendants`]).
    #[must_use]
    pub fn type_substitutions(&self, model: &BmmModel) -> Vec<String> {
        match model.class_definition(&self.conformance_type_name()) {
            Some(class) => class.all_descendants(model),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerType;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerTypeData;
    use crate::v1_1::bmm::core::bmm_generic_class::BmmGenericClass;
    use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
    use crate::v1_1::bmm::core::bmm_generic_type::BmmGenericType;
    use crate::v1_1::bmm::core::bmm_indexed_container_type::BmmIndexedContainerType;
    use crate::v1_1::bmm::core::bmm_open_type::BmmOpenType;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;
    use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
    use crate::v1_1::bmm::core::bmm_type::BmmType;

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

    /// A generic class named `name` with one formal parameter `T`, optionally
    /// constrained.
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
            generic_parameters: [(
                "T".to_owned(),
                BmmGenericParameter {
                    documentation: None,
                    name: "T".to_owned(),
                    conforms_to_type: constraint.map(simple_class),
                    inheritance_precursor: None,
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    /// A `BMM_SIMPLE_TYPE` over the class named `name`.
    fn simple(name: &str) -> BmmType {
        BmmType::BmmSimpleType(BmmSimpleType {
            documentation: None,
            base_class: simple_class(name),
        })
    }

    /// `Root<parameter>` as a `BMM_GENERIC_TYPE`.
    fn generic(root: &str, parameter: BmmType) -> BmmType {
        BmmType::BmmGenericType(BmmGenericType {
            documentation: None,
            generic_parameters: vec![parameter],
            base_class: generic_class(root, None),
        })
    }

    /// `Container<item>` as a `BMM_CONTAINER_TYPE`.
    fn container(container_class: &str, item: BmmType) -> BmmType {
        BmmType::BmmContainerType(Box::new(BmmContainerType::BmmContainerType(
            BmmContainerTypeData {
                documentation: None,
                container_type: simple_class(container_class),
                base_type: Box::new(item),
            },
        )))
    }

    /// `Hash<index,item>` as a `BMM_INDEXED_CONTAINER_TYPE`.
    fn indexed(container_class: &str, index: &str, item: BmmType) -> BmmType {
        BmmType::BmmContainerType(Box::new(BmmContainerType::BmmIndexedContainerType(
            Box::new(BmmIndexedContainerType {
                documentation: None,
                container_type: simple_class(container_class),
                base_type: item,
                index_type: BmmSimpleType {
                    documentation: None,
                    base_class: simple_class(index),
                },
            }),
        )))
    }

    #[test]
    fn type_names_follow_master05_basics() {
        assert_eq!(simple("ELEMENT").type_name(), "ELEMENT");
        assert_eq!(
            container("List", simple("ELEMENT")).type_name(),
            "List<ELEMENT>"
        );
        assert_eq!(
            generic("Interval", simple("Time")).type_name(),
            "Interval<Time>"
        );
        assert_eq!(
            indexed("Hash", "String", simple("EVENT_ACTION")).type_name(),
            "Hash<String,EVENT_ACTION>"
        );
    }

    #[test]
    fn conformance_type_name_reduces_container_and_generic_forms() {
        // master05-core.adoc §Basics: the contained type for a container, the
        // root type for a generic type, the class name for a simple type.
        assert_eq!(simple("ELEMENT").conformance_type_name(), "ELEMENT");
        assert_eq!(
            container("List", simple("ELEMENT")).conformance_type_name(),
            "ELEMENT"
        );
        assert_eq!(
            generic("Interval", simple("Time")).conformance_type_name(),
            "Interval"
        );
    }

    #[test]
    fn flattened_type_list_flattens_nested_generics() {
        let nested = indexed("Hash", "String", generic("Interval", simple("Time")));
        assert_eq!(
            nested.flattened_type_list(),
            [
                "Hash".to_owned(),
                "String".to_owned(),
                "Interval".to_owned(),
                "Time".to_owned()
            ]
        );
    }

    #[test]
    fn flattened_type_list_collapses_duplicates() {
        let hash = indexed("Hash", "String", simple("String"));
        assert_eq!(
            hash.flattened_type_list(),
            ["Hash".to_owned(), "String".to_owned()]
        );
    }

    #[test]
    fn generic_type_signature_carries_the_constrainer() {
        let interval = BmmType::BmmGenericType(BmmGenericType {
            documentation: None,
            generic_parameters: vec![BmmType::BmmOpenType(BmmOpenType {
                documentation: None,
                generic_constraint: BmmGenericParameter {
                    documentation: None,
                    name: "T".to_owned(),
                    conforms_to_type: Some(simple_class("Ordered")),
                    inheritance_precursor: None,
                },
            })],
            base_class: generic_class("Interval", Some("Ordered")),
        });
        assert_eq!(interval.type_name(), "Interval<T>");
        assert_eq!(interval.type_signature(), "Interval<T:Ordered>");
        assert_eq!(interval.conformance_type_name(), "Interval");
        assert_eq!(
            interval.flattened_type_list(),
            ["Interval".to_owned(), "Ordered".to_owned()]
        );
    }
}
