// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the access evaluator reads of a versioned object where it leaves the
//! store.
//!
//! It reads the object's EHR, the template and root archetype of its trunk
//! head, the systems that head came from, when it was committed and, once a
//! scope names one, the episodes it is placed in.
//!
//! A read selects `head_facts!` beside its own columns, on the head row it
//! already joins as `h`, so judging an object adds no statement
//! ([`crate::access_eval::condition::admits`]). The same attributes are what
//! a query's condition reads in SQL ([`crate::aql::sql`]). No openEHR spec
//! governs this: our own design (#5513).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments: the \
              version.origins stamp is read as stored"
)]

use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::access_eval::scope::{Item, Object};
use crate::ids::{EhrId, VoId};
use crate::storage::access_projection::Revision;
use crate::storage::error::StorageError;
use crate::versioning::episode::EpisodeKey;

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
    /// The episodes the trunk head is placed in, empty when none; `None` when
    /// the read did not select them.
    pub episodes: Option<Vec<EpisodeKey>>,
    /// The revision of the owning EHR's settings row the read found.
    pub revision: Revision,
}

impl HeadFacts {
    /// The facts of `vo_id` a row selecting `head_facts!` holds.
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
            origins: origins.as_ref().map(stamped),
            recorded: row
                .try_get::<Option<jiff_sqlx::Timestamp>, _>("access_recorded")?
                .map(jiff_sqlx::Timestamp::to_jiff),
            episodes: episodes(row)?,
            revision: Revision::read(row)?,
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
            episodes: self.episodes.as_deref(),
        })
    }

    /// Whether the running request may read the object.
    #[must_use]
    pub fn admitted(&self) -> bool {
        crate::access_eval::condition::admits(self.ehr_id, &self.revision, &self.item())
    }
}

/// Whether the running request may read the version `stored`, judged by its
/// template, root archetype and origins and its object's last commit.
#[must_use]
pub fn admits_stored(stored: &crate::storage::version_repo::read::StoredVersion) -> bool {
    let origins = stored.origins.as_ref().map(stamped);
    let item = Item::Object(Object {
        vo_id: stored.vo_id.0,
        template: stored.template_id.as_deref(),
        archetype: stored.root_archetype.as_deref(),
        origins: origins.as_deref(),
        recorded: stored.head_committed_at,
        episodes: stored.head_episodes.as_deref(),
    });
    crate::access_eval::condition::admits(stored.ehr_id, &stored.access_revision, &item)
}

/// Whether `stored` may be served to the current request.
///
/// The access evaluator admits it ([`admits_stored`]), and an object an EHDS
/// Art. 3(3) delay withholds is absent to a patient-context request
/// (`docs/law/eu/ehds/text.html`). No openEHR spec governs either: our own
/// design/extension.
#[must_use]
pub fn visible(stored: &crate::storage::version_repo::read::StoredVersion) -> bool {
    use crate::system_log::access_context::{access_delay_enforced, access_delay_enforced_on};
    let delayed = access_delay_enforced() || stored.ehr_id.is_some_and(access_delay_enforced_on);
    admits_stored(stored) && (!stored.person_withheld || !delayed)
}

/// The episodes `head_episodes!` selected into `row`, empty for a head placed
/// in none; `None` when the statement did not select them.
///
/// # Errors
/// [`StorageError::Database`] when the column does not decode, and
/// [`StorageError::InvalidRows`] for a key that is no 32 octets.
pub fn episodes(row: &PgRow) -> Result<Option<Vec<EpisodeKey>>, StorageError> {
    // NOTE: our own design (#6416), a read selects the episodes only once a scope names one.
    let keys = match row.try_get::<Option<Vec<Vec<u8>>>, _>("access_episodes") {
        Ok(keys) => keys.unwrap_or_default(),
        Err(sqlx::Error::ColumnNotFound(_)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    keys.iter()
        .map(|octets| {
            EpisodeKey::from_octets(octets).ok_or_else(|| {
                StorageError::InvalidRows(
                    "vo_head.episodes holds a key of other than 32 octets".into(),
                )
            })
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

/// The statement `plain`, or `episodes`, its twin that selects the head's
/// episodes, when reads select them ([`crate::access_eval::episodes`]).
#[must_use]
pub fn with_episodes(plain: &'static str, episodes: &'static str) -> &'static str {
    if crate::access_eval::episodes::selected() {
        episodes
    } else {
        plain
    }
}

/// One read's statement in each form a request may run it in.
///
/// A ride adds the party store's revisions
/// (`party_binding::request::reads_door`, #6320), and a read that selects
/// episodes adds the head's ([`crate::access_eval::episodes`]).
#[derive(Debug, Clone, Copy)]
pub struct Statements {
    /// Neither.
    pub plain: &'static str,
    /// With the party store's revisions.
    pub ride: &'static str,
    /// With the head's episodes.
    pub episodes: &'static str,
    /// With both.
    pub ride_episodes: &'static str,
}

impl Statements {
    /// The form the running request runs.
    #[must_use]
    pub fn chosen(&self) -> &'static str {
        match (
            crate::party_binding::request::reads_door(),
            crate::access_eval::episodes::selected(),
        ) {
            (false, false) => self.plain,
            (true, false) => self.ride,
            (false, true) => self.episodes,
            (true, true) => self.ride_episodes,
        }
    }
}

/// The system ids a `version.origins` stamp names.
fn stamped(origins: &Value) -> Vec<String> {
    origins
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// The facts of each of `vo_ids` this store holds, in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn of_objects(
    conn: &mut sqlx::PgConnection,
    vo_ids: &[VoId],
) -> Result<Vec<HeadFacts>, StorageError> {
    macro_rules! objects_sql {
        ($($facts:ident)?) => {
            concat!(
                "SELECT h.vo_id, ",
                head_facts!($($facts)?),
                " FROM vo_head h WHERE h.vo_id = ANY($1) ORDER BY h.vo_id"
            )
        };
    }
    let ids: Vec<Uuid> = vo_ids.iter().map(|id| id.0).collect();
    let rows = sqlx::query(with_episodes(objects_sql!(), objects_sql!(episodes)))
        .bind(ids)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|row| HeadFacts::read(row, VoId(row.try_get("vo_id")?)))
        .collect()
}

/// The facts of every versioned object each of `contributions` committed a
/// version of, as `(contribution, facts)` in contribution and object order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn of_contributions(
    conn: &mut sqlx::PgConnection,
    contributions: &[Uuid],
) -> Result<Vec<(Uuid, HeadFacts)>, StorageError> {
    macro_rules! contributions_sql {
        ($($facts:ident)?) => {
            concat!(
                "SELECT c.contribution_id, h.vo_id, ",
                head_facts!($($facts)?),
                " FROM (SELECT DISTINCT v.contribution_id, v.vo_id FROM version v \
                   WHERE v.contribution_id = ANY($1)) c \
                 JOIN vo_head h ON h.vo_id = c.vo_id \
                 ORDER BY c.contribution_id, h.vo_id"
            )
        };
    }
    let rows = sqlx::query(with_episodes(
        contributions_sql!(),
        contributions_sql!(episodes),
    ))
    .bind(contributions)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("contribution_id")?,
                HeadFacts::read(row, VoId(row.try_get("vo_id")?))?,
            ))
        })
        .collect()
}
