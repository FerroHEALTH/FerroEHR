// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The schemas RFC 7643 defines: the core User (§4.1), the core Group (§4.2),
//! the Enterprise User extension (§4.3) and the common attributes (§3.1).
//!
//! Every characteristic is the one the RFC 7643 §8.7.1 representation prints,
//! which a test reads from the vendored RFC and compares; the descriptions are
//! our own words. Three entries depart from §8.7.1, each where the RFC
//! contradicts that non-normative example ("Where permitted, individual
//! values and schema MAY change"): Group `displayName` is required (§4.2),
//! and `members` carries `display` and `addresses` carries `primary`, which
//! the RFC's own examples send.

use crate::scim::protocol::schema::{
    AttributeDefinition, AttributeType, Mutability, Returned, Schema, Uniqueness,
};
use crate::scim::protocol::urn;

/// Returns the core User schema (RFC 7643 §4.1).
#[must_use]
pub fn core_user() -> Schema {
    let mut attributes = user_singular_attributes();
    attributes.extend(user_multi_valued_attributes());
    schema(urn::CORE_USER, "User", "User Account", attributes)
}

/// The singular attributes of the User (RFC 7643 §4.1.1).
fn user_singular_attributes() -> Vec<AttributeDefinition> {
    vec![
        string(
            "userName",
            "The provider's unique identifier for the user, with which the user signs in.",
        )
        .required()
        .unique(Uniqueness::Server),
        complex(
            "name",
            "The components of the user's name.",
            vec![
                string("formatted", "The full name, formatted for display."),
                string("familyName", "The family name, or last name."),
                string("givenName", "The given name, or first name."),
                string("middleName", "The middle names."),
                string("honorificPrefix", "The honorific prefixes, or title."),
                string("honorificSuffix", "The honorific suffixes."),
            ],
        )
        .unique(Uniqueness::None),
        string("displayName", "The name of the user, suitable for display."),
        string("nickName", "The casual way to address the user."),
        reference(
            "profileUrl",
            "A URL of the user's online profile.",
            &["external"],
        ),
        string("title", "The user's title, such as a job title."),
        string(
            "userType",
            "The relationship between the organization and the user, such as Employee.",
        ),
        string(
            "preferredLanguage",
            "The user's preferred languages, as an Accept-Language value.",
        ),
        string(
            "locale",
            "The user's default location for localization, as a language tag.",
        ),
        string(
            "timezone",
            "The user's time zone, in IANA Time Zone database format.",
        ),
        boolean("active", "The user's administrative status."),
        string(
            "password",
            "The user's cleartext password, which is never returned.",
        )
        .mutability(Mutability::WriteOnly)
        .returned(Returned::Never),
    ]
}

/// The multi-valued attributes of the User (RFC 7643 §4.1.2).
fn user_multi_valued_attributes() -> Vec<AttributeDefinition> {
    vec![
        typed_values(
            "emails",
            "The user's email addresses.",
            string("value", "An email address."),
            &["work", "home", "other"],
        )
        .unique(Uniqueness::None),
        typed_values(
            "phoneNumbers",
            "The user's phone numbers.",
            string("value", "A phone number."),
            &["work", "home", "mobile", "fax", "pager", "other"],
        ),
        typed_values(
            "ims",
            "The user's instant messaging addresses.",
            string("value", "An instant messaging address."),
            &["aim", "gtalk", "icq", "xmpp", "msn", "skype", "qq", "yahoo"],
        ),
        typed_values(
            "photos",
            "URLs of images of the user.",
            reference("value", "The URL of an image of the user.", &["external"]),
            &["photo", "thumbnail"],
        ),
        complex(
            "addresses",
            "The user's physical mailing addresses.",
            vec![
                string(
                    "formatted",
                    "The full address, formatted for display or a mailing label.",
                ),
                string(
                    "streetAddress",
                    "The street address, with house number and street name.",
                ),
                string("locality", "The city or locality."),
                string("region", "The state or region."),
                string("postalCode", "The postal code."),
                string("country", "The country, as an ISO 3166-1 alpha-2 code."),
                string("type", "The kind of address.").canonical(&["work", "home", "other"]),
                // NOTE: RFC 7643 §2.4 and the RFC 7644 §3.5.2.3 example give addresses a `primary`; §8.7.1 omits it.
                boolean("primary", "Whether this is the user's preferred address."),
            ],
        )
        .multi()
        .unique(Uniqueness::None),
        complex(
            "groups",
            "The groups the user belongs to, which change only through the Group resource.",
            vec![
                string("value", "The id of the group.").mutability(Mutability::ReadOnly),
                reference("$ref", "The URI of the group.", &["User", "Group"])
                    .mutability(Mutability::ReadOnly),
                string("display", "The name of the group, for display.")
                    .mutability(Mutability::ReadOnly),
                string("type", "How the membership is derived.")
                    .canonical(&["direct", "indirect"])
                    .mutability(Mutability::ReadOnly),
            ],
        )
        .multi()
        .mutability(Mutability::ReadOnly),
        typed_values(
            "entitlements",
            "The things the user has.",
            string("value", "An entitlement."),
            &[],
        )
        .without_canonical_types(),
        typed_values(
            "roles",
            "The roles of the user.",
            string("value", "A role."),
            &[],
        ),
        typed_values(
            "x509Certificates",
            "The user's X.509 certificates.",
            binary("value", "A DER-encoded X.509 certificate, in base64."),
            &[],
        )
        .case_exact(false),
    ]
}

