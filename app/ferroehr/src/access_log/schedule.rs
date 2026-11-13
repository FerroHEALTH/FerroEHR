// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled checkpoint of the access log (#5129): one run per
//! [`INTERVAL`] seals the head of the chain when it moved since the last
//! checkpoint, whenever the keyring holds an `OpenPGP` key (#5562).
//!
//! A run that cannot seal logs at `ERROR` and counts nothing as sealed; the
//! next run tries again. No openEHR spec governs log signing: our own design.

use std::time::Duration;

use crate::access_log::checkpoint::Checkpoints;

/// The time between two scheduled checkpoints: five minutes.
///
/// Links are not signed one by one, so the interval bounds how long a record
/// waits for a signature to cover it; a run on a log that did not grow writes
/// nothing, and one that did costs one signature and one row.
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
    }
}
