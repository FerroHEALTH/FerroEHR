// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written **v3** `BMM_MODEL` navigation
//! (`org.openehr.lang.bmm3.bmm_model.adoc` §Functions).
//!
//! The model-level counterpart of the type and class surfaces
//! ([`crate::v1_1::bmm3::core::entity::bmm_type_impl`],
//! [`crate::v1_1::bmm3::core::entity::bmm_class_impl`]): the queries that need the
//! whole `class_definitions` map rather than one class — class lookup,
//! transitive ancestors, flattened property lookup, and type conformance.
//!
//! The stable v2.x generation carries its own, structurally different model
//! surface at [`crate::v1_1::bmm::core::bmm_model_impl`]; the two never share an
//! impl, because `BMM_MODEL`, `BMM_CLASS` and `BMM_PROPERTY` are different
//! classes in the two generations
//! (`LANG/docs/bmm3/master00-amendment_record.adoc` SPECLANG-14).
//!
//! The conformance algorithm is `LANG/docs/bmm3/master06-core-types.adoc`
//! §Type Conformance, whose Tuple and Signature branches are empty upstream
//! (L251, L253) and are therefore not realized here either.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use crate::v1_1::bmm3::core::entity::bmm_type_impl::ANY_TYPE_NAME;
use crate::v1_1::bmm3::core::feature::bmm_property::BmmProperty;
use crate::v1_1::bmm3::core::model::bmm_model::BmmModel;

/// The root name of a type reference — the type name with any generic part
/// removed.
///
/// "Names of classes are just the root name, even if the class is generic"
/// (`org.openehr.lang.bmm3.bmm_class.adoc` §Attributes), so a lookup keyed by
/// class name strips the generic part first.
fn type_root(a_type_name: &str) -> &str {
    a_type_name.split('<').next().unwrap_or(a_type_name).trim()
}

/// Splits a type name into its root and its TOP-LEVEL generic parameters:
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
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in inner.char_indices() {
        match ch {
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
        out.push(part.trim());
    }
    out.retain(|part| !part.is_empty());
    out
}

/// Whether `name` is a FORMAL generic parameter name rather than a class name.
///
/// `master06-core-types.adoc` §Type Conformance states the test the algorithm
/// uses verbatim: "We assume type names of 1 letter are open parameters".
fn is_open_parameter_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.chars().count() == 1 && trimmed.chars().all(char::is_alphabetic)
}

impl BmmModel {
    /// `BMM_MODEL.class_definition`: the class definition of `a_type_name`,
    /// whose generic part is stripped before the lookup.
    ///
    /// Matching is CASE-INSENSITIVE with underscores significant —
    /// `LANG/docs/bmm3/master05-core-model.adoc` §Naming Convention: "When used
    /// computationally within an instantiated BMM model, it is assumed that
    /// case-insensitive matching is used. This means that the class name
    /// `"Hashable"` refers to the same class as `"HASHABLE"`. Note however that
    /// underscores are not removed during matching". An exact-key hit wins
    /// without a scan.
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

