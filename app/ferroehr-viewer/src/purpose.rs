// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The purpose of use the viewer declares on every authenticated CDR request
//! (#6105).
//!
//! The CDR lists the purposes it accepts at `GET {extensions}/access/purposes`
//! with the header they are declared in. The reader chooses one on the
//! Preferences page, which keeps it in [`COOKIE`] for the session; without a
//! choice the viewer declares `[cdr] purpose_of_use`. A configured code the
//! CDR does not list is refused with an error naming it, before any request
//! carries it, and a chosen code the CDR no longer lists is not declared. No
//! openEHR spec governs the purpose of use: our own design.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::Form;
use axum::response::{IntoResponse, Redirect, Response};
use ferrohealth_session::record::Credential;
use http::header::SET_COOKIE;
use serde::Deserialize;

use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;

/// The cookie the reader's chosen purpose of use is kept in.
pub const COOKIE: &str = "ferroehr-viewer-purpose";

/// The route the purpose chooser posts to.
pub const ROUTE: &str = "/purpose";

/// How long the CDR's list is held before it is read again.
const HELD_FOR: Duration = Duration::from_secs(300);

/// The purposes the CDR accepts, as its purposes route answers them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Offered {
    /// The request header a purpose is declared in.
    pub header: String,
    /// The request header the reason of a justified access is stated in;
    /// `None` from a CDR that takes no justification.
    #[serde(default)]
    pub reason_header: Option<String>,
    /// The code system the codes belong to.
    pub system: String,
    /// The accepted codes; empty when the CDR accepts any code.
    pub codes: Vec<Code>,
}

/// One accepted purpose of use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Code {
    /// The code the viewer declares.
    pub code: String,
    /// The code's display text, when the CDR's terminology server gives one.
    #[serde(default)]
    pub display: Option<String>,
}

impl Offered {
    /// Whether `code` is one the CDR accepts.
    #[must_use]
    pub fn accepts(&self, code: &str) -> bool {
        self.codes.is_empty() || self.codes.iter().any(|offered| offered.code == code)
    }
}

/// The CDR's purposes route, its last answer and the configured default.
#[derive(Debug, Clone)]
pub struct Purposes {
    inner: Arc<Inner>,
}

/// The CDR's last answer while it is held: its list, or `None` when it
/// keeps no access log.
struct Held(Option<Offered>);

#[derive(Debug)]
struct Inner {
    http: reqwest::Client,
    url: String,
    default: Option<String>,
    held: Mutex<Option<(Instant, Option<Offered>)>>,
}

impl Purposes {
    /// The purposes read from `url` with `http`, declaring `default` when the
    /// reader chose none.
    #[must_use]
    pub fn new(http: reqwest::Client, url: String, default: Option<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                url,
                default,
                held: Mutex::new(None),
            }),
        }
    }

    /// What the CDR accepts, read as `credential`; `None` when the CDR keeps
    /// no access log (`404`) or refuses the credential (`401`), where no
    /// purpose is declared and the request itself answers.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] when the CDR does not answer or its
    /// list does not read, and [`ViewerError::Cdr`] on any other refusal.
    pub async fn offered(&self, credential: &Credential) -> Result<Option<Offered>, ViewerError> {
        if let Some(Held(held)) = self.held() {
            return Ok(held);
        }
        let request = self
            .inner
            .http
            .get(&self.inner.url)
            .header(http::header::ACCEPT, "application/json");
        let response = crate::cdr::authorize(request, Some(credential))
            .send()
            .await
            .map_err(ViewerError::unreachable)?;
        let status = response.status();
        if status == http::StatusCode::UNAUTHORIZED {
            return Ok(None);
        }
        let offered = if status == http::StatusCode::NOT_FOUND {
            None
        } else if status.is_success() {
            Some(
                response
                    .json::<Offered>()
                    .await
                    .map_err(ViewerError::unreachable)?,
            )
        } else {
            return Err(ViewerError::Cdr {
                status: status.as_u16(),
                message: crate::locale::tr(ConsoleMsg::PurposeListRefused),
            });
        };
        if let Ok(mut held) = self.inner.held.lock() {
            *held = Some((Instant::now(), offered.clone()));
        }
        Ok(offered)
    }

    /// The list the CDR last answered, while it is held; `None` before the
    /// first read, after it expires, and when the CDR keeps no access log.
    #[must_use]
    pub fn last_offered(&self) -> Option<Offered> {
        self.held().and_then(|Held(offered)| offered)
    }

    fn held(&self) -> Option<Held> {
        let held = self.inner.held.lock().ok()?;
        held.as_ref()
            .filter(|(at, _)| at.elapsed() < HELD_FOR)
            .map(|(_, offered)| Held(offered.clone()))
    }

    /// The header and code a request as `credential` declares: the reader's
    /// choice when the CDR still lists it, else the configured default;
    /// `None` when neither applies.
    ///
    /// # Errors
    /// The errors of [`Self::offered`], and [`ViewerError::Invalid`] when the
    /// configured default is a code the CDR does not list.
    pub async fn declared(
        &self,
        credential: &Credential,
    ) -> Result<Option<(String, String)>, ViewerError> {
        let chosen = chosen();
        let Some(offered) = self.offered(credential).await? else {
            return Ok(None);
        };
        if let Some(code) = chosen.filter(|code| offered.accepts(code)) {
            return Ok(Some((offered.header, code)));
        }
        match &self.inner.default {
            Some(code) if offered.accepts(code) => Ok(Some((offered.header, code.clone()))),
            Some(code) => Err(ViewerError::Invalid(crate::locale::tr_with(
                ConsoleMsg::PurposeUnlisted,
                &[
                    ("code", code.clone().into()),
                    ("system", offered.system.into()),
                ],
            ))),
            None => Ok(None),
        }
    }
}

