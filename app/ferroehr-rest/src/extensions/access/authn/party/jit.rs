// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Just-in-time creation at the login binding: which caller of a `persons`
//! issuer gets a provisional `PERSON` at a first login, and what it is
//! written from.
//!
//! Only a person or a delegated caller whose login no party carries is
//! created, a guest only where an admission says so, and only when the token
//! meets every condition of a `create_at_login` grant in force that names
//! its issuer ([`ferroehr::party_binding::grants::admission`]) and carries a
//! mapped name claim. The grants are CDR data, read through this replica's
//! copy on each first login, so a change applies to the next login with no
//! restart. The person's data are read from the access token (OpenID
//! Connect Core 1.0 §5.1, RFC 9068 §2.2.2): the binding key, the `userName`,
//! the names, and every other attribute only where its mapping sets
//! `jit = true` (GDPR Art. 25(2)). No openEHR spec governs when a login
//! creates a party: our own design.

use std::collections::BTreeMap;

use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::binding::person::{PersonAttribute, Source};
use ferroehr::party_binding::grants::admission::Admission;
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
    /// The claim the `userName` is read from; the key when unset.
    user_name_claim: Option<String>,
    /// The claim the identity provider is read from.
    idp_claim: String,
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
    /// No `create_at_login` grant in force names the issuer.
    NoAdmission,
    /// The caller is a guest and no admission admits guests.
    Guest,
    /// The token's identity provider is not one an admission names.
    Idp,
    /// The token holds no group an admission names.
    Group,
    /// The caller holds no role an admission names.
    Role,
    /// The token's assurance level is below an admission's.
    Assurance,
    /// The token carries no mapped name claim.
    NoName,
}

impl NotAdmitted {
    /// The reason a log line names.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::NoAdmission => "no_admission",
            Self::Guest => "guest",
            Self::Idp => "idp",
            Self::Group => "group",
            Self::Role => "role",
            Self::Assurance => "assurance",
            Self::NoName => "no_name_claim",
        }
    }
}

/// The just-in-time creation of each `persons` issuer an `[[auth.party]]`
/// table binds, keyed by issuer, read with its `[[authz.party_binding.issuer]]`
/// table when it has one.
///
/// # Errors
/// Why the mapping cannot write a provisional `PERSON`
/// ([`ferroehr::config::binding::jit::mapping_errors`]); a login an
/// admission admits is then not created.
pub(super) fn tables(
    binding: &PartyBindingConfig,
    resolved: &Resolved,
) -> Result<BTreeMap<String, Jit>, Vec<String>> {
    let errors = ferroehr::config::binding::jit::mapping_errors(binding);
    let template = binding.person().template.iter().find(|t| t.default);
    let (Some(template), Some(user_name), Some(role), true) = (
        template,
        template.and_then(|t| t.user_name.as_ref()),
        &binding.professional_role,
        errors.is_empty(),
    ) else {
        return Err(errors);
    };
    let claimed = |attribute: &PersonAttribute| {
        Some(Claimed {
            claim: attribute.oidc.clone()?,
            path: attribute.path_for(&template.template_id)?.to_owned(),
            identifier_type: attribute.oidc.clone()?,
        })
    };
    let mut names: Vec<(bool, Claimed)> = binding
        .person()
        .attribute
        .iter()
        .filter(|a| a.source() == Source::Name)
        .filter_map(|a| Some((!a.name, claimed(a)?)))
        .collect();
    names.sort_by_key(|(later, _)| *later);
    let names: Vec<Claimed> = names.into_iter().map(|(_, named)| named).collect();
    let others: Vec<Claimed> = binding
        .person()
        .attribute
        .iter()
        .filter(|a| a.jit && a.source() != Source::Name)
        .filter_map(claimed)
        .collect();
    let mut out = BTreeMap::new();
    for issuer in ferroehr::config::binding::jit::login_issuers(resolved) {
        let Some(party) = resolved.party.iter().find(|p| p.issuer == issuer) else {
            continue;
        };
        let table = binding
            .issuer
            .iter()
            .find(|t| t.issuer == issuer)
            .cloned()
            .unwrap_or_default();
        out.entry(issuer.expose().to_owned())
            .or_insert_with(|| Jit {
                user_name_claim: table.user_name_claim,
                idp_claim: table.idp_claim,
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
            });
    }
    Ok(out)
}

