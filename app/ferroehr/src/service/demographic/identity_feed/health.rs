// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `pmir_refused_messages` health indicator (#5725): how many refused
//! ITI-93 messages wait for an operator.
//!
//! A refused message keeps the identifiers and record subjects it split
//! unresolved until an operator retries or settles it, so the indicator
//! reads `DEGRADED` once their number reaches `[pix] refused_degraded_at`.
//! The split fails closed and the instance still serves, so it never
//! withdraws readiness. Each check also records the
//! `pmir_messages_refused` gauge. No specification governs health probes:
//! our own design.

use std::sync::Arc;

use ferrohealth_health::registry::{Check, HealthIndicator, Outcome};

use crate::service::FerroEhrService;

/// `pmir_refused_messages`, registered while `[pix]` is set.
#[derive(Debug)]
pub struct RefusedMessagesHealth {
    service: Arc<FerroEhrService>,
    degraded_at: u32,
}

impl RefusedMessagesHealth {
    /// Constructs over `service`, reading `DEGRADED` from `degraded_at`
    /// refused messages on.
    #[must_use]
    pub fn new(service: Arc<FerroEhrService>, degraded_at: u32) -> Self {
        Self {
            service,
            degraded_at,
        }
    }
}

impl HealthIndicator for RefusedMessagesHealth {
    fn name(&self) -> &'static str {
        "pmir_refused_messages"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            match self.service.pmir_refused_count().await {
                Ok(count) => judge(count, self.degraded_at),
                Err(error) => {
                    tracing::warn!(
                        error = %crate::error::ErrorChain::new(&error),
                        "health: the refused ITI-93 messages could not be counted"
                    );
                    Outcome::degraded("the refused ITI-93 messages could not be counted")
                }
            }
        })
    }
}

/// The outcome of `count` refused messages against the threshold
/// `degraded_at`.
fn judge(count: u64, degraded_at: u32) -> Outcome {
    if count >= u64::from(degraded_at) {
        Outcome::degraded(format!(
            "{count} refused ITI-93 messages keep the identifiers they split unresolved: list \
             them with GET /admin/pmir/messages or `ferroehr pmir list`, then retry or settle each"
        ))
    } else {
        Outcome::up()
    }
}

#[cfg(test)]
mod tests {
    use super::judge;
    use ferrohealth_health::document::State;

    #[test]
    fn the_indicator_degrades_from_the_threshold_on() {
        assert_eq!(judge(0, 1).state(), State::Up);
        assert_eq!(judge(1, 1).state(), State::Degraded);
        assert_eq!(judge(2, 3).state(), State::Up);
        assert_eq!(judge(3, 3).state(), State::Degraded);
    }
}
