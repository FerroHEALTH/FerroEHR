// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled checkpoint of the access log (#5129): one run per interval
//! seals the head of the chain when it moved since the last checkpoint.
//!
//! A run that cannot seal logs at `ERROR` and counts nothing as sealed; the
//! next run tries again. No openEHR spec governs log signing: our own design.

use std::time::Duration;

use crate::access_log::checkpoint::Checkpoints;

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