    /// `BMM_MODEL.all_ancestor_classes`: every ancestor class name of
    /// `a_class`, up to the root class, excluding `a_class` itself.
    ///
    /// The walk unions two sources so it is total on both persisted shapes: the
    /// ancestor TYPES a class carries ([`BmmClass::all_ancestors`] — v3 states
    /// inheritance as types, `org.openehr.lang.bmm3.bmm_class.adoc`
    /// §Description) and, for every name reached, that name's own definition in
    /// this model, so a class whose embedded ancestors are stubs still resolves
    /// all the way up. Cycle-safe; deduped.
    ///
    /// The `Any` top is implicit: "the `Any` type … will be used as the
    /// inheritance parent for every class in the model that doesn't have any
    /// other inheritance parent. As a result, the inheritance graph will always
    /// have the `Any` type as its top node"
    /// (`LANG/docs/bmm3/master05-core-model.adoc` §The Any Class and Type), so
    /// a defined, parentless class other than `Any` itself closes its ancestor
    /// list with `Any`.
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
            if !seen.insert(name.to_uppercase()) {
                continue;
            }
            out.push(name.clone());
            if let Some(class) = self.class_definition(&name) {
                let parents = class.all_ancestors();
                if parents.is_empty() && !class.name().eq_ignore_ascii_case(ANY_TYPE_NAME) {
                    rootless = true;
                }
                queue.extend(parents);
            }
        }
        if rootless && !seen.contains(&ANY_TYPE_NAME.to_uppercase()) {
            out.push(ANY_TYPE_NAME.to_owned());
        }
        out
    }

    /// `BMM_MODEL.property_definition`: the property `a_prop_name` of the
    /// FLATTENED class corresponding to `a_type_name`, so an inherited property
    /// resolves too.
    #[must_use]
    pub fn property_definition(
        &self,
        a_type_name: &str,
        a_prop_name: &str,
    ) -> Option<&BmmProperty> {
        self.flat_properties(a_type_name)?.get(a_prop_name).copied()
    }

    /// The flat property set of the class named `a_type_name`, flattened DOWN
    /// THE MODEL: at every inheritance step the ancestor's own definition in
    /// `class_definitions` is preferred over the copy embedded in the
    /// descendant's `ancestors` map.
    ///
    /// The embedded ancestor of a v3 class is a name-bearing stub, so a
    /// class-level walk alone would hide inherited properties; since
    /// `BMM_MODEL.class_definitions` is the model's full class map
    /// (`org.openehr.lang.bmm3.bmm_model.adoc` §Attributes), it is the richer
    /// source. Nearer class wins; cycle-safe.
    fn flat_properties(&self, a_type_name: &str) -> Option<BTreeMap<&str, &BmmProperty>> {
        let class = self.class_definition(a_type_name)?;
        let mut out = BTreeMap::new();
        let mut seen = BTreeSet::new();
        self.merge_flat_properties(class, &mut out, &mut seen);
        Some(out)
    }

    /// Merges `class`'s model-resolved ancestor properties, then its own, into
    /// `out` — ancestors first, so a redefinition in `class` overwrites.
    fn merge_flat_properties<'a>(
        &'a self,
        class: &'a BmmClass,
        out: &mut BTreeMap<&'a str, &'a BmmProperty>,
        seen: &mut BTreeSet<String>,
    ) {
        if !seen.insert(class.name().to_uppercase()) {
            return;
        }
        for ancestor in class.all_ancestors() {
            if let Some(definition) = self.class_definition(&ancestor) {
                self.merge_flat_properties(definition, out, seen);
            }
        }
        for (name, property) in class.properties().iter().copied().flatten() {
            out.insert(name.as_str(), property);
        }
    }

    /// `BMM_MODEL.type_conforms_to`: whether `a_desc_type` conforms to
    /// `an_anc_type`, both as type-name strings.
    ///
    /// `master06-core-types.adoc` §Type Conformance is the algorithm, and its
    /// three admitting branches are realized as written:
    ///
    /// * base-class test — the roots are equal case-insensitively, or the
    ///   descendant root "has_ancestor_class" the ancestor root;
    /// * both generic and passing the base-class test, with the same number of
    ///   generic parameters, each recursively conformant after open-parameter
    ///   substitution;
    /// * descendant generic, ancestor not, passing the base-class test —
    ///   "Conforms - case where anc type is not provided in generic form, but
    ///   desc is, e.g. `Interval<Integer>` conforms to `Interval`".
    ///
    /// Each generic branch is gated on BOTH halves the section states —
    /// "`valid_generic_type_name (a_type)` and `bmm_def_class instanceOf
    /// (BMM_GENERIC_CLASS)`" — so a generic-SHAPED name whose root class the
    /// model does not define as a `BMM_GENERIC_CLASS` takes the non-generic
    /// path.
    ///
    /// A non-generic descendant against a GENERIC ancestor is therefore not
    /// conformant: the section's final `else` returns "not
    /// valid_generic_type_name (anc_type)".
    ///
    /// An open ancestor parameter is replaced by its conformance type before
    /// the recursive test, per the section's
    /// `generic_parameter_conformance_type` step
    /// (`org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions), with `Any`
    /// as the unconstrained fallback
    /// (`…bmm3.bmm_definitions.adoc` §Functions `Any_class`).
    #[must_use]
    pub fn type_conforms_to(&self, a_desc_type: &str, an_anc_type: &str) -> bool {
        let (descendant_root, descendant_parameters) = split_type(a_desc_type);
        let (ancestor_root, ancestor_parameters) = split_type(an_anc_type);
        if !self.base_class_conforms_to(descendant_root, ancestor_root) {
            return false;
        }
        if !self.is_generic_type(descendant_root, &descendant_parameters) {
            return ancestor_parameters.is_empty();
        }
        if !self.is_generic_type(ancestor_root, &ancestor_parameters) {
            return true;
        }
        if descendant_parameters.len() != ancestor_parameters.len() {
            // NOTE: no openEHR spec governs this — our own design/extension;
            // §Type Conformance's pseudocode returns nothing when two generic
            // types differ in parameter count, and a count mismatch is refused.
            return false;
        }
        descendant_parameters
            .iter()
            .zip(ancestor_parameters.iter())
            .all(|(descendant, ancestor)| {
                let target = if is_open_parameter_name(ancestor) {
                    self.generic_parameter_conformance_type(ancestor_root, ancestor)
                } else {
                    (*ancestor).to_owned()
                };
                let source = if is_open_parameter_name(descendant) {
                    self.generic_parameter_conformance_type(descendant_root, descendant)
                } else {
                    (*descendant).to_owned()
                };
                self.type_conforms_to(&source, &target)
            })
    }

    /// The generic-branch guard of §Type Conformance:
    /// "`valid_generic_type_name (a_type)` and `bmm_def_class instanceOf
    /// (BMM_GENERIC_CLASS)`".
    ///
    /// The name must both CARRY generic parameters and root in a class this
    /// model defines as a `BMM_GENERIC_CLASS`
    /// (`org.openehr.lang.bmm3.bmm_generic_class.adoc` §Description); a class
    /// the model does not define satisfies no `instanceOf` test, so it takes
    /// the non-generic path.
    fn is_generic_type(&self, root: &str, parameters: &[&str]) -> bool {
        !parameters.is_empty()
            && matches!(
                self.class_definition(root),
                Some(BmmClass::BmmGenericClass(_))
            )
    }

    /// The base-class half of §Type Conformance: "`base_class`
    /// `.is_case_insensitive_equal (anc_base_class)` or else
    /// `class_definition (base_class).has_ancestor_class (anc_base_class)`".
    fn base_class_conforms_to(&self, descendant_root: &str, ancestor_root: &str) -> bool {
        if descendant_root.eq_ignore_ascii_case(ancestor_root) {
            return true;
        }
        if ancestor_root.eq_ignore_ascii_case(ANY_TYPE_NAME) {
            return true;
        }
        self.all_ancestor_classes(descendant_root)
            .iter()
            .any(|name| name.eq_ignore_ascii_case(ancestor_root))
    }

    /// The conformance type of the formal parameter `name` of the class
    /// `a_class_name`, or `Any` when the class declares no such parameter
    /// (`org.openehr.lang.bmm3.bmm_generic_class.adoc` §Functions).
    fn generic_parameter_conformance_type(&self, a_class_name: &str, name: &str) -> String {
        match self.class_definition(a_class_name) {
            Some(BmmClass::BmmGenericClass(generic)) => generic
                .generic_parameter_conformance_type(name)
                .unwrap_or_else(|| ANY_TYPE_NAME.to_owned()),
            _ => ANY_TYPE_NAME.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model;
    use crate::v1_1::bmm_persistence::reader::read_schema;
    use crate::v1_1::bmm3::core::model::bmm_model::BmmModel;

    /// A schema listing `ORDERED`, `INTEGER` and an `INTERVAL` whose definition
    /// the caller supplies, in the `master04-syntax.adoc` §Header Items shape.
    fn model(interval: &str) -> BmmModel {
        let src = format!(
            r#"
            bmm_version = <"2.4">
            rm_publisher = <"openehr">
            schema_name = <"bmm3_conformance">
            rm_release = <"1.0.2">
            packages = <
                ["test"] = <
                    name = <"test">
                    classes = <"ORDERED", "INTEGER", "INTERVAL">
                >
            >
            class_definitions = <
                ["ORDERED"] = < name = <"ORDERED"> >
                ["INTEGER"] = < name = <"INTEGER"> ancestors = <"ORDERED"> >
                {interval}
            >
            "#
        );
        create_bmm3_model(&read_schema(&src).expect("the fixture reads"))
            .expect("the fixture materialises")
    }

    /// A generic class taking one parameter constrained to `ORDERED`.
    const GENERIC_INTERVAL: &str = r#"["INTERVAL"] = <
            name = <"INTERVAL">
            generic_parameter_defs = <
                ["T"] = < name = <"T"> conforms_to_type = <"ORDERED"> >
            >
        >"#;

    /// The same name declared WITHOUT generic parameters.
    const PLAIN_INTERVAL: &str = r#"["INTERVAL"] = < name = <"INTERVAL"> >"#;

    /// `master06-core-types.adoc` §Type Conformance gates the generic branch on
    /// "`valid_generic_type_name (a_type)` and `bmm_def_class instanceOf
    /// (BMM_GENERIC_CLASS)`", so a generic-SHAPED name over a class the model
    /// defines as non-generic takes the section's final `else` — "return not
    /// valid_generic_type_name (anc_type)".
    #[test]
    fn a_generic_shaped_name_over_a_non_generic_class_is_not_generic() {
        let model = model(PLAIN_INTERVAL);
        assert!(!model.type_conforms_to("INTERVAL<INTEGER>", "INTERVAL<ORDERED>"));
        // The same `else` admits a non-generic ancestor name.
        assert!(model.type_conforms_to("INTERVAL<INTEGER>", "INTERVAL"));
    }

    /// The generic branches themselves, over a root the model DOES define as a
    /// `BMM_GENERIC_CLASS`: parameters recurse pairwise, and an ancestor stated
    /// without its generic form conforms ("e.g. `Interval<Integer>` conforms to
    /// `Interval`").
    #[test]
    fn a_generic_class_compares_its_parameters() {
        let model = model(GENERIC_INTERVAL);
        assert!(model.type_conforms_to("INTERVAL<INTEGER>", "INTERVAL<ORDERED>"));
        assert!(!model.type_conforms_to("INTERVAL<ORDERED>", "INTERVAL<INTEGER>"));
        assert!(model.type_conforms_to("INTERVAL<INTEGER>", "INTERVAL"));
        assert!(!model.type_conforms_to("INTERVAL", "INTERVAL<ORDERED>"));
    }

    /// Two generic types differing in parameter count: the section's pseudocode
    /// returns nothing, and this implementation refuses.
    #[test]
    fn generic_types_of_differing_parameter_counts_do_not_conform() {
        let model = model(GENERIC_INTERVAL);
        assert!(!model.type_conforms_to("INTERVAL<INTEGER,INTEGER>", "INTERVAL<ORDERED>"));
    }
}
