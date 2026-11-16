// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The default party mapping: `[authz.party_binding]` with no key set names
//! the shipped definitions of `deploy/definitions/`, which every boot stores
//! when absent. No openEHR spec governs a default mapping: our own design.

use crate::config::binding::{PartyBindingConfig, person};
use crate::party_binding::shipped;

impl Default for PartyBindingConfig {
    fn default() -> Self {
        Self {
            provider: None,
            // NOTE: ITS-REST overview, Requests_and_responses.md §openehr-version, names
            // `committer.external_ref.namespace="demographic"` in its example.
            namespace: "demographic".to_owned(),
            basic_user: Vec::new(),
            // The default mapping names the shipped definitions of
            // `deploy/definitions/`, which every boot stores when absent.
            // The patient template is a person template too, never the default: a SCIM
            // client of a patients issuer provisions a portal representative in it (#5622).
            person: Some(person::PersonSection {
                template: vec![
                    person_template(shipped::PROFESSIONAL_PERSON, true),
                    person_template(shipped::PATIENT, false),
                ],
                attribute: vec![
                    attribute(
                        "name.formatted",
                        true,
                        &[
                            "/identities[id4]/details[id5]/items[id6]",
                            "/identities[id6]/details[id7]/items[id8]",
                        ],
                    ),
                    attribute(
                        "name.givenName",
                        false,
                        &[
                            "/identities[id4]/details[id5]/items[id7]",
                            "/identities[id6]/details[id7]/items[id9]",
                        ],
                    ),
                    attribute(
                        "name.familyName",
                        false,
                        &[
                            "/identities[id4]/details[id5]/items[id8]",
                            "/identities[id6]/details[id7]/items[id10]",
                        ],
                    ),
                    attribute(
                        "emails[type eq \"work\"].value",
                        false,
                        &["/contacts[id9]/addresses[id10]/details[id11]/items[id12]"],
                    ),
                ],
            }),
            professional_role: Some(person::RoleSection {
                template_id: shipped::PROFESSIONAL_ROLE.to_owned(),
            }),
            group: Some(person::GroupSection {
                template_id: shipped::CARE_TEAM.to_owned(),
                external_id: selector(IDENTIFIER, "scim_external_id"),
                display_name: Some("/identities[id4]/details[id5]/items[id6]".to_owned()),
            }),
            membership: Some(person::MembershipSection {
                archetype_id: shipped::MEMBERSHIP.to_owned(),
            }),
            patient: Some(person::PatientSection {
                template_id: shipped::PATIENT.to_owned(),
                identifier: IDENTIFIER.to_owned(),
                name: Some("/identities[id6]/details[id7]/items[id8]".to_owned()),
            }),
            issuer: Vec::new(),
            provisional_report_after: ferrohealth_config::units::Duration::from_days(30),
        }
    }
}

/// The identifier element of each default `PERSON` and `GROUP` definition.
const IDENTIFIER: &str = "/details[id2]/items[id3]";

/// The selector of the identifiers of `identifier_type` at `path`.
fn selector(path: &str, identifier_type: &str) -> person::Selector {
    person::Selector {
        path: path.to_owned(),
        identifier_type: identifier_type.to_owned(),
    }
}

/// The attribute mapping the SCIM attribute `scim` to `paths`: the first of
/// the default professional's `PERSON`, the second, when given, of the
/// patient's.
fn attribute(scim: &str, name: bool, paths: &[&str]) -> person::PersonAttribute {
    let templates = [shipped::PROFESSIONAL_PERSON, shipped::PATIENT];
    person::PersonAttribute {
        scim: Some(scim.to_owned()),
        oidc: None,
        jit: false,
        name,
        path: templates
            .iter()
            .zip(paths)
            .map(|(template, path)| ((*template).to_owned(), (*path).to_owned()))
            .collect(),
    }
}

/// The default person template `template_id`, its key, `userName` and
/// `externalId` identifiers at the shared identifier element.
fn person_template(template_id: &str, default: bool) -> person::PersonTemplate {
    person::PersonTemplate {
        template_id: template_id.to_owned(),
        key: selector(IDENTIFIER, "login"),
        user_name: Some(selector(IDENTIFIER, "scim_user_name")),
        external_id: Some(selector(IDENTIFIER, "scim_external_id")),
        default,
    }
}
