// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_CLASS` (and its `BMM_GENERIC_CLASS`
//! refinement) — the BMM v2 core class-level computed features.
//!
//! Spec: `LANG/docs/bmm/master05-core.adoc` §Semantics — §Basics (the
//! `BMM_CLASSIFIER` naming trio `type_name` / `type_signature` /
//! `conformance_type_name`), §Inheritance ("The evaluation of inheritance
//! relations defined in a BMM schema results in an acyclic graph such that
//! ancestors and descendants can be visualised for any class"), §Classes and
//! Properties ("The features _properties_ and _flat_properties_ defined on
//! `BMM_CLASS` provide access to these two lists for any class") — plus the
//! class definitions `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_class.adoc`
//! §Functions and `…/org.openehr.lang.bmm.bmm_generic_class.adoc` §Functions.
//! Where the v2 prose is terse, the v3 counterpart
//! (`…/org.openehr.lang.bmm3.bmm_class.adoc` §Functions) states the same
//! function with sharpened wording and is cited at the site it settles.
//!
//! NOTE: the persisted BMM graph is upward-complete but downward NOMINAL
//! (`org.openehr.lang.bmm.bmm_class.adoc` §Attributes:
//! `immediate_descendants: List<String>`), so the downward functions and the
//! supplier filter take the owning [`BmmModel`] to resolve names — the class
//! doc's parameterless signatures assume the live back-references the
//! generated persistence shapes deliberately do not carry.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumeration;
use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::core::bmm_package::BmmPackage;
use crate::v1_1::bmm::core::bmm_property::BmmProperty;
use crate::v1_1::bmm::core::bmm_type::BmmType;

/// The `BMM_CLASS` attributes every generated variant carries, projected out of
/// the variant's own struct so the computed features are written once.
struct ClassCommon<'a> {
    /// `BMM_CLASS.name`.
    name: &'a str,
    /// `BMM_CLASS.ancestors` — immediate inheritance parents, embedded whole.
    ancestors: Option<&'a BTreeMap<String, BmmClass>>,
    /// `BMM_CLASS.package`.
    package: &'a BmmPackage,
    /// `BMM_CLASS.properties` — the *differential* property set.
    properties: Option<&'a BTreeMap<String, BmmProperty<BmmType>>>,
    /// `BMM_CLASS.immediate_descendants` — names only.
    immediate_descendants: &'a [String],
    /// `BMM_CLASS.is_abstract`.
    is_abstract: bool,
    /// `BMM_CLASS.is_primitive_type`.
    is_primitive_type: bool,
}

/// Projects one generated `BMM_CLASS`-family struct onto [`ClassCommon`].
macro_rules! class_common {
    ($c:expr) => {
        ClassCommon {
            name: $c.name.as_str(),
            ancestors: $c.ancestors.as_ref(),
            package: &$c.package,
            properties: $c.properties.as_ref(),
            immediate_descendants: $c.immediate_descendants.as_deref().unwrap_or_default(),
            is_abstract: $c.is_abstract,
            is_primitive_type: $c.is_primitive_type,
        }
    };
}