/// Returns the core Group schema (RFC 7643 §4.2).
#[must_use]
pub fn core_group() -> Schema {
    schema(
        urn::CORE_GROUP,
        "Group",
        "Group",
        vec![
            // NOTE: RFC 7643 §4.2 makes displayName "REQUIRED"; the §8.7.1 example prints false.
            string("displayName", "The name of the group, for display.").required(),
            complex(
                "members",
                "The members of the group, whose sub-attributes are immutable.",
                vec![
                    string("value", "The id of the member.").mutability(Mutability::Immutable),
                    reference("$ref", "The URI of the member.", &["User", "Group"])
                        .mutability(Mutability::Immutable),
                    string("type", "The resource type of the member.")
                        .canonical(&["User", "Group"])
                        .mutability(Mutability::Immutable),
                    // NOTE: RFC 7643 §8.4 and the RFC 7644 §3.5.2.1 examples send members' `display`; §8.7.1 omits it.
                    string("display", "The name of the member, for display.")
                        .mutability(Mutability::Immutable),
                ],
            )
            .multi(),
        ],
    )
}

/// Returns the Enterprise User extension (RFC 7643 §4.3).
#[must_use]
pub fn enterprise_user() -> Schema {
    schema(
        urn::ENTERPRISE_USER,
        "EnterpriseUser",
        "Enterprise User",
        vec![
            string(
                "employeeNumber",
                "The identifier the organization assigns to the person.",
            ),
            string("costCenter", "The name of a cost center."),
            string("organization", "The name of an organization."),
            string("division", "The name of a division."),
            string("department", "The name of a department."),
            complex(
                "manager",
                "The user's manager.",
                vec![
                    string("value", "The id of the manager's User resource."),
                    reference("$ref", "The URI of the manager's User resource.", &["User"]),
                    string("displayName", "The display name of the manager.")
                        .mutability(Mutability::ReadOnly),
                ],
            ),
        ],
    )
}

/// Returns the common attributes every resource carries outside its schema
/// (RFC 7643 §3.1): `id`, `externalId` and `meta`.
#[must_use]
pub fn common_attributes() -> Vec<AttributeDefinition> {
    vec![
        string("id", "The provider's stable identifier of the resource.")
            .case_exact(true)
            .mutability(Mutability::ReadOnly)
            .returned(Returned::Always)
            .unique(Uniqueness::Server),
        string(
            "externalId",
            "The provisioning client's identifier of the resource.",
        )
        .case_exact(true),
        complex(
            "meta",
            "The resource's metadata, which the provider assigns.",
            vec![
                string("resourceType", "The name of the resource's type.")
                    .case_exact(true)
                    .mutability(Mutability::ReadOnly),
                date_time("created", "When the resource was added."),
                date_time("lastModified", "When the resource was last updated."),
                reference("location", "The URI of the resource.", &["uri"])
                    .mutability(Mutability::ReadOnly),
                string("version", "The version of the resource, its entity tag.")
                    .case_exact(true)
                    .mutability(Mutability::ReadOnly),
            ],
        )
        .mutability(Mutability::ReadOnly),
    ]
}

