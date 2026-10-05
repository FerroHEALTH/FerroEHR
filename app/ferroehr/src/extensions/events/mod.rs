// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[events]` config section + the platform-coupled halves of the
//! eventing extension: the outbox relay (`publisher`) and the subscription
//! CRUD store (`subscription`).
//!
//! **No openEHR spec governs this — our own design/extension.** The
//! transport-side core (the `EventPublisher` seam, the AMQP publisher, the
//! routing-key grammar) lives in `ferroehr_ext::events` behind the `events`
//! cargo feature; what stays here is DB- and service-coupled.

pub mod config;

#[cfg(feature = "events")]
pub mod publisher;
#[cfg(feature = "events")]
pub mod subscription;

/// The loud slim-build refusal: a configuration that enables eventing on a
/// binary compiled without the `events` cargo feature is a boot error,
/// never a silent ignore.
///
/// # Errors
/// The refusal message when `events.enabled` is set.
#[cfg(not(feature = "events"))]
pub fn require_disabled(cfg: &config::EventsConfig) -> Result<(), String> {
    if cfg.enabled {
        return Err(
            "events.enabled = true, but this binary was built without the \
             `events` cargo feature"
                .to_owned(),
        );
    }
    Ok(())
}
