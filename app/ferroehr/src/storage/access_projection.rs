// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the `EHR_ACCESS` projection: each EHR's settings with its
//! settings class and facts (`access_settings`), and the classes
//! (`access_class`, [`crate::access_eval::class`]).
//!
//! An EHR's row is replaced in the transaction that commits its `EHR_ACCESS`
//! version, and every change to it notifies every replica on [`CHANNEL`] at
//! commit (the trigger of migration 0040); a new class notifies [`NEW_CLASS`]
//! (migration 0042). No openEHR spec governs the projection or its storage
//! SQL: our own design.

use sqlx::{PgConnection, PgPool, Row};

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The channel a change to an EHR's settings row notifies, with the EHR id
/// as its payload, and a refill with [`EVERY_EHR`].
pub const CHANNEL: &str = "ferroehr_ehr_access";

/// The payload of a notification that drops every EHR's copy.
pub const EVERY_EHR: &str = "*";

/// The payload of a notification that a new settings class is stored.
pub const NEW_CLASS: &str = "class";

/// The `EHR_ACCESS` version an EHR's rows are projected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// The EHR.
    pub ehr_id: EhrId,
    /// Its `EHR_ACCESS` versioned object.
    pub vo_id: VoId,
    /// The storage ordinal of the version.
    pub sys_version: i32,
}

/// The settings row of one EHR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    /// The access default code.
    pub default_access: &'static str,
    /// The settings in canonical JSON text, their class and their facts;
    /// `None` holds the EHR closed.
    pub settings: Option<Classed>,
}

/// Settings as the projection stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classed {
    /// The settings in canonical JSON text.
    pub settings: String,
    /// The SHA-256 of the class's JSON text.
    pub digest: Vec<u8>,
    /// The class in canonical JSON text.
    pub class: String,
    /// The facts the class sets aside, as JSON array text.
    pub facts: String,
}

/// What one EHR's rows become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rows {
    /// The EHR's `EHR_ACCESS` carries no settings: it has no row.
    None,
    /// The settings row.
    Settings(SettingsRow),
}

/// Replaces the row of `source.ehr_id` with `rows`, on `tx`, storing its
/// class when no EHR made it before.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace(
    tx: &mut PgConnection,
    source: Source,
    rows: &Rows,
) -> Result<(), StorageError> {
    let Rows::Settings(row) = rows else {
        sqlx::query("DELETE FROM access_settings WHERE ehr_id = $1")
            .bind(source.ehr_id)
            .execute(&mut *tx)
            .await?;
        return Ok(());
    };
    let class_id = match &row.settings {
        Some(classed) => Some(class_id(tx, classed).await?),
        None => None,
    };
    let classed = row.settings.as_ref();
    sqlx::query(
        "INSERT INTO access_settings \
             (ehr_id, vo_id, sys_version, held_closed, default_access, settings, class_id, facts) \
         VALUES ($1, $2, $3, $4 IS NULL, $5, $4::jsonb, $6, $7::jsonb) \
         ON CONFLICT (ehr_id) DO UPDATE SET vo_id = EXCLUDED.vo_id, \
             sys_version = EXCLUDED.sys_version, held_closed = EXCLUDED.held_closed, \
             default_access = EXCLUDED.default_access, settings = EXCLUDED.settings, \
             class_id = EXCLUDED.class_id, facts = EXCLUDED.facts",
    )
    .bind(source.ehr_id)
    .bind(source.vo_id)
    .bind(source.sys_version)
    .bind(classed.map(|c| c.settings.as_str()))
    .bind(row.default_access)
    .bind(class_id)
    .bind(classed.map(|c| c.facts.as_str()))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The id of the class of `classed`, stored on `tx` when no EHR made it
/// before, in one statement: a class a concurrent commit stores first is
/// returned once that commit ends, the insert waiting for it (PostgreSQL 19,
/// `INSERT`, "ON CONFLICT DO SELECT ... allows an atomic INSERT or SELECT
/// outcome").
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
async fn class_id(tx: &mut PgConnection, classed: &Classed) -> Result<i64, StorageError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "INSERT INTO access_class (digest, settings) VALUES ($1, $2::jsonb) \
         ON CONFLICT (digest) DO SELECT RETURNING id",
    )
    .bind(&classed.digest)
    .bind(&classed.class)
    .fetch_one(&mut *tx)
    .await?)
}

/// One EHR's settings row as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StoredSettings {
    /// The EHR has no settings row.
    #[default]
    None,
    /// The EHR is held closed.
    HeldClosed,
    /// The settings in canonical JSON text.
    Settings(String),
}

/// An EHR's subject reference: the `namespace` and the `id.value` of its
/// `EHR_STATUS.subject.external_ref`.
///
/// A reference in this CDR's own demographic namespace names a `PERSON` of
/// the party store by its container; any other, such as an identity source's
/// pseudonym, names the `PERSON` that declares it
/// ([`crate::party_binding::grants::live::LiveGrants::person_of`]). Both are
/// legal subject references (BASE `OBJECT_REF.namespace`), so they compare
/// by namespace and id together.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EhrSubject {
    /// The `OBJECT_REF.namespace`.
    pub namespace: String,
    /// The `OBJECT_REF.id.value`.
    pub id: String,
}

