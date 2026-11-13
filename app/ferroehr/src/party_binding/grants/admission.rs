// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The admission rules of just-in-time creation: which first logins of a
//! persons issuer FerroEHR creates a provisional `PERSON` for, read from a
//! [`Operation::CreateAtLogin`] grant's `credentials`.
//!
//! A `CAPABILITY` whose `credentials` hold the code `create_at_login` of
//! [`crate::party_binding::policy_mark::TERMINOLOGY`] admits the logins of each
//! token issuer its [`LOGIN_ISSUER`] elements name that meet every condition
//! it declares: an identity provider of [`ADMIT_IDP`], a group of
//! [`ADMIT_GROUP`], a role of [`ADMIT_ROLE`], the level of
//! [`ADMIT_ASSURANCE`] or above, and a guest only under [`ADMIT_GUESTS`].
//! A grant names at least one issuer and one condition, or its commit is
//! refused and a stored one is never honoured, so no grant admits every
//! login of an issuer. Several grants in force admit a login when any one
//! does. No openEHR spec governs when a login creates a party, or gives a
//! capability an authorization meaning: our own design.

use ferrohealth_smart::assurance::AssuranceLevel;

use crate::error::{ServiceError, Violation};
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::capabilities::Credentials;
use crate::party_binding::grants::operation::Operation;

/// The code naming an element whose text is the token issuer whose logins
/// the grant admits.
pub const LOGIN_ISSUER: &str = "login_issuer";

/// The code naming an element whose text is an identity provider whose
/// logins the grant admits, read from the issuer's `idp_claim`.
pub const ADMIT_IDP: &str = "admit_idp";

/// The code naming an element whose text is a group, read from the
/// `[[auth.party]]` table's `group_claim`, one of which an admitted login
/// holds.
pub const ADMIT_GROUP: &str = "admit_group";

/// The code naming an element whose text is a role, read from
/// `[auth] role_claims`, one of which an admitted login holds.
pub const ADMIT_ROLE: &str = "admit_role";

/// The code naming the element whose text, `low`, `substantial` or `high`, is
/// the lowest `[authz.assurance]` level an admitted login carries.
pub const ADMIT_ASSURANCE: &str = "admit_assurance";

/// The code naming the element whose `DV_BOOLEAN` admits a guest the
/// issuer's `[[auth.party]]` table admits.
pub const ADMIT_GUESTS: &str = "admit_guests";

/// Why a `create_at_login` grant's credentials admit nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionRefused {
    /// No [`LOGIN_ISSUER`] element names an issuer.
    #[error(
        "names no login issuer: a create_at_login grant carries a {LOGIN_ISSUER} element \
         naming the token issuer whose logins it admits"
    )]
    NoIssuer,
    /// No condition is declared.
    #[error(
        "declares no admission condition: a create_at_login grant carries at least one \
         {ADMIT_IDP}, {ADMIT_GROUP}, {ADMIT_ROLE} or {ADMIT_ASSURANCE} element"
    )]
    NoCondition,
    /// An element of this code holds no value of the type it is read as.
    #[error(
        "carries a {0} element whose value does not read: {LOGIN_ISSUER}, {ADMIT_IDP}, \
         {ADMIT_GROUP}, {ADMIT_ROLE} and {ADMIT_ASSURANCE} hold a non-empty text and \
         {ADMIT_GUESTS} a DV_BOOLEAN"
    )]
    Unreadable(&'static str),
    /// The [`ADMIT_ASSURANCE`] text names no level.
    #[error("reads {0:?} as {ADMIT_ASSURANCE}, which is none of low, substantial and high")]
    Assurance(String),
    /// More than one [`ADMIT_ASSURANCE`] or [`ADMIT_GUESTS`] element.
    #[error("carries more than one {0} element")]
    Several(&'static str),
}

/// The admission rules of one `create_at_login` grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    issuers: Vec<String>,
    idp: Vec<String>,
    groups: Vec<String>,
    roles: Vec<String>,
    assurance: Option<AssuranceLevel>,
    guests: bool,
}

impl Admission {
    /// The admission rules `credentials` hold.
    ///
    /// # Errors
    /// [`AdmissionRefused`] for credentials that name no issuer, declare no
    /// condition, or hold an element that does not read.
    pub fn read(credentials: Credentials<'_>) -> Result<Self, AdmissionRefused> {
        let issuers = texts(credentials, LOGIN_ISSUER)?;
        if issuers.is_empty() {
            return Err(AdmissionRefused::NoIssuer);
        }
        let assurance = match texts(credentials, ADMIT_ASSURANCE)?.as_slice() {
            [] => None,
            [level] => Some(level_of(level)?),
            _ => return Err(AdmissionRefused::Several(ADMIT_ASSURANCE)),
        };
        let guests = match credentials.named(ADMIT_GUESTS).len() {
            0 => false,
            1 => credentials
                .boolean(ADMIT_GUESTS)
                .ok_or(AdmissionRefused::Unreadable(ADMIT_GUESTS))?,
            _ => return Err(AdmissionRefused::Several(ADMIT_GUESTS)),
        };
        let admission = Self {
            issuers,
            idp: texts(credentials, ADMIT_IDP)?,
            groups: texts(credentials, ADMIT_GROUP)?,
            roles: texts(credentials, ADMIT_ROLE)?,
            assurance,
            guests,
        };
        if admission.idp.is_empty()
            && admission.groups.is_empty()
            && admission.roles.is_empty()
            && admission.assurance.is_none()
        {
            return Err(AdmissionRefused::NoCondition);
        }
        Ok(admission)
    }

    /// Whether the grant admits logins of the token issuer `issuer`.
    #[must_use]
    pub fn names(&self, issuer: &str) -> bool {
        self.issuers.iter().any(|named| named == issuer.trim())
    }

