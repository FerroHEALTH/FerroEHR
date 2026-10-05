// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_MODEL` (plus the `schema_id` it inherits
//! from `BMM_SCHEMA_CORE`) — the model-level lookups and the type-conformance
//! relation.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_model.adoc` §Functions
//! (`primitive_types`, `enumeration_types`, `class_definition`,
//! `enumeration_definition`, `property_definition`,
//! `ms_conformant_property_type`, `property_definition_at_path`,
//! `all_ancestor_classes`, `type_conforms_to`),
//! `…bmm.bmm_schema_core.adoc` §Functions (`schema_id`) and
//! `…bmm.bmm_package_container.adoc` §Functions (the three package-container
//! functions, implemented once in
//! [`crate::v1_1::bmm::core::bmm_package_impl`]), read against
//! `LANG/docs/bmm/master05-core.adoc` §Semantics — §Classes and Types (the
//! class/type distinction the lookups turn on) and §Inheritance (the acyclic
//! ancestor graph `all_ancestor_classes` walks).
//!
//! The v3 counterpart `…bmm3.bmm_model.adoc` §Functions carries the same
//! function set with the conformance rules written out explicitly, and is cited
//! at each site where it settles v2 wording.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumeration;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::core::bmm_package::BmmPackage;
use crate::v1_1::bmm::core::bmm_package_impl::do_recursive_packages_in;
use crate::v1_1::bmm::core::bmm_package_impl::package_at_path_in;
use crate::v1_1::bmm::core::bmm_property::BmmProperty;
use crate::v1_1::bmm::core::bmm_schema_core::BmmSchemaCore;
use crate::v1_1::bmm::core::bmm_type::BmmType;
use crate::v1_1::bmm::core::bmm_type_impl::ANY_TYPE_NAME;
use crate::v1_1::bmm_persistence::p_bmm_schema_impl::compose_schema_id;
use openehr_base::containers::present;

/// The delimiter separating the segments of the property path
/// `BMM_MODEL.property_definition_at_path` navigates.
///
/// NOTE: no openEHR spec governs this — our own design/extension. The class doc
/// (`org.openehr.lang.bmm.bmm_model.adoc` §Functions) names the argument
/// `a_property_path` without defining its lexis, and `BMM_DEFINITIONS`
/// (`…bmm.bmm_definitions.adoc` §Constants) defines delimiters only for schema
/// ids (`"::"`) and package paths (`"."`). `'/'` is the separator openEHR paths
/// use everywhere else, so it is what this surface accepts.
const PROPERTY_PATH_DELIMITER: char = '/';

/// The root name of a type reference, i.e. the type name with any generic part
/// removed: `class_definition` is specified for a name "which may contain a
/// generic part" (`org.openehr.lang.bmm.bmm_model.adoc` §Functions).
fn type_root(a_type_name: &str) -> &str {
    a_type_name.split('<').next().unwrap_or(a_type_name).trim()
}

/// Splits a type name into its root and its top-level generic parameters:
/// `Hash<String,Interval<Time>>` → `("Hash", ["String", "Interval<Time>"])`.
///
/// Delimiters per `org.openehr.lang.bmm3.bmm_definitions.adoc` §Constants
/// (`Generic_left_delimiter` `'<'`, `Generic_separator` `','`,
/// `Generic_right_delimiter` `'>'`).
fn split_type(a_type_name: &str) -> (&str, Vec<&str>) {
    let trimmed = a_type_name.trim();
    let (Some(open), Some(close)) = (trimmed.find('<'), trimmed.rfind('>')) else {
        return (trimmed, Vec::new());
    };
    let (Some(root), Some(inner)) = (trimmed.get(..open), trimmed.get(open + 1..close)) else {
        return (trimmed, Vec::new());
    };
    (root.trim(), split_generic_parameters(inner))
}

