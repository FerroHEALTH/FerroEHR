// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The event-filter subscription store: CRUD over `event_subscription` as
//! methods on `FerroEhrService`.
//!
//! **No openEHR spec governs this — our own design/extension** (the
//! event/subscription semantics are our own model, not any SM interface).
//! Part of the `events` extension; gate: `events.enabled` (a subscription is
//! inert unless the publisher is spawned to bind its queue).
//!
//! A subscription is a small predicate record (`kind` / `change_type` /
//! `template_id`, each NULL = wildcard) that the publisher turns
//! into an AMQP topic binding on the events exchange so the broker fans events
//! out to a durable per-subscription queue ([`super`]). This module owns only
//! the CRUD; queue declaration is the drainer's concern (it re-syncs the
//! enabled set when it changes or the broker connection is fresh — the service
//! is kept broker-free).
//
// The CRUD helpers below read the `pub(crate)` `pool` field of
// `crate::service::FerroEhrService`.

use sqlx::Row;
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::{CallStatusType, SmError};

/// A stored event-subscription record (the subscription admin API's response
/// row).
///
/// A `None` predicate is the wildcard, rendered as JSON `null`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubscriptionRecord {
    /// The subscription's surrogate UUID key.
    pub id: Uuid,
    /// The unique subscription name (the AMQP queue-name suffix).
    pub name: String,
    /// The versioned-object kind predicate (`None` = wildcard).
    pub kind: Option<String>,
    /// The audit change-type predicate (`None` = wildcard).
    pub change_type: Option<String>,
    /// The template-id predicate (`None` = wildcard).
    pub template_id: Option<String>,
    /// Whether the publisher binds this subscription's queue.
    pub enabled: bool,
    /// The row's creation instant.
    pub created_at: jiff::Timestamp,
}

/// A client-submitted subscription definition (create): required `name`,
/// optional predicates (absent/`null`/empty = wildcard), `enabled` default
/// `true`.
#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubscriptionDefinition {
    /// The subscription name (required; validated to `[A-Za-z0-9_.-]`). An
    /// absent name arrives empty and is refused by `validated_name`.
    pub name: String,
    /// The versioned-object kind predicate (absent/`null` = wildcard).
    pub kind: Option<String>,
    /// The audit change-type predicate (absent/`null` = wildcard).
    pub change_type: Option<String>,
    /// The template-id predicate (absent/`null` = wildcard).
    pub template_id: Option<String>,
    /// Whether the subscription starts enabled (default `true`).
    pub enabled: bool,
}

impl Default for SubscriptionDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: None,
            change_type: None,
            template_id: None,
            enabled: true,
        }
    }
}

/// A client-submitted subscription update: the predicates + `enabled`.
///
/// The `name` is immutable — it is the queue key — so an echoed `name` from a
/// prior GET is tolerated and ignored, like the other echoed read-only fields.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubscriptionUpdate {
    /// Echoed read-only members from a prior GET, tolerated and ignored so a
    /// client can round-trip the whole record; every OTHER unknown key is
    /// refused (`deny_unknown_fields`), so a removed or misspelled predicate
    /// fails loudly instead of silently dropping.
    #[serde(rename = "name")]
    pub echoed_name: Option<serde::de::IgnoredAny>,
    /// See [`Self::echoed_name`].
    #[serde(rename = "id")]
    pub echoed_id: Option<serde::de::IgnoredAny>,
    /// See [`Self::echoed_name`].
    #[serde(rename = "created_at")]
    pub echoed_created_at: Option<serde::de::IgnoredAny>,
    /// The versioned-object kind predicate (absent/`null` = wildcard).
    pub kind: Option<String>,
    /// The audit change-type predicate (absent/`null` = wildcard).
    pub change_type: Option<String>,
    /// The template-id predicate (absent/`null` = wildcard).
    pub template_id: Option<String>,
    /// Whether the publisher binds this subscription's queue. REQUIRED on an
    /// update: the operation is a full replace, and a defaulted `true` here
    /// silently re-enabled a deliberately disabled subscription — the
    /// caller states the whole intent or the request is refused.
    pub enabled: Option<bool>,
}

impl FerroEhrService {
    /// Map an `event_subscription` row to its PHI-free typed record.
    fn subscription_row(row: &sqlx::postgres::PgRow) -> Result<SubscriptionRecord, ServiceError> {
        Ok(SubscriptionRecord {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            kind: row.try_get("kind")?,
            change_type: row.try_get("change_type")?,
            template_id: row.try_get("template_id")?,
            enabled: row.try_get("enabled")?,
            created_at: row
                .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
                .to_jiff(),
        })
    }