    /// The identity providers whose logins it admits; empty for any.
    #[must_use]
    pub fn idp(&self) -> &[String] {
        &self.idp
    }

    /// The groups one of which an admitted login holds; empty for any.
    #[must_use]
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    /// The roles one of which an admitted login holds; empty for any.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    /// The lowest level an admitted login carries; `None` for any.
    #[must_use]
    pub fn assurance(&self) -> Option<AssuranceLevel> {
        self.assurance
    }

    /// Whether it admits a guest.
    #[must_use]
    pub fn guests(&self) -> bool {
        self.guests
    }
}

/// The trimmed text of every element `code` names in `credentials`.
fn texts(
    credentials: Credentials<'_>,
    code: &'static str,
) -> Result<Vec<String>, AdmissionRefused> {
    let texts: Vec<String> = credentials
        .texts(code)
        .into_iter()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect();
    if texts.len() == credentials.named(code).len() {
        Ok(texts)
    } else {
        Err(AdmissionRefused::Unreadable(code))
    }
}

/// The level `text` names, without regard to case.
fn level_of(text: &str) -> Result<AssuranceLevel, AdmissionRefused> {
    [
        AssuranceLevel::Low,
        AssuranceLevel::Substantial,
        AssuranceLevel::High,
    ]
    .into_iter()
    .find(|level| level.as_str().eq_ignore_ascii_case(text))
    .ok_or_else(|| AdmissionRefused::Assurance(text.to_owned()))
}

/// Refuses the committed body `facts` of a `ROLE` when a capability of it
/// grants [`Operation::CreateAtLogin`] with credentials that admit nothing.
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) naming the capability and what
/// its credentials lack.
pub(crate) fn check(facts: &Facts) -> Result<(), ServiceError> {
    let code = Operation::CreateAtLogin.code();
    for capability in facts.grants.capabilities.iter().filter(|c| c.code == code) {
        if let Err(refused) = Admission::read(Credentials::of(capability.credentials.as_ref())) {
            return Err(ServiceError::content_invalid(
                Violation::new(format!("the capability granting {code} {refused}"))
                    .with_path(format!("/capabilities[{}]/credentials", capability.ordinal)),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 1): a capability's `credentials` \
                  ITEM_TREE as the party version holds it"
    )]

    use serde_json::{Value, json};

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    fn named(code: &str, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1",
               "name": {"_type": "DV_CODED_TEXT", "value": code,
                        "defining_code": {"terminology_id": {"value": TERMINOLOGY},
                                          "code_string": code}},
               "value": value})
    }

    fn text(value: &str) -> Value {
        json!({"_type": "DV_TEXT", "value": value})
    }

    fn read(items: &[Value]) -> Result<Admission, AdmissionRefused> {
        let tree = json!({"_type": "ITEM_TREE", "items": items});
        Admission::read(Credentials::of(Some(&tree)))
    }

    #[test]
    fn an_admission_reads_its_issuers_and_conditions_anywhere() {
        let admission = read(&[
            named(
                LOGIN_ISSUER,
                &json!({"_type": "DV_URI", "value": "https://login.example"}),
            ),
            json!({"_type": "CLUSTER", "items": [
                named(ADMIT_IDP, &text(" https://sts.example/ ")),
                named(ADMIT_GROUP, &text("clinicians")),
            ]}),
            named(ADMIT_ROLE, &text("clinical")),
            named(ADMIT_ASSURANCE, &text("Substantial")),
            named(ADMIT_GUESTS, &json!({"_type": "DV_BOOLEAN", "value": true})),
        ])
        .expect("the admission reads");
        assert!(admission.names("https://login.example"));
        assert!(!admission.names("https://elsewhere.example"));
        assert_eq!(admission.idp(), ["https://sts.example/"]);
        assert_eq!(admission.groups(), ["clinicians"]);
        assert_eq!(admission.roles(), ["clinical"]);
        assert_eq!(admission.assurance(), Some(AssuranceLevel::Substantial));
        assert!(admission.guests());
    }

    #[test]
    fn an_admission_without_an_issuer_a_condition_or_a_readable_value_is_refused() {
        let issuer = || named(LOGIN_ISSUER, &text("https://login.example"));
        let idp = || named(ADMIT_IDP, &text("https://sts.example/"));
        assert_eq!(read(&[idp()]), Err(AdmissionRefused::NoIssuer));
        assert_eq!(read(&[issuer()]), Err(AdmissionRefused::NoCondition));
        assert_eq!(
            read(&[
                issuer(),
                named(ADMIT_GUESTS, &json!({"_type": "DV_BOOLEAN", "value": true}))
            ]),
            Err(AdmissionRefused::NoCondition),
            "admitting guests is no condition"
        );
        assert_eq!(
            read(&[issuer(), named(ADMIT_IDP, &text("  "))]),
            Err(AdmissionRefused::Unreadable(ADMIT_IDP))
        );
        assert_eq!(
            read(&[issuer(), idp(), named(ADMIT_GUESTS, &text("yes"))]),
            Err(AdmissionRefused::Unreadable(ADMIT_GUESTS))
        );
        assert_eq!(
            read(&[issuer(), named(ADMIT_ASSURANCE, &text("medium"))]),
            Err(AdmissionRefused::Assurance("medium".to_owned()))
        );
        assert_eq!(
            read(&[
                issuer(),
                named(ADMIT_ASSURANCE, &text("low")),
                named(ADMIT_ASSURANCE, &text("high"))
            ]),
            Err(AdmissionRefused::Several(ADMIT_ASSURANCE))
        );
        assert_eq!(
            Admission::read(Credentials::of(None)),
            Err(AdmissionRefused::NoIssuer)
        );
    }
}
