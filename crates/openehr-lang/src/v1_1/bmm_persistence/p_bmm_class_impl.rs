// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `P_BMM_CLASS` — the declared `is_generic`
//! predicate plus uniform access to the attributes every concrete
//! `P_BMM_CLASS` leaf carries.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_class.adoc`
//! §Attributes (`name`, `ancestors`, `constants`, `properties`, `functions`,
//! `invariants`, `is_abstract`, `is_override`, `generic_parameter_defs`,
//! `source_schema_id`, `uid`, `ancestor_defs`) and §Functions (`is_generic`,
//! whose postcondition is `Result := generic_parameter_defs /= Void`).
//!
//! `P_BMM_CLASS` is emitted as a polymorphic slot over its own least-rich form,
//! the `P_BMM_ENUMERATION` family, and `P_BMM_INTERFACE`, so every attribute
//! read here reaches through all of them; the enumeration-specific attributes
//! live in [`crate::v1_1::bmm_persistence::p_bmm_enumeration_impl`].
//!
//! NOTE: `P_BMM_INTERFACE` is a member of this slot
//! (`LANG/docs/bmm_persistence/master02-overview.adoc` §Conceptual Approach;
//! openEHR's own schemas serialise interfaces inside `class_definitions`)
//! while inheriting only `P_BMM_MODEL_ELEMENT` — it declares just
//! `documentation`/`name`/`functions`, so every other accessor answers the
//! absent value for an interface.

use std::collections::BTreeMap;

use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::p_bmm_constant::PBmmConstant;
use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumeration;
use crate::v1_1::bmm_persistence::p_bmm_function::PBmmFunction;
use crate::v1_1::bmm_persistence::p_bmm_generic_parameter::PBmmGenericParameter;
use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
use crate::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;

/// Reads an attribute EVERY leaf of the slot declares — the three
/// `P_BMM_MODEL_ELEMENT`/interface-shared ones — by applying `$body` to the leaf
/// the value holds.
macro_rules! every_leaf {
    ($value:expr, |$leaf:ident| $body:expr) => {
        match $value {
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumerationInteger($leaf)) => $body,
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumerationString($leaf)) => $body,
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumeration($leaf)) => $body,
            PBmmClass::PBmmClass($leaf) => $body,
            PBmmClass::PBmmInterface($leaf) => $body,
        }
    };
}

/// Reads an attribute only the CLASS-shaped leaves declare, answering `$absent`
/// for the function-only `P_BMM_INTERFACE` leaf.
macro_rules! class_leaf {
    ($value:expr, |$leaf:ident| $body:expr, $absent:expr) => {
        match $value {
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumerationInteger($leaf)) => $body,
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumerationString($leaf)) => $body,
            PBmmClass::PBmmEnumeration(PBmmEnumeration::PBmmEnumeration($leaf)) => $body,
            PBmmClass::PBmmClass($leaf) => $body,
            PBmmClass::PBmmInterface(_) => $absent,
        }
    };
}

impl PBmmClass {
    /// `P_BMM_CLASS.name`: "Name of the class" (class doc §Attributes) —
    /// "Name of this interface" for the interface leaf
    /// (`…p_bmm_interface.adoc` §Attributes).
    #[must_use]
    pub fn name(&self) -> &str {
        every_leaf!(self, |leaf| leaf.name.as_str())
    }

    /// `P_BMM_MODEL_ELEMENT.documentation`: "Optional documentation of this
    /// element"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_model_element.adoc`
    /// §Attributes).
    #[must_use]
    pub fn documentation(&self) -> Option<&str> {
        every_leaf!(self, |leaf| leaf.documentation.as_deref())
    }

    /// `P_BMM_CLASS.ancestors`: "List of immediate inheritance parents. If
    /// there are generic ancestors, use `_ancestor_defs_` instead" (class doc
    /// §Attributes) — empty for an interface, which declares none.
    #[must_use]
    pub fn ancestors(&self) -> &[String] {
        class_leaf!(
            self,
            |leaf| leaf.ancestors.as_deref().unwrap_or_default(),
            &[]
        )
    }

    /// `P_BMM_CLASS.ancestor_defs`: "List of structured inheritance ancestors,
    /// used only in the case of generic inheritance" (class doc §Attributes) —
    /// empty for an interface.
    #[must_use]
    pub fn ancestor_defs(&self) -> &[PBmmGenericType] {
        class_leaf!(
            self,
            |leaf| leaf.ancestor_defs.as_deref().unwrap_or_default(),
            &[]
        )
    }

