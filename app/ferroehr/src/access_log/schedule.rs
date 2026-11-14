// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled checkpoint of the access log (#5129) and the seal of the
//! system chain beside it (#5573).
//!
//! One run per [`INTERVAL`] seals the head of each chain when it moved since
//! its last seal, whenever the keyring holds an `OpenPGP` key (#5562).
//!
//! A run that cannot seal logs at `ERROR` and counts nothing as sealed; the
//! next run tries again. No openEHR spec governs log signing: our own design.

use std::time::Duration;

use crate::access_log::checkpoint::Checkpoints;

/// The time between two scheduled checkpoints: five minutes.
///
/// A checkpoint fixes the tail against a rewrite of the whole chain after it;
/// a run on a log that did not grow writes nothing, and one that did costs
/// one signature and one row.
// NOTE: our own design (#5562), no spec fixes the interval; a day holds at most 288 checkpoints.
pub const INTERVAL: Duration = Duration::from_mins(5);

/// One seal per `interval`, the first after one interval.
#[expect(
    clippy::infinite_loop,
    reason = "the checkpoint schedule is a detached background task with no shutdown channel; it \
              ends when the runtime drops the task, as the audit chain check does"
)]
pub async fn scheduled(checkpoints: Checkpoints, interval: Duration) {
    let first = tokio::time::Instant::now() + interval;
    let mut ticks = tokio::time::interval_at(first, interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        match checkpoints.seal().await {
            Ok(Some(checkpoint)) => tracing::debug!(
                sequence = checkpoint.sequence,
                "access-log checkpoint sealed"
            ),
            Ok(None) => tracing::debug!("access-log checkpoint: the chain is empty"),
            Err(error) => tracing::error!(
                error = %error,
                "the access-log checkpoint could not be sealed"
            ),
        }
        match checkpoints.system_seals().seal().await {
            Ok(Some(seal)) => tracing::debug!(sequence = seal.sequence, "system chain sealed"),
            Ok(None) => tracing::debug!("system chain seal: the chain is empty"),
            Err(error) => tracing::error!(
                error = %error,
                "the system chain could not be sealed"
            ),
        }
    }
}
