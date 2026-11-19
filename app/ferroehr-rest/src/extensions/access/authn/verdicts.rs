// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The token verdicts of the bearer issuers, for the `cache` gauges.

use std::sync::Arc;

/// The token verdicts `ferrohealth-smart` holds for the issuers of an
/// [`ferrohealth_smart::resource::IssuerSet`], reported under
/// `cache="token_verdicts"` (#6049).
#[derive(Debug, Clone)]
pub(crate) struct TokenVerdicts(pub(super) Arc<ferrohealth_smart::resource::IssuerSet>);

impl ferroehr::service::observed::HeldCaches for TokenVerdicts {
    fn held(&self) -> Vec<(&'static str, ferroehr::memory::Held)> {
        let usage = self.0.verdict_usage();
        vec![(
            "token_verdicts",
            ferroehr::memory::Held {
                entries: u64::try_from(usage.entries).unwrap_or(u64::MAX),
                bytes: u64::try_from(usage.bytes).unwrap_or(u64::MAX),
            },
        )]
    }
}
