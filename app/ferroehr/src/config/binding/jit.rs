// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[[authz.party_binding.issuer]]`: what FerroEHR does for one `persons`
//! issuer beyond looking its logins up, which is just-in-time creation of a
//! provisional `PERSON` at a first login.
//!
//! Creation is off by default and needs declared admission conditions (an
//! `idp` allow-list, a required group or role, or a minimum assurance level)
//! and a mapping that can write the binding key, the `userName`, a name and
//! the professional `ROLE`. The person's own data come from the access token
//! (RFC 9068 §2.2.2), and only what the mapping marks is written (GDPR Art.
//! 25(2)). No openEHR spec governs when a party is created at a login: our
//! own design.

use std::collections::BTreeSet;

use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use serde::{Deserialize, Serialize};

use super::PartyBindingConfig;
use super::person::{NEVER_A_KEY, Source};

/// One `[[authz.party_binding.issuer]]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct IssuerBinding {
    /// The `persons` issuer the table holds for.
    pub issuer: String,
    /// Whether a first login with no party creates a provisional `PERSON`.
    pub jit: bool,
    /// Whether a guest the issuer's `[[auth.party]]` table admits is created
    /// too.
    pub guests_jit: bool,
    /// The claim the `userName` is read from; the issuer profile's
    /// `person_key` when unset.
    pub user_name_claim: Option<String>,
    /// The dotted claim path the identity provider is read from.
    pub idp_claim: String,
    /// The identity providers whose logins are created.
    pub admit_idp: Vec<String>,
    /// The groups, read from the `[[auth.party]]` table's `group_claim`, one
    /// of which a created login holds.
    pub admit_groups: Vec<String>,
    /// The roles, read from `[auth] role_claims`, one of which a created
    /// login holds.
    pub admit_roles: Vec<String>,
    /// The lowest `[authz.assurance]` level a created login carries.
    pub admit_assurance: Option<AssuranceLevel>,
}

impl Default for IssuerBinding {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            jit: false,
            guests_jit: false,
            user_name_claim: None,
            // NOTE: Microsoft Entra access-token-claims-reference.md, `idp`: the identity
            // provider that authenticated the subject.
            idp_claim: "idp".to_owned(),
            admit_idp: Vec::new(),
            admit_groups: Vec::new(),
            admit_roles: Vec::new(),
            admit_assurance: None,
        }
    }
}

impl IssuerBinding {
    /// Whether the table declares at least one admission condition.
    #[must_use]
    pub fn admits_by_condition(&self) -> bool {
        !self.admit_idp.is_empty()
            || !self.admit_groups.is_empty()
            || !self.admit_roles.is_empty()
            || self.admit_assurance.is_some()
    }
}

/// The boot refusals of the `[[authz.party_binding.issuer]]` tables of
/// `binding`, judged against the trusted issuers of `resolved` and the levels
/// `assurance` maps.
#[must_use]
pub fn errors(
    binding: &PartyBindingConfig,
    resolved: &Resolved,
    assurance: &AssuranceSection,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for table in &binding.issuer {
        let at = format!(
            "the [[authz.party_binding.issuer]] table of {:?}",
            table.issuer
        );
        if !seen.insert(table.issuer.as_str()) {
            errors.push(format!("{at} is listed twice"));
        }
        let party = resolved.party.iter().find(|p| p.issuer == table.issuer);
        let persons = resolved.kinds.get(&table.issuer) == Some(&IssuerKind::Persons);
        let Some(party) = party.filter(|_| persons) else {
            errors.push(format!(
                "{at} names no persons issuer an [[auth.party]] table binds"
            ));
            continue;
        };
        if table.guests_jit && !table.jit {
            errors.push(format!("{at} sets guests_jit without jit"));
        }
        if table.guests_jit && !party.guests {
            errors.push(format!(
                "{at} sets guests_jit, and its [[auth.party]] table admits no guest"
            ));
        }
        if !table.admit_groups.is_empty() && party.group_claim.trim().is_empty() {
            errors.push(format!(
                "{at} sets admit_groups, and its [[auth.party]] table reads no group_claim"
            ));
        }
        if !table.admit_idp.is_empty() && table.idp_claim.trim().is_empty() {
            errors.push(format!("{at} sets admit_idp with an empty idp_claim"));
        }
        if let Some(level) = table.admit_assurance
            && !assurance.levels.values().any(|mapped| *mapped >= level)
        {
            errors.push(format!(
                "{at} sets admit_assurance = {:?}, which no [authz.assurance] levels value \
                 reaches",
                level.as_str()
            ));
        }
        if let Some(claim) = &table.user_name_claim
            && (claim.trim().is_empty() || NEVER_A_KEY.contains(&claim.as_str()))
        {
            errors.push(format!(
                "{at} reads the userName from {claim:?}: the userName comes from a claim the \
                 issuer controls, never an email address, a phone number, preferred_username, \
                 name, upn or unique_name"
            ));
        }
        if table.jit {
            if !table.admits_by_condition() {
                errors.push(format!(
                    "{at} sets jit without an admission condition: set at least one of \
                     admit_idp, admit_groups, admit_roles and admit_assurance"
                ));
            }
            mapping_errors(binding, &at, &mut errors);
        }
    }
    attribute_errors(binding, &mut errors);
    errors
}