impl Purposes {
    /// The headers a request as `credential` carries under the justification
    /// `justification`: its purpose in the purpose header and its reason,
    /// percent-encoded, in the reason header; `None` when the CDR names no
    /// reason header, so it takes no justification.
    ///
    /// # Errors
    /// The errors of [`Self::offered`].
    pub async fn justified(
        &self,
        credential: &Credential,
        justification: &ferrohealth_session::justification::Justification,
    ) -> Result<Option<[(String, String); 2]>, ViewerError> {
        let Some(offered) = self.offered(credential).await? else {
            return Ok(None);
        };
        Ok(offered.reason_header.map(|reason_header| {
            [
                (offered.header, justification.purpose().to_owned()),
                (
                    reason_header,
                    crate::justified::encoded(justification.reason().expose()),
                ),
            ]
        }))
    }
}

/// The purpose the reader chose for this session, from [`COOKIE`].
#[must_use]
pub fn chosen() -> Option<String> {
    ferrohealth_ui::theme::cookie(&crate::request::cookies(), COOKIE)
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
}

/// The chooser's post: the code, empty to clear the choice, and where to go.
#[derive(Debug, Deserialize)]
pub struct Chosen {
    /// The chosen code; empty clears the choice.
    #[serde(default)]
    pub code: String,
    /// The same-origin page to return to.
    #[serde(default)]
    pub back: Option<String>,
}

/// Keeps the reader's choice in [`COOKIE`] for the session, `HttpOnly` and
/// `SameSite=Strict`, `Secure` whenever the session cookie is, and returns to
/// `back`.
pub async fn choose(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    Form(chosen): Form<Chosen>,
) -> Response {
    let secure = state.config.session.secure_cookie;
    let code = chosen.code.trim();
    let valid = code
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let keep = !code.is_empty() && valid;
    let value = if keep { code } else { "" };
    let expiry = if keep { "" } else { "; Max-Age=0" };
    let secure = if secure { "; Secure" } else { "" };
    let cookie = format!("{COOKIE}={value}; Path=/; SameSite=Strict; HttpOnly{expiry}{secure}");
    let back = crate::prefs::preference_return(chosen.back.as_deref().unwrap_or("/"), "purpose");
    ([(SET_COOKIE, cookie)], Redirect::to(&back)).into_response()
}

#[cfg(test)]
mod tests {
    use super::{Code, Offered};

    fn offered(codes: &[&str]) -> Offered {
        Offered {
            header: "x-purpose-of-use".to_owned(),
            reason_header: Some("x-access-reason".to_owned()),
            system: "http://terminology.hl7.org/CodeSystem/v3-ActReason".to_owned(),
            codes: codes
                .iter()
                .map(|code| Code {
                    code: (*code).to_owned(),
                    display: None,
                })
                .collect(),
        }
    }

    #[test]
    fn a_listed_code_is_accepted_and_an_unlisted_one_is_not() {
        let list = offered(&["TREAT", "ETREAT"]);
        assert!(list.accepts("TREAT"));
        assert!(!list.accepts("HRESCH"));
    }

    #[test]
    fn an_empty_list_accepts_any_code() {
        assert!(offered(&[]).accepts("HRESCH"));
    }
}