fn schema(id: &str, name: &str, description: &str, attributes: Vec<AttributeDefinition>) -> Schema {
    Schema {
        schemas: None,
        id: id.to_owned(),
        name: Some(name.to_owned()),
        description: Some(description.to_owned()),
        attributes,
        meta: None,
    }
}

/// A single-valued, optional, read-write boolean attribute, returned by
/// default, as RFC 7643 §2.2 defaults every characteristic but the type; the
/// other builders change what differs.
fn boolean(name: &str, description: &str) -> AttributeDefinition {
    AttributeDefinition {
        name: name.to_owned(),
        kind: AttributeType::Boolean,
        sub_attributes: None,
        multi_valued: false,
        description: description.to_owned(),
        required: false,
        canonical_values: None,
        case_exact: None,
        mutability: Mutability::ReadWrite,
        returned: Returned::Default,
        uniqueness: None,
        reference_types: None,
    }
}

/// A string attribute: §8.7.1 prints `caseExact` and `uniqueness` for each
/// textual attribute.
fn string(name: &str, description: &str) -> AttributeDefinition {
    AttributeDefinition {
        kind: AttributeType::String,
        case_exact: Some(false),
        uniqueness: Some(Uniqueness::None),
        ..boolean(name, description)
    }
}

fn binary(name: &str, description: &str) -> AttributeDefinition {
    AttributeDefinition {
        kind: AttributeType::Binary,
        ..string(name, description)
    }
}

fn reference(name: &str, description: &str, types: &[&str]) -> AttributeDefinition {
    AttributeDefinition {
        kind: AttributeType::Reference,
        reference_types: Some(types.iter().map(|t| (*t).to_owned()).collect()),
        ..string(name, description)
    }
}

fn date_time(name: &str, description: &str) -> AttributeDefinition {
    AttributeDefinition {
        kind: AttributeType::DateTime,
        mutability: Mutability::ReadOnly,
        ..boolean(name, description)
    }
}

fn complex(
    name: &str,
    description: &str,
    sub_attributes: Vec<AttributeDefinition>,
) -> AttributeDefinition {
    AttributeDefinition {
        kind: AttributeType::Complex,
        sub_attributes: Some(sub_attributes),
        ..boolean(name, description)
    }
}

/// A multi-valued complex attribute with the default sub-attributes of RFC
/// 7643 §2.4: `value`, `display`, `type` with its canonical values, and `primary`.
fn typed_values(
    name: &str,
    description: &str,
    value: AttributeDefinition,
    types: &[&str],
) -> AttributeDefinition {
    complex(
        name,
        description,
        vec![
            value,
            string(
                "display",
                "A human-readable form of the value, for display.",
            ),
            string(
                "type",
                "A label for the value's function, such as work or home.",
            )
            .canonical(types),
            boolean(
                "primary",
                "Whether this is the preferred value; true appears at most once.",
            ),
        ],
    )
    .multi()
}

impl AttributeDefinition {
    const fn required(mut self) -> Self {
        self.required = true;
        self
    }

    const fn multi(mut self) -> Self {
        self.multi_valued = true;
        self
    }

    const fn mutability(mut self, mutability: Mutability) -> Self {
        self.mutability = mutability;
        self
    }

    const fn returned(mut self, returned: Returned) -> Self {
        self.returned = returned;
        self
    }

    const fn unique(mut self, uniqueness: Uniqueness) -> Self {
        self.uniqueness = Some(uniqueness);
        self
    }

    const fn case_exact(mut self, case_exact: bool) -> Self {
        self.case_exact = Some(case_exact);
        self
    }

    fn canonical(mut self, values: &[&str]) -> Self {
        self.canonical_values = Some(values.iter().map(|v| (*v).to_owned()).collect());
        self
    }

    /// Drops the canonical values of the `type` sub-attribute, which §8.7.1
    /// gives no list for `entitlements`.
    fn without_canonical_types(mut self) -> Self {
        if let Some(subs) = self.sub_attributes.as_mut() {
            for sub in subs.iter_mut().filter(|sub| sub.name == "type") {
                sub.canonical_values = None;
            }
        }
        self
    }
}