    /// `P_BMM_CLASS.properties`: "List of attributes defined in this class",
    /// keyed by property name (class doc §Attributes) — always `None` for an
    /// interface: interfaces "declare only functions and carry no state"
    /// (`master02-overview.adoc` §Conceptual Approach).
    #[must_use]
    pub fn properties(&self) -> Option<&BTreeMap<String, PBmmProperty>> {
        class_leaf!(self, |leaf| leaf.properties.as_ref(), None)
    }

    /// `P_BMM_CLASS.functions`: "List of functions (routines) defined in this
    /// class, keyed by name" (class doc §Attributes) — "Functions (routines)
    /// declared by this interface" for the interface leaf
    /// (`…p_bmm_interface.adoc` §Attributes).
    #[must_use]
    pub fn functions(&self) -> Option<&BTreeMap<String, PBmmFunction>> {
        every_leaf!(self, |leaf| leaf.functions.as_ref())
    }

    /// `P_BMM_CLASS.constants`: "Constants defined in this class, keyed by
    /// name" (class doc §Attributes) — `None` for an interface.
    #[must_use]
    pub fn constants(&self) -> Option<&BTreeMap<String, PBmmConstant>> {
        class_leaf!(self, |leaf| leaf.constants.as_ref(), None)
    }

    /// `P_BMM_CLASS.invariants`: "Invariants defined on this class, as a Hash
    /// of assertion expressions keyed by tag" (class doc §Attributes) — `None`
    /// for an interface.
    #[must_use]
    pub fn invariants(&self) -> Option<&BTreeMap<String, String>> {
        class_leaf!(self, |leaf| leaf.invariants.as_ref(), None)
    }

    /// `P_BMM_CLASS.generic_parameter_defs`: "List of generic parameter
    /// definitions" (class doc §Attributes) — `None` for an interface.
    #[must_use]
    pub fn generic_parameter_defs(&self) -> Option<&BTreeMap<String, PBmmGenericParameter>> {
        class_leaf!(self, |leaf| leaf.generic_parameter_defs.as_ref(), None)
    }

