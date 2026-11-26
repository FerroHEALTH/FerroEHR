// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The purposes of use a caller may declare.
//!
//! They are the `[audit] purpose_codes` this deployment accepts, under
//! `[audit] purpose_system`, each with the display text the terminology
//! server's `CodeSystem/$lookup` gives it, so a client can offer them by name.
//! No openEHR spec governs the purpose of use: our own design (#6105).

use crate::service::FerroEhrService;
use crate::service::terminology::types::TermEntry;

/// The purposes of use this deployment accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Purposes {
    /// The request header a caller declares its purpose in, lowercased.
    pub header: String,
    /// The request header a caller states the reason of a justified access
    /// in (`[audit] reason_header`), lowercased.
    pub reason_header: String,
    /// The canonical URL of the code system the codes belong to.
    pub system: String,
    /// The accepted codes in configured order; empty when the deployment
    /// accepts any code.
    pub codes: Vec<Purpose>,
}

/// One accepted purpose of use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Purpose {
    /// The code a caller declares.
    pub code: String,
    /// The code's display text from the terminology server; `None` when the
    /// deployment configures no terminology server or its lookup failed.
    pub display: Option<String>,
}

impl FerroEhrService {
    /// The purposes of use a caller may declare, `None` when no audit sender
    /// is wired.
    ///
    /// Each code is looked up on the terminology server under the purpose
    /// system; a failed lookup leaves that code without display text and is
    /// logged, since the list names no personal data and a client shows the
    /// code itself.
    pub async fn audit_purposes(&self) -> Option<Purposes> {
        let sender = self.audit.as_ref()?;
        let system = sender.purpose_system();
        let mut codes = Vec::with_capacity(sender.purpose_codes().len());
        for code in sender.purpose_codes() {
            let display = match self.terminology_server() {
                Some(server) => match server.get_term(system, code, None, None).await {
                    Ok(extract) => extract
                        .terms
                        .and_then(|mut terms| match terms.remove(code) {
                            Some(TermEntry::Defined(term)) => Some(term.text),
                            Some(TermEntry::Bare(_)) | None => None,
                        }),
                    Err(error) => {
                        tracing::warn!(system, code, %error, "a purpose code has no display text");
                        None
                    }
                },
                None => None,
            };
            codes.push(Purpose {
                code: code.clone(),
                display,
            });
        }
        Some(Purposes {
            header: sender.purpose_header().to_owned(),
            reason_header: sender.reason_header().to_owned(),
            system: system.to_owned(),
            codes,
        })
    }
}