impl BmmClass {
    /// The `BMM_CLASS` attributes shared by every variant.
    fn common(&self) -> ClassCommon<'_> {
        match self {
            Self::BmmEnumeration(BmmEnumeration::BmmEnumerationInteger(c)) => class_common!(c),
            Self::BmmEnumeration(BmmEnumeration::BmmEnumerationString(c)) => class_common!(c),
            Self::BmmEnumeration(BmmEnumeration::BmmEnumeration(c)) => class_common!(c),
            Self::BmmGenericClass(c) => class_common!(c),
            Self::BmmClass(c) => class_common!(c),
        }
    }

    /// `BMM_CLASS.name`: the root name of this class, "even if the class is
    /// generic" (class doc §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        self.common().name
    }

    /// `BMM_CLASS.ancestors`: the immediate inheritance parents, keyed by class
    /// name (class doc §Attributes).
    #[must_use]
    pub fn ancestors(&self) -> Option<&BTreeMap<String, BmmClass>> {
        self.common().ancestors
    }

    /// `BMM_CLASS.package`: the package this class belongs to (class doc
    /// §Attributes).
    #[must_use]
    pub fn package(&self) -> &BmmPackage {
        self.common().package
    }

    /// `BMM_CLASS.properties`: the *differential* property set, i.e. only the
    /// properties this class introduces with respect to its inheritance parents
    /// (`master05-core.adoc` §Classes and Properties). The effective set is
    /// [`Self::flat_properties`].
    #[must_use]
    pub fn properties(&self) -> Option<&BTreeMap<String, BmmProperty<BmmType>>> {
        self.common().properties
    }

    /// `BMM_CLASS.immediate_descendants`: the names of the immediate
    /// inheritance descendants (class doc §Attributes).
    #[must_use]
    pub fn immediate_descendants(&self) -> &[String] {
        self.common().immediate_descendants
    }

    /// `BMM_CLASS.is_abstract`: true if this class is abstract in its model
    /// (class doc §Attributes).
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.common().is_abstract
    }

    /// `BMM_CLASS.is_primitive_type`: true if this class is designated a
    /// primitive type "within the overall type system of the schema" (class doc
    /// §Attributes).
    #[must_use]
    pub fn is_primitive_type(&self) -> bool {
        self.common().is_primitive_type
    }

    /// `BMM_GENERIC_CLASS.generic_parameters`: the formal generic parameter
    /// definitions, keyed by parameter name — `Some` only for a generic class
    /// (`org.openehr.lang.bmm.bmm_generic_class.adoc` §Attributes).
    #[must_use]
    pub fn generic_parameters(&self) -> Option<&BTreeMap<String, BmmGenericParameter>> {
        match self {
            Self::BmmGenericClass(c) => Some(&c.generic_parameters),
            Self::BmmEnumeration(_) | Self::BmmClass(_) => None,
        }
    }

    /// `BMM_CLASS.all_ancestors`: "List of all inheritance parent class names,
    /// recursively" (class doc §Functions).
    ///
    /// Walked over the embedded `ancestors` copies — breadth-first over each
    /// map's (sorted) key order, so the result is deterministic — and deduped:
    /// multiple inheritance ("`DV_INTERVAL<T>` multiply inheriting from
    /// `Interval<T>` and `DATA_VALUE`", `master05-core.adoc` §Inheritance) can
    /// reach the same ancestor by more than one path.
    #[must_use]
    pub fn all_ancestors(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut frontier: Vec<&BmmClass> = vec![self];
        while !frontier.is_empty() {
            let mut next: Vec<&BmmClass> = Vec::new();
            for class in frontier {
                for parent in class.ancestors().into_iter().flat_map(BTreeMap::values) {
                    if seen.insert(parent.name().to_owned()) {
                        out.push(parent.name().to_owned());
                        next.push(parent);
                    }
                }
            }
            frontier = next;
        }
        out
    }

    /// `BMM_CLASS.suppliers`: "List of names of immediate supplier classes,
    /// including concrete generic parameters, concrete descendants of abstract
    /// statically defined types, and inherited suppliers. This list includes
    /// primitive types." (class doc §Functions).
    ///
    /// Composed of the flattened type list of every property type
    /// ([`BmmType::flattened_type_list`]), the conformance constraint of every
    /// formal generic parameter for a generic class
    /// (`org.openehr.lang.bmm.bmm_generic_class.adoc` §Functions: "Add
    /// suppliers from generic parameters"), and the same computed recursively
    /// over the embedded ancestors (the "inherited suppliers" clause). The
    /// result is deduped and sorted.
    ///
    /// NOTE: an unconstrained generic parameter contributes nothing
    /// (`org.openehr.lang.bmm3.bmm_class.adoc` §Functions: "no class name is
    /// added, since logically it would be `Any`"), and the v2
    /// concrete-descendants clause needs the downward edges the persisted
    /// shape records only as names — that expansion is
    /// [`Self::all_descendants`] over a [`BmmModel`], kept out so this stays
    /// a pure function of the class.
    #[must_use]
    pub fn suppliers(&self) -> Vec<String> {
        let mut acc = BTreeSet::new();
        self.collect_suppliers(&mut acc);
        acc.into_iter().collect()
    }

    /// Accumulates this class's own suppliers and those of its ancestors.
    fn collect_suppliers(&self, acc: &mut BTreeSet<String>) {
        for property in self.properties().into_iter().flat_map(BTreeMap::values) {
            acc.extend(property.flattened_type_list());
        }
        for parameter in self
            .generic_parameters()
            .into_iter()
            .flat_map(BTreeMap::values)
        {
            if let Some(constraint) = parameter.flattened_conforms_to_type() {
                acc.insert(constraint.to_owned());
            }
        }
        for parent in self.ancestors().into_iter().flat_map(BTreeMap::values) {
            parent.collect_suppliers(acc);
        }
    }

    /// `BMM_CLASS.suppliers_non_primitive`: "Same as `suppliers` minus
    /// primitive types, as defined in input schema" (class doc §Functions) —
    /// hence the [`BmmModel`] argument: "as defined in input schema" means the
    /// `is_primitive_type` flag on the supplier's own class definition, not a
    /// property of the supplying reference.
    #[must_use]
    pub fn suppliers_non_primitive(&self, model: &BmmModel) -> Vec<String> {
        self.suppliers()
            .into_iter()
            .filter(|name| {
                !model
                    .class_definition(name)
                    .is_some_and(BmmClass::is_primitive_type)
            })
            .collect()
    }

    /// `BMM_CLASS.supplier_closure`: "List of names of all classes in full
    /// supplier closure, including concrete generic parameters. This list
    /// includes primitive types." (class doc §Functions).
    ///
    /// The transitive closure of [`Self::suppliers`], each supplier name
    /// resolved through [`BmmModel::class_definition`]. A name with no
    /// definition in `model` (a type supplied by an included schema that is not
    /// part of this model) terminates that branch. Cycle-safe: a class already
    /// in the closure is never expanded twice, which is what makes the function
    /// total on the recursive type graphs the RM is full of.
    #[must_use]
    pub fn supplier_closure(&self, model: &BmmModel) -> Vec<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = self.suppliers().into();
        while let Some(name) = queue.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(class) = model.class_definition(&name) {
                for supplier in class.suppliers() {
                    if !seen.contains(&supplier) {
                        queue.push_back(supplier);
                    }
                }
            }
        }
        seen.into_iter().collect()
    }

    /// `BMM_CLASS.all_descendants`: "Compute all descendants by following
    /// immediate_descendants" (class doc §Functions).
    ///
    /// Takes the [`BmmModel`] because `immediate_descendants` holds names, not
    /// class references (module NOTE). Cycle-safe and sorted; the inheritance
    /// graph is acyclic by construction (`master05-core.adoc` §Inheritance),
    /// but a malformed schema must not hang the walk.
    #[must_use]
    pub fn all_descendants(&self, model: &BmmModel) -> Vec<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = self.immediate_descendants().iter().cloned().collect();
        while let Some(name) = queue.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(class) = model.class_definition(&name) {
                queue.extend(class.immediate_descendants().iter().cloned());
            }
        }
        seen.into_iter().collect()
    }

    /// `BMM_CLASS.base_class` (redefined): "Main design class for this type,
    /// from which properties etc can be extracted" (class doc §Functions) — a
    /// class IS its own base class.
    #[must_use]
    pub fn base_class(&self) -> &Self {
        self
    }

    /// `BMM_CLASS.package_path`: "Fully qualified package name, of form:
    /// 'package.package'" (class doc §Functions) — i.e.
    /// [`BmmPackage::path`] of the owning package.
    #[must_use]
    pub fn package_path(&self) -> &str {
        self.package().path()
    }

    /// `BMM_CLASS.class_path`: "Fully qualified class name, of form:
    /// 'package.package.CLASS' with package path in lower-case and class in
    /// original case" (class doc §Functions).
    #[must_use]
    pub fn class_path(&self) -> String {
        let package = self.package_path().to_lowercase();
        let name = self.name();
        format!("{package}.{name}")
    }

    /// `BMM_CLASS.flat_properties`: "List of all properties due to current and
    /// ancestor classes, keyed by property name" (class doc §Functions) — the
    /// *effective* (flat) set, "the result of evaluating these lists of
    /// properties down the inheritance hierarchy" (`master05-core.adoc`
    /// §Classes and Properties).
    ///
    /// Ancestors are merged first (recursively, so the deepest ancestor lands
    /// first) and this class's own differential properties last, so a
    /// redefinition in a nearer class wins over the same property name in a
    /// farther one.
    #[must_use]
    pub fn flat_properties(&self) -> BTreeMap<&str, &BmmProperty<BmmType>> {
        let mut out = BTreeMap::new();
        self.merge_flat_properties(&mut out);
        out
    }

    /// Merges the ancestor-then-own property sets into `out`.
    fn merge_flat_properties<'a>(&'a self, out: &mut BTreeMap<&'a str, &'a BmmProperty<BmmType>>) {
        for parent in self.ancestors().into_iter().flat_map(BTreeMap::values) {
            parent.merge_flat_properties(out);
        }
        for property in self.properties().into_iter().flat_map(BTreeMap::values) {
            out.insert(property.name(), property);
        }
    }

    /// `BMM_CLASS.type_name` (redefined): "Formal string form of the type as
    /// per UML" (class doc §Functions) — the class name for a simple class,
    /// and the generic form `Name<T,U>` over the FORMAL parameter names for a
    /// generic class (`master05-core.adoc` §Basics: "for generic and container
    /// classes it will be generic name such as `List<T>`, `Interval<T>`";
    /// `org.openehr.lang.bmm3.bmm_class.adoc` §Description: "Use `type_name()`
    /// to obtain the qualified type name").
    ///
    /// NOTE: the persisted `generic_parameters` is a keyed map, so the formal
    /// declaration order the class doc mandates is not recoverable here —
    /// parameters render in sorted key order, which `Inv_generic_name`
    /// (single upper-case letters) keeps stable and, for conventional
    /// `T`/`U`/`V` naming, identical to declaration order.
    #[must_use]
    pub fn type_name(&self) -> String {
        self.generic_form(|parameter| parameter.name.clone())
    }

    /// `BMM_GENERIC_CLASS.type_signature` (redefined): "Signature form of the
    /// type, which for generics includes generic parameter constrainer types
    /// e.g. Interval<T:Ordered>"
    /// (`org.openehr.lang.bmm.bmm_generic_class.adoc` §Functions). For a
    /// non-generic class this is the class name, i.e.
    /// [`Self::type_name`] (`org.openehr.lang.bmm3.bmm_type.adoc` §Functions:
    /// `type_signature` "Defaults to the value of `type_name()`").
    #[must_use]
    pub fn type_signature(&self) -> String {
        self.generic_form(BmmGenericParameter::type_signature)
    }

    /// The `Name<p1,p2>` rendering of this class, each formal parameter
    /// rendered by `render`; the bare name when the class is not generic.
    ///
    /// Delimiters per `org.openehr.lang.bmm3.bmm_definitions.adoc` §Constants
    /// (`Generic_left_delimiter` `'<'`, `Generic_separator` `','`,
    /// `Generic_right_delimiter` `'>'`).
    fn generic_form(&self, render: impl Fn(&BmmGenericParameter) -> String) -> String {
        match self.generic_parameters() {
            Some(parameters) if !parameters.is_empty() => {
                let rendered: Vec<String> = parameters.values().map(render).collect();
                let name = self.name();
                let joined = rendered.join(",");
                format!("{name}<{joined}>")
            }
            _ => self.name().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_generic_class::BmmGenericClass;
    use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;
    use crate::v1_1::bmm::core::bmm_property::BmmProperty;
    use crate::v1_1::bmm::core::bmm_property::BmmPropertyData;
    use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
    use crate::v1_1::bmm::core::bmm_type::BmmType;

    /// A package node named `name` with no children.
    fn package(name: &str) -> BmmPackage {
        BmmPackage {
            documentation: None,
            packages: None,
            name: name.to_owned(),
            classes: openehr_base::containers::present(Vec::new()),
        }
    }

    /// A `BMM_SIMPLE_CLASS` with the given name, ancestors and properties.
    fn simple_class(
        name: &str,
        ancestors: &[BmmClass],
        properties: &[(&str, BmmType)],
    ) -> BmmClass {
        BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: name.to_owned(),
            ancestors: if ancestors.is_empty() {
                None
            } else {
                Some(
                    ancestors
                        .iter()
                        .map(|a| (a.name().to_owned(), a.clone()))
                        .collect(),
                )
            },
            package: package("org.openehr.rm.test"),
            properties: if properties.is_empty() {
                None
            } else {
                Some(
                    properties
                        .iter()
                        .map(|(prop, ty)| ((*prop).to_owned(), property(prop, ty.clone())))
                        .collect(),
                )
            },
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
        })
    }

    /// A unitary `BMM_PROPERTY` of the given name and type.
    fn property(name: &str, ty: BmmType) -> BmmProperty<BmmType> {
        BmmProperty::BmmProperty(BmmPropertyData {
            documentation: None,
            name: name.to_owned(),
            is_mandatory: None,
            is_computed: None,
            r#type: ty,
            is_im_runtime: None,
            is_im_infrastructure: None,
        })
    }

    /// A `BMM_SIMPLE_TYPE` over a freshly minted simple class named `name`.
    fn simple_type(name: &str) -> BmmType {
        BmmType::BmmSimpleType(BmmSimpleType {
            documentation: None,
            base_class: simple_class(name, &[], &[]),
        })
    }

    /// A generic parameter, optionally constrained to a class named
    /// `conforms_to`.
    fn generic_parameter(name: &str, conforms_to: Option<&str>) -> BmmGenericParameter {
        BmmGenericParameter {
            documentation: None,
            name: name.to_owned(),
            conforms_to_type: conforms_to.map(|c| simple_class(c, &[], &[])),
            inheritance_precursor: None,
        }
    }

    /// A `BMM_GENERIC_CLASS` with the given formal parameters.
    fn generic_class(name: &str, parameters: &[BmmGenericParameter]) -> BmmClass {
        BmmClass::BmmGenericClass(BmmGenericClass {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: package("org.openehr.base.foundation_types"),
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
            generic_parameters: parameters
                .iter()
                .map(|p| (p.name.clone(), p.clone()))
                .collect(),
        })
    }

    #[test]
    fn accessors_read_every_variant() {
        let class = BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: "LOCATABLE".to_owned(),
            ancestors: None,
            package: package("org.openehr.rm.common.archetyped"),
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: Some(vec!["ENTRY".to_owned()]),
            is_abstract: true,
            is_primitive_type: false,
            is_override: false,
        });
        assert_eq!(class.name(), "LOCATABLE");
        assert!(class.is_abstract());
        assert!(!class.is_primitive_type());
        assert_eq!(class.immediate_descendants(), ["ENTRY".to_owned()]);
        assert_eq!(class.ancestors(), None);
        assert_eq!(class.properties(), None);
        assert_eq!(class.generic_parameters(), None);
        assert_eq!(class.base_class(), &class);
    }

    #[test]
    fn all_ancestors_is_transitive_and_deduped() {
        // ANY <- DATA_VALUE <- DV_ORDERED, and ANY <- DV_ORDERED directly:
        // the diamond must yield ANY exactly once.
        let any = simple_class("ANY", &[], &[]);
        let data_value = simple_class("DATA_VALUE", std::slice::from_ref(&any), &[]);
        let dv_ordered = simple_class("DV_ORDERED", &[data_value, any], &[]);
        assert_eq!(
            dv_ordered.all_ancestors(),
            ["ANY".to_owned(), "DATA_VALUE".to_owned()]
        );
    }

    #[test]
    fn flat_properties_lets_the_nearer_class_override() {
        let ancestor = simple_class(
            "DV_ORDERED",
            &[],
            &[
                ("magnitude", simple_type("Real")),
                ("units", simple_type("String")),
            ],
        );
        let class = simple_class(
            "DV_QUANTITY",
            &[ancestor],
            &[("magnitude", simple_type("Integer"))],
        );
        let flat = class.flat_properties();
        assert_eq!(
            flat.keys().copied().collect::<Vec<_>>(),
            ["magnitude", "units"]
        );
        // The differential (own) definition of `magnitude` wins over the
        // ancestor's: master05-core.adoc §Classes and Properties.
        let magnitude = flat.get("magnitude").expect("magnitude is flat-visible");
        assert_eq!(magnitude.conformance_type_name(), "Integer");
    }

    #[test]
    fn suppliers_walks_properties_generic_parameters_and_ancestors() {
        let ancestor = simple_class(
            "DATA_VALUE",
            &[],
            &[("encoding", simple_type("CODE_PHRASE"))],
        );
        let class = simple_class(
            "DV_TEXT",
            &[ancestor],
            &[
                ("value", simple_type("String")),
                ("mappings", simple_type("TERM_MAPPING")),
            ],
        );
        assert_eq!(
            class.suppliers(),
            [
                "CODE_PHRASE".to_owned(),
                "String".to_owned(),
                "TERM_MAPPING".to_owned()
            ]
        );

        // A generic class adds its constrained parameters; an unconstrained
        // one contributes nothing (bmm3.bmm_class.adoc §Functions).
        let interval = generic_class(
            "Interval",
            &[
                generic_parameter("T", Some("Ordered")),
                generic_parameter("U", None),
            ],
        );
        assert_eq!(interval.suppliers(), ["Ordered".to_owned()]);
    }

    #[test]
    fn naming_trio_renders_the_generic_forms() {
        let interval = generic_class("Interval", &[generic_parameter("T", Some("Ordered"))]);
        assert_eq!(interval.type_name(), "Interval<T>");
        assert_eq!(interval.type_signature(), "Interval<T:Ordered>");

        let hash = generic_class(
            "Hash",
            &[
                generic_parameter("K", Some("Ordered")),
                generic_parameter("V", None),
            ],
        );
        assert_eq!(hash.type_name(), "Hash<K,V>");
        assert_eq!(hash.type_signature(), "Hash<K:Ordered,V>");

        let simple = simple_class("ELEMENT", &[], &[]);
        assert_eq!(simple.type_name(), "ELEMENT");
        assert_eq!(simple.type_signature(), "ELEMENT");
    }

    #[test]
    fn package_and_class_paths_lower_case_only_the_package() {
        let class = simple_class("DV_QUANTITY", &[], &[]);
        assert_eq!(class.package_path(), "org.openehr.rm.test");
        assert_eq!(class.class_path(), "org.openehr.rm.test.DV_QUANTITY");

        let mixed = BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: "EHR_STATUS".to_owned(),
            ancestors: None,
            package: package("Org.OpenEHR.RM.EHR"),
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
        });
        assert_eq!(mixed.class_path(), "org.openehr.rm.ehr.EHR_STATUS");
    }

    #[test]
    fn generic_parameters_are_visible_only_on_a_generic_class() {
        let interval = generic_class("Interval", &[generic_parameter("T", Some("Ordered"))]);
        let parameters = interval
            .generic_parameters()
            .expect("a generic class carries formal parameters");
        assert_eq!(parameters.keys().collect::<Vec<_>>(), ["T"]);
        assert_eq!(
            simple_class("ELEMENT", &[], &[]).generic_parameters(),
            None::<&BTreeMap<String, BmmGenericParameter>>
        );
    }
}