/// Splits a generic parameter list on its TOP-LEVEL separators, so a nested
/// generic parameter stays one item.
fn split_generic_parameters(inner: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth: usize = 0;
    let mut start: usize = 0;
    for (index, character) in inner.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if let Some(part) = inner.get(start..index) {
                    out.push(part.trim());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if let Some(part) = inner.get(start..) {
        let part = part.trim();
        if !part.is_empty() {
            out.push(part);
        }
    }
    out
}

/// Whether `a_type_name` is an OPEN generic parameter name rather than a class
/// name: `BMM_GENERIC_PARAMETER` invariant `Inv_generic_name`
/// (`org.openehr.lang.bmm.bmm_generic_parameter.adoc` §Invariants) pins the
/// lexis exactly — `name.count = 1 and name.is_upper`.
fn is_open_parameter_name(a_type_name: &str) -> bool {
    let mut characters = a_type_name.chars();
    match (characters.next(), characters.next()) {
        (Some(single), None) => single.is_uppercase(),
        _ => false,
    }
}

impl BmmModel {
    /// `BMM_SCHEMA_CORE.schema_id`: "Derived name of schema, based on model
    /// publisher, model name, model release"
    /// (`org.openehr.lang.bmm.bmm_schema_core.adoc` §Functions).
    ///
    /// Rendered `<rm_publisher>_<schema_name>_<rm_release>`, lower-cased.
    ///
    /// NOTE: the v2 function states the inputs but not the join; the v3
    /// counterpart's explicit template and all five examples agree on `'_'`
    /// (`org.openehr.lang.bmm3.bmm_model.adoc` §Functions `model_id`:
    /// `<rm_publisher>_<model_name>_<rm_release>`), so the underscore join
    /// wins and `create_schema_id`'s lone `'-'` prose is read as an editorial
    /// slip.
    #[must_use]
    pub fn schema_id(&self) -> String {
        compose_schema_id(&self.rm_publisher, &self.schema_name, &self.rm_release)
    }

    /// `BMM_MODEL.class_definition`: "Retrieve the class definition
    /// corresponding to `a_type_name` (which may contain a generic part)"
    /// (class doc §Functions) — the generic part is stripped before the lookup,
    /// because `class_definitions` is keyed by class name and "names of classes
    /// are just the root name, even if the class is generic"
    /// (`org.openehr.lang.bmm.bmm_class.adoc` §Attributes).
    ///
    /// Matching is CASE-INSENSITIVE with underscores significant —
    /// `LANG/docs/bmm3/master05-core-model.adoc` §Naming Convention: "When
    /// used computationally within an instantiated BMM model, it is assumed
    /// that case-insensitive matching is used. This means that the class name
    /// `"Hashable"` refers to the same class as `"HASHABLE"`. Note however
    /// that underscores are not removed during matching". An exact-key hit
    /// wins without a scan; the fold never rewrites the returned class's own
    /// name.
    #[must_use]
    pub fn class_definition(&self, a_type_name: &str) -> Option<&BmmClass> {
        let definitions = self.class_definitions.as_ref()?;
        let root = type_root(a_type_name);
        if let Some(class) = definitions.get(root) {
            return Some(class);
        }
        let folded = root.to_uppercase();
        definitions
            .iter()
            .find(|(key, _)| key.to_uppercase() == folded)
            .map(|(_, class)| class)
    }

    /// `BMM_MODEL.enumeration_definition`: "Retrieve the enumeration definition
    /// corresponding to `a_type_name`" (class doc §Functions) — i.e. the class
    /// definition when, and only when, it is a `BMM_ENUMERATION`.
    #[must_use]
    pub fn enumeration_definition(&self, a_type_name: &str) -> Option<&BmmEnumeration> {
        match self.class_definition(a_type_name) {
            Some(BmmClass::BmmEnumeration(enumeration)) => Some(enumeration),
            _ => None,
        }
    }

    /// `BMM_MODEL.primitive_types`: "List of keys in `class_definitions` of
    /// items marked as primitive types, as defined in input schema" (class doc
    /// §Functions).
    #[must_use]
    pub fn primitive_types(&self) -> Vec<&str> {
        self.class_definitions
            .iter()
            .flatten()
            .filter(|(_, class)| class.is_primitive_type())
            .map(|(key, _)| key.as_str())
            .collect()
    }

    /// `BMM_MODEL.enumeration_types`: "List of keys in `class_definitions` of
    /// items that are enumeration types, as defined in input schema" (class doc
    /// §Functions).
    #[must_use]
    pub fn enumeration_types(&self) -> Vec<&str> {
        self.class_definitions
            .iter()
            .flatten()
            .filter(|(_, class)| matches!(class, BmmClass::BmmEnumeration(_)))
            .map(|(key, _)| key.as_str())
            .collect()
    }

    /// `BMM_MODEL.all_ancestor_classes`: "Return all ancestor types of
    /// `a_class_name` up to root class (usually 'ANY', 'Object' or something
    /// similar). Does not include current class. Returns empty list if none."
    /// (class doc §Functions).
    ///
    /// The walk unions two sources so it is total on both persisted shapes: the
    /// embedded ancestor copies a class carries
    /// ([`BmmClass::all_ancestors`]) and, for every name reached, that name's
    /// own definition in this model — a class whose embedded copies are shallow
    /// still resolves all the way up. Cycle-safe; deduped.
    ///
    /// The `Any` top is implicit: "The `Any` type defined by the model's
    /// `Any` class, or else the default one … will be used as the inheritance
    /// parent for every class in the model that doesn't have any other
    /// inheritance parent. As a result, the inheritance graph will always
    /// have the `Any` type as its top node"
    /// (`LANG/docs/bmm3/master05-core-model.adoc` §The Any Class and Type) —
    /// so a defined, parentless class (other than `Any` itself) closes its
    /// ancestor list with `Any`.
    #[must_use]
    pub fn all_ancestor_classes(&self, a_class: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut rootless = false;
        if let Some(class) = self.class_definition(a_class) {
            let parents = class.all_ancestors();
            if parents.is_empty() && !class.name().eq_ignore_ascii_case(ANY_TYPE_NAME) {
                rootless = true;
            }
            queue.extend(parents);
        }
        while let Some(name) = queue.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            out.push(name.clone());
            if let Some(class) = self.class_definition(&name) {
                let parents = class.all_ancestors();
                if parents.is_empty() && !class.name().eq_ignore_ascii_case(ANY_TYPE_NAME) {
                    rootless = true;
                }
                for parent in parents {
                    if !seen.contains(&parent) {
                        queue.push_back(parent);
                    }
                }
            }
        }
        if rootless
            && !seen
                .iter()
                .any(|name| name.eq_ignore_ascii_case(ANY_TYPE_NAME))
        {
            out.push(ANY_TYPE_NAME.to_owned());
        }
        out
    }

    /// `BMM_MODEL.any_class_definition`: the model's own `Any` class where it
    /// defines one, else the standard default — "A BMM model may define its
    /// own `Any` class, but if it does not, the `BMM_MODEL` instance
    /// representing the model will produce a standard 'Any' class"
    /// (`LANG/docs/bmm3/master05-core-model.adoc` §The Any Class and Type).
    /// The default is abstract, parentless and property-free, in a package
    /// named after the delimiter-free model root (the section's default
    /// package structure), and is generated fresh per call — it is never
    /// inserted into `class_definitions`.
    #[must_use]
    pub fn any_class_definition(&self) -> BmmClass {
        if let Some(own) = self.class_definition(ANY_TYPE_NAME) {
            return own.clone();
        }
        BmmClass::BmmClass(crate::v1_1::bmm::core::bmm_class::BmmClassData {
            documentation: Some(
                "Standard default Any class (LANG/docs/bmm3/master05-core-model.adoc §The Any \
                     Class and Type)"
                    .to_owned(),
            ),
            name: ANY_TYPE_NAME.to_owned(),
            ancestors: None,
            package: BmmPackage {
                documentation: None,
                packages: None,
                name: self.schema_name.clone(),
                classes: present(Vec::new()),
            },
            properties: None,
            source_schema_id: self.schema_id(),
            immediate_descendants: present(Vec::new()),
            is_abstract: true,
            is_primitive_type: false,
            is_override: false,
        })
    }

    /// `BMM_MODEL.property_definition`: "Retrieve the property definition for
    /// `a_prop_name` in flattened class corresponding to `a_type_name`" (class
    /// doc §Functions) — the FLAT property set, so an inherited property
    /// resolves too ("the _effective_ set of properties for a class is the
    /// result of evaluating these lists of properties down the inheritance
    /// hierarchy", `master05-core.adoc` §Semantics §Classes and Properties).
    #[must_use]
    pub fn property_definition(
        &self,
        a_type_name: &str,
        a_prop_name: &str,
    ) -> Option<&BmmProperty<BmmType>> {
        self.flat_properties(a_type_name)?.get(a_prop_name).copied()
    }

    /// The flat property set of the class named `a_type_name`, flattened DOWN
    /// THE MODEL: at every inheritance step the ancestor's own definition in
    /// `class_definitions` is preferred over the copy embedded in the
    /// descendant's `ancestors` map.
    ///
    /// NOTE: the class-level [`BmmClass::flat_properties`] sees only what the
    /// class embeds, and shallow ancestor copies would hide inherited
    /// properties — so the model-level flattening consults
    /// `BMM_MODEL.class_definitions` ("All classes in this schema",
    /// `org.openehr.lang.bmm.bmm_model.adoc` §Attributes): a strict superset
    /// with the same nearer-class-wins precedence, cycle-safe.
    fn flat_properties(&self, a_type_name: &str) -> Option<BTreeMap<&str, &BmmProperty<BmmType>>> {
        let class = self.class_definition(a_type_name)?;
        let mut out = BTreeMap::new();
        let mut seen = BTreeSet::new();
        self.merge_flat_properties(class, &mut out, &mut seen);
        Some(out)
    }

    /// Merges `class`'s model-resolved ancestor properties, then its own, into
    /// `out`.
    fn merge_flat_properties<'a>(
        &'a self,
        class: &'a BmmClass,
        out: &mut BTreeMap<&'a str, &'a BmmProperty<BmmType>>,
        seen: &mut BTreeSet<String>,
    ) {
        if !seen.insert(class.name().to_owned()) {
            return;
        }
        for embedded in class.ancestors().into_iter().flatten().map(|(_, a)| a) {
            let parent = match self.class_definition(embedded.name()) {
                Some(defined) => defined,
                None => embedded,
            };
            self.merge_flat_properties(parent, out, seen);
        }
        for property in class.properties().into_iter().flatten().map(|(_, p)| p) {
            out.insert(property.name(), property);
        }
    }

    /// `BMM_MODEL.property_definition_at_path`: "Retrieve the property
    /// definition for `a_property_path` in flattened class corresponding to
    /// `a_type_name`" (class doc §Functions).
    ///
    /// Each segment is resolved on the flat property set of the previous
    /// segment's type, reduced with `conformance_type_name` so a container step
    /// navigates into the CONTAINED type ("the _contained_ type for a container
    /// type (e.g. `ELEMENT` from the type `List<ELEMENT>`)",
    /// `master05-core.adoc` §Semantics §Basics). See
    /// the module-level `PROPERTY_PATH_DELIMITER` note for the delimiter
    /// adjudication. An empty
    /// path, or a segment that does not resolve, yields `None`.
    #[must_use]
    pub fn property_definition_at_path(
        &self,
        a_type_name: &str,
        a_property_path: &str,
    ) -> Option<&BmmProperty<BmmType>> {
        let mut current_type = a_type_name.to_owned();
        let mut found: Option<&BmmProperty<BmmType>> = None;
        for segment in a_property_path
            .split(PROPERTY_PATH_DELIMITER)
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
        {
            let property = self.property_definition(&current_type, segment)?;
            current_type = property.conformance_type_name();
            found = Some(property);
        }
        found
    }

    /// `BMM_MODEL.type_conforms_to`: "Check conformance of `a_desc_type` to
    /// `an_anc_type`; the types may be generic, and may contain open generic
    /// parameters like 'T' etc. These are replaced with their appropriate
    /// constrainer types, or Any during the conformance testing process."
    /// (class doc §Functions).
    ///
    /// The three rules are stated verbatim in the v3 counterpart
    /// (`org.openehr.lang.bmm3.bmm_model.adoc` §Functions) and implemented
    /// exactly as written:
    ///
    /// * "[base class test] types are non-generic, and either type names are
    ///   identical, or else `_a_desc_type_` has `_an_anc_type_` in its
    ///   ancestors";
    /// * "both types are generic and pass base class test; number of generic
    ///   params matches, and each generic parameter type, after 'open parameter'
    ///   substitution, recursively passes";
    /// * "descendant type is generic and ancestor type is not, and they pass
    ///   base classes test".
    ///
    /// A non-generic descendant against a generic ancestor is therefore NOT
    /// conformant — the rule set admits generic parameters only on the
    /// descendant side. Deliberate: the published algorithm's final case
    /// returns exactly `not valid_generic_type_name(anc_type)`
    /// (`master06-core-types.adoc` §Type Conformance), so a simple class
    /// closing a generic ancestor (`Multiplicity_interval` against
    /// `Interval<Ordered>`) has NO published substitution rule; the gap is
    /// reported upstream, and this stays the faithful reading until a rule
    /// is published.
    ///
    /// An open ancestor parameter is "replaced with their appropriate
    /// constrainer types, or Any": the constrainer is resolved POSITIONALLY
    /// from the ancestor root's own generic-parameter declarations
    /// (`BMM_GENERIC_CLASS.generic_parameter_conformance_type` —
    /// `org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions;
    /// `master06-core-types.adoc` §Type Conformance's
    /// `generic_parameter_conformance_type` step), and `Any` remains only the
    /// genuinely-unconstrained fallback
    /// (`org.openehr.lang.bmm3.bmm_definitions.adoc` §Functions `Any_class`).
    #[must_use]
    pub fn type_conforms_to(&self, a_desc_type: &str, an_anc_type: &str) -> bool {
        let (descendant_root, descendant_parameters) = split_type(a_desc_type);
        let (ancestor_root, ancestor_parameters) = split_type(an_anc_type);
        if !self.base_class_conforms_to(descendant_root, ancestor_root) {
            return false;
        }
        if ancestor_parameters.is_empty() {
            // Rules 1 and 3: both non-generic, or the descendant alone is
            // generic — the base class test is the whole test.
            return true;
        }
        if descendant_parameters.len() != ancestor_parameters.len() {
            return false;
        }
        descendant_parameters
            .iter()
            .zip(ancestor_parameters.iter())
            .enumerate()
            .all(|(position, (descendant, ancestor))| {
                let target = if is_open_parameter_name(ancestor) {
                    self.generic_parameter_conformance_type(ancestor_root, position)
                } else {
                    (*ancestor).to_owned()
                };
                self.type_conforms_to(descendant, &target)
            })
    }

    /// `BMM_GENERIC_CLASS.generic_parameter_conformance_type` for the
    /// parameter at `position` of the class named `a_class_name`: the
    /// parameter's ultimate conformance constraint, else `Any`
    /// (`org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions;
    /// `bmm_generic_parameter.adoc` `effective_conforms_to_type`).
    ///
    /// NOTE: the generated `generic_parameters` map is name-keyed and
    /// declaration order is lost (the sorted-map deviation recorded at
    /// [`BmmClass::type_name`]); single-letter upper-case parameter names make
    /// the sorted order the declaration order in practice.
    fn generic_parameter_conformance_type(&self, a_class_name: &str, position: usize) -> String {
        self.class_definition(a_class_name)
            .and_then(BmmClass::generic_parameters)
            .and_then(|parameters| parameters.values().nth(position))
            .map_or_else(
                || ANY_TYPE_NAME.to_owned(),
                crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter::effective_conforms_to_type_name,
            )
    }

    /// The "base class test" of `BMM_MODEL.type_conforms_to`: identical names,
    /// or the ancestor among the descendant's ancestors, with an open
    /// DESCENDANT parameter substituted by `Any` (a bare descendant-side `T`
    /// has no owner in scope — the class doc's stated fallback). Name
    /// comparison is case-insensitive per §6.6's own algorithm
    /// (`master06-core-types.adoc` §Type Conformance:
    /// `base_class.is_case_insensitive_equal (anc_base_class)`) and the §5.2
    /// naming convention (underscores significant).
    fn base_class_conforms_to(&self, descendant_root: &str, ancestor_root: &str) -> bool {
        let descendant = substitute_open_parameter(descendant_root);
        let ancestor = substitute_open_parameter(ancestor_root);
        if ancestor.eq_ignore_ascii_case(ANY_TYPE_NAME) || descendant.eq_ignore_ascii_case(ancestor)
        {
            return true;
        }
        self.all_ancestor_classes(descendant)
            .iter()
            .any(|name| name.eq_ignore_ascii_case(ancestor))
    }

    /// `BMM_MODEL.ms_conformant_property_type`: "True if `a_ms_property_type` is
    /// a valid 'MS' dynamic type for `a_property` in BMM type `a_bmm_type_name`.
    /// 'MS' conformance means 'model-semantic' conformance, which abstracts away
    /// container types like List<>, Set<> etc and compares the dynamic type with
    /// the relation target type in the UML sense, i.e. regardless of whether
    /// there is single or multiple containment." (class doc §Functions).
    ///
    /// The container abstraction is exactly `conformance_type_name` on the
    /// property's declared type (`master05-core.adoc` §Semantics §Basics: "the
    /// _contained_ type for a container type"); the supplied dynamic type is
    /// then tested against that target with
    /// [`Self::type_conforms_to`]. An unknown type or property is not
    /// conformant.
    #[must_use]
    pub fn ms_conformant_property_type(
        &self,
        a_bmm_type_name: &str,
        a_bmm_property_name: &str,
        a_ms_property_type: &str,
    ) -> bool {
        let Some(property) = self.property_definition(a_bmm_type_name, a_bmm_property_name) else {
            return false;
        };
        self.type_conforms_to(a_ms_property_type, &property.conformance_type_name())
    }

    /// `BMM_PACKAGE_CONTAINER.package_at_path`: "Package at the path `a_path`"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    ///
    /// Keys are matched case-insensitively and the longest matching key prefix
    /// wins at each level, per
    /// [`crate::v1_1::bmm::core::bmm_package_impl`].
    #[must_use]
    pub fn package_at_path(&self, a_path: &str) -> Option<&BmmPackage> {
        package_at_path_in(self.packages.as_ref(), a_path)
    }

    /// `BMM_PACKAGE_CONTAINER.has_package_path`: "True if there is a package at
    /// the path `a_path`; paths are delimited with Package_name_delimiter"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    #[must_use]
    pub fn has_package_path(&self, a_path: &str) -> bool {
        self.package_at_path(a_path).is_some()
    }

    /// `BMM_PACKAGE_CONTAINER.do_recursive_packages`: "Recursively execute
    /// `action`, which is a procedure taking a BMM_PACKAGE argument, on all
    /// members of packages"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    pub fn do_recursive_packages(&self, action: &mut dyn FnMut(&BmmPackage)) {
        do_recursive_packages_in(self.packages.as_ref(), action);
    }
}