    /// List every stored subscription (newest first).
    async fn list_subscriptions(&self) -> Result<Vec<SubscriptionRecord>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, name, kind, change_type, template_id, enabled, created_at \
             FROM event_subscription ORDER BY created_at DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::subscription_row).collect()
    }

    /// Fetch one subscription by id, or `NotFound`.
    async fn get_subscription(&self, id: Uuid) -> Result<SubscriptionRecord, ServiceError> {
        let row = sqlx::query(
            "SELECT id, name, kind, change_type, template_id, enabled, created_at \
             FROM event_subscription WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("event subscription {id}"),
            )
        })?;
        Self::subscription_row(&row)
    }

    /// Create a subscription from a [`SubscriptionDefinition`]. The `name` is
    /// validated; the predicates normalize to `None` (wildcard) when empty. A
    /// duplicate name is a `Conflict`.
    async fn create_subscription(
        &self,
        body: &SubscriptionDefinition,
    ) -> Result<SubscriptionRecord, ServiceError> {
        let name = validated_name(&body.name)?;
        let row = sqlx::query(
            "INSERT INTO event_subscription \
             (name, kind, change_type, template_id, enabled) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, name, kind, change_type, template_id, enabled, created_at",
        )
        .bind(&name)
        .bind(predicate(body.kind.as_deref()))
        .bind(predicate(body.change_type.as_deref()))
        .bind(predicate(body.template_id.as_deref()))
        .bind(body.enabled)
        .fetch_one(&self.pool)
        .await
        .map_err(map_insert_error)?;
        Self::subscription_row(&row)
    }

    /// Replace a subscription's predicates + `enabled` (its `name` is immutable —
    /// it is the queue key). `NotFound` if the id is unknown.
    async fn update_subscription(
        &self,
        id: Uuid,
        body: &SubscriptionUpdate,
    ) -> Result<SubscriptionRecord, ServiceError> {
        let Some(enabled) = body.enabled else {
            return Err(ServiceError::precondition(
                "event subscription update is a full replace: 'enabled' must be                  stated explicitly — omitting it would silently re-enable a                  disabled subscription"
                    .to_owned(),
            ));
        };
        let row = sqlx::query(
            "UPDATE event_subscription \
             SET kind = $2, change_type = $3, template_id = $4, enabled = $5 \
             WHERE id = $1 \
             RETURNING id, name, kind, change_type, template_id, enabled, created_at",
        )
        .bind(id)
        .bind(predicate(body.kind.as_deref()))
        .bind(predicate(body.change_type.as_deref()))
        .bind(predicate(body.template_id.as_deref()))
        .bind(enabled)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("event subscription {id}"),
            )
        })?;
        Self::subscription_row(&row)
    }

    /// Delete a subscription by id. `NotFound` if the id is unknown.
    ///
    /// NOTE: the broker queue the deleted subscription bound is not torn
    /// down here — the service is broker-free. A durable queue simply stops
    /// being (re)bound; operators reap orphaned queues out of band. Re-binding
    /// of the *remaining* subscriptions is the drainer's job.
    async fn delete_subscription(&self, id: Uuid) -> Result<(), ServiceError> {
        let deleted = sqlx::query("DELETE FROM event_subscription WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if deleted == 0 {
            return Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("event subscription {id}"),
            ));
        }
        Ok(())
    }
}

/// Validate the submitted `name`: non-empty after trimming, and restricted to
/// `[A-Za-z0-9_.-]` so it is a clean AMQP queue-name suffix
/// (`ferroehr.events.<name>`).
fn validated_name(raw: &str) -> Result<String, ServiceError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ServiceError::precondition(
            "event subscription requires a non-empty 'name'".to_owned(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(ServiceError::precondition(
            "event subscription 'name' must match [A-Za-z0-9_.-]".to_owned(),
        ));
    }
    Ok(name.to_owned())
}

/// Normalize a submitted predicate: a trimmed non-empty string, else `None`
/// (wildcard). An absent field, JSON `null`, and an empty string are all
/// "wildcard".
fn predicate(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Map an INSERT failure: a unique-name violation is a client `Conflict` (409);
/// anything else is the underlying DB error.
fn map_insert_error(e: sqlx::Error) -> ServiceError {
    if let sqlx::Error::Database(db) = &e
        && db.is_unique_violation()
    {
        return ServiceError::conflict("an event subscription with that name exists".to_owned());
    }
    ServiceError::Database(e)
}

impl FerroEhrService {
    /// List every stored event subscription (newest first) as PHI-free typed
    /// records.
    ///
    /// # Errors
    /// [`SmError`] wrapping a database failure.
    pub async fn event_subscription_list(&self) -> Result<Vec<SubscriptionRecord>, SmError> {
        Ok(self.list_subscriptions().await?)
    }

    /// Create an event subscription from a [`SubscriptionDefinition`].
    ///
    /// # Errors
    /// `BadRequest` when `name` is empty or not `[A-Za-z0-9_.-]`; `Conflict`
    /// on a duplicate name; otherwise a database failure.
    pub async fn event_subscription_create(
        &self,
        a_subscription: SubscriptionDefinition,
    ) -> Result<SubscriptionRecord, SmError> {
        Ok(self.create_subscription(&a_subscription).await?)
    }

    /// Fetch one event subscription by id.
    ///
    /// # Errors
    /// `NotFound` when the id is unknown; otherwise a database failure.
    pub async fn event_subscription_get(
        &self,
        a_subscription_id: Uuid,
    ) -> Result<SubscriptionRecord, SmError> {
        Ok(self.get_subscription(a_subscription_id).await?)
    }

    /// Replace a subscription's predicates + `enabled` (the `name` is
    /// immutable — it is the queue key).
    ///
    /// # Errors
    /// `NotFound` when the id is unknown; otherwise a database failure.
    pub async fn event_subscription_update(
        &self,
        a_subscription_id: Uuid,
        a_subscription: SubscriptionUpdate,
    ) -> Result<SubscriptionRecord, SmError> {
        Ok(self
            .update_subscription(a_subscription_id, &a_subscription)
            .await?)
    }

    /// Delete an event subscription by id (the broker queue is not torn down —
    /// see the NOTE on the private helper).
    ///
    /// # Errors
    /// `NotFound` when the id is unknown; otherwise a database failure.
    pub async fn event_subscription_delete(&self, a_subscription_id: Uuid) -> Result<(), SmError> {
        Ok(self.delete_subscription(a_subscription_id).await?)
    }
}
