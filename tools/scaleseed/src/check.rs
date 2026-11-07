// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The check: a sample of seeded compositions against the prototypes the
//! commit path wrote, in the stored form and as the service serves them.
//!
//! For each sampled clone it holds three things. The stored rows are the
//! prototype's, column for column, but for the identifiers a clone replaces
//! and the signature. The version body is the prototype's with the versioned
//! object's id replaced. And the service reads every version of the clone as
//! it reads the prototype's, its signature verified on read
//! (`verify_on_read = strict`, the default), and serves the same envelope and
//! the same composition with the identifiers replaced.

use anyhow::{Context as _, anyhow};
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::response::ReadBody;

/// The `node` columns a clone keeps, the identifiers left out.
const NODE_KEPT: &str = "tier, sys_version, num, num_cap, parent_num, rm_type, archetype, \
    arch_entity, arch_concept, arch_major, name, name_code, name_terminology, path, data, \
    context_start, value_type, value_num, value_text, value_code, value_terminology, rm_version, \
    language, encoding, origin_compact, time_compact, party_self, uid_derived";

/// The `version` columns a clone keeps, the identifiers, the signature and
/// the body left out.
const VERSION_KEPT: &str = "tier, kind, sys_version, trunk_version, branch_number, \
    branch_version, lifecycle_state, creating_system_id, signature_client_supplied, \
    wrapped_original, other_input_version_uids, origins, template_id, stable_compatible, \
    committed_at, content_digest, content_sys_version, content_key";

/// The `vo_head` columns a clone keeps, the identifiers left out.
const HEAD_KEPT: &str = "kind, tier, head_sys_version, trunk_head_sys_version, lifecycle_state, \
    template_id, committed_at, restricted_at, retention_hold_at, archived_at, archive_reason, \
    access_restricted_at, person_withheld_until, trunk_head_content_key";

/// Checks up to `limit` sampled clones; returns how many it checked.
///
/// # Errors
/// The first clone whose rows or served form differ from its prototype's,
/// named with the difference.
pub(crate) async fn run(svc: &FerroEhrService, pool: &PgPool, limit: i64) -> anyhow::Result<u64> {
    let proto_ehr: Uuid = sqlx::query_scalar("SELECT ehr_id FROM scaleseed.proto_ehr")
        .fetch_one(pool)
        .await?;
    let sample: Vec<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("SELECT vo_id, proto, ehr_id FROM scaleseed.sample ORDER BY vo_id LIMIT $1")
            .bind(limit)
            .fetch_all(pool)
            .await?;
    if sample.is_empty() {
        return Err(anyhow!("no sampled clone: run `scaleseed seed` first"));
    }
    let mut checked = 0_u64;
    for (vo, proto, ehr) in sample {
        rows(pool, vo, proto)
            .await
            .with_context(|| format!("clone {vo} of {proto}"))?;
        served(svc, pool, (vo, ehr), (proto, proto_ehr))
            .await
            .with_context(|| format!("clone {vo} of {proto}"))?;
        checked += 1;
    }
    Ok(checked)
}

/// The stored rows of clone `vo` against those of `proto`.
async fn rows(pool: &PgPool, vo: Uuid, proto: Uuid) -> anyhow::Result<()> {
    for (table, kept) in [
        ("node", NODE_KEPT),
        ("version", VERSION_KEPT),
        ("vo_head", HEAD_KEPT),
    ] {
        // NOTE: the column lists are this file's constants and the table names
        // literals of the loop, never input.
        let differ: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM ( \
               (SELECT {kept} FROM clinical.{table} WHERE vo_id = $1 \
                EXCEPT ALL SELECT {kept} FROM clinical.{table} WHERE vo_id = $2) \
               UNION ALL \
               (SELECT {kept} FROM clinical.{table} WHERE vo_id = $2 \
                EXCEPT ALL SELECT {kept} FROM clinical.{table} WHERE vo_id = $1)) d"
        )))
        .bind(vo)
        .bind(proto)
        .fetch_one(pool)
        .await?;
        if differ != 0 {
            return Err(anyhow!("{differ} {table} rows differ from the prototype's"));
        }
    }
    let bodies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM clinical.version c JOIN clinical.version p \
           ON p.vo_id = $2 AND p.sys_version = c.sys_version \
         WHERE c.vo_id = $1 AND c.body IS DISTINCT FROM replace(p.body, $2::text, $1::text)",
    )
    .bind(vo)
    .bind(proto)
    .fetch_one(pool)
    .await?;
    if bodies != 0 {
        return Err(anyhow!(
            "{bodies} version bodies differ from the prototype's"
        ));
    }
    Ok(())
}