impl BmmSchemaCore {
    /// `BMM_SCHEMA_CORE.schema_id`: "Derived name of schema, based on model
    /// publisher, model name, model release"
    /// (`org.openehr.lang.bmm.bmm_schema_core.adoc` §Functions), answered by
    /// whichever descendant this slot carries.
    ///
    /// The join is the one adjudicated on [`BmmModel::schema_id`].
    #[must_use]
    pub fn schema_id(&self) -> String {
        match self {
            Self::BmmModel(model) => model.schema_id(),
            Self::PBmmSchema(schema) => schema.schema_id(),
            Self::BmmSchemaCore(core) => {
                compose_schema_id(&core.rm_publisher, &core.schema_name, &core.rm_release)
            }
        }
    }
}

/// Replaces an open generic parameter name with `Any`, per the
/// `type_conforms_to` NOTE on [`BmmModel::type_conforms_to`].
fn substitute_open_parameter(a_type_name: &str) -> &str {
    if is_open_parameter_name(a_type_name) {
        ANY_TYPE_NAME
    } else {
        a_type_name
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerType;
    use crate::v1_1::bmm::core::bmm_container_type::BmmContainerTypeData;
    use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumeration;
    use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumerationData;
    use crate::v1_1::bmm::core::bmm_generic_class::BmmGenericClass;
    use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
    use crate::v1_1::bmm::core::bmm_model::BmmModel;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;
    use crate::v1_1::bmm::core::bmm_property::BmmProperty;
    use crate::v1_1::bmm::core::bmm_property::BmmPropertyData;
    use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
    use crate::v1_1::bmm::core::bmm_type::BmmType;

    /// An empty package node named `name`.
    fn package(name: &str) -> BmmPackage {
        BmmPackage {
            documentation: None,
            packages: None,
            name: name.to_owned(),
            classes: openehr_base::containers::present(Vec::new()),
        }
    }

    /// A unitary property of the given name over the simple type `type_name`.
    fn property(name: &str, type_name: &str) -> BmmProperty<BmmType> {
        BmmProperty::BmmProperty(BmmPropertyData {
            documentation: None,
            name: name.to_owned(),
            is_mandatory: Some(true),
            is_computed: None,
            r#type: BmmType::BmmSimpleType(BmmSimpleType {
                documentation: None,
                base_class: class(type_name, &[], &[], false),
            }),
            is_im_runtime: None,
            is_im_infrastructure: None,
        })
    }

    /// A `List<item>` container property.
    fn container_property(name: &str, item: &str) -> BmmProperty<BmmType> {
        BmmProperty::BmmProperty(BmmPropertyData {
            documentation: None,
            name: name.to_owned(),
            is_mandatory: Some(true),
            is_computed: None,
            r#type: BmmType::BmmContainerType(Box::new(BmmContainerType::BmmContainerType(
                BmmContainerTypeData {
                    documentation: None,
                    container_type: class("List", &[], &[], false),
                    base_type: Box::new(BmmType::BmmSimpleType(BmmSimpleType {
                        documentation: None,
                        base_class: class(item, &[], &[], false),
                    })),
                },
            ))),
            is_im_runtime: None,
            is_im_infrastructure: None,
        })
    }

    /// A simple class with ancestors (by name only, embedded as shallow copies),
    /// properties and a primitive flag.
    fn class(
        name: &str,
        ancestors: &[&str],
        properties: &[BmmProperty<BmmType>],
        is_primitive_type: bool,
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
                        .map(|parent| ((*parent).to_owned(), class(parent, &[], &[], false)))
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
                        .map(|p| (p.name().to_owned(), p.clone()))
                        .collect(),
                )
            },
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type,
            is_override: false,
        })
    }

    /// A model over the given class definitions and top-level packages.
    fn model(classes: Vec<BmmClass>, packages: Vec<BmmPackage>) -> BmmModel {
        BmmModel {
            rm_publisher: "openEHR".to_owned(),
            rm_release: "1.2.0".to_owned(),
            schema_name: "RM".to_owned(),
            schema_revision: "1.2.0".to_owned(),
            schema_lifecycle_state: "stable".to_owned(),
            schema_author: "openEHR SEC".to_owned(),
            schema_description: "test schema".to_owned(),
            schema_contributors: openehr_base::containers::present(Vec::new()),
            archetype_parent_class: None,
            archetype_data_value_parent_class: None,
            archetype_rm_closure_packages: openehr_base::containers::present(Vec::new()),
            archetype_visualise_descendants_of: None,
            documentation: None,
            packages: if packages.is_empty() {
                None
            } else {
                Some(
                    packages
                        .into_iter()
                        .map(|p| (p.name.to_uppercase(), p))
                        .collect(),
                )
            },
            class_definitions: if classes.is_empty() {
                None
            } else {
                Some(
                    classes
                        .into_iter()
                        .map(|c| (c.name().to_owned(), c))
                        .collect(),
                )
            },
        }
    }

    #[test]
    fn schema_id_is_the_lower_cased_publisher_name_release() {
        let model = model(Vec::new(), Vec::new());
        assert_eq!(model.schema_id(), "openehr_rm_1.2.0");
    }

    /// The `BMM_SCHEMA_CORE` slot derives the SAME id from every descendant it
    /// can carry, and from its own least-rich form — the three-part derivation
    /// is the class's, not `BMM_MODEL`'s.
    #[test]
    fn the_schema_core_slot_derives_one_id_for_every_form() {
        use crate::v1_1::bmm::core::bmm_schema_core::BmmSchemaCore;
        use crate::v1_1::bmm::core::bmm_schema_core::BmmSchemaCoreData;

        let core = BmmSchemaCoreData {
            rm_publisher: "openEHR".to_owned(),
            rm_release: "1.2.0".to_owned(),
            schema_name: "RM".to_owned(),
            schema_revision: "1.2.0".to_owned(),
            schema_lifecycle_state: "stable".to_owned(),
            schema_author: "openEHR SEC".to_owned(),
            schema_description: "test schema".to_owned(),
            schema_contributors: openehr_base::containers::present(Vec::new()),
            archetype_parent_class: None,
            archetype_data_value_parent_class: None,
            archetype_rm_closure_packages: openehr_base::containers::present(Vec::new()),
            archetype_visualise_descendants_of: None,
        };
        assert_eq!(
            BmmSchemaCore::BmmSchemaCore(core).schema_id(),
            "openehr_rm_1.2.0"
        );
        assert_eq!(
            BmmSchemaCore::BmmModel(model(Vec::new(), Vec::new())).schema_id(),
            "openehr_rm_1.2.0"
        );
    }

    #[test]
    fn class_definition_strips_the_generic_part() {
        let model = model(vec![class("Interval", &[], &[], false)], Vec::new());
        assert_eq!(
            model.class_definition("Interval<Time>").map(BmmClass::name),
            Some("Interval")
        );
        assert_eq!(
            model.class_definition("Interval").map(BmmClass::name),
            Some("Interval")
        );
        assert_eq!(model.class_definition("MISSING"), None);
    }

    /// `LANG/docs/bmm3/master05-core-model.adoc` §Naming Convention:
    /// case-insensitive matching, underscores significant — `"Hashable"` ≡
    /// `"HASHABLE"`, but `"HashMap"` ≢ `"HASH_MAP"`.
    #[test]
    fn class_lookup_is_case_insensitive_with_significant_underscores() {
        let model = model(
            vec![
                class("Hashable", &[], &[], false),
                class("HashMap", &[], &[], false),
            ],
            Vec::new(),
        );
        assert_eq!(
            model.class_definition("HASHABLE").map(BmmClass::name),
            Some("Hashable"),
            "case folds"
        );
        assert_eq!(
            model.class_definition("hashable").map(BmmClass::name),
            Some("Hashable")
        );
        assert_eq!(
            model.class_definition("HASH_MAP"),
            None,
            "underscores are not removed during matching"
        );
    }

    /// `LANG/docs/bmm3/master05-core-model.adoc` §The Any Class and Type: a
    /// model without its own `Any` gets the standard default; a defined,
    /// parentless class closes its ancestor list with `Any`; `Any` itself
    /// does not.
    #[test]
    fn any_class_semantics() {
        let model = model(
            vec![
                class("ITEM", &[], &[], false),
                class("Any", &[], &[], false),
            ],
            Vec::new(),
        );
        // the model defines its own Any → returned as-is.
        assert_eq!(model.any_class_definition().name(), "Any");
        assert_eq!(model.all_ancestor_classes("ITEM"), vec!["Any".to_owned()]);
        assert!(model.all_ancestor_classes("Any").is_empty());

        // no own Any → the standard default (abstract, parentless).
        let bare = super::super::bmm_model_impl::tests::model(
            vec![class("ITEM", &[], &[], false)],
            Vec::new(),
        );
        let default_any = bare.any_class_definition();
        assert_eq!(default_any.name(), "Any");
        assert!(default_any.is_abstract());
        assert_eq!(bare.all_ancestor_classes("ITEM"), vec!["Any".to_owned()]);
    }

    #[test]
    fn primitive_and_enumeration_type_keys() {
        let enumeration =
            BmmClass::BmmEnumeration(BmmEnumeration::BmmEnumeration(BmmEnumerationData {
                documentation: None,
                name: "MATCH_KIND".to_owned(),
                ancestors: None,
                package: package("org.openehr.base"),
                properties: None,
                source_schema_id: "openehr_test_1.0.0".to_owned(),
                immediate_descendants: openehr_base::containers::present(Vec::new()),
                is_abstract: false,
                is_primitive_type: false,
                is_override: false,
                item_names: Some(vec!["equal".to_owned()]),
                item_values: openehr_base::containers::present(Vec::new()),
                underlying_type_name: "Integer".to_owned(),
            }));
        let model = model(
            vec![
                class("String", &[], &[], true),
                class("ELEMENT", &[], &[], false),
                enumeration,
            ],
            Vec::new(),
        );
        assert_eq!(model.primitive_types(), ["String"]);
        assert_eq!(model.enumeration_types(), ["MATCH_KIND"]);
        assert!(model.enumeration_definition("MATCH_KIND").is_some());
        assert!(model.enumeration_definition("ELEMENT").is_none());
    }

    #[test]
    fn all_ancestor_classes_resolves_through_the_model() {
        // DV_QUANTITY -> DV_AMOUNT -> DV_ORDERED -> DATA_VALUE, each class
        // embedding only its IMMEDIATE parent (shallow copies).
        let model = model(
            vec![
                class("DATA_VALUE", &[], &[], false),
                class("DV_ORDERED", &["DATA_VALUE"], &[], false),
                class("DV_AMOUNT", &["DV_ORDERED"], &[], false),
                class("DV_QUANTITY", &["DV_AMOUNT"], &[], false),
            ],
            Vec::new(),
        );
        let mut ancestors = model.all_ancestor_classes("DV_QUANTITY");
        ancestors.sort();
        // `Any` closes the walk: the parentless DATA_VALUE takes the implicit
        // `Any` inheritance parent (`master05-core-model.adoc` §The Any Class
        // and Type — "the inheritance graph will always have the Any type as
        // its top node").
        assert_eq!(
            ancestors,
            [
                "Any".to_owned(),
                "DATA_VALUE".to_owned(),
                "DV_AMOUNT".to_owned(),
                "DV_ORDERED".to_owned()
            ]
        );
        assert_eq!(
            model.all_ancestor_classes("DATA_VALUE"),
            vec!["Any".to_owned()]
        );
        assert!(model.all_ancestor_classes("MISSING").is_empty());
    }

    #[test]
    fn property_definition_reads_the_flat_set() {
        let model = model(
            vec![
                class(
                    "DV_ORDERED",
                    &[],
                    &[property("normal_status", "CODE_PHRASE")],
                    false,
                ),
                class(
                    "DV_QUANTITY",
                    &["DV_ORDERED"],
                    &[property("magnitude", "Real")],
                    false,
                ),
            ],
            Vec::new(),
        );
        assert!(
            model
                .property_definition("DV_QUANTITY", "magnitude")
                .is_some()
        );
        // Inherited through the embedded ancestor copy.
        assert!(
            model
                .property_definition("DV_QUANTITY", "normal_status")
                .is_some()
        );
        assert!(
            model
                .property_definition("DV_QUANTITY", "missing")
                .is_none()
        );
    }

    #[test]
    fn property_definition_at_path_walks_one_nesting_step() {
        let model = model(
            vec![
                class("ELEMENT", &[], &[property("value", "DV_TEXT")], false),
                class(
                    "ITEM_TREE",
                    &[],
                    &[container_property("items", "ELEMENT")],
                    false,
                ),
            ],
            Vec::new(),
        );
        let value = model
            .property_definition_at_path("ITEM_TREE", "items/value")
            .expect("the nested property resolves through the contained type");
        assert_eq!(value.name(), "value");
        assert_eq!(value.conformance_type_name(), "DV_TEXT");
        assert!(
            model
                .property_definition_at_path("ITEM_TREE", "items/missing")
                .is_none()
        );
        assert!(model.property_definition_at_path("ITEM_TREE", "").is_none());
    }

    #[test]
    fn type_conforms_to_covers_the_three_rules() {
        let model = model(
            vec![
                class("DATA_VALUE", &[], &[], false),
                class("DV_ORDERED", &["DATA_VALUE"], &[], false),
                class("Ordered", &[], &[], false),
                class("Time", &["Ordered"], &[], false),
                class("Interval", &[], &[], false),
                class("List", &[], &[], false),
            ],
            Vec::new(),
        );

        // Rule 1, identical names and ancestor lookup.
        assert!(model.type_conforms_to("DV_ORDERED", "DV_ORDERED"));
        assert!(model.type_conforms_to("DV_ORDERED", "DATA_VALUE"));
        assert!(!model.type_conforms_to("DATA_VALUE", "DV_ORDERED"));

        // Rule 2, both generic: parameters recurse pairwise.
        assert!(model.type_conforms_to("Interval<Time>", "Interval<Ordered>"));
        assert!(!model.type_conforms_to("Interval<Ordered>", "Interval<Time>"));
        assert!(!model.type_conforms_to("Interval<Time,Time>", "Interval<Ordered>"));

        // Rule 3, descendant generic and ancestor not.
        assert!(model.type_conforms_to("Interval<Time>", "Interval"));
        // ... and not the other way round: the rule set admits parameters only
        // on the descendant side.
        assert!(!model.type_conforms_to("Interval", "Interval<Time>"));

        // Open parameters substitute to Any, which everything conforms to.
        assert!(model.type_conforms_to("Interval<Time>", "Interval<T>"));
        assert!(!model.type_conforms_to("Interval<T>", "Interval<Time>"));
        assert!(model.type_conforms_to("DV_ORDERED", "Any"));
        assert!(!model.type_conforms_to("Any", "DV_ORDERED"));

        // §6.6's own algorithm compares base classes case-INSENSITIVELY
        // (`base_class.is_case_insensitive_equal (anc_base_class)`), with
        // underscores significant (§5.2 Naming Convention).
        assert!(model.type_conforms_to("dv_ordered", "DV_ORDERED"));
        assert!(model.type_conforms_to("DV_ORDERED", "data_value"));
        assert!(!model.type_conforms_to("dvordered", "DV_ORDERED"));
    }

    /// An open ANCESTOR parameter substitutes to its declared conformance
    /// constraint, not blanket `Any`
    /// (`BMM_GENERIC_CLASS.generic_parameter_conformance_type`,
    /// `org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions): with
    /// `Interval<T:Ordered>` in the model, `Interval<String>` does NOT
    /// conform to `Interval<T>` because `String` is not `Ordered`.
    #[test]
    fn open_ancestor_parameters_substitute_their_constraint() {
        let interval = BmmClass::BmmGenericClass(BmmGenericClass {
            documentation: None,
            name: "Interval".to_owned(),
            ancestors: None,
            package: package("org.openehr.base.test"),
            properties: None,
            source_schema_id: "test".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
            generic_parameters: [(
                "T".to_owned(),
                BmmGenericParameter {
                    documentation: None,
                    name: "T".to_owned(),
                    conforms_to_type: Some(class("Ordered", &[], &[], false)),
                    inheritance_precursor: None,
                },
            )]
            .into_iter()
            .collect(),
        });
        let model = model(
            vec![
                interval,
                class("Ordered", &[], &[], false),
                class("Time", &["Ordered"], &[], false),
                class("String", &[], &[], false),
            ],
            Vec::new(),
        );
        assert!(model.type_conforms_to("Interval<Time>", "Interval<T>"));
        assert!(
            !model.type_conforms_to("Interval<String>", "Interval<T>"),
            "String does not conform to the T:Ordered constraint"
        );
    }

    #[test]
    fn ms_conformance_abstracts_the_container_away() {
        let model = model(
            vec![
                class("ELEMENT", &[], &[], false),
                class("CLUSTER", &["ITEM"], &[], false),
                class("ITEM", &[], &[], false),
                class("List", &[], &[], false),
                class(
                    "ITEM_TREE",
                    &[],
                    &[container_property("items", "ITEM")],
                    false,
                ),
            ],
            Vec::new(),
        );
        // The property is declared List<ITEM>; MS conformance compares the
        // dynamic type against ITEM regardless of the containment.
        assert!(model.ms_conformant_property_type("ITEM_TREE", "items", "ITEM"));
        assert!(model.ms_conformant_property_type("ITEM_TREE", "items", "CLUSTER"));
        assert!(!model.ms_conformant_property_type("ITEM_TREE", "items", "ELEMENT"));
        assert!(!model.ms_conformant_property_type("ITEM_TREE", "missing", "ITEM"));
    }

    #[test]
    fn package_paths_match_case_insensitively_from_the_model_root() {
        let mut composition = package("composition");
        composition.packages = Some(
            [("CONTENT".to_owned(), package("content"))]
                .into_iter()
                .collect::<BTreeMap<String, BmmPackage>>(),
        );
        let model = model(Vec::new(), vec![composition]);
        assert!(model.has_package_path("composition"));
        assert!(model.has_package_path("COMPOSITION"));
        assert!(model.has_package_path("Composition.Content"));
        assert!(!model.has_package_path("ehr"));

        let mut visited: Vec<String> = Vec::new();
        model.do_recursive_packages(&mut |package| visited.push(package.name.clone()));
        assert_eq!(visited, ["composition".to_owned(), "content".to_owned()]);
    }
}