/// Whether `caller` is one a first login may create: `Some(false)` for a
/// person or a delegated caller with a bearer token, `Some(true)` for a guest
/// of either, which only an admission admitting guests creates, and `None`
/// for every other caller.
#[expect(
    clippy::match_same_arms,
    reason = "each kind is named, so a kind ferrohealth-smart adds is read here"
)]
pub(super) fn candidate(caller: &Caller) -> Option<bool> {
    if caller.via() == Via::Basic {
        return None;
    }
    match caller.kind() {
        CallerKind::Person | CallerKind::Delegated { .. } => Some(false),
        CallerKind::Guest(inner) => matches!(
            inner.as_ref(),
            CallerKind::Person | CallerKind::Delegated { .. }
        )
        .then_some(true),
        CallerKind::Patient
        | CallerKind::System
        | CallerKind::Representative { .. }
        | CallerKind::PatientRepresentative { .. }
        | CallerKind::ExternalPerson(_)
        | CallerKind::ExternalOrganisation(_) => None,
        // NOTE: ferrohealth-smart `kind::CallerKind` is `#[non_exhaustive]`; a kind a
        // later release adds is created by no issuer until it is matched above.
        _ => None,
    }
}

impl Jit {
    /// The provisional `PERSON` of `caller`, who holds `roles`, named under
    /// `namespace` with `provider`, when one of `admissions` admits it,
    /// judged by `assurance`.
    ///
    /// # Errors
    /// The condition the token does not meet, of the first admission when
    /// none admits it, and [`NotAdmitted::NoName`] for a token with no
    /// mapped name claim.
    pub(super) fn person(
        &self,
        caller: &Caller,
        admissions: &[Admission],
        (roles, assurance): (&[String], &AssuranceSection),
        (namespace, provider): (&str, Option<(String, String)>),
    ) -> Result<ProvisionalPerson, NotAdmitted> {
        let guest = candidate(caller) == Some(true);
        let mut refused = None;
        let admitted = admissions.iter().any(|admission| {
            let judged = self.admits(admission, caller, guest, roles, assurance);
            if let Err(reason) = judged {
                refused.get_or_insert(reason);
            }
            judged.is_ok()
        });
        if !admitted {
            return Err(refused.unwrap_or(NotAdmitted::NoAdmission));
        }
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
        let user_name = match &self.user_name_claim {
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

    /// Whether `caller`, a guest when `guest`, holding `roles`, meets every
    /// condition `admission` declares; each list admits any one of its
    /// values.
    fn admits(
        &self,
        admission: &Admission,
        caller: &Caller,
        guest: bool,
        roles: &[String],
        assurance: &AssuranceSection,
    ) -> Result<(), NotAdmitted> {
        if guest && !admission.guests() {
            return Err(NotAdmitted::Guest);
        }
        let claims = caller.claims();
        if !admission.idp().is_empty() {
            let idp = ferrohealth_smart::roles::claim_text(claims, &self.idp_claim);
            if !idp.is_some_and(|idp| admission.idp().iter().any(|admitted| admitted == idp)) {
                return Err(NotAdmitted::Idp);
            }
        }
        if !admission.groups().is_empty() {
            let groups = ferrohealth_smart::roles::from_claims(claims, &[&self.group_claim]);
            if !groups
                .iter()
                .any(|group| admission.groups().contains(group))
            {
                return Err(NotAdmitted::Group);
            }
        }
        if !admission.roles().is_empty()
            && !admission
                .roles()
                .iter()
                .any(|role| ferrohealth_smart::roles::holds(roles, role))
        {
            return Err(NotAdmitted::Role);
        }
        if let Some(minimum) = admission.assurance()
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
    use ferroehr::party_binding::grants::admission::{
        ADMIT_ASSURANCE, ADMIT_GROUP, ADMIT_GUESTS, ADMIT_IDP, ADMIT_ROLE, LOGIN_ISSUER,
    };
    use ferroehr::party_binding::grants::capabilities::Credentials;
    use ferroehr::party_binding::policy_mark::TERMINOLOGY;
    use ferrohealth_smart::assurance::AssuranceLevel;
    use ferrohealth_smart::config::{AuthSection, IssuerEntry};
    use ferrohealth_smart::kind::IssuerKind;
    use ferrohealth_smart::party::config::PartySection;
    use serde_json::{Value, json};

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

    fn binding() -> PartyBindingConfig {
        let selector = |kind: &str| Selector {
            path: "/details[id2]/items[id3]".to_owned(),
            identifier_type: kind.to_owned(),
        };
        let mut binding = PartyBindingConfig::default();
        binding.person.get_or_insert_default().template = vec![PersonTemplate {
            template_id: "professional".to_owned(),
            key: selector("guid"),
            user_name: Some(selector("scim_user_name")),
            external_id: Some(selector("scim_external_id")),
            default: true,
        }];
        binding.person.get_or_insert_default().attribute = vec![
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
        binding
    }

    fn jit() -> Jit {
        tables(&binding(), &resolved())
            .expect("the mapping can write")
            .remove(ISSUER)
            .expect("the bound persons issuer creates")
    }

    fn element(code: &str, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1",
               "name": {"_type": "DV_CODED_TEXT", "value": code,
                        "defining_code": {"terminology_id": {"value": TERMINOLOGY},
                                          "code_string": code}},
               "value": value})
    }

    /// The admission of `ISSUER` whose credentials add `items`.
    fn admission(items: &[(&str, Value)]) -> Admission {
        let mut elements = vec![element(
            LOGIN_ISSUER,
            &json!({"_type": "DV_URI", "value": ISSUER}),
        )];
        elements.extend(items.iter().map(|(code, value)| element(code, value)));
        let tree = json!({"_type": "ITEM_TREE", "items": elements});
        Admission::read(Credentials::of(Some(&tree))).expect("the admission reads")
    }

    fn text(value: &str) -> Value {
        json!({"_type": "DV_TEXT", "value": value})
    }

    fn admitted() -> Admission {
        admission(&[(ADMIT_IDP, text("https://sts.example/"))])
    }

    fn caller(kind: CallerKind, claims: &Value) -> Caller {
        let claims = claims.as_object().cloned().unwrap_or_default();
        Caller::new("sub".to_owned(), None, "", None, false, kind)
            .with_issuer(ISSUER.to_owned())
            .with_claims(claims)
            .with_person_key(Some(KEY.to_owned()), None)
    }

    fn named() -> Value {
        json!({ "idp": "https://sts.example/", "name": "Dr. A. Jansen", "given_name": "Anna",
                "email": "a.jansen@example.org", "phone_number": "+99 999" })
    }

    fn run(
        admissions: &[Admission],
        kind: CallerKind,
        claims: &Value,
        roles: &[String],
    ) -> Result<ProvisionalPerson, NotAdmitted> {
        let levels = [(
            "http://eidas.europa.eu/LoA/substantial".to_owned(),
            AssuranceLevel::Substantial,
        )]
        .into();
        let assurance = AssuranceSection {
            levels,
            ..AssuranceSection::default()
        };
        jit().person(
            &caller(kind, claims),
            admissions,
            (roles, &assurance),
            ("demographic", None),
        )
    }

    #[test]
    fn only_a_person_or_a_delegated_bearer_and_a_guest_are_candidates() {
        let person = |kind| caller(kind, &named());
        assert_eq!(candidate(&person(CallerKind::Person)), Some(false));
        assert_eq!(
            candidate(&person(CallerKind::Delegated { actor: None })),
            Some(false)
        );
        assert_eq!(
            candidate(&person(CallerKind::Guest(Box::new(CallerKind::Person)))),
            Some(true)
        );
        for kind in [
            CallerKind::Patient,
            CallerKind::System,
            CallerKind::Representative {
                represented: "p".to_owned(),
                ground: None,
            },
            CallerKind::Guest(Box::new(CallerKind::Patient)),
        ] {
            assert_eq!(candidate(&person(kind.clone())), None, "{kind:?}");
        }
        assert_eq!(
            candidate(&Caller::basic("alice".to_owned(), "")),
            None,
            "a Basic user is bound by its configured key"
        );
    }

    #[test]
    fn a_guest_is_created_only_under_an_admission_admitting_guests() {
        let guest = CallerKind::Guest(Box::new(CallerKind::Person));
        assert_eq!(
            run(&[admitted()], guest.clone(), &named(), &[]).map(|_| ()),
            Err(NotAdmitted::Guest)
        );
        let guests = admission(&[
            (ADMIT_IDP, text("https://sts.example/")),
            (ADMIT_GUESTS, json!({"_type": "DV_BOOLEAN", "value": true})),
        ]);
        assert!(run(&[admitted(), guests], guest, &named(), &[]).is_ok());
    }

    #[test]
    fn a_mapping_that_cannot_write_creates_no_issuer() {
        let mut unwritable = binding();
        unwritable.professional_role = None;
        let errors = tables(&unwritable, &resolved()).expect_err("no ROLE to commit");
        assert!(
            errors.iter().any(|e| e.contains("professional_role")),
            "{errors:?}"
        );
    }

    #[test]
    fn the_person_carries_the_key_user_name_names_and_jit_attributes_only() {
        let person = jit()
            .person(
                &caller(CallerKind::Person, &named()),
                &[admitted()],
                (&[], &AssuranceSection::default()),
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
        let judge = |admission: Admission, claims: Value, roles: &[String]| {
            run(&[admission], CallerKind::Person, &claims, roles).map(|_| ())
        };
        let mut other_idp = named();
        other_idp["idp"] = json!("https://elsewhere.example/");
        assert_eq!(judge(admitted(), other_idp, &[]), Err(NotAdmitted::Idp));
        let groups = || admission(&[(ADMIT_GROUP, text("clinicians"))]);
        assert_eq!(judge(groups(), named(), &[]), Err(NotAdmitted::Group));
        let mut grouped = named();
        grouped["groups"] = json!(["clinicians"]);
        assert_eq!(judge(groups(), grouped, &[]), Ok(()));
        let roles = || admission(&[(ADMIT_ROLE, text("clinical"))]);
        assert_eq!(judge(roles(), named(), &[]), Err(NotAdmitted::Role));
        assert_eq!(judge(roles(), named(), &["CLINICAL".to_owned()]), Ok(()));
        let assured = || admission(&[(ADMIT_ASSURANCE, text("substantial"))]);
        assert_eq!(judge(assured(), named(), &[]), Err(NotAdmitted::Assurance));
        let mut substantial = named();
        substantial["acr"] = json!("http://eidas.europa.eu/LoA/substantial");
        assert_eq!(judge(assured(), substantial, &[]), Ok(()));
        let both = admission(&[
            (ADMIT_IDP, text("https://sts.example/")),
            (ADMIT_ROLE, text("clinical")),
        ]);
        assert_eq!(
            judge(both, named(), &[]),
            Err(NotAdmitted::Role),
            "every declared condition must hold"
        );
        assert_eq!(
            judge(admitted(), json!({ "idp": "https://sts.example/" }), &[]),
            Err(NotAdmitted::NoName)
        );
    }

    #[test]
    fn any_admission_in_force_admits_and_none_refuses() {
        assert!(
            run(
                &[roles_only(), admitted()],
                CallerKind::Person,
                &named(),
                &[]
            )
            .is_ok()
        );
        assert_eq!(
            run(&[], CallerKind::Person, &named(), &[]).map(|_| ()),
            Err(NotAdmitted::NoAdmission),
            "no admission admits nothing"
        );
    }

    fn roles_only() -> Admission {
        admission(&[(ADMIT_ROLE, text("clinical"))])
    }
}