impl crate::memory::Weigh for EhrSubject {
    fn heap(&self) -> usize {
        self.namespace.capacity().saturating_add(self.id.capacity())
    }
}

impl EhrSubject {
    /// The reference to `id` under `namespace`.
    #[must_use]
    pub fn new(namespace: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            id: id.into(),
        }
    }

    /// The reference the promoted `ehr.subject_namespace` and
    /// `ehr.subject_id` columns hold; `None` unless both are set.
    #[must_use]
    pub fn stored(namespace: Option<String>, id: Option<String>) -> Option<Self> {
        Some(Self::new(namespace?, id?))
    }
}

/// What a decision on one EHR reads that can change on another replica.
///
/// It is the storage ordinal of the `EHR_ACCESS` version the EHR's settings
/// row was projected from, and the subject reference its `EHR_STATUS` names,
/// whose `PERSON`'s care relationships the decision reads.
///
/// A read that leaves the store selects it beside its own columns
/// (`access_revision!`), so a replica compares the revision its copy was
/// judged on with the rows in force without a statement of its own. The
/// ordinal grows with every version of the object, so two equal revisions
/// name one version and one subject. No openEHR spec governs this: our own
/// design (#5589, #5634).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Revision {
    /// The ordinal; `None` when the EHR has no settings row.
    pub settings: Option<i32>,
    /// The subject reference; `None` when the status names none.
    pub subject: Option<EhrSubject>,
}

impl Revision {
    /// The revision a row selecting `access_revision!` holds: the JSON array
    /// `[<ordinal>, <namespace>, <id>]` from the clinical schema, `NULL` from
    /// the party schema.
    ///
    /// # Errors
    /// [`sqlx::Error`] when the column does not decode or does not read as a
    /// revision.
    pub fn read(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        observe_revisions(row)?;
        let Some(text) = row.try_get::<Option<String>, _>("access_revision")? else {
            return Ok(Self::default());
        };
        let (settings, namespace, id): (Option<i32>, Option<String>, Option<String>) =
            serde_json::from_str(&text).map_err(|error| sqlx::Error::ColumnDecode {
                index: "access_revision".to_owned(),
                source: Box::new(error),
            })?;
        Ok(Self {
            settings,
            subject: EhrSubject::stored(namespace, id),
        })
    }
}

