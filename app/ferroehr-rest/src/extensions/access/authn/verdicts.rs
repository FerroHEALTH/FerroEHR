// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The token verdicts of the bearer issuers, for the `cache` gauges.

use std::sync::Arc;

/// The token verdicts `ferrohealth-smart` holds for the issuers of an
/// [`ferrohealth_smart::resource::IssuerSet`], reported under
/// `cache="token_verdicts"` (#6049).
#[derive(Debug, Clone)]
pub(crate) struct TokenVerdicts(pub(super) Arc<ferrohealth_smart::resource::IssuerSet>);

impl TokenVerdicts {
    /// Whether every issuer of the set has been read.
    pub(crate) fn is_ready(&self) -> bool {
        self.0.is_ready()
    }
}

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

/// The readiness indicator of the bearer issuers: down while an issuer read
/// in the background has not been read, since its tokens are refused with
/// `503` (no specification governs this: our own design).
#[derive(Debug)]
pub(crate) struct IssuersHealth(pub(crate) Arc<super::Authenticator>);

impl ferrohealth_health::registry::HealthIndicator for IssuersHealth {
    fn name(&self) -> &'static str {
        "auth_issuers"
    }

    fn check(&self) -> ferrohealth_health::registry::Check<'_> {
        Box::pin(async {
            if self.0.issuers_ready() {
                ferrohealth_health::registry::Outcome::up()
            } else {
                ferrohealth_health::registry::Outcome::down(
                    "an [auth] issuer's metadata or key set has not been read yet; its tokens \
                     are refused with 503",
                )
            }
        })
    }
}
