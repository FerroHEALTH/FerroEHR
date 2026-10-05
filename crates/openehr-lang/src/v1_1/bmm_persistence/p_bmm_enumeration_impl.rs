// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `P_BMM_ENUMERATION` — uniform access to the
//! enumeration item lists, plus the underlying-type name each concrete
//! `BMM_ENUMERATION_*` form is based on.
//!
//! Spec:
//! `LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_enumeration.adoc`
//! §Attributes (`item_names`, `item_values`, `item_documentations`) and
//! `LANG/docs/bmm_persistence/master04-syntax.adoc` §Enumerated Types
//! ("enumerated types are treated as constrained forms of standard types with
//! open ranges, currently only `Integer` and `String`" — "just names
//! (`_items_names_` meta-property) or both names and values (`_item_values_`
//! meta-property) can be specified").

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumeration;

/// The underlying type name `BMM_ENUMERATION_INTEGER` redefines
/// `underlying_type_name` to (`org.openehr.lang.bmm.bmm_enumeration_integer.adoc`
/// §Attributes: `{default = "INTEGER"}`).
pub const INTEGER_UNDERLYING_TYPE_NAME: &str = "INTEGER";

/// The underlying type name `BMM_ENUMERATION_STRING` redefines
/// `underlying_type_name` to (`org.openehr.lang.bmm.bmm_enumeration_string.adoc`
/// §Attributes: `{default = "STRING"}`).
pub const STRING_UNDERLYING_TYPE_NAME: &str = "STRING";

/// The type an un-subtyped `BMM_ENUMERATION` is based on.
///
/// It applies when the schema states no ancestor: "It is designed so that the
/// default type is Integer"
/// (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Description).
pub const DEFAULT_UNDERLYING_TYPE_NAME: &str = "Integer";

/// Borrows one `P_BMM_ENUMERATION` attribute from whichever concrete leaf a
/// [`PBmmEnumeration`] holds.
macro_rules! enumeration_field {
    ($value:expr, $field:ident) => {
        match $value {
            PBmmEnumeration::PBmmEnumerationInteger(leaf) => &leaf.$field,
            PBmmEnumeration::PBmmEnumerationString(leaf) => &leaf.$field,
            PBmmEnumeration::PBmmEnumeration(leaf) => &leaf.$field,
        }
    };
}

impl PBmmEnumeration {
    /// `P_BMM_ENUMERATION.item_names` — "The list of names of the enumeration.
    /// If no values are supplied, the integer values 0, 1, 2, ... are assumed"
    /// (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes).
    #[must_use]
    pub fn item_names(&self) -> &[String] {
        enumeration_field!(self, item_names)
            .as_deref()
            .unwrap_or_default()
    }

    /// `P_BMM_ENUMERATION.item_values` — "Optional list of specific values.
    /// Must be 1:1 with `item_names` list"
    /// (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes).
    #[must_use]
    pub fn item_values(&self) -> &[serde_json::Value] {
        enumeration_field!(self, item_values)
            .as_deref()
            .unwrap_or_default()
    }

    /// The inheritance ancestors this persisted enumeration states
    /// (`P_BMM_CLASS.ancestors`, `org.openehr.lang.bmm_persistence.p_bmm_class.adoc`
    /// §Attributes).
    ///
    /// An enumeration "may have only one ancestor"
    /// (`LANG/docs/bmm3/master07-core-classes.adoc` §Range-Constrained Classes),
    /// so this list is the input to that validity rule and to
    /// [`PBmmEnumeration::underlying_type_name`].
    #[must_use]
    pub fn ancestors(&self) -> &[String] {
        enumeration_field!(self, ancestors)
            .as_deref()
            .unwrap_or_default()
    }

    /// `P_BMM_ENUMERATION.item_documentations`: "Optional documentation strings
    /// for the enumeration items, in the same order as `_item_names_`" (class
    /// doc §Attributes).
    #[must_use]
    pub fn item_documentations(&self) -> &[String] {
        enumeration_field!(self, item_documentations)
            .as_deref()
            .unwrap_or_default()
    }

