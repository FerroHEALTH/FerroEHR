// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of the `P_BMM_TYPE` family.
//!
//! Covers the declared `as_type_string` rendering, the two persisted-form
//! target-type accessors (`P_BMM_CONTAINER_TYPE.type_ref`,
//! `P_BMM_GENERIC_TYPE.generic_parameter_refs`) and the `type_def` functions
//! that GENERATE a `P_BMM_TYPE` object from a single property's `type` name
//! (`…p_bmm_single_property.adoc` / `…p_bmm_single_property_open.adoc`
//! §Functions), which live here because their result is a type object.
//!
//! Spec:
//! `LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_type.adoc`
//! §Functions (`as_type_string`: "Formal name of the type for display";
//! "effected in descendants"), with each descendant's parts from its own class
//! doc (`…p_bmm_simple_type.adoc` `type`, `…p_bmm_open_type.adoc` `type`,
//! `…p_bmm_generic_type.adoc` `root_type` + `generic_parameters` +
//! `generic_parameter_defs`, `…p_bmm_container_type.adoc` `container_type` +
//! `type`/`type_def`, `…p_bmm_indexed_container_type.adoc` `index_type`) and
//! the written forms of `LANG/docs/bmm_persistence/master04-syntax.adoc`
//! §Generic Classes (`DV_INTERVAL<DV_QUANTITY>`, `List<Reference<Party>>`) and
//! §Container Properties (`Hash <String, EVENT_ACTION>`).
//!
//! Generic delimiters `'<'`, `','`, `'>'` per
//! `org.openehr.lang.bmm3.bmm_definitions.adoc` §Constants
//! (`Generic_left_delimiter`, `Generic_separator`, `Generic_right_delimiter`) —
//! the same literals [`crate::v1_1::bmm::core::bmm_type_impl`] renders with,
//! because the generated `BMM_DEFINITIONS` carries no constant for them.

use crate::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerTypeData;
use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
use crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType;
use crate::v1_1::bmm_persistence::p_bmm_open_type::PBmmOpenType;
use crate::v1_1::bmm_persistence::p_bmm_simple_type::PBmmSimpleType;
use crate::v1_1::bmm_persistence::p_bmm_single_property::PBmmSingleProperty;
use crate::v1_1::bmm_persistence::p_bmm_single_property_open::PBmmSinglePropertyOpen;
use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;

/// The display form of a container's target type: its `type` name, else the
/// nested `type_def`, else the empty string when the schema states neither
/// (both are `0..1` on `P_BMM_CONTAINER_TYPE`).
fn container_target(r#type: Option<&String>, type_def: Option<&PBmmBaseType>) -> String {
    match (r#type, type_def) {
        (Some(name), _) => name.clone(),
        (None, Some(nested)) => nested.as_type_string(),
        (None, None) => String::new(),
    }
}

impl PBmmSimpleType {
    /// `P_BMM_TYPE.as_type_string`: "Formal name of the type for display"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_type.adoc` §Functions) — for a
    /// simple type the class name itself ("Name of type - must be a simple
    /// class name", `…p_bmm_simple_type.adoc` §Attributes).
    #[must_use]
    pub fn as_type_string(&self) -> String {
        self.r#type.clone()
    }
}

impl PBmmOpenType {
    /// `P_BMM_TYPE.as_type_string` for an open type: the generic parameter name,
    /// "a single letter like 'T', 'G' etc"
    /// (`…p_bmm_open_type.adoc` §Attributes).
    #[must_use]
    pub fn as_type_string(&self) -> String {
        self.r#type.clone()
    }
}

impl PBmmGenericType {
    /// `P_BMM_TYPE.as_type_string` for a generic type: `root_type<p1,p2>`, the
    /// form `master04-syntax.adoc` §Generic Classes writes
    /// (`DV_INTERVAL<DV_QUANTITY>`).
    ///
    /// The string parameters (`generic_parameters`) come first, then the
    /// structural ones (`generic_parameter_defs`) — the two lists are the
    /// simple and complex halves of one parameter list ("use
    /// `_generic_parameters_` for a list of string types; use
    /// `_generic_parameter_defs_` for a list of complex type references",
    /// §Generic Classes), and a schema uses one or the other, never both for
    /// the same position.
    #[must_use]
    pub fn as_type_string(&self) -> String {
        let mut parameters: Vec<String> = self.generic_parameters.clone().unwrap_or_default();
        parameters.extend(
            self.generic_parameter_defs
                .iter()
                .map(PBmmType::as_type_string),
        );
        if parameters.is_empty() {
            return self.root_type.clone();
        }
        let root = &self.root_type;
        let joined = parameters.join(",");
        format!("{root}<{joined}>")
    }
}

