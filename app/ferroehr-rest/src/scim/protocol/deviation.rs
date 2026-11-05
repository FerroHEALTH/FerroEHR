// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The client deviations from RFC 7643 and RFC 7644 that the provider
//! tolerates as input.
//!
//! The provider follows RFC 7644. A client that is documented to send
//! something the RFCs do not allow is met here and nowhere else: each
//! [`Deviation`] names the rule it departs from and the vendored document that
//! shows the client sending it, and every parser that applies one reports it,
//! so a caller can record which tolerances a request needed. Nothing in this
//! layer names a client; the sources happen to be Microsoft's Entra ID
//! provisioning documents, vendored under `docs/specs/microsoft-entra/`.

use crate::scim::protocol::filter::{AttrExp, Logic, ValueFilter};
use crate::scim::protocol::path::AttrName;

/// The URN prefix of a schema extension in the SCIM registry namespace.
const EXTENSION_PREFIX: &str = "urn:ietf:params:scim:schemas:extension:";

/// A tolerated client deviation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Deviation {
    /// A PATCH `op` in another case than `add`, `remove` or `replace`, read as
    /// the operation it names.
    OperationCase,
    /// A `remove` of a multi-valued attribute with no filter and a `value`
    /// list, read as removing only the listed values.
    RemoveValueList,
    /// An extension container whose URN the body's `schemas` omits, read as
    /// if `schemas` listed it.
    SchemasOmitExtension,
    /// A `schemas` entry the resource type does not define, under which the
    /// body carries no attribute, ignored.
    UnlistedSchemaUri,
    /// A value path followed by a sub-attribute comparison in a filter,
    /// `emails[type eq "work"].value eq "x"`, read as the value filter
    /// `emails[type eq "work" and value eq "x"]`.
    ValuePathSubAttribute,
    /// A provider extension URN in the IANA-assigned `urn:ietf:params:scim`
    /// namespace without a registration, which the client requires for a
    /// custom attribute.
    UnregisteredExtensionUrn,
    /// The client never sends a null, so a value it clears never reaches the
    /// provider; nothing in a request shows this, and the provider keeps the
    /// value until it is removed another way.
    NoNulls,
}

impl Deviation {
    /// Every deviation.
    pub const ALL: [Self; 7] = [
        Self::OperationCase,
        Self::RemoveValueList,
        Self::SchemasOmitExtension,
        Self::UnlistedSchemaUri,
        Self::ValuePathSubAttribute,
        Self::UnregisteredExtensionUrn,
        Self::NoNulls,
    ];

    /// Returns the vendored document, and the section in it, that shows the
    /// client sending the deviation.
    #[must_use]
    pub const fn source(self) -> &'static str {
        match self {
            Self::OperationCase => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md",
                ", General: \"Microsoft Entra ID emits the values of op as Add, Replace, and Remove\""
            ),
            Self::RemoveValueList => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md",
                ", Update Group [Remove Members]"
            ),
            Self::SchemasOmitExtension => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/customize-application-attributes.md",
                ", Provisioning a custom extension attribute: \"The custom extension schema header is omitted\""
            ),
            Self::UnlistedSchemaUri => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md",
                ", Create Group"
            ),
            Self::ValuePathSubAttribute => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md",
                ", /Users: emails[type eq \"work\"].value eq \"user@contoso.com\""
            ),
            Self::UnregisteredExtensionUrn => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/customize-application-attributes.md",
                ", Provisioning a custom extension attribute: \"the attribute name must follow the pattern\""
            ),
            Self::NoNulls => concat!(
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/customize-application-attributes.md",
                ", What you should know: \"doesn't send null values to target systems\""
            ),
        }
    }

    /// Returns the rule of RFC 7643 or RFC 7644 the deviation departs from.
    #[must_use]
    pub const fn departs_from(self) -> &'static str {
        match self {
            Self::OperationCase => {
                "RFC 7644 §3.5.2: op MAY be one of \"add\", \"remove\", or \"replace\""
            }
            Self::RemoveValueList => {
                "RFC 7644 §3.5.2.2: with no filter, \"the attribute and all values are removed\""
            }
            Self::SchemasOmitExtension => {
                "RFC 7643 §3: \"schemas\" contains the URIs that \"define the attributes present\""
            }
            Self::UnlistedSchemaUri => {
                "RFC 7643 §3: \"schemas\" MUST only contain the resource type's \"schema\" and \"schemaExtensions\""
            }
            Self::ValuePathSubAttribute => {
                "RFC 7644 §3.4.2.2, Figure 1: a valuePath carries no subAttr in a filter"
            }
            Self::UnregisteredExtensionUrn => {
                "RFC 7643 §10.2.1: schema identifiers in urn:ietf:params:scim are assigned after review"
            }
            Self::NoNulls => "RFC 7644 §3.5.1: clients MAY specify null to clear a value",
        }
    }
}

