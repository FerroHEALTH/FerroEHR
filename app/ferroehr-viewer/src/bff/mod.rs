// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's reads and writes of the CDR, area by area: the functions the
//! pages call as they are drawn, and the pure readings of what the CDR
//! answers.
//!
//! Every function that reaches the CDR takes the session's handle, [`cdr`],
//! first (it is a public endpoint), and the CDR credential never leaves the
//! server. The wire
//! is ITS-REST (`docs/specs/openehr/ITS-REST/`); the pages are our own design.

pub mod audit;
pub mod composition;
pub mod dashboard;
pub mod demographics;
pub mod ehr_detail;
pub mod ehr_tags;
pub mod ehrs;
pub mod item_tags;
pub mod system;
pub mod template_adl2;
pub mod template_detail;
pub mod templates;
pub mod wire;

use crate::cdr::CdrCall;
use crate::error::ViewerError;

/// The signed-in session's handle on the CDR: the client with the
/// session's credential, and the client's URL builders through `Deref`.
#[derive(Debug)]
pub struct Cdr {
    client: crate::cdr::CdrClient,
    credential: crate::session::Credential,
    identity: String,
}

/// The handle of the session the request carries.
///
/// The errors every read and write of this module may answer start here and
/// in [`CdrCall`]: [`ViewerError::Unauthenticated`] without a session,
/// [`ViewerError::CdrUnreachable`] when the CDR does not answer,
/// [`ViewerError::CdrUnauthorized`], [`ViewerError::Forbidden`] and
/// [`ViewerError::Cdr`] for a refusal ([`CdrCall::ok`]), and
/// [`ViewerError::Invalid`] or [`ViewerError::Internal`] for an input or an
/// answer the function cannot use.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a session, which also ends the
/// page's session.
pub fn cdr() -> Result<Cdr, ViewerError> {
    let session = crate::session::require_session()?;
    Ok(Cdr {
        client: crate::scope::state()?.cdr,
        credential: session.credential,
        identity: session.identity,
    })
}

impl Cdr {
    /// Who the session signed in as.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// A `GET` of `url`.
    pub fn get(&self, url: &str) -> CdrCall {
        self.client.call(http::Method::GET, url, &self.credential)
    }

    /// A `POST` to `url`.
    pub fn post(&self, url: &str) -> CdrCall {
        self.client.call(http::Method::POST, url, &self.credential)
    }

    /// A `PUT` to `url`.
    pub fn put(&self, url: &str) -> CdrCall {
        self.client.call(http::Method::PUT, url, &self.credential)
    }

    /// A `DELETE` of `url`.
    pub fn delete(&self, url: &str) -> CdrCall {
        self.client
            .call(http::Method::DELETE, url, &self.credential)
    }
}

impl std::ops::Deref for Cdr {
    type Target = crate::cdr::CdrClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

/// The rows one page of a listing holds: FerroUI's default page size.
pub const PAGE_SIZE: u32 = ferrohealth_ui::table::model::DEFAULT_SIZE;

/// A body pretty-printed for reading in `format`: JSON indented, canonical
/// XML as the CDR sent it, and a body that is no JSON passed through whole.
#[must_use]
#[expect(
    clippy::disallowed_types,
    reason = "pretty-printing reads the CDR's JSON wire as an untyped tree"
)]
pub fn pretty_body(body: &str, format: crate::format::ReprFormat) -> String {
    match format {
        crate::format::ReprFormat::CanonicalXml => body.to_owned(),
        // NOTE: no specification governs this: our own design. A body that is
        // not JSON is legitimately not of this form and is shown as sent.
        _ => serde_json::from_str::<serde_json::Value>(body)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or_else(|_| body.to_owned()),
    }
}