    /// `P_BMM_CLASS.is_abstract`: "True if this is an abstract type" (class doc
    /// §Attributes) — a `0..1` flag, absent meaning not abstract.
    ///
    /// NOTE (adjudicated): an interface answers `True`. It declares no
    /// `is_abstract` attribute of its own (`…p_bmm_interface.adoc` §Attributes),
    /// but it is by definition not instantiable, being one of the
    /// "class-like definitions that declare only functions and carry no state"
    /// (`master02-overview.adoc` §Conceptual Approach) — so reporting it as
    /// concrete would be wrong, and this is the same answer
    /// [`crate::v1_1::bmm_persistence::create_model`] materialises into
    /// `BMM_CLASS.is_abstract`.
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        class_leaf!(self, |leaf| leaf.is_abstract.unwrap_or(false), true)
    }

    /// `P_BMM_CLASS.is_override`: "True if this class definition overrides one
    /// found in an included schema" (class doc §Attributes) — a `0..1` flag,
    /// absent meaning no override, and never set for an interface (see
    /// [`PBmmClass::set_is_override`]).
    #[must_use]
    pub fn is_override(&self) -> bool {
        class_leaf!(self, |leaf| leaf.is_override.unwrap_or(false), false)
    }

    /// Records that this class definition overrides one of the same name in an
    /// included schema (`P_BMM_CLASS.is_override`, class doc §Attributes) —
    /// set by [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`]
    /// when it detects the collision.
    ///
    /// NOTE (honest boundary): an interface has nowhere to record the flag —
    /// `P_BMM_INTERFACE` inherits `P_BMM_MODEL_ELEMENT`
    /// (`…p_bmm_interface.adoc` §Inherit), which declares no `is_override`
    /// attribute — so the call is ignored for an interface leaf. Only the record
    /// of the override is lost; the merge precedence itself (the includer's
    /// definition wins) is unaffected.
    pub fn set_is_override(&mut self, value: bool) {
        class_leaf!(self, |leaf| leaf.is_override = Some(value), ());
    }

    /// `P_BMM_CLASS.source_schema_id`: "Reference to original source schema
    /// defining this class. Set during `BMM_SCHEMA` materialise" (class doc
    /// §Attributes) — `None` for an interface, which declares no such
    /// attribute.
    #[must_use]
    pub fn source_schema_id(&self) -> Option<&str> {
        class_leaf!(self, |leaf| Some(leaf.source_schema_id.as_str()), None)
    }

    /// `P_BMM_CLASS.uid`: "Unique id generated for later comparison during
    /// merging, in order to detect if two classes are the same. Assigned in
    /// post-load processing" (class doc §Attributes) — `None` for an interface,
    /// which declares no such attribute.
    #[must_use]
    pub fn uid(&self) -> Option<i32> {
        class_leaf!(self, |leaf| Some(leaf.uid), None)
    }

    /// `P_BMM_CLASS.is_generic`: "True if this class is a generic class", with
    /// postcondition `Result := generic_parameter_defs /= Void` (class doc
    /// §Functions).
    ///
    /// NOTE (adjudicated): the postcondition tests only for the presence of the
    /// attribute, and an ODIN `generic_parameter_defs = <>` block reads as
    /// present-but-empty; a class with no formal parameter is not a type
    /// generator ("Generic classes are those that have one or more
    /// substitutable generic type parameters",
    /// `LANG/docs/bmm_persistence/master04-syntax.adoc` §Generic Classes), so
    /// an empty map answers `False`.
    #[must_use]
    pub fn is_generic(&self) -> bool {
        self.generic_parameter_defs()
            .is_some_and(|parameters| !parameters.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
    use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClassData;
    use crate::v1_1::bmm_persistence::p_bmm_generic_parameter::PBmmGenericParameter;
    use crate::v1_1::bmm_persistence::p_bmm_interface::PBmmInterface;

    /// A bare class definition named `name`, with the given formal generic
    /// parameter declarations.
    fn class(
        name: &str,
        generic_parameter_defs: Option<BTreeMap<String, PBmmGenericParameter>>,
    ) -> PBmmClass {
        PBmmClass::PBmmClass(PBmmClassData {
            documentation: None,
            name: name.to_owned(),
            ancestors: openehr_base::containers::present(Vec::new()),
            constants: None,
            properties: None,
            functions: None,
            invariants: None,
            is_abstract: None,
            is_override: None,
            generic_parameter_defs,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            bmm_class: None,
            uid: 1,
            ancestor_defs: openehr_base::containers::present(Vec::new()),
        })
    }

    /// The formal parameter `T` conforming to `Ordered`.
    fn ordered_t() -> BTreeMap<String, PBmmGenericParameter> {
        [(
            "T".to_owned(),
            PBmmGenericParameter {
                documentation: None,
                name: "T".to_owned(),
                conforms_to_type: Some("Ordered".to_owned()),
                bmm_generic_parameter: None,
            },
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn absent_flags_read_as_false() {
        let class = class("ELEMENT", None);
        assert!(!class.is_abstract());
        assert!(!class.is_override());
        assert!(!class.is_generic());
        assert_eq!(class.name(), "ELEMENT");
        assert_eq!(class.source_schema_id(), Some("openehr_test_1.0.0"));
        assert_eq!(class.uid(), Some(1));
        assert!(class.ancestors().is_empty());
        assert!(class.ancestor_defs().is_empty());
        assert!(class.properties().is_none());
        assert!(class.functions().is_none());
        assert!(class.constants().is_none());
        assert!(class.invariants().is_none());
        assert!(class.documentation().is_none());
    }

    #[test]
    fn is_generic_needs_at_least_one_formal_parameter() {
        assert!(!class("Interval", Some(BTreeMap::new())).is_generic());
        assert!(class("Interval", Some(ordered_t())).is_generic());
    }

    #[test]
    fn set_is_override_marks_the_including_definition() {
        let mut class = class("DV_TEXT", None);
        class.set_is_override(true);
        assert!(class.is_override());
    }

    #[test]
    fn an_interface_leaf_reads_its_own_three_attributes_and_absents_the_rest() {
        let mut interface = PBmmClass::PBmmInterface(PBmmInterface {
            documentation: Some("a pure operation interface".to_owned()),
            name: "TERMINOLOGY_ACCESS".to_owned(),
            functions: Some(BTreeMap::new()),
        });
        assert_eq!(interface.name(), "TERMINOLOGY_ACCESS");
        assert_eq!(
            interface.documentation(),
            Some("a pure operation interface")
        );
        assert!(interface.functions().is_some());
        // An interface declares only functions and carries no state.
        assert!(interface.ancestors().is_empty());
        assert!(interface.ancestor_defs().is_empty());
        assert!(interface.properties().is_none());
        assert!(interface.constants().is_none());
        assert!(interface.invariants().is_none());
        assert!(interface.generic_parameter_defs().is_none());
        assert!(!interface.is_generic());
        // Not instantiable, and no slot for either processing-stamped attribute.
        assert!(interface.is_abstract());
        assert_eq!(interface.source_schema_id(), None);
        assert_eq!(interface.uid(), None);
        // The override flag is unrecordable, so the setter is a no-op.
        interface.set_is_override(true);
        assert!(!interface.is_override());
    }
}