    /// The `BMM_ENUMERATION.underlying_type_name` this persisted enumeration
    /// materialises to: "Name of type any concrete BMM_ENUMERATION_* sub-type
    /// is based on, i.e. the name of type bound to 'T' in a declared use of
    /// this type" (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes).
    ///
    /// The two concrete forms redefine it to a constant
    /// ([`INTEGER_UNDERLYING_TYPE_NAME`], [`STRING_UNDERLYING_TYPE_NAME`]).
    ///
    /// NOTE (adjudicated): for the un-subtyped `P_BMM_ENUMERATION` form the
    /// class docs give no default, so the first declared ancestor is used —
    /// `master04-syntax.adoc` §Enumerated Types writes every example as a
    /// class whose `ancestors` names the underlying primitive
    /// (`ancestors = <"Integer", ...>`, `ancestors = <"String", ...>`) — and
    /// [`DEFAULT_UNDERLYING_TYPE_NAME`] when there is no ancestor either.
    #[must_use]
    pub fn underlying_type_name(&self) -> &str {
        match self {
            Self::PBmmEnumerationInteger(_) => INTEGER_UNDERLYING_TYPE_NAME,
            Self::PBmmEnumerationString(_) => STRING_UNDERLYING_TYPE_NAME,
            Self::PBmmEnumeration(leaf) => leaf
                .ancestors
                .as_deref()
                .unwrap_or_default()
                .first()
                .map_or(DEFAULT_UNDERLYING_TYPE_NAME, String::as_str),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumeration;
    use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumerationData;
    use crate::v1_1::bmm_persistence::p_bmm_enumeration_integer::PBmmEnumerationInteger;

    /// The `master04-syntax.adoc` §Enumerated Types `PROPORTION_KIND_2` example.
    fn proportion_kind_2() -> PBmmEnumeration {
        PBmmEnumeration::PBmmEnumerationInteger(PBmmEnumerationInteger {
            documentation: None,
            name: "PROPORTION_KIND_2".to_owned(),
            ancestors: Some(vec!["Integer".to_owned()]),
            constants: None,
            properties: None,
            functions: None,
            invariants: None,
            is_abstract: None,
            is_override: None,
            generic_parameter_defs: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            bmm_class: None,
            uid: 1,
            ancestor_defs: openehr_base::containers::present(Vec::new()),
            item_names: Some(vec!["pk_ratio".to_owned(), "pk_unitary".to_owned()]),
            item_values: Some(vec![
                serde_json::Value::from(0),
                serde_json::Value::from(1001_i64),
            ]),
            item_documentations: openehr_base::containers::present(Vec::new()),
        })
    }

    #[test]
    fn integer_enumeration_reports_its_redefined_underlying_type() {
        let enumeration = proportion_kind_2();
        assert_eq!(enumeration.underlying_type_name(), "INTEGER");
        assert_eq!(enumeration.item_names().len(), 2);
        assert_eq!(enumeration.item_values().len(), 2);
        assert!(enumeration.item_documentations().is_empty());
    }

    #[test]
    fn un_subtyped_enumeration_falls_back_to_its_first_ancestor_then_integer() {
        let mut data = PBmmEnumerationData {
            documentation: None,
            name: "MAGNITUDE_STATUS".to_owned(),
            ancestors: Some(vec!["String".to_owned()]),
            constants: None,
            properties: None,
            functions: None,
            invariants: None,
            is_abstract: None,
            is_override: None,
            generic_parameter_defs: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            bmm_class: None,
            uid: 1,
            ancestor_defs: openehr_base::containers::present(Vec::new()),
            item_names: openehr_base::containers::present(Vec::new()),
            item_values: openehr_base::containers::present(Vec::new()),
            item_documentations: openehr_base::containers::present(Vec::new()),
        };
        assert_eq!(
            PBmmEnumeration::PBmmEnumeration(data.clone()).underlying_type_name(),
            "String"
        );
        data.ancestors = None;
        assert_eq!(
            PBmmEnumeration::PBmmEnumeration(data).underlying_type_name(),
            "Integer"
        );
    }
}