/// Notes the policy and definition revisions a row selecting
/// `access_revision!` read beside the settings revision, for the running
/// request (`policy_register::observed`, #6251), or the hidden revision
/// columns of a judged AQL statement (#6320), or those a party statement read
/// through the clinical door, and in a ride the party store's revisions
/// (`party_binding::request`, #6320). A row of the party schema, or of a
/// statement that selects the view itself, holds none.
///
/// # Errors
/// [`sqlx::Error`] when a column does not decode.
pub(crate) fn observe_revisions(row: &sqlx::postgres::PgRow) -> Result<(), sqlx::Error> {
    let read = |column: &str| match row.try_get::<Option<i64>, _>(column) {
        Ok(value) => Ok(value),
        // NOTE: sqlx Error::ColumnNotFound, a statement that selects the view itself reads none.
        Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
        Err(error) => Err(error),
    };
    if let (Some(policy), Some(definitions)) =
        (read("policy_revision")?, read("definition_revision")?)
    {
        crate::policy_register::observed::observe(policy, definitions);
    }
    // Outside a party statement behind the clinical door no statement selects them.
    if crate::policy_register::observed::reads_clinical_door() {
        match row.try_get::<Option<(i64, i64)>, _>(POLICY_REVISIONS) {
            Ok(Some((policy, definitions))) => {
                crate::policy_register::observed::observe(policy, definitions);
            }
            // NOTE: sqlx Error::ColumnNotFound, only a door statement selects them.
            Ok(None) | Err(sqlx::Error::ColumnNotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    // Outside a ride no statement selects them, and a missing column's error is an allocation.
    let party = if crate::party_binding::request::riding() {
        match row.try_get::<Option<(i64, bool, i64)>, _>(PARTY_REVISIONS) {
            Ok(party) => party,
            // NOTE: sqlx Error::ColumnNotFound, only a ride's statement reads the party revisions.
            Err(sqlx::Error::ColumnNotFound(_)) => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    if let Some((grants, facts_behind, bindings)) = party {
        crate::party_binding::request::observe_door(grants, facts_behind, bindings);
    }
    Ok(())
}

/// The column a ride's statement reads the party store's grant and binding
/// revisions and its facts flag into ([`party_revisions!`], #6320).
const PARTY_REVISIONS: &str = "party_revisions";

/// The column a party statement reads the policy and definition revisions
/// into through the clinical door ([`policy_revisions!`], #6320).
const POLICY_REVISIONS: &str = "policy_revisions";

/// One EHR's settings row and subject as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    /// The settings.
    pub settings: StoredSettings,
    /// The uid of the `EHR_ACCESS` version the settings row was projected
    /// from; `None` when the EHR has no settings row.
    pub version: Option<String>,
    /// The revision of the settings row and the subject.
    pub revision: Revision,
}

/// One EHR's settings row and its subject, in one statement.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn settings_of(pool: &PgPool, ehr_id: EhrId) -> Result<Stored, StorageError> {
    let row = sqlx::query(
        "SELECT s.ehr_id IS NOT NULL AS has_settings, s.settings::text AS settings, \
                (SELECT ar.revision FROM access_revision_of ar WHERE ar.ehr_id = k.id) AS access_revision, s.vo_id, v.creating_system_id, v.trunk_version, \
                v.branch_number, v.branch_version \
         FROM (SELECT $1::uuid AS id) k \
         LEFT JOIN access_settings s ON s.ehr_id = k.id \
         LEFT JOIN version v ON v.vo_id = s.vo_id AND v.sys_version = s.sys_version",
    )
    .bind(ehr_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    Ok(stored(&row)?)
}

/// The settings row and subject one row of [`settings_of`] or
/// [`settings_of_many`] reads.
///
/// # Errors
/// [`sqlx::Error`] when a column does not decode.
fn stored(row: &sqlx::postgres::PgRow) -> Result<Stored, sqlx::Error> {
    let has_settings: bool = row.try_get("has_settings")?;
    let settings = match row.try_get::<Option<String>, _>("settings")? {
        Some(settings) => StoredSettings::Settings(settings),
        None if has_settings => StoredSettings::HeldClosed,
        None => StoredSettings::None,
    };
    let version = version_of(row)?;
    Ok(Stored {
        settings,
        version,
        revision: Revision::read(row)?,
    })
}

/// The uid of the version a row names by its `vo_id`, `creating_system_id`,
/// `trunk_version`, `branch_number` and `branch_version` columns; `None` when
/// the row joined no version.
///
/// # Errors
/// [`sqlx::Error`] when a column does not decode.
pub(crate) fn version_of(row: &sqlx::postgres::PgRow) -> Result<Option<String>, sqlx::Error> {
    match (
        row.try_get::<Option<VoId>, _>("vo_id")?,
        row.try_get::<Option<String>, _>("creating_system_id")?,
    ) {
        (Some(vo_id), Some(system_id)) => Ok(Some(crate::storage::compact::version_uid(
            vo_id,
            &system_id,
            row.try_get("trunk_version")?,
            row.try_get("branch_number")?,
            row.try_get("branch_version")?,
        ))),
        _ => Ok(None),
    }
}

/// The settings rows of `ehr_ids` and their subjects, in one statement: an
/// id with no EHR answers [`Stored::default`].
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn settings_of_many(
    pool: &PgPool,
    ehr_ids: &[EhrId],
) -> Result<Vec<(EhrId, Stored)>, StorageError> {
    let ids: Vec<uuid::Uuid> = ehr_ids.iter().map(|id| id.0).collect();
    let rows = sqlx::query(
        "SELECT k.id AS ehr_id, s.ehr_id IS NOT NULL AS has_settings, \
                s.settings::text AS settings, (SELECT ar.revision FROM access_revision_of ar WHERE ar.ehr_id = k.id) AS access_revision, s.vo_id, \
                v.creating_system_id, v.trunk_version, \
                v.branch_number, v.branch_version \
         FROM unnest($1::uuid[]) AS k(id) \
         LEFT JOIN access_settings s ON s.ehr_id = k.id \
         LEFT JOIN version v ON v.vo_id = s.vo_id AND v.sys_version = s.sys_version",
    )
    .bind(&ids)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("ehr_id")?, stored(row)?)))
        .collect()
}

/// Every settings class, by id, as its JSON text.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn every_class(pool: &PgPool) -> Result<Vec<(i64, String)>, StorageError> {
    let rows = sqlx::query("SELECT id, settings::text AS settings FROM access_class ORDER BY id")
        .fetch_all(&mut *acquire(pool).await?)
        .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("id")?, row.try_get("settings")?)))
        .collect()
}

/// Takes the lock a refill holds: a commit's write of the rows waits for the
/// refill, and the refill for every commit that has written them.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lock(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "LOCK TABLE access_settings, access_class, access_projection_state \
         IN SHARE ROW EXCLUSIVE MODE",
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether the rows were filled from the stored versions, read under
/// [`lock`].
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn filled(tx: &mut PgConnection) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT filled_at IS NOT NULL FROM access_projection_state WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?)
}

/// Deletes every row and every class, before a refill writes them again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn clear(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM access_settings")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM access_class")
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Records that the rows were filled, and notifies every replica to drop its
/// copy of every EHR.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn mark_filled(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "WITH filled AS (UPDATE access_projection_state SET filled_at = now() WHERE singleton) \
         SELECT pg_notify($1, $2)",
    )
    .bind(CHANNEL)
    .bind(EVERY_EHR)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
