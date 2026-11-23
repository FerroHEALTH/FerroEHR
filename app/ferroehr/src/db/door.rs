// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two doors between the clinical and party domains (#6320).
//!
//! Where the two domains share a database, a statement of one confirms the
//! copies its decision rests on by reading the other's revisions: a clinical
//! statement the party store's through `party_door.party_revisions()`, a party
//! statement the policy and definition revisions through
//! `clinical_door.policy_revisions()`. Each door is a schema the other
//! domain's roles may enter, so the boot gate holds it to its one function,
//! running as its owner with a fixed `search_path`, and nothing else. No
//! openEHR spec governs database roles: our own design.

use sqlx::PgPool;

use super::DbError;
use super::acquire::acquire;

/// The schema the party set creates beside its own: the one function the
/// clinical roles may run (`party/0009`).
pub(super) const PARTY_DOOR: &str = "party_door";

/// The schema the clinical set creates beside its own: the one function the
/// party roles may run (`clinical/0009`).
pub(super) const CLINICAL_DOOR: &str = "clinical_door";

/// The first object of the door schema `$1` that is not its one function
/// `$2()`, running as its owner with a fixed `search_path`, and what is
/// wrong with it.
const BREACH_SQL: &str = "SELECT object, problem FROM (
         SELECT $1 || '.' || c.relname AS object, 'is a relation' AS problem
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1
         UNION ALL
         SELECT $1 || '.' || p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')',
                CASE
                    WHEN p.proname <> $2 OR p.pronargs <> 0
                        THEN 'is not the door''s function'
                    WHEN NOT p.prosecdef THEN 'does not run as its owner'
                    WHEN NOT EXISTS (
                        SELECT FROM unnest(p.proconfig) AS c(setting)
                        WHERE c.setting LIKE 'search_path=%'
                    ) THEN 'runs on the caller''s search_path'
                END
         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = $1
         UNION ALL
         SELECT $1 || '.' || t.typname, 'is a type'
         FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
         LEFT JOIN pg_type e ON e.oid = t.typelem
         WHERE n.nspname = $1 AND t.typrelid = 0 AND COALESCE(e.typrelid, 0) = 0
     ) breaches
     WHERE problem IS NOT NULL
     ORDER BY object
     LIMIT 1";

/// The exceptions to the grant sweep: the schema `party_door`, which the
/// clinical roles may enter, holds exactly the function `party_revisions()`,
/// and the schema `clinical_door`, which the party roles may enter, exactly
/// `policy_revisions()`, each running as its owner with a fixed
/// `search_path`, and nothing else. A database without a door passes.
///
/// # Errors
/// [`DbError::PartyDoorBreached`] or [`DbError::ClinicalDoorBreached`] naming
/// the object, or [`DbError::Sqlx`] when a catalog read fails.
pub(super) async fn verify_doors(pool: &PgPool) -> Result<(), DbError> {
    if let Some((object, problem)) = breach(pool, PARTY_DOOR, "party_revisions").await? {
        return Err(DbError::PartyDoorBreached { object, problem });
    }
    if let Some((object, problem)) = breach(pool, CLINICAL_DOOR, "policy_revisions").await? {
        return Err(DbError::ClinicalDoorBreached { object, problem });
    }
    Ok(())
}

/// The first object of the door `schema` that is not its one `function`,
/// and what is wrong with it.
async fn breach(
    pool: &PgPool,
    schema: &str,
    function: &str,
) -> Result<Option<(String, &'static str)>, DbError> {
    let breach: Option<(String, String)> = sqlx::query_as(BREACH_SQL)
        .bind(schema)
        .bind(function)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    Ok(breach.map(|(object, problem)| {
        let problem = match problem.as_str() {
            "is a relation" => "is a relation",
            "is a type" => "is a type",
            "does not run as its owner" => "does not run as its owner",
            "runs on the caller's search_path" => "runs on the caller's search_path",
            _ => "is not the door's function",
        };
        (object, problem)
    }))
}
