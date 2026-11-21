// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login the CDR authenticates a session as, which the placeholder
//! committer names.
//!
//! ITS-REST `schemas/common/UpdateAudit.yaml` requires a `committer` in every
//! audit a CONTRIBUTION carries, and the CDR writes who committed from the
//! authenticated caller, refusing a stated committer that differs. The
//! placeholder names the caller and states nothing else, so the CDR accepts
//! it and records its own committer (#5019).

use ferrohealth_session::record::{Credential, Session};

/// A login the CDR authenticates: its issuer and its id under that issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    issuer: String,
    id: String,
}

impl Login {
    /// The Basic login `username`, which the CDR issues itself under
    /// `basic_issuer`, its `[api] basic_issuer` (`[cdr] basic_issuer`).
    #[must_use]
    pub fn basic(username: impl Into<String>, basic_issuer: impl Into<String>) -> Self {
        Self {
            issuer: basic_issuer.into(),
            id: username.into(),
        }
    }

    /// The login of `session` whose OpenID Provider is `oidc_issuer`: the
    /// provider and the verified `sub` of an OpenID Connect sign-in, or
    /// `basic_issuer` and the user name of a Basic one; `None` for a bearer
    /// session without a `sub`.
    #[must_use]
    pub fn of(session: &Session, oidc_issuer: &str, basic_issuer: &str) -> Option<Self> {
        Self::from_credential(
            session.credential(),
            session.subject(),
            oidc_issuer,
            basic_issuer,
        )
    }

    /// The login a session holding `credential` and `subject` signs in as.
    fn from_credential(
        credential: &Credential,
        subject: Option<&str>,
        oidc_issuer: &str,
        basic_issuer: &str,
    ) -> Option<Self> {
        match credential {
            Credential::Basic { username, .. } => Some(Self::basic(username.clone(), basic_issuer)),
            Credential::Bearer(_) => subject.map(|subject| Self {
                issuer: oidc_issuer.to_owned(),
                id: subject.to_owned(),
            }),
        }
    }

    /// The authority that issued the login.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The login under its issuer.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_secret::secret::Secret;

    use super::*;

    #[test]
    fn a_basic_session_is_the_user_under_the_cdr_and_a_bearer_one_its_subject() {
        let basic = Credential::Basic {
            username: "ferroehr".to_owned(),
            password: Secret::new("x".to_owned()),
        };
        let login = Login::from_credential(&basic, None, "https://idp.example.org", "cdr.example")
            .expect("a Basic login");
        assert_eq!((login.issuer(), login.id()), ("cdr.example", "ferroehr"));
        let bearer = Credential::Bearer(Secret::new("t".to_owned()));
        let login = Login::from_credential(
            &bearer,
            Some("aca7c611"),
            "https://idp.example.org",
            "cdr.example",
        )
        .expect("a bearer login");
        assert_eq!(
            (login.issuer(), login.id()),
            ("https://idp.example.org", "aca7c611")
        );
        assert!(
            Login::from_credential(&bearer, None, "https://idp.example.org", "cdr.example")
                .is_none()
        );
    }
}
