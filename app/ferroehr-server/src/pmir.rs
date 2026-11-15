// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PMIR Patient Identity Consumer's background tasks: the ITI-94
//! subscription kept at the Registry, and this replica's applier of the
//! stored ITI-93 messages (PMIR 1.6.0 §2:3.93, §2:3.94).
//!
//! Every replica runs both: the subscription check takes an advisory lock, so
//! one replica at a time checks it, and the applier takes a lease on each
//! message, so one replica applies it. No specification governs the
//! schedule: our own design.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::config::FEED_PATH;
use ferroehr::service::demographic::identity_feed::subscribe::Subscription;

/// How often the subscription is checked once it is held.
const CHECK_EVERY: Duration = Duration::from_secs(600);

/// How long a failed check waits before the next.
const RETRY: Duration = Duration::from_secs(30);

/// Starts the subscription check and the applier when `[pix]` is set.
///
/// # Errors
/// When the `[pix.endpoint]` table does not build the ITI-94 client.
pub(crate) fn start(
    service: &Arc<FerroEhrService>,
    config: &ferroehr::config::FerroEhrConfig,
) -> anyhow::Result<()> {
    let Some(pix) = &config.pix else {
        return Ok(());
    };
    let root = config
        .server
        .public_root()
        .context("[pix] needs [server] public_base_url")?;
    let subscription = Subscription::new(pix, &root.at(FEED_PATH))
        .context("building the ITI-94 subscriber of [pix.endpoint]")?;
    tokio::spawn(keep_subscribed(Arc::clone(service), subscription));
    service.spawn_pmir_applier(uuid::Uuid::now_v7());
    Ok(())
}

/// Checks the subscription now and every [`CHECK_EVERY`], every [`RETRY`]
/// while a check fails.
#[expect(
    clippy::infinite_loop,
    reason = "a detached background task with no shutdown channel; it ends when the runtime \
              drops it, and `-> !` cannot be spawned on stable"
)]
async fn keep_subscribed(service: Arc<FerroEhrService>, subscription: Subscription) {
    loop {
        let wait = match service.pmir_subscribe(&subscription).await {
            Ok(outcome) => {
                tracing::info!(?outcome, "the PMIR subscription is checked");
                CHECK_EVERY
            }
            Err(error) => {
                tracing::warn!(
                    error = %ferroehr::error::ErrorChain::new(&error),
                    "the PMIR subscription could not be checked; it is checked again"
                );
                RETRY
            }
        };
        tokio::time::sleep(wait).await;
    }
}