/// Returns the URN of a provider's own extension in the form the client
/// requires for a custom attribute,
/// `urn:ietf:params:scim:schemas:extension:<name>:2.0:<resource>`.
///
/// The URN sits in the `urn:ietf:params:scim` namespace without a
/// registration: [`Deviation::UnregisteredExtensionUrn`].
#[must_use]
pub fn custom_extension_urn(name: &str, resource_type: &str) -> String {
    format!("{EXTENSION_PREFIX}{name}:2.0:{resource_type}")
}

/// Returns whether `id` has the form [`custom_extension_urn`] builds.
#[must_use]
pub fn is_custom_extension_urn(id: &str) -> bool {
    let prefix = id
        .get(..EXTENSION_PREFIX.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(EXTENSION_PREFIX));
    let Some(rest) = id.get(EXTENSION_PREFIX.len()..).filter(|_rest| prefix) else {
        return false;
    };
    let mut parts = rest.split(':');
    let name = parts
        .next()
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let version = parts.next() == Some("2.0");
    let resource = parts
        .next()
        .is_some_and(|r| r.chars().next().is_some_and(|c| c.is_ascii_alphabetic()));
    name && version && resource && parts.next().is_none()
}

/// Folds the sub-attribute comparison that follows a value path into its
/// value filter: [`Deviation::ValuePathSubAttribute`].
///
/// "Any value of `emails` with `type eq "work"` whose `value` equals x" is
/// the meaning of both forms, so the result is a standard value filter.
#[must_use]
pub fn value_path_comparison(filter: ValueFilter, comparison: AttrExp<AttrName>) -> ValueFilter {
    match filter {
        Logic::And(mut terms) => {
            terms.push(Logic::Leaf(comparison));
            Logic::And(terms)
        }
        other => Logic::And(vec![other, Logic::Leaf(comparison)]),
    }
}

/// Returns the PATCH operation name `op` spells in another case than RFC 7644
/// §3.5.2 does, with [`Deviation::OperationCase`]; `None` when `op` names no
/// operation in any case, or already is the lower-case name.
#[must_use]
pub fn operation_in_any_case(op: &str) -> Option<&'static str> {
    ["add", "remove", "replace"]
        .into_iter()
        .find(|name| *name != op && name.eq_ignore_ascii_case(op))
}

#[cfg(test)]
mod tests {
    use super::{custom_extension_urn, is_custom_extension_urn, operation_in_any_case};

    #[test]
    fn the_custom_urn_form_is_recognised() {
        assert!(is_custom_extension_urn(&custom_extension_urn(
            "ferrohealth",
            "User"
        )));
        assert!(is_custom_extension_urn(
            "urn:ietf:params:scim:schemas:extension:CustomExtensionName:2.0:User"
        ));
        for not_custom in [
            "urn:ietf:params:scim:schemas:core:2.0:Badge",
            "urn:ietf:params:scim:schemas:extension:x:1.0:User",
            "urn:ietf:params:scim:schemas:extension::2.0:User",
            "urn:ietf:params:scim:schemas:extension:x:2.0:User:extra",
            "urn:example:x",
        ] {
            assert!(!is_custom_extension_urn(not_custom), "{not_custom}");
        }
    }

    #[test]
    fn only_another_case_is_a_deviation() {
        assert_eq!(operation_in_any_case("Add"), Some("add"));
        assert_eq!(operation_in_any_case("REPLACE"), Some("replace"));
        assert_eq!(operation_in_any_case("remove"), None);
        assert_eq!(operation_in_any_case("move"), None);
    }
}