/// The refusals of a mapping just-in-time creation cannot write with.
fn mapping_errors(binding: &PartyBindingConfig, at: &str, errors: &mut Vec<String>) {
    let Some(template) = binding.person.template.iter().find(|t| t.default) else {
        errors.push(format!(
            "{at} sets jit, and no [[authz.party_binding.person.template]] sets default = true"
        ));
        return;
    };
    if template.user_name.is_none() {
        errors.push(format!(
            "{at} sets jit, and the default template {:?} names no user_name selector",
            template.template_id
        ));
    }
    if binding.professional_role.is_none() {
        errors.push(format!(
            "{at} sets jit, which commits the professional ROLE, and \
             [authz.party_binding.professional_role] is not set"
        ));
    }
    let named = binding.person.attribute.iter().any(|attribute| {
        attribute.oidc.is_some()
            && attribute.source() == Source::Name
            && attribute.path_for(&template.template_id).is_some()
    });
    if !named {
        errors.push(format!(
            "{at} sets jit, and no [[authz.party_binding.person.attribute]] maps an OpenID \
             Connect name claim to the default template {:?}",
            template.template_id
        ));
    }
}

/// The refusals of an attribute just-in-time creation is told to write and
/// cannot read.
fn attribute_errors(binding: &PartyBindingConfig, errors: &mut Vec<String>) {
    for attribute in binding.person.attribute.iter().filter(|a| a.jit) {
        if attribute.oidc.is_none() {
            errors.push(format!(
                "{} sets jit = true and names no oidc claim to read it from",
                attribute.label()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::config::{AuthSection, IssuerEntry};
    use ferrohealth_smart::party::config::PartySection;

    use super::*;
    use crate::config::binding::person::{PersonAttribute, PersonTemplate, RoleSection, Selector};

    const ISSUER: &str = "https://login.example";

    fn resolved(guests: bool) -> Resolved {
        let section = AuthSection {
            enabled: true,
            issuer: vec![IssuerEntry {
                issuer: ISSUER.to_owned(),
                kind: Some(IssuerKind::Persons),
                audience: vec!["ferroehr".to_owned()],
                ..IssuerEntry::default()
            }],
            party: vec![PartySection {
                issuer: ISSUER.to_owned(),
                identifier_issuer: "urn:login".to_owned(),
                identifier_type: "guid".to_owned(),
                guests,
                ..PartySection::default()
            }],
            ..AuthSection::default()
        };
        crate::config::auth::resolve(&section).expect("the [auth] section resolves")
    }

    fn mapped() -> PartyBindingConfig {
        let selector = |kind: &str| Selector {
            path: "/details[id2]/items[id3]".to_owned(),
            identifier_type: kind.to_owned(),
        };
        let mut binding = PartyBindingConfig::default();
        binding.person.template = vec![PersonTemplate {
            template_id: "professional".to_owned(),
            key: selector("guid"),
            user_name: Some(selector("scim_user_name")),
            external_id: None,
            default: true,
        }];
        binding.person.attribute = vec![PersonAttribute {
            oidc: Some("name".to_owned()),
            name: true,
            path: [(
                "professional".to_owned(),
                "/identities[id4]/details[id5]/items[id6]".to_owned(),
            )]
            .into(),
            ..PersonAttribute::default()
        }];
        binding.professional_role = Some(RoleSection {
            template_id: "professional role".to_owned(),
        });
        binding
    }

    fn with(table: IssuerBinding) -> PartyBindingConfig {
        PartyBindingConfig {
            issuer: vec![table],
            ..mapped()
        }
    }

    fn jit() -> IssuerBinding {
        IssuerBinding {
            issuer: ISSUER.to_owned(),
            jit: true,
            admit_idp: vec!["https://sts.example/".to_owned()],
            ..IssuerBinding::default()
        }
    }

    #[test]
    fn jit_with_an_admission_condition_and_a_full_mapping_boots() {
        let errors = errors(&with(jit()), &resolved(false), &AssuranceSection::default());
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn jit_without_an_admission_condition_is_refused() {
        let table = IssuerBinding {
            admit_idp: Vec::new(),
            ..jit()
        };
        let errors = errors(&with(table), &resolved(false), &AssuranceSection::default());
        assert!(
            errors
                .iter()
                .any(|e| e.contains("without an admission condition")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_table_of_no_bound_persons_issuer_is_refused() {
        let table = IssuerBinding {
            issuer: "https://elsewhere.example".to_owned(),
            ..jit()
        };
        let errors = errors(&with(table), &resolved(false), &AssuranceSection::default());
        assert!(
            errors.iter().any(|e| e.contains("names no persons issuer")),
            "{errors:?}"
        );
    }

    #[test]
    fn guests_jit_needs_jit_and_a_table_admitting_guests() {
        let table = IssuerBinding {
            jit: false,
            guests_jit: true,
            ..jit()
        };
        let errors = errors(&with(table), &resolved(false), &AssuranceSection::default());
        assert!(
            errors.iter().any(|e| e.contains("guests_jit without jit")),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("admits no guest")),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unreachable_assurance_level_and_an_editable_user_name_claim_are_refused() {
        let table = IssuerBinding {
            admit_assurance: Some(AssuranceLevel::High),
            user_name_claim: Some("preferred_username".to_owned()),
            ..jit()
        };
        let errors = errors(&with(table), &resolved(false), &AssuranceSection::default());
        assert!(
            errors
                .iter()
                .any(|e| e.contains("no [authz.assurance] levels value")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("a claim the issuer controls")),
            "{errors:?}"
        );
    }

    #[test]
    fn jit_needs_a_user_name_selector_a_role_and_a_name_claim() {
        let mut binding = with(jit());
        binding.person.template[0].user_name = None;
        binding.professional_role = None;
        binding.person.attribute[0].oidc = None;
        binding.person.attribute[0].scim = Some("name.formatted".to_owned());
        let errors = errors(&binding, &resolved(false), &AssuranceSection::default());
        for wanted in ["user_name selector", "professional_role", "name claim"] {
            assert!(
                errors.iter().any(|e| e.contains(wanted)),
                "{wanted}: {errors:?}"
            );
        }
    }

    #[test]
    fn a_jit_attribute_without_a_claim_is_refused() {
        let mut binding = with(jit());
        binding.person.attribute.push(PersonAttribute {
            scim: Some("emails[type eq \"work\"].value".to_owned()),
            jit: true,
            ..PersonAttribute::default()
        });
        let errors = errors(&binding, &resolved(false), &AssuranceSection::default());
        assert!(
            errors.iter().any(|e| e.contains("names no oidc claim")),
            "{errors:?}"
        );
    }
}
