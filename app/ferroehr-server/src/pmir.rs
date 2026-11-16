// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PMIR Patient Identity Consumer on the server, and `ferroehr pmir …`.
//!
//! The background tasks are the ITI-94 subscription kept at the Registry and
//! this replica's applier of the stored ITI-93 messages (PMIR 1.6.0 §2:3.93,
//! §2:3.94), with the `pmir_refused_messages` indicator. `ferroehr pmir …`
//! is the operator's list, retry and settlement of the refused messages
//! (#5725).
//!
//! Every replica runs both: the subscription check takes an advisory lock, so
//! one replica at a time checks it, and the applier takes a lease on each
//! message, so one replica applies it. No specification governs the
//! schedule: our own design.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::config::FEED_PATH;
use ferroehr::service::demographic::identity_feed::health::RefusedMessagesHealth;
use ferroehr::service::demographic::identity_feed::operator::Decision;
use ferroehr::service::demographic::identity_feed::subscribe::Subscription;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use ferrohealth_health::registry::HealthIndicator;

/// `ferroehr pmir …` subcommands. Each names its operator, whom the access
/// record of every message it lists, retries or settles names, beside the
/// operating-system account that ran the command.
#[derive(Debug, Subcommand)]
pub enum PmirCmd {
    /// Print the refused messages still to retry or settle, oldest first, as
    /// JSON: each with its refusal code, receipt and refusal time, attempts,
    /// the Registry's identities it names and the `PERSON`s it keeps split,
    /// as counts. No identifier value is printed.
    List {
        /// The operator the access records name.
        #[arg(long, value_name = "NAME")]
        operator: String,
    },
    /// Put a refused message back at its arrival position once its cause is
    /// fixed, such as a `[[pix.identifier_domain]]` entry or a grant added; a
    /// running replica applies it next. Refused when a message received after
    /// it that names one of its identities has applied.
    Retry {
        /// The message's `MessageHeader.id`.
        message_id: String,
        /// The operator the access record names.
        #[arg(long, value_name = "NAME")]
        operator: String,
    },
    /// Close a refused message without applying it: `discard` keeps its
    /// splits, `lift-splits` removes them so its identifiers and record
    /// subjects resolve as before the message. A discarded message may still
    /// have its splits lifted.
    Settle {
        /// The message's `MessageHeader.id`.
        message_id: String,
        /// `discard` or `lift-splits`.
        #[arg(long, value_name = "DECISION")]
        decision: String,
        /// Why, 1 to 1000 characters naming no identifier.
        #[arg(long, value_name = "TEXT")]
        reason: String,
        /// The operator the row and the access record name.
        #[arg(long, value_name = "NAME")]
        operator: String,
    },
}

/// How often the subscription is checked once it is held.
const CHECK_EVERY: Duration = Duration::from_secs(600);

/// How long a failed check waits before the next.
const RETRY: Duration = Duration::from_secs(30);

/// Starts the subscription check and the applier, and registers the
/// `pmir_refused_messages` indicator in `indicators`, when `[pix]` is set.
///
/// # Errors
/// When the `[pix.endpoint]` table does not build the ITI-94 client.
pub(crate) fn start(
    service: &Arc<FerroEhrService>,
    config: &ferroehr::config::FerroEhrConfig,
    indicators: &mut Vec<Arc<dyn HealthIndicator>>,
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
    indicators.push(Arc::new(RefusedMessagesHealth::new(
        Arc::clone(service),
        pix.refused_degraded_at,
    )));
    Ok(())
}

/// Runs one `ferroehr pmir …` subcommand under the loaded `config` as its
/// operator, with the configured audit trail recording each act, and prints
/// its answer.
///
/// # Errors
/// The configuration does not load or validate, the decision is unknown, the
/// store refuses, or the act is refused (no such message, not refused,
/// superseded, or a reason out of bounds).
#[expect(
    clippy::print_stdout,
    reason = "the pmir subcommands' PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn run(cmd: &PmirCmd, config: &ferroehr::config::FerroEhrConfig) -> anyhow::Result<()> {
    config.validate()?;
    let pools = crate::connect_pool(config).await?;
    let (sender, handle) = crate::start_audit(&config.audit, &pools.clinical, &pools.audit).await?;
    let mut service = FerroEhrService::new(&pools);
    if let Some(sender) = sender {
        service = service.with_audit(sender);
    }
    let answer = Box::pin(act(&service, cmd)).await;
    drop(service);
    if let Some(handle) = handle {
        handle.shutdown(crate::AUDIT_STOP_TIMEOUT).await;
    }
    println!("{}", answer?);
    Ok(())
}

/// Runs `cmd` on `service` as its operator; the JSON answer.
///
/// # Errors
/// As [`run`].
pub async fn act(service: &FerroEhrService, cmd: &PmirCmd) -> anyhow::Result<String> {
    let (PmirCmd::List { operator }
    | PmirCmd::Retry { operator, .. }
    | PmirCmd::Settle { operator, .. }) = cmd;
    anyhow::ensure!(!operator.trim().is_empty(), "--operator names the operator");
    let account = os_account()?;
    with_committer(Some(committer(operator, &account)), async {
        match cmd {
            PmirCmd::List { .. } => {
                let listed = service.pmir_refused().await.context("listing")?;
                Ok::<_, anyhow::Error>(serde_json::to_string_pretty(&listed)?)
            }
            PmirCmd::Retry { message_id, .. } => {
                let retried = service.pmir_retry(message_id).await.context("retrying")?;
                Ok(serde_json::to_string_pretty(&retried)?)
            }
            PmirCmd::Settle {
                message_id,
                decision,
                reason,
                ..
            } => {
                let decision = Decision::of(decision).with_context(|| {
                    format!("--decision {decision:?} is neither discard nor lift-splits")
                })?;
                let settled = service
                    .pmir_settle(message_id, decision, reason)
                    .await
                    .context("settling")?;
                Ok(serde_json::to_string_pretty(&settled)?)
            }
        }
    })
    .await
}

/// The committer an operator's act runs as.
///
/// Its subject is the operator's name from the command line of a process that
/// holds the database credentials; its client is the operating-system
/// `account` that ran it, which every access record of the act names beside
/// the operator.
#[must_use]
pub fn committer(operator: &str, account: &str) -> CommitterIdentity {
    CommitterIdentity {
        subject: operator.trim().to_owned(),
        id_type: "operator",
        issuer: None,
        client: Some(account.to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: None,
    }
}

/// The operating-system account this process runs as: its effective user's
/// name and id, as `os:<name>/<uid>`, or `os:<uid>` when the user database
/// names none.
///
/// # Errors
/// When the user database cannot be read.
#[cfg(unix)]
pub fn os_account() -> anyhow::Result<String> {
    let uid = nix::unistd::geteuid();
    let user = nix::unistd::User::from_uid(uid).context("reading the effective user")?;
    Ok(match user {
        Some(user) => format!("os:{}/{uid}", user.name),
        None => format!("os:{uid}"),
    })
}

/// The operating-system account this process runs as; a platform without
/// Unix user ids names none, so an operator's act is refused there.
///
/// # Errors
/// Always.
#[cfg(not(unix))]
pub fn os_account() -> anyhow::Result<String> {
    anyhow::bail!("ferroehr pmir records the effective user, which this platform does not name")
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