/// Every version of the clone `(vo, ehr)` and its latest composition, as the
/// service serves them, against the prototype's `(proto, proto_ehr)`.
#[expect(
    clippy::disallowed_types,
    reason = "a served envelope is compared as the canonical JSON the service serves (#1694)"
)]
async fn served(
    svc: &FerroEhrService,
    pool: &PgPool,
    (vo, ehr): (Uuid, Uuid),
    (proto, proto_ehr): (Uuid, Uuid),
) -> anyhow::Result<()> {
    let versions: Vec<(String, i32, i32, i32)> = sqlx::query_as(
        "SELECT creating_system_id, trunk_version, branch_number, branch_version \
         FROM clinical.version WHERE vo_id = $1 ORDER BY sys_version",
    )
    .bind(vo)
    .fetch_all(pool)
    .await?;
    let contributions: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.contribution_id::text, c.contribution_id::text FROM clinical.version c \
         JOIN clinical.version p ON p.vo_id = $2 AND p.sys_version = c.sys_version \
         WHERE c.vo_id = $1",
    )
    .bind(vo)
    .bind(proto)
    .fetch_all(pool)
    .await?;
    let swap = |text: String| {
        contributions.iter().fold(
            text.replace(&proto.to_string(), &vo.to_string())
                .replace(&proto_ehr.to_string(), &ehr.to_string()),
            |acc, (from, to)| acc.replace(from.as_str(), to),
        )
    };
    for (system, trunk, branch, branch_version) in versions {
        let tree = if branch == 0 {
            trunk.to_string()
        } else {
            format!("{trunk}.{branch}.{branch_version}")
        };
        let clone_uid = format!("{vo}::{system}::{tree}");
        let proto_uid = format!("{proto}::{system}::{tree}");
        let mut clone = svc
            .composition_version_envelope(
                EhrId(ehr),
                clone_uid.parse().map_err(|e| anyhow!("{e:?}"))?,
            )
            .await
            .with_context(|| format!("read {clone_uid}"))?
            .body;
        let expected = svc
            .composition_version_envelope(
                EhrId(proto_ehr),
                proto_uid.parse().map_err(|e| anyhow!("{e:?}"))?,
            )
            .await
            .with_context(|| format!("read {proto_uid}"))?
            .body;
        // NOTE: the signature is the one value a clone recomputes; the read
        // above verified it (`verify_on_read = strict`), so the rest compares.
        let mut expected = swap(expected.to_string()).parse::<serde_json::Value>()?;
        for envelope in [&mut clone, &mut expected] {
            if let Some(object) = envelope.as_object_mut() {
                object.remove("signature");
            }
        }
        if clone != expected {
            return Err(anyhow!("the served envelope of {clone_uid} differs"));
        }
    }
    let latest = |body: ReadBody| match body {
        ReadBody::Value(value) => value.to_string(),
        ReadBody::RawJson(text) => text,
    };
    let clone = latest(
        svc.read_composition_raw(EhrId(ehr), VoId(vo), None)
            .await?
            .body,
    );
    let expected = latest(
        svc.read_composition_raw(EhrId(proto_ehr), VoId(proto), None)
            .await?
            .body,
    );
    if clone != swap(expected) {
        return Err(anyhow!("the served composition differs"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "test assertions, diagnostics and fixtures"
    )]

    use super::run;
    use crate::seed::Target;

    /// The prototypes, a small seed and the check over every sampled clone;
    /// then a clone whose node row or signature no longer matches is refused.
    #[tokio::test]
    async fn the_check_passes_the_seed_and_refuses_a_changed_clone() {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        let svc = ferroehr::service::FerroEhrService::new(
            &ferroehr::db::domain::DomainPools::from_shared(&pool),
        );
        crate::proto::commit(&svc, &pool, 14)
            .await
            .expect("prototypes");
        let target = Target {
            ehrs: 6,
            compositions: 30,
            chunk: 2,
            jobs: 2,
        };
        crate::seed::run(svc.clone(), pool.clone(), target)
            .await
            .expect("seed");
        assert!(
            run(&svc, &pool, 100).await.expect("check") >= 3,
            "one sample per chunk at least"
        );

        let (vo,): (uuid::Uuid,) = sqlx::query_as("SELECT vo_id FROM scaleseed.sample LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("a sample");
        sqlx::query(
            "UPDATE clinical.node SET name = name || ' changed' \
             WHERE vo_id = $1 AND num = (SELECT max(num) FROM clinical.node WHERE vo_id = $1)",
        )
        .bind(vo)
        .execute(&pool)
        .await
        .expect("change a node");
        let refused = run(&svc, &pool, 100)
            .await
            .expect_err("a changed node row is refused");
        assert!(
            format!("{refused:#}").contains("node rows differ"),
            "{refused:#}"
        );

        sqlx::query(
            "UPDATE clinical.node SET name = left(name, length(name) - 8) \
             WHERE vo_id = $1 AND name LIKE '% changed'",
        )
        .bind(vo)
        .execute(&pool)
        .await
        .expect("restore the node");
        sqlx::query("UPDATE clinical.version SET signature = 'sha256:AAAA' WHERE vo_id = $1")
            .bind(vo)
            .execute(&pool)
            .await
            .expect("break the signature");
        let refused = run(&svc, &pool, 100)
            .await
            .expect_err("a broken signature is refused");
        assert!(format!("{refused:#}").contains("read "), "{refused:#}");
    }
}
