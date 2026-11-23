// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot refusal of a database that carries a retired migration set: the
//! first storage generation, or the v5 history the v6.0.0 baseline replaced.
//!
//! No openEHR spec governs migration bookkeeping: our own design.

use sqlx::PgConnection;

use crate::db::DbError;

/// The release line a retired migration set belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetiredRelease {
    /// The first storage generation, which the storage rewrite replaced.
    FirstGeneration,
    /// The v5 sets, which the v6.0.0 baseline replaced.
    V5,
}

/// One retired migration set: the schema it ran in, and how to recognise its
/// bookkeeping.
#[derive(Debug, Clone, Copy)]
struct RetiredSet {
    /// The schema the retired set ran in.
    schema: &'static str,
    /// The release line the set belongs to, which picks the refusal.
    release: RetiredRelease,
    /// The description sqlx recorded for that set's version 1, when a later
    /// set reused the schema name; `None` when this build owns no set of that
    /// name and any bookkeeping in it is the signature.
    ///
    /// sqlx derives the description from the file name after the version
    /// number, with underscores replaced by spaces, so `0001_baseline.sql`
    /// records `baseline`.
    first_description: Option<&'static str>,
}

/// The retired migration sets, which this build does not migrate and cannot
/// read: a database that carries one is refused at boot with the way forward
/// rather than half-adopted.
///
/// Neither retired set is upgraded in place (owner decision 2026-11-20, #6139).
///
/// Where a schema NAME survives (`ext`, `audit`, `clinical`, `party`), the
/// signature cannot be the bookkeeping's existence; it is which migration ran
/// FIRST. Version 1 is the one row a set can never lack, and its description
/// names the file. The first generation opened `ext` with
/// `0001_openehr_functions.sql` and `audit` with `0001_baseline.sql`; v5 opened
/// `ext` and `audit` with `0001_schema_and_roles.sql` and `clinical` and `party`
/// with `0001_schema_and_grants_baseline.sql`; the v6 baseline opens them with
/// files of other names. For `ehr`, `demographic` and `linkage`, which this
/// build owns no set for, any bookkeeping at all is the signature: a bare
/// schema of the same name is not, because `CREATE SCHEMA` is cheap and someone
/// may have made one, while the bookkeeping table is written only by a migrator
/// that ran there. The first generation's `linkage` is listed first, so its
/// signature picks its own refusal.
///
/// Without this, a retired set in a surviving schema would reach this build's
/// migrator and fail on a checksum mismatch: an error about a hash where the
/// operator needs the remedy.
const RETIRED_SETS: &[RetiredSet] = &[
    RetiredSet {
        schema: "ehr",
        release: RetiredRelease::FirstGeneration,
        first_description: None,
    },
    RetiredSet {
        schema: "demographic",
        release: RetiredRelease::FirstGeneration,
        first_description: None,
    },
    RetiredSet {
        schema: "ext",
        release: RetiredRelease::FirstGeneration,
        first_description: Some("openehr functions"),
    },
    RetiredSet {
        schema: "linkage",
        release: RetiredRelease::FirstGeneration,
        first_description: Some("baseline"),
    },
    RetiredSet {
        schema: "audit",
        release: RetiredRelease::FirstGeneration,
        first_description: Some("baseline"),
    },
    RetiredSet {
        schema: "ext",
        release: RetiredRelease::V5,
        first_description: Some("schema and roles"),
    },
    RetiredSet {
        schema: "clinical",
        release: RetiredRelease::V5,
        first_description: Some("schema and grants baseline"),
    },
    RetiredSet {
        schema: "party",
        release: RetiredRelease::V5,
        first_description: Some("schema and grants baseline"),
    },
    RetiredSet {
        schema: "audit",
        release: RetiredRelease::V5,
        first_description: Some("schema and roles"),
    },
    RetiredSet {
        schema: "linkage",
        release: RetiredRelease::V5,
        first_description: None,
    },
];

/// Refuse a database that carries a retired migration set.
///
/// Left to itself the sequence would create this build's schemas beside the old
/// ones, or apply its baseline over a v5 schema, and serve an empty or broken
/// repository with the operator's data unreachable in the same database.
///
/// The signature is [`RETIRED_SETS`]: bookkeeping in a schema this build owns
/// no set for, or bookkeeping whose version 1 names the file a retired set
/// opened that schema with.
///
/// `to_regclass` answers `NULL` for a missing relation instead of failing
/// (<https://www.postgresql.org/docs/18/functions-info.html>), so the presence
/// probe is one statement over a fresh database and an old one alike.
pub(super) async fn guard(conn: &mut PgConnection) -> Result<(), DbError> {
    for set in RETIRED_SETS {
        let present: bool =
            sqlx::query_scalar("SELECT to_regclass($1 || '._sqlx_migrations') IS NOT NULL")
                .bind(set.schema)
                .fetch_one(&mut *conn)
                .await?;
        if !present {
            continue;
        }
        let retired = match set.first_description {
            // This build owns no set of that name, so the bookkeeping can only
            // be a retired set's.
            None => true,
            Some(signature) => {
                // The schema name survives, so which migration ran FIRST is
                // what separates the sets. `set.schema` is one of the literals
                // in `RETIRED_SETS` above, never input.
                let sql = format!(
                    "SELECT description FROM {}._sqlx_migrations WHERE version = 1",
                    set.schema
                );
                let first: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                    .fetch_optional(&mut *conn)
                    .await?;
                first.as_deref() == Some(signature)
            }
        };
        if retired {
            return Err(match set.release {
                RetiredRelease::FirstGeneration => {
                    DbError::FirstGenerationDatabase { schema: set.schema }
                }
                RetiredRelease::V5 => DbError::V5Database { schema: set.schema },
            });
        }
    }
    Ok(())
}