impl PBmmContainerTypeData {
    /// `P_BMM_TYPE.as_type_string` for a container type: `container_type<target>`,
    /// the form `master04-syntax.adoc` §Container Properties writes
    /// (`items: List<ITEM>`).
    #[must_use]
    pub fn as_type_string(&self) -> String {
        let container = &self.container_type;
        let target = container_target(self.r#type.as_ref(), self.type_def.as_ref());
        format!("{container}<{target}>")
    }
}

impl PBmmIndexedContainerType {
    /// `P_BMM_TYPE.as_type_string` for an indexed container type:
    /// `container_type<index_type,target>`, the form `master04-syntax.adoc`
    /// §Container Properties writes (`custom_actions: Hash <String,
    /// EVENT_ACTION>`).
    #[must_use]
    pub fn as_type_string(&self) -> String {
        let container = &self.container_type;
        let index = &self.index_type;
        let target = container_target(self.r#type.as_ref(), self.type_def.as_ref());
        format!("{container}<{index},{target}>")
    }
}

impl PBmmContainerTypeData {
    /// `P_BMM_CONTAINER_TYPE.type_ref`: "The target type; this converts to the
    /// first parameter in `_generic_parameters_` in `BMM_GENERIC_TYPE`"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_container_type.adoc`
    /// §Functions).
    ///
    /// The target is written EITHER as the `type` name (a simple class
    /// reference) OR as the nested `type_def` ("Type definition of `_type_`, if
    /// not a simple String type reference", class doc §Attributes), so at most
    /// one of the two is set; a `type_def` wins where a schema states both, the
    /// same precedence the `P_BMM_SCHEMA` → `BMM_MODEL` transform reads a
    /// container target with. `None` when the schema states neither — both
    /// carriers are `0..1`.
    #[must_use]
    pub fn type_ref(&self) -> Option<PBmmBaseType> {
        match (self.type_def.as_ref(), self.r#type.as_ref()) {
            (Some(nested), _) => Some(nested.clone()),
            (None, Some(name)) => Some(PBmmBaseType::PBmmSimpleType(PBmmSimpleType {
                bmm_type: None,
                value_constraint: None,
                r#type: name.clone(),
            })),
            (None, None) => None,
        }
    }
}

impl PBmmGenericType {
    /// `P_BMM_GENERIC_TYPE.generic_parameter_refs`: "Generic parameters of the
    /// root_type in this type specifier. The order must match the order of the
    /// owning class's formal generic parameter declarations"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_generic_type.adoc` §Functions).
    ///
    /// The string parameters (`generic_parameters`) come first, then the
    /// structural ones (`generic_parameter_defs`) — the order
    /// [`PBmmGenericType::as_type_string`] renders with, grounded in
    /// `master04-syntax.adoc` §Generic Classes ("use `_generic_parameters_` for
    /// a list of string types; use `_generic_parameter_defs_` for a list of
    /// complex type references").
    #[must_use]
    pub fn generic_parameter_refs(&self) -> Vec<PBmmType> {
        let mut refs: Vec<PBmmType> = self
            .generic_parameters
            .iter()
            .flatten()
            .map(|name| {
                PBmmType::PBmmSimpleType(PBmmSimpleType {
                    bmm_type: None,
                    value_constraint: None,
                    r#type: name.clone(),
                })
            })
            .collect();
        refs.extend(self.generic_parameter_defs.iter().cloned());
        refs
    }
}

impl PBmmContainerType {
    /// `P_BMM_TYPE.as_type_string` dispatched over the `P_BMM_CONTAINER_TYPE`
    /// forms.
    #[must_use]
    pub fn as_type_string(&self) -> String {
        match self {
            Self::PBmmIndexedContainerType(indexed) => indexed.as_type_string(),
            Self::PBmmContainerType(data) => data.as_type_string(),
        }
    }
}

impl PBmmBaseType {
    /// `P_BMM_TYPE.as_type_string` dispatched over the `P_BMM_BASE_TYPE` forms.
    #[must_use]
    pub fn as_type_string(&self) -> String {
        match self {
            Self::PBmmGenericType(generic) => generic.as_type_string(),
            Self::PBmmOpenType(open) => open.as_type_string(),
            Self::PBmmSimpleType(simple) => simple.as_type_string(),
        }
    }
}

impl PBmmType {
    /// `P_BMM_TYPE.as_type_string`: "Formal name of the type for display"
    /// (class doc §Functions), dispatched over every `P_BMM_TYPE` form.
    #[must_use]
    pub fn as_type_string(&self) -> String {
        match self {
            Self::PBmmContainerType(container) => container.as_type_string(),
            Self::PBmmGenericType(generic) => generic.as_type_string(),
            Self::PBmmOpenType(open) => open.as_type_string(),
            Self::PBmmSimpleType(simple) => simple.as_type_string(),
        }
    }
}

impl PBmmSingleProperty {
    /// `P_BMM_SINGLE_PROPERTY.type_def`: "Generate `_type_ref_` from `_type_`
    /// and save"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_single_property.adoc`
    /// §Functions), whose declared result is a `P_BMM_SIMPLE_TYPE` — the type of
    /// the `type_ref` attribute, not of the same class's inherited `type_def`
    /// attribute (`…p_bmm_property.adoc` §Attributes, a `P_BMM_TYPE`).
    ///
    /// An already-computed `type_ref` is returned as it stands; otherwise the
    /// simple type is generated from `type`, "the type name" a simple property
    /// carries (class doc §Attributes). `None` when the schema states neither.
    ///
    /// NOTE: the "and save" half of the meaning is memoisation of the value this
    /// function derives, so a by-value computation carries the whole semantics.
    #[must_use]
    pub fn type_def(&self) -> Option<PBmmSimpleType> {
        match (self.type_ref.as_ref(), self.r#type.as_ref()) {
            (Some(type_ref), _) => Some(type_ref.clone()),
            (None, Some(name)) => Some(PBmmSimpleType {
                bmm_type: None,
                value_constraint: None,
                r#type: name.clone(),
            }),
            (None, None) => None,
        }
    }
}

impl PBmmSinglePropertyOpen {
    /// `P_BMM_SINGLE_PROPERTY_OPEN.type_def`: "Generate `_type_ref_` from
    /// `_type_` and save"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_single_property_open.adoc`
    /// §Functions), whose declared result is a `P_BMM_OPEN_TYPE`.
    ///
    /// The open form's `type` is the persisted single-letter parameter name
    /// ("Really we should use `_type_def_` to be regular in the schema, but that
    /// makes the schema more wordy and less clear. So we use this persisted
    /// String value, and compute the `_type_def_` on the fly", class doc
    /// §Attributes) — which is exactly this computation. `None` when the schema
    /// states neither carrier.
    #[must_use]
    pub fn type_def(&self) -> Option<PBmmOpenType> {
        match (self.type_ref.as_ref(), self.r#type.as_ref()) {
            (Some(type_ref), _) => Some(type_ref.clone()),
            (None, Some(name)) => Some(PBmmOpenType {
                bmm_type: None,
                value_constraint: None,
                r#type: name.clone(),
            }),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
    use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
    use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerTypeData;
    use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
    use crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType;
    use crate::v1_1::bmm_persistence::p_bmm_simple_type::PBmmSimpleType;
    use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;

    /// A `P_BMM_GENERIC_TYPE` over string parameters.
    fn generic(root: &str, parameters: &[&str]) -> PBmmGenericType {
        PBmmGenericType {
            bmm_type: None,
            value_constraint: None,
            root_type: root.to_owned(),
            generic_parameter_defs: Vec::new(),
            generic_parameters: openehr_base::containers::present(
                parameters.iter().map(|p| (*p).to_owned()).collect(),
            ),
        }
    }

    #[test]
    fn simple_and_generic_forms() {
        let simple = PBmmSimpleType {
            bmm_type: None,
            value_constraint: None,
            r#type: "DV_TEXT".to_owned(),
        };
        assert_eq!(simple.as_type_string(), "DV_TEXT");
        assert_eq!(
            generic("DV_INTERVAL", &["DV_QUANTITY"]).as_type_string(),
            "DV_INTERVAL<DV_QUANTITY>"
        );
        // A root type with no stated parameters renders bare.
        assert_eq!(generic("DV_INTERVAL", &[]).as_type_string(), "DV_INTERVAL");
    }

    #[test]
    fn nested_container_of_generic_renders_the_master04_example() {
        // master04-syntax.adoc §Generic Classes: `careProvider:
        // List<Reference<Party>>`.
        let nested = PBmmContainerType::PBmmContainerType(PBmmContainerTypeData {
            bmm_type: None,
            container_type: "List".to_owned(),
            type_def: Some(PBmmBaseType::PBmmGenericType(generic(
                "Reference",
                &["Party"],
            ))),
            r#type: None,
        });
        assert_eq!(nested.as_type_string(), "List<Reference<Party>>");
    }

    #[test]
    fn indexed_container_names_its_index_type() {
        // master04-syntax.adoc §Container Properties: `custom_actions: Hash
        // <String, EVENT_ACTION>`.
        let indexed = PBmmIndexedContainerType {
            bmm_type: None,
            container_type: "Hash".to_owned(),
            type_def: None,
            r#type: Some("EVENT_ACTION".to_owned()),
            index_type: "String".to_owned(),
        };
        assert_eq!(indexed.as_type_string(), "Hash<String,EVENT_ACTION>");
    }

    /// `type_ref` resolves the target type from EITHER carrier, prefers the
    /// structural one, and is absent when the schema states neither.
    #[test]
    fn a_container_type_ref_resolves_either_carrier() {
        use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerTypeData;

        let named = PBmmContainerTypeData {
            bmm_type: None,
            container_type: "List".to_owned(),
            type_def: None,
            r#type: Some("ELEMENT".to_owned()),
        };
        assert_eq!(
            named.type_ref().map(|t| t.as_type_string()),
            Some("ELEMENT".to_owned())
        );

        let structural = PBmmContainerTypeData {
            bmm_type: None,
            container_type: "List".to_owned(),
            type_def: Some(PBmmBaseType::PBmmGenericType(generic(
                "Reference",
                &["Party"],
            ))),
            r#type: Some("IGNORED".to_owned()),
        };
        assert_eq!(
            structural.type_ref().map(|t| t.as_type_string()),
            Some("Reference<Party>".to_owned())
        );

        let neither = PBmmContainerTypeData {
            bmm_type: None,
            container_type: "List".to_owned(),
            type_def: None,
            r#type: None,
        };
        assert!(neither.type_ref().is_none());
    }

    /// The parameter refs are the one list `as_type_string` renders: string
    /// parameters first, structural ones after, in declaration order.
    #[test]
    fn generic_parameter_refs_follow_the_rendered_order() {
        let mut root = generic("REFERENCE_RANGE", &["Integer"]);
        root.generic_parameter_defs = vec![PBmmType::PBmmGenericType(generic(
            "DV_INTERVAL",
            &["DV_QUANTITY"],
        ))];
        assert_eq!(
            root.generic_parameter_refs()
                .iter()
                .map(PBmmType::as_type_string)
                .collect::<Vec<_>>(),
            ["Integer".to_owned(), "DV_INTERVAL<DV_QUANTITY>".to_owned()]
        );
        // A root type with no stated parameters has no refs.
        assert!(
            generic("DV_INTERVAL", &[])
                .generic_parameter_refs()
                .is_empty()
        );
    }

    /// `type_def` generates the type object from the persisted `type` name, and
    /// returns an already-computed `type_ref` unchanged.
    #[test]
    fn a_single_property_type_def_is_generated_from_its_type_name() {
        use crate::v1_1::bmm_persistence::p_bmm_single_property::PBmmSingleProperty;
        use crate::v1_1::bmm_persistence::p_bmm_single_property_open::PBmmSinglePropertyOpen;

        let mut simple = PBmmSingleProperty {
            documentation: None,
            name: "value".to_owned(),
            is_mandatory: Some(true),
            is_computed: None,
            is_im_infrastructure: None,
            is_im_runtime: None,
            type_def: None,
            bmm_property: None,
            r#type: Some("DV_TEXT".to_owned()),
            type_ref: None,
        };
        assert_eq!(
            simple.type_def().map(|t| t.as_type_string()),
            Some("DV_TEXT".to_owned())
        );
        simple.type_ref = Some(PBmmSimpleType {
            bmm_type: None,
            value_constraint: None,
            r#type: "DV_CODED_TEXT".to_owned(),
        });
        assert_eq!(
            simple.type_def().map(|t| t.as_type_string()),
            Some("DV_CODED_TEXT".to_owned())
        );
        simple.r#type = None;
        simple.type_ref = None;
        assert!(simple.type_def().is_none());

        let open = PBmmSinglePropertyOpen {
            documentation: None,
            name: "item".to_owned(),
            is_mandatory: Some(true),
            is_computed: None,
            is_im_infrastructure: None,
            is_im_runtime: None,
            type_def: None,
            bmm_property: None,
            type_ref: None,
            r#type: Some("T".to_owned()),
        };
        assert_eq!(
            open.type_def().map(|t| t.as_type_string()),
            Some("T".to_owned())
        );
    }

    #[test]
    fn mixed_generic_parameter_defs_render_after_the_string_parameters() {
        // master04-syntax.adoc §Generic Classes CRAZY_TYPE shape, reduced: a
        // structural parameter beside a string one.
        let mut root = generic("REFERENCE_RANGE", &["Integer"]);
        root.generic_parameter_defs = vec![PBmmType::PBmmGenericType(generic(
            "DV_INTERVAL",
            &["DV_QUANTITY"],
        ))];
        assert_eq!(
            root.as_type_string(),
            "REFERENCE_RANGE<Integer,DV_INTERVAL<DV_QUANTITY>>"
        );
    }
}
