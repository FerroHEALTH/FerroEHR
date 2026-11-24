// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the authenticator says of its issuers outside a request: whether
//! each has been read, which the readiness indicator reports, and the SMART
//! discovery document of the described issuer, built once its metadata is
//! read. No openEHR spec governs when an issuer is read: our own design.

use ferrohealth_smart::resource::configuration::SmartOptions;

use super::Authenticator;
use super::build::Primary;
use super::verdicts;

/// The SMART discovery document of the described issuer, as far as it is
/// known.
#[derive(Debug)]
pub(crate) enum Described {
    /// No issuer is described: the document is not served.
    None,
    /// The described issuer has not been read yet.
    Unread,
    /// The document.
    Document(serde_json::Value),
}

impl Authenticator {
    /// Whether an issuer is read in the background, so readiness waits for it.
    pub(crate) fn reads_in_background(&self) -> bool {
        self.reads.started()
    }

    /// Whether every bearer issuer has been read, the answer of the
    /// readiness indicator: an issuer read in the background answers its
    /// tokens with `503` until it is.
    pub(crate) fn issuers_ready(&self) -> bool {
        self.bearer
            .as_ref()
            .is_none_or(verdicts::TokenVerdicts::is_ready)
    }

    /// Reads the described issuer when it is read in the background and has
    /// not been read yet, sharing a read in flight, as its first token does;
    /// a read that failed within its refresh interval is not tried again.
    pub(crate) async fn read_described(&self) {
        if let Some((Primary::Lazy(lazy), _)) = &self.primary
            && let Err(rejection) = lazy.server().await
        {
            tracing::debug!(%rejection, "the described issuer is not read yet");
        }
    }

    /// Whether the SMART discovery document describes an issuer.
    pub(crate) fn describes(&self) -> bool {
        self.primary.is_some()
    }

    /// Whether an issuer is trusted to assert a patient context, so a
    /// `patient/` grant counts and the discovery document says so.
    pub(crate) fn patient_context(&self) -> bool {
        self.patient_context
    }

    /// The SMART discovery document of the described issuer under `options`.
    ///
    /// The document is built for the purpose `describe` describes the issuer
    /// for (SMART App Launch 2.2.0, Conformance, §Metadata).
    pub(crate) fn smart_configuration(&self, options: &SmartOptions) -> Described {
        let Some((primary, purpose)) = &self.primary else {
            return Described::None;
        };
        let options = SmartOptions {
            purpose: *purpose,
            ..options.clone()
        };
        Described::Document(match primary {
            Primary::Jwt(server) => server.smart_configuration(&options),
            Primary::Lazy(lazy) => match lazy.get() {
                Some(server) => server.smart_configuration(&options),
                None => return Described::Unread,
            },
            Primary::Introspection(introspector) => introspector.smart_configuration(&options),
        })
    }
}
