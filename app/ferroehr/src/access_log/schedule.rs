// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled seal of the system chain (#5573).
//!
//! One run per [`INTERVAL`] seals the head of the system chain when it moved
//! since its last seal, whenever the keyring holds an `OpenPGP` key (#5562).
//! The access log needs no schedule: every append seals itself (#5704).
//!
//! Each run is timed under `audit_seal_duration`; one that cannot seal logs at
//! `ERROR`, counts as `failed` there and leaves `audit_seal_last_success`; the
//! next run tries again. No openEHR spec governs log signing: our own design.

use std::time::{Duration, Instant};

use opentelemetry::KeyValue;

use crate::access_log::checkpoint::Checkpoints;

/// The time between two scheduled seals: five minutes.
///
/// A seal fixes the tail against a rewrite of the whole chain after it; a run
/// on a chain that did not grow writes nothing, and one that did costs one
/// signature and one row.
// NOTE: our own design (#5562), no spec fixes the interval; a day holds at most 288 seals.
pub const INTERVAL: Duration = Duration::from_mins(5);

/// One seal per `interval`, the first after one interval.
#[expect(
    clippy::infinite_loop,
    reason = "the seal schedule is a detached background task with no shutdown channel; it \
              ends when the runtime drops the task, as the audit chain check does"
)]
pub async fn scheduled(checkpoints: Checkpoints, interval: Duration) {
    let first = tokio::time::Instant::now() + interval;
    let mut ticks = tokio::time::interval_at(first, interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let started = Instant::now();
        let sealed = checkpoints.system_seals().seal().await;
        measured(SYSTEM, started, &sealed.as_ref().map(Option::is_some));
        match sealed {
            Ok(Some(seal)) => tracing::debug!(sequence = seal.sequence, "system chain sealed"),
            Ok(None) => tracing::debug!("system chain seal: the chain is empty"),
            Err(error) => tracing::error!(
                error = %error,
                "the system chain could not be sealed"
            ),
        }
    }
}

/// The `chain` label of the system chain's seals.
const SYSTEM: &str = "system";

/// Record one seal run over `chain`, begun at `started`: whether it sealed
/// a head, found the chain empty, or failed. No openEHR spec governs the
/// metrics: our own design (#5591).
fn measured<E>(chain: &'static str, started: Instant, sealed: &Result<bool, E>) {
    let metrics = crate::telemetry::access_path::metrics();
    let outcome = match sealed {
        Ok(true) => "sealed",
        Ok(false) => "empty",
        Err(_) => "failed",
    };
    metrics.seal_duration.record(
        crate::telemetry::access_path::seconds_since(started),
        &[
            KeyValue::new("chain", chain),
            KeyValue::new("outcome", outcome),
        ],
    );
    if sealed.is_ok() {
        metrics.seal_last_success.record(
            crate::telemetry::access_path::unix_now(),
            &[KeyValue::new("chain", chain)],
        );
    }
}
