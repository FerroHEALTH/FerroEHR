// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[[authz.party_binding.issuer]]`: how a `persons` issuer's tokens are
//! read when a first login creates a provisional `PERSON`.
//!
//! The table names the claim the `userName` comes from and the one naming
//! the identity provider.
//!
//! Whether a login is created is policy, held in the CDR: a
//! `create_at_login` grant of the operation terminology names the issuer and
//! its admission conditions
//! ([`crate::party_binding::grants::admission`]), read on every first
//! login. This table only says how the token is read, and creation needs a
//! mapping that can write the binding key, the `userName`, a name and the
//! professional `ROLE` ([`mapping_errors`]). The person's own data come from
//! the access token (RFC 9068 §2.2.2), and only what the mapping marks is
//! written (GDPR Art. 25(2)). No openEHR spec governs when a party is
//! created at a login: our own design.

use std::collections::BTreeSet;

use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use serde::{Deserialize, Serialize};

use super::PartyBindingConfig;
use super::person::{NEVER_A_KEY, Source};

/// One `[[authz.party_binding.issuer]]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct IssuerBinding {
    /// The `persons` issuer the table holds for. It renders with any userinfo masked.
    pub issuer: SecretUrl,
    /// The claim the `userName` is read from; the issuer profile's
    /// `person_key` when unset.
    pub user_name_claim: Option<String>,
    /// The dotted claim path the identity provider an `admit_idp` admission
    /// condition matches is read from.
    pub idp_claim: String,
}

impl Default for IssuerBinding {
    fn default() -> Self {
        Self {
            issuer: SecretUrl::default(),
            user_name_claim: None,
            // NOTE: Microsoft Entra access-token-claims-reference.md, `idp`: the identity
            // provider that authenticated the subject.
            idp_claim: "idp".to_owned(),
        }
    }
}

/// The boot refusals of the `[[authz.party_binding.issuer]]` tables of
/// `binding`, judged against the trusted issuers of `resolved`.
#[must_use]
pub fn errors(binding: &PartyBindingConfig, resolved: &Resolved) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for table in &binding.issuer {
        let at = format!(
            "the [[authz.party_binding.issuer]] table of {:?}",
            table.issuer
        );
        if !seen.insert(table.issuer.expose()) {
            errors.push(format!("{at} is listed twice"));
        }
        let bound = resolved.party.iter().any(|p| p.issuer == table.issuer);
        let persons = resolved.kinds.get(table.issuer.expose()) == Some(&IssuerKind::Persons);
        if !(bound && persons) {
            errors.push(format!(
                "{at} names no persons issuer an [[auth.party]] table binds"
            ));
            continue;
        }
        if table.idp_claim.trim().is_empty() {
            errors.push(format!("{at} sets an empty idp_claim"));
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
    }
    attribute_errors(binding, &mut errors);
    errors
}

/// The `persons` issuers an `[[auth.party]]` table of `resolved` binds: the
/// issuers whose first logins a `create_at_login` grant may create a
/// `PERSON` for.
#[must_use]
pub fn login_issuers(resolved: &Resolved) -> Vec<SecretUrl> {
    resolved
        .party
        .iter()
        .filter(|table| resolved.kinds.get(table.issuer.expose()) == Some(&IssuerKind::Persons))
        .map(|table| table.issuer.clone())
        .collect()
}

/// Why the mapping of `binding` cannot write a provisional `PERSON`.
///
/// It needs a default person template with a `userName` selector, a
/// professional `ROLE`, and a name attribute read from an OpenID Connect
/// claim. Empty when just-in-time creation can write.
#[must_use]
pub fn mapping_errors(binding: &PartyBindingConfig) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(template) = binding.person().template.iter().find(|t| t.default) else {
        errors.push("no [[authz.party_binding.person.template]] sets default = true".to_owned());
        return errors;
    };
    if template.user_name.is_none() {
        errors.push(format!(
            "the default template {:?} names no user_name selector",
            template.template_id
        ));
    }
    if binding.professional_role.is_none() {
        errors.push(
            "[authz.party_binding.professional_role], the ROLE a created PERSON performs, is \
             not set"
                .to_owned(),
        );
    }
    let named = binding.person().attribute.iter().any(|attribute| {
        attribute.oidc.is_some()
            && attribute.source() == Source::Name
            && attribute.path_for(&template.template_id).is_some()
    });
    if !named {
        errors.push(format!(
            "no [[authz.party_binding.person.attribute]] maps an OpenID Connect name claim to \
             the default template {:?}",
            template.template_id
        ));
    }
    errors
}

