// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-94 subscription FerroEHR holds at the Patient Identity Registry
//! (PMIR 1.6.0 §2:3.94).
//!
//! One replica at a time, under a transaction-level advisory lock, makes sure
//! one subscription exists: it reads the status of the subscription whose
//! `Location` the store records (§2:3.94.4.3), and creates one when the store
//! records none or the Registry no longer holds it (§2:3.94.4.1). The criteria are every `Patient` (§2:3.94.4.1.2.1.1):
//! a Registry matches criteria against the `Patient` after the change, so a
//! criterion on an identifier system would not report the unlink that
//! removes the identifier it matches. A subscription the Registry set to
//! `error` or `off` is logged, since its messages wait until it is enabled
//! there. No specification governs when the subscriber checks: our own
//! design.

use std::time::Duration;

use ferrohealth_smart::client::outbound::OutboundClient;
use ferrohealth_smart::config::ServerTls;
use ihe_iti::pmir::PmirSubscriber;
use ihe_iti::pmir::subscription::{Criteria, Subscribed, SubscriptionRequest, SubscriptionStatus};
use sqlx::Connection as _;
use url::Url;

use super::config::PixConfig;
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::storage::error::StorageError;

/// The advisory lock key the subscribing replica holds ("pmir" in ASCII).
const LOCK: i64 = 0x706d_6972;

/// What a check of the subscription found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeOutcome {
    /// A subscription for the feed route exists with this status.
    Held(SubscriptionStatus),
    /// This check created the subscription.
    Created,
    /// Another replica is checking.
    Busy,
}

/// Why the subscription could not be checked.
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    /// The `[pix.endpoint]` table does not build a client.
    #[error("the [pix.endpoint] table does not build a client")]
    Endpoint(#[from] ferrohealth_smart::client::outbound::OutboundError),
    /// The subscription request is not valid ITI-94 input.
    #[error("the subscription request is not valid ITI-94 input")]
    Input(#[from] ihe_iti::pmir::error::InvalidInput),
    /// The Registry did not answer as ITI-94 has it.
    #[error("the Patient Identity Registry did not answer the subscription")]
    Registry(#[from] ihe_iti::pmir::error::SubscribeError),
    /// The store did not answer.
    #[error(transparent)]
    Service(#[from] ServiceError),
}

impl From<StorageError> for SubscribeError {
    fn from(error: StorageError) -> Self {
        Self::Service(error.into())
    }
}

impl From<sqlx::Error> for SubscribeError {
    fn from(error: sqlx::Error) -> Self {
        Self::Service(ServiceError::from(StorageError::from(error)))
    }
}

/// The subscriber `config` names, reaching the Registry for the feed route
/// `endpoint`.
#[derive(Debug, Clone)]
pub struct Subscription {
    subscriber: PmirSubscriber,
    request: SubscriptionRequest,
    timeout: Duration,
}

impl Subscription {
    /// The subscriber of `config` whose channel is `endpoint`.
    ///
    /// # Errors
    /// [`SubscribeError::Endpoint`] or [`SubscribeError::Input`] for a table
    /// or an endpoint the ITI-94 client refuses.
    pub fn new(config: &PixConfig, endpoint: &str) -> Result<Self, SubscribeError> {
        let client = OutboundClient::build(&config.endpoint, "pix.endpoint", &ServerTls)?;
        let endpoint = Url::parse(endpoint).map_err(|_unparsable| {
            SubscribeError::Input(ihe_iti::pmir::error::InvalidInput::Endpoint)
        })?;
        Ok(Self {
            subscriber: PmirSubscriber::new(client)?,
            request: SubscriptionRequest::new(Criteria::AllPatients, endpoint)?,
            timeout: config.endpoint.timeout.get(),
        })
    }
}

impl Subscription {
    /// The status of the subscription at the stored `location`, and the
    /// location; `None` when the Registry no longer holds it (`404`, `410`,
    /// FHIR R4 http.html, read) or the location lies outside its base, so
    /// a new one is created.
    async fn held(
        &self,
        location: &str,
    ) -> Result<Option<(String, SubscriptionStatus)>, SubscribeError> {
        // NOTE: a stored location outside the Registry's base names no subscription of it.
        let Ok(subscribed) = Subscribed::at(self.subscriber.endpoint(), location) else {
            return Ok(None);
        };
        match self.subscriber.status(&subscribed, self.timeout).await {
            Ok(status) => Ok(Some((subscribed.location().to_string(), status))),
            Err(ihe_iti::pmir::error::SubscribeError::Rejected { status, .. })
                if status == http::StatusCode::NOT_FOUND || status == http::StatusCode::GONE =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl FerroEhrService {
    /// Makes sure `subscription` is held at the Registry: the one whose
    /// `Location` the store records, or a new one.
    ///
    /// # Errors
    /// A [`SubscribeError`] when the Registry or the store does not answer.
    pub async fn pmir_subscribe(
        &self,
        subscription: &Subscription,
    ) -> Result<SubscribeOutcome, SubscribeError> {
        let mut conn = acquire(&self.demographic_pool)
            .await
            .map_err(ServiceError::from)?;
        let mut tx = conn.begin().await?;
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
            .bind(LOCK)
            .fetch_one(&mut *tx)
            .await?;
        if !locked {
            return Ok(SubscribeOutcome::Busy);
        }
        let endpoint = subscription.request.endpoint().as_str();
        let recorded: Option<String> =
            sqlx::query_scalar("SELECT location FROM pmir_subscription WHERE endpoint = $1")
                .bind(endpoint)
                .fetch_optional(&mut *tx)
                .await?;
        let held = match recorded {
            Some(location) => subscription.held(&location).await?,
            None => None,
        };
        let (location, status, outcome) = if let Some((location, status)) = held {
            (location, status, SubscribeOutcome::Held(status))
        } else {
            let created = subscription
                .subscriber
                .subscribe(&subscription.request, subscription.timeout)
                .await?;
            (
                created.location().to_string(),
                SubscriptionStatus::Requested,
                SubscribeOutcome::Created,
            )
        };
        sqlx::query(
            "INSERT INTO pmir_subscription (location, endpoint, status) VALUES ($1, $2, $3) \
             ON CONFLICT (singleton) DO UPDATE SET location = EXCLUDED.location, \
               endpoint = EXCLUDED.endpoint, status = EXCLUDED.status, checked_at = now()",
        )
        .bind(&location)
        .bind(endpoint)
        .bind(status.as_str())
        .execute(&mut *tx)
        .await?;
        // NOTE: our own design (#5762), the PMIR subscription, which runs in the background.
        tx.commit().await?;
        if matches!(status, SubscriptionStatus::Error | SubscriptionStatus::Off) {
            tracing::error!(
                status = status.as_str(),
                "the PMIR subscription is not active at the Registry: its ITI-93 messages wait \
                 until it is enabled there"
            );
        }
        Ok(outcome)
    }
}
