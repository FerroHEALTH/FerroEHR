// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the access evaluator reads of a versioned object where it leaves the
//! store: its EHR, the template and root archetype of its trunk head, the
//! systems that head came from and when it was committed.
//!
//! A read selects [`head_facts!`] beside its own columns, on the head row it
//! already joins as `h`, so judging an object adds no statement
//! ([`crate::access_eval::condition::admits`]). The same attributes are what
//! a query's condition reads in SQL ([`crate::aql::sql`]). No openEHR spec
//! governs this: our own design (#5513).

use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::access_eval::scope::{Item, Object};
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// What the evaluator reads of one versioned object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadFacts {
    /// The object.
    pub vo_id: VoId,
    /// The owning EHR; `None` for a demographic party.
    pub ehr_id: Option<EhrId>,
    /// The trunk head's template.
    pub template: Option<String>,
    /// The trunk head's root archetype.
    pub archetype: Option<String>,
    /// The trunk head's originating systems; `None` when nothing stamped them.
    pub origins: Option<Vec<String>>,
    /// The trunk head's commit instant.
    pub recorded: Option<jiff::Timestamp>,
}

impl HeadFacts {
    /// The facts of `vo_id` a row selecting [`head_facts!`] holds.
    ///
    /// # Errors
    /// [`StorageError::Database`] when a column does not decode.
    pub fn read(row: &PgRow, vo_id: VoId) -> Result<Self, StorageError> {
        let origins: Option<Value> = row.try_get("access_origins")?;
        Ok(Self {
            vo_id,
            ehr_id: row.try_get::<Option<Uuid>, _>("access_ehr_id")?.map(EhrId),
            template: row.try_get("access_template")?,
            archetype: row.try_get("access_archetype")?,
            origins: origins.map(|origins| {
                origins
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            }),
            recorded: row
                .try_get::<Option<jiff_sqlx::Timestamp>, _>("access_recorded")?
                .map(jiff_sqlx::Timestamp::to_jiff),
        })
    }

    /// The object as the evaluator judges it.
    #[must_use]
    pub fn item(&self) -> Item<'_> {
        Item::Object(Object {
            vo_id: self.vo_id.0,
            template: self.template.as_deref(),
            archetype: self.archetype.as_deref(),
            origins: self.origins.as_deref(),
            recorded: self.recorded,
        })
    }

    /// Whether the running request may read the object.
    #[must_use]
    pub fn admitted(&self) -> bool {
        crate::access_eval::condition::admits(self.ehr_id, &self.item())
    }
}

/// The facts of each of `vo_ids` this store holds, in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn of_objects(
    conn: &mut sqlx::PgConnection,
    vo_ids: &[VoId],
) -> Result<Vec<HeadFacts>, StorageError> {
    let ids: Vec<Uuid> = vo_ids.iter().map(|id| id.0).collect();
    let rows = sqlx::query(concat!(
        "SELECT h.vo_id, ",
        head_facts!(),
        " FROM vo_head h WHERE h.vo_id = ANY($1) ORDER BY h.vo_id"
    ))
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| HeadFacts::read(row, VoId(row.try_get("vo_id")?)))
        .collect()
}