/// The refusals of an attribute just-in-time creation is told to write and
/// cannot read.
fn attribute_errors(binding: &PartyBindingConfig, errors: &mut Vec<String>) {
    for attribute in binding.person().attribute.iter().filter(|a| a.jit) {
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

    fn resolved() -> Resolved {
        let section = AuthSection {
            enabled: true,
            issuer: vec![IssuerEntry {
                issuer: SecretUrl::new(ISSUER),
                kind: Some(IssuerKind::Persons),
                audience: vec!["ferroehr".to_owned()],
                ..IssuerEntry::default()
            }],
            party: vec![PartySection {
                issuer: SecretUrl::new(ISSUER),
                identifier_issuer: "urn:login".to_owned(),
                identifier_type: "guid".to_owned(),
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
        binding.person.get_or_insert_default().template = vec![PersonTemplate {
            template_id: "professional".to_owned(),
            key: selector("guid"),
            user_name: Some(selector("scim_user_name")),
            external_id: None,
            default: true,
        }];
        binding.person.get_or_insert_default().attribute = vec![PersonAttribute {
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

    fn table() -> IssuerBinding {
        IssuerBinding {
            issuer: SecretUrl::new(ISSUER),
            ..IssuerBinding::default()
        }
    }

    #[test]
    fn a_table_of_a_bound_persons_issuer_and_a_full_mapping_boot() {
        let errors = errors(&with(table()), &resolved());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(mapping_errors(&mapped()), Vec::<String>::new());
    }

    #[test]
    fn a_table_of_no_bound_persons_issuer_is_refused() {
        let other = IssuerBinding {
            issuer: SecretUrl::new("https://elsewhere.example"),
            ..table()
        };
        let errors = errors(&with(other), &resolved());
        assert!(
            errors.iter().any(|e| e.contains("names no persons issuer")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_table_listed_twice_an_empty_idp_claim_and_an_editable_user_name_claim_are_refused() {
        let odd = IssuerBinding {
            idp_claim: " ".to_owned(),
            user_name_claim: Some("preferred_username".to_owned()),
            ..table()
        };
        let mut binding = with(odd);
        binding.issuer.push(table());
        let errors = errors(&binding, &resolved());
        for wanted in [
            "listed twice",
            "empty idp_claim",
            "a claim the issuer controls",
        ] {
            assert!(
                errors.iter().any(|e| e.contains(wanted)),
                "{wanted}: {errors:?}"
            );
        }
    }

    #[test]
    fn creation_needs_a_user_name_selector_a_role_and_a_name_claim() {
        let mut binding = mapped();
        binding.person.get_or_insert_default().template[0].user_name = None;
        binding.professional_role = None;
        binding.person.get_or_insert_default().attribute[0].oidc = None;
        binding.person.get_or_insert_default().attribute[0].scim =
            Some("name.formatted".to_owned());
        let errors = mapping_errors(&binding);
        for wanted in ["user_name selector", "professional_role", "name claim"] {
            assert!(
                errors.iter().any(|e| e.contains(wanted)),
                "{wanted}: {errors:?}"
            );
        }
        binding.person.get_or_insert_default().template[0].default = false;
        assert!(
            mapping_errors(&binding)
                .iter()
                .any(|e| e.contains("default = true")),
            "no default template"
        );
    }

    #[test]
    fn the_admission_keys_are_unknown_keys() {
        for key in [
            "jit = true",
            "guests_jit = true",
            "admit_idp = [\"https://sts.example/\"]",
            "admit_groups = [\"clinicians\"]",
            "admit_roles = [\"clinical\"]",
            "admit_assurance = \"substantial\"",
        ] {
            let table = format!("issuer = \"{ISSUER}\"\n{key}\n");
            assert!(
                toml::from_str::<IssuerBinding>(&table).is_err(),
                "{key} is CDR data, refused as an unknown key"
            );
        }
        let kept =
            format!("issuer = \"{ISSUER}\"\nuser_name_claim = \"oid\"\nidp_claim = \"idp\"\n");
        assert!(toml::from_str::<IssuerBinding>(&kept).is_ok());
    }

    #[test]
    fn a_jit_attribute_without_a_claim_is_refused() {
        let mut binding = with(table());
        binding
            .person
            .get_or_insert_default()
            .attribute
            .push(PersonAttribute {
                scim: Some("emails[type eq \"work\"].value".to_owned()),
                jit: true,
                ..PersonAttribute::default()
            });
        let errors = errors(&binding, &resolved());
        assert!(
            errors.iter().any(|e| e.contains("names no oidc claim")),
            "{errors:?}"
        );
    }
}
