// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Just-in-time creation at the login binding: which caller of an issuer with
//! `jit = true` gets a provisional `PERSON` at a first login, and what it is
//! written from.
//!
//! Only a person or a delegated caller of that issuer whose login no party
//! carries is created, and only when the token meets every admission
//! condition the issuer's `[[authz.party_binding.issuer]]` table declares and
//! carries a mapped name claim; a guest only under `guests_jit`. The person's
//! data are read from the access token (OpenID Connect Core 1.0 §5.1, RFC
//! 9068 §2.2.2): the binding key, the `userName`, the names, and every other
//! attribute only where its mapping sets `jit = true` (GDPR Art. 25(2)). No
//! openEHR spec governs when a login creates a party: our own design.

use std::collections::BTreeMap;

use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::binding::jit::IssuerBinding;
use ferroehr::config::binding::person::{PersonAttribute, Source};
use ferroehr::service::demographic::jit::ProvisionalPerson;
use ferrohealth_smart::assurance::AssuranceSection;
use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::resource::{Caller, Via};

/// One mapped attribute just-in-time creation reads from a claim: the
/// claim, the path in the default template, and the `DV_IDENTIFIER.type` an
/// identifier there is written as.
#[derive(Debug, Clone)]
struct Claimed {
    claim: String,
    path: String,
    identifier_type: String,
}

/// What just-in-time creation needs for one issuer.
#[derive(Debug, Clone)]
pub(super) struct Jit {
    /// The issuer's table.
    table: IssuerBinding,
    /// The `identifier_issuer` of the issuer's `[[auth.party]]` table.
    key_issuer: String,
    /// The `group_claim` of that table.
    group_claim: String,
    /// The default person template.
    template_id: String,
    /// The professional `ROLE` template.
    role_template: String,
    /// The key selector's path and type.
    key: (String, String),
    /// The `userName` selector's path and type.
    user_name: (String, String),
    /// The name attributes, the one the committer reads first.
    names: Vec<Claimed>,
    /// The other attributes whose mapping sets `jit = true`.
    others: Vec<Claimed>,
}

/// Why a caller is not created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NotAdmitted {
    /// The token's identity provider is not on `admit_idp`.
    Idp,
    /// The token holds no group of `admit_groups`.
    Group,
    /// The caller holds no role of `admit_roles`.
    Role,
    /// The token's assurance level is below `admit_assurance`.
    Assurance,
    /// The token carries no mapped name claim.
    NoName,
}

impl NotAdmitted {
    /// The reason a log line names.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Idp => "idp",
            Self::Group => "group",
            Self::Role => "role",
            Self::Assurance => "assurance",
            Self::NoName => "no_name_claim",
        }
    }
}

/// The just-in-time creation of each issuer whose table sets `jit = true`,
/// keyed by issuer; empty when the mapping cannot write, which boot refuses.
pub(super) fn tables(binding: &PartyBindingConfig, resolved: &Resolved) -> BTreeMap<String, Jit> {
    let mut out = BTreeMap::new();
    let Some(template) = binding.person.template.iter().find(|t| t.default) else {
        return out;
    };
    let (Some(user_name), Some(role)) = (&template.user_name, &binding.professional_role) else {
        return out;
    };
    let claimed = |attribute: &PersonAttribute| {
        Some(Claimed {
            claim: attribute.oidc.clone()?,
            path: attribute.path_for(&template.template_id)?.to_owned(),
            identifier_type: attribute.oidc.clone()?,
        })
    };
    let mut names: Vec<(bool, Claimed)> = binding
        .person
        .attribute
        .iter()
        .filter(|a| a.source() == Source::Name)
        .filter_map(|a| Some((!a.name, claimed(a)?)))
        .collect();
    names.sort_by_key(|(later, _)| *later);
    let names: Vec<Claimed> = names.into_iter().map(|(_, named)| named).collect();
    let others: Vec<Claimed> = binding
        .person
        .attribute
        .iter()
        .filter(|a| a.jit && a.source() != Source::Name)
        .filter_map(claimed)
        .collect();
    for table in binding.issuer.iter().filter(|t| t.jit) {
        let Some(party) = resolved.party.iter().find(|p| p.issuer == table.issuer) else {
            continue;
        };
        out.insert(
            table.issuer.expose().to_owned(),
            Jit {
                table: table.clone(),
                key_issuer: party.identifier_issuer.clone(),
                group_claim: party.group_claim.clone(),
                template_id: template.template_id.clone(),
                role_template: role.template_id.clone(),
                key: (
                    template.key.path.clone(),
                    template.key.identifier_type.clone(),
                ),
                user_name: (user_name.path.clone(), user_name.identifier_type.clone()),
                names: names.clone(),
                others: others.clone(),
            },
        );
    }
    out
}

impl Jit {
    /// Whether `caller` is one this issuer creates: a person or a delegated
    /// caller with a bearer token, and a guest of either under `guests_jit`.
    #[expect(
        clippy::match_same_arms,
        reason = "each kind is named, so a kind ferrohealth-smart adds is read here"
    )]
    pub(super) fn creates(&self, caller: &Caller) -> bool {
        if caller.via() == Via::Basic {
            return false;
        }
        match caller.kind() {
            CallerKind::Person | CallerKind::Delegated { .. } => true,
            CallerKind::Guest(inner) => {
                self.table.guests_jit
                    && matches!(
                        inner.as_ref(),
                        CallerKind::Person | CallerKind::Delegated { .. }
                    )
            }
            CallerKind::Patient
            | CallerKind::System
            | CallerKind::Representative { .. }
            | CallerKind::PatientRepresentative { .. }
            | CallerKind::ExternalPerson(_)
            | CallerKind::ExternalOrganisation(_) => false,
            // NOTE: ferrohealth-smart `kind::CallerKind` is `#[non_exhaustive]`; a kind a
            // later release adds is created by no issuer until it is matched above.
            _ => false,
        }
    }

    /// The provisional `PERSON` of `caller`, who holds `roles`, named under
    /// `namespace` with `provider`, judged by `assurance`.
    ///
    /// # Errors
    /// The first admission condition the token does not meet, and
    /// [`NotAdmitted::NoName`] for a token with no mapped name claim.
    pub(super) fn person(
        &self,
        caller: &Caller,
        roles: &[String],
        assurance: &AssuranceSection,
        (namespace, provider): (&str, Option<(String, String)>),
    ) -> Result<ProvisionalPerson, NotAdmitted> {
        self.admits(caller, roles, assurance)?;
        let claims = caller.claims();
        let text = |claim: &str| ferrohealth_smart::roles::claim_text(claims, claim);
        let key = caller
            .person_key()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or(NotAdmitted::NoName)?;
        let present = |attributes: &[Claimed]| -> Vec<(String, String, String)> {
            attributes
                .iter()
                .filter_map(|a| {
                    Some((
                        a.path.clone(),
                        text(&a.claim)?.to_owned(),
                        a.identifier_type.clone(),
                    ))
                })
                .collect()
        };
        let names = present(&self.names);
        let name = names.first().map(|(_, value, _)| value.clone());
        if names.is_empty() {
            return Err(NotAdmitted::NoName);
        }
        let user_name = match &self.table.user_name_claim {
            Some(claim) => text(claim).ok_or(NotAdmitted::NoName)?,
            None => key,
        };
        let mut texts = vec![(
            self.user_name.0.clone(),
            user_name.to_owned(),
            self.user_name.1.clone(),
        )];
        texts.extend(names);
        texts.extend(present(&self.others));
        Ok(ProvisionalPerson {
            template_id: self.template_id.clone(),
            role_template: self.role_template.clone(),
            namespace: namespace.to_owned(),
            login_issuer: caller.issuer().unwrap_or_default().to_owned(),
            key_issuer: self.key_issuer.clone(),
            key: (self.key.0.clone(), key.to_owned(), self.key.1.clone()),
            texts,
            name,
            provider,
        })
    }

    /// Whether `caller`, holding `roles`, meets every admission condition
    /// the table declares; each list admits any one of its values.
    fn admits(
        &self,
        caller: &Caller,
        roles: &[String],
        assurance: &AssuranceSection,
    ) -> Result<(), NotAdmitted> {
        let table = &self.table;
        let claims = caller.claims();
        if !table.admit_idp.is_empty() {
            let idp = ferrohealth_smart::roles::claim_text(claims, &table.idp_claim);
            if !idp.is_some_and(|idp| table.admit_idp.iter().any(|admitted| admitted == idp)) {
                return Err(NotAdmitted::Idp);
            }
        }
        if !table.admit_groups.is_empty() {
            let groups = ferrohealth_smart::roles::from_claims(claims, &[&self.group_claim]);
            if !groups
                .iter()
                .any(|group| table.admit_groups.contains(group))
            {
                return Err(NotAdmitted::Group);
            }
        }
        if !table.admit_roles.is_empty()
            && !table
                .admit_roles
                .iter()
                .any(|role| ferrohealth_smart::roles::holds(roles, role))
        {
            return Err(NotAdmitted::Role);
        }
        if let Some(minimum) = table.admit_assurance
            && assurance
                .assess(claims)
                .level
                .is_none_or(|level| level < minimum)
        {
            return Err(NotAdmitted::Assurance);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ferroehr::config::binding::person::{PersonTemplate, RoleSection, Selector};
    use ferrohealth_smart::assurance::AssuranceLevel;
    use ferrohealth_smart::config::{AuthSection, IssuerEntry};
    use ferrohealth_smart::kind::IssuerKind;
    use ferrohealth_smart::party::config::PartySection;
    use serde_json::json;

    use super::*;

    const ISSUER: &str = "https://login.example";
    const KEY: &str = "2819C223-7F76-453A-919D-413861904646";

    fn resolved() -> Resolved {
        let section = AuthSection {
            enabled: true,
            issuer: vec![IssuerEntry {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
                kind: Some(IssuerKind::Persons),
                audience: vec!["ferroehr".to_owned()],
                ..IssuerEntry::default()
            }],
            party: vec![PartySection {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
                identifier_issuer: "urn:login".to_owned(),
                identifier_type: "guid".to_owned(),
                group_claim: "groups".to_owned(),
                group_identifier_issuer: "urn:groups".to_owned(),
                ..PartySection::default()
            }],
            ..AuthSection::default()
        };
        ferroehr::config::auth::resolve(&section).expect("the [auth] section resolves")
    }

    fn attribute(oidc: &str, path: &str, jit: bool, name: bool) -> PersonAttribute {
        PersonAttribute {
            oidc: Some(oidc.to_owned()),
            jit,
            name,
            path: [("professional".to_owned(), path.to_owned())].into(),
            ..PersonAttribute::default()
        }
    }

    fn jit(table: IssuerBinding) -> Jit {
        let selector = |kind: &str| Selector {
            path: "/details[id2]/items[id3]".to_owned(),
            identifier_type: kind.to_owned(),
        };
        let mut binding = PartyBindingConfig::default();
        binding.person.template = vec![PersonTemplate {
            template_id: "professional".to_owned(),
            key: selector("guid"),
            user_name: Some(selector("scim_user_name")),
            external_id: Some(selector("scim_external_id")),
            default: true,
        }];
        binding.person.attribute = vec![
            attribute(
                "given_name",
                "/identities[id4]/details[id5]/items[id15]",
                false,
                false,
            ),
            attribute(
                "name",
                "/identities[id4]/details[id5]/items[id6]",
                false,
                true,
            ),
            attribute(
                "email",
                "/contacts[id10]/addresses[id11]/details[id12]/items[id13]",
                false,
                false,
            ),
            attribute(
                "phone_number",
                "/contacts[id10]/addresses[id11]/details[id12]/items[id14]",
                true,
                false,
            ),
        ];
        binding.professional_role = Some(RoleSection {
            template_id: "professional role".to_owned(),
        });
        binding.issuer = vec![IssuerBinding {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
            jit: true,
            ..table
        }];
        tables(&binding, &resolved())
            .remove(ISSUER)
            .expect("the issuer creates")
    }

    fn caller(kind: CallerKind, claims: &serde_json::Value) -> Caller {
        let claims = claims.as_object().cloned().unwrap_or_default();
        Caller::new("sub".to_owned(), None, "", None, false, kind)
            .with_issuer(ISSUER.to_owned())
            .with_claims(claims)
            .with_person_key(Some(KEY.to_owned()), None)
    }

    fn admitted() -> IssuerBinding {
        IssuerBinding {
            admit_idp: vec!["https://sts.example/".to_owned()],
            ..IssuerBinding::default()
        }
    }

    fn named() -> serde_json::Value {
        json!({ "idp": "https://sts.example/", "name": "Dr. A. Jansen", "given_name": "Anna",
                "email": "a.jansen@example.org", "phone_number": "+99 999" })
    }

    #[test]
    fn only_a_person_or_a_delegated_bearer_and_a_guest_under_guests_jit_are_created() {
        let plain = jit(admitted());
        let person = |kind| caller(kind, &named());
        assert!(plain.creates(&person(CallerKind::Person)));
        assert!(plain.creates(&person(CallerKind::Delegated { actor: None })));
        for kind in [
            CallerKind::Patient,
            CallerKind::System,
            CallerKind::Representative {
                represented: "p".to_owned(),
                ground: None,
            },
            CallerKind::Guest(Box::new(CallerKind::Person)),
        ] {
            assert!(!plain.creates(&person(kind.clone())), "{kind:?}");
        }
        assert!(
            !plain.creates(&Caller::basic("alice".to_owned(), "")),
            "a Basic user is bound by its configured key"
        );
        let guests = jit(IssuerBinding {
            guests_jit: true,
            ..admitted()
        });
        assert!(guests.creates(&person(CallerKind::Guest(Box::new(CallerKind::Person)))));
        assert!(!guests.creates(&person(CallerKind::Guest(Box::new(CallerKind::Patient)))));
    }

    #[test]
    fn the_person_carries_the_key_user_name_names_and_jit_attributes_only() {
        let person = jit(admitted())
            .person(
                &caller(CallerKind::Person, &named()),
                &[],
                &AssuranceSection::default(),
                ("demographic", Some(("urn:ura".to_owned(), "1".to_owned()))),
            )
            .expect("the login is admitted");
        assert_eq!(person.key.1, KEY);
        assert_eq!(person.key_issuer, "urn:login");
        assert_eq!(person.login_issuer, ISSUER);
        assert_eq!(
            person.name.as_deref(),
            Some("Dr. A. Jansen"),
            "the name = true claim"
        );
        let written: Vec<&str> = person
            .texts
            .iter()
            .map(|(_, value, _)| value.as_str())
            .collect();
        assert_eq!(
            written,
            [KEY, "Dr. A. Jansen", "Anna", "+99 999"],
            "the userName defaults to the key; the email, without jit, is not written"
        );
    }

    #[test]
    fn each_admission_condition_and_a_missing_name_refuse() {
        let run = |table: IssuerBinding, claims: serde_json::Value, roles: &[String]| {
            let levels = [(
                "http://eidas.europa.eu/LoA/substantial".to_owned(),
                AssuranceLevel::Substantial,
            )]
            .into();
            let assurance = AssuranceSection {
                levels,
                ..AssuranceSection::default()
            };
            jit(table)
                .person(
                    &caller(CallerKind::Person, &claims),
                    roles,
                    &assurance,
                    ("demographic", None),
                )
                .map(|_| ())
        };
        let mut other_idp = named();
        other_idp["idp"] = json!("https://elsewhere.example/");
        assert_eq!(run(admitted(), other_idp, &[]), Err(NotAdmitted::Idp));
        let groups = IssuerBinding {
            admit_groups: vec!["clinicians".to_owned()],
            ..IssuerBinding::default()
        };
        assert_eq!(run(groups.clone(), named(), &[]), Err(NotAdmitted::Group));
        let mut grouped = named();
        grouped["groups"] = json!(["clinicians"]);
        assert_eq!(run(groups, grouped, &[]), Ok(()));
        let roles = IssuerBinding {
            admit_roles: vec!["clinical".to_owned()],
            ..IssuerBinding::default()
        };
        assert_eq!(run(roles.clone(), named(), &[]), Err(NotAdmitted::Role));
        assert_eq!(run(roles, named(), &["CLINICAL".to_owned()]), Ok(()));
        let assured = IssuerBinding {
            admit_assurance: Some(AssuranceLevel::Substantial),
            ..IssuerBinding::default()
        };
        assert_eq!(
            run(assured.clone(), named(), &[]),
            Err(NotAdmitted::Assurance)
        );
        let mut substantial = named();
        substantial["acr"] = json!("http://eidas.europa.eu/LoA/substantial");
        assert_eq!(run(assured, substantial, &[]), Ok(()));
        let both = IssuerBinding {
            admit_roles: vec!["clinical".to_owned()],
            ..admitted()
        };
        assert_eq!(
            run(both, named(), &[]),
            Err(NotAdmitted::Role),
            "every declared condition must hold"
        );
        assert_eq!(
            run(admitted(), json!({ "idp": "https://sts.example/" }), &[]),
            Err(NotAdmitted::NoName)
        );
    }
}
