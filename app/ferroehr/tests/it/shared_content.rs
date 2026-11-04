// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A version whose content equals its predecessor's stores that content
//! once (#4415), against a real `PostgreSQL` 18 (shared testkit harness):
//! the commit is recorded in full, the node rows are written once, and every
//! reader answers each version with its own uid, whether it holds its rows or
//! reads its predecessor's.
//!
//! The proof commits a chain of identical and differing versions, reads
//! every version by uid, through the node rows and through AQL, copies the
//! shared rows back under each sharer's own key and reads again (the same
//! bytes), and holds the tree across a delete, an archive, a restore and the
//! node rebuild. The opt-in re-send rule is proved to commit nothing, to name
//! the existing version, to leave one `resend` record on the access log and
//! to match the current version only. No openEHR spec governs the physical
//! sharing of stored content: our own design.

use sqlx::PgPool;

use ferroehr::config::versioning::RESEND_OPERATION;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::storage::integrity::StorageParityScope;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender;

use crate::fixtures::{create_ehr, observation_composition, uv};
use crate::node_readers::aql_answers;

/// The AQL statements whose answers must not move when a version shares its
/// predecessor's rows: every version with its own uid, the whole object and
/// the content leaves, over the latest version and over all of them.
const QUERIES: [&str; 5] = [
    "SELECT v/uid/value, c/uid/value, c/name/value, c/archetype_node_id, c \
     FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
    "SELECT c/uid/value, o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude, o \
     FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    "SELECT c/uid/value, c/name/value, c FROM EHR e CONTAINS COMPOSITION c",
    "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
     WHERE o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude > 1",
    "SELECT c/uid/value FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c \
     WHERE c/name/value = 'same' ORDER BY c/uid/value",
];

/// The chain the tests read: v1 `same`, v2 `same` again (shares v1's rows),
/// v3 `other` (its own rows), v4 `other` again (shares v3's rows).
struct Chain {
    ehr_id: EhrId,
    vo_id: VoId,
    /// The four version uids, in commit order.
    uids: Vec<String>,
}

async fn commit_chain(svc: &FerroEhrService) -> Chain {
    let ehr_id = create_ehr(svc).await;
    let v1 = svc
        .create_composition(
            ehr_id,
            uv(&observation_composition("same", 120.0), "249", None),
        )
        .await
        .expect("v1 commits")
        .version_uid();
    let vo_id: VoId = v1
        .split("::")
        .next()
        .expect("object id")
        .parse()
        .expect("uuid");
    let mut uids = vec![v1];
    for body in [
        observation_composition("same", 120.0),
        observation_composition("other", 130.0),
        observation_composition("other", 130.0),
    ] {
        let preceding = uids.last().expect("a preceding version").clone();
        let committed = svc
            .update_composition(ehr_id, vo_id, uv(&body, "251", Some(&preceding)))
            .await
            .expect("the update commits");
        assert!(!committed.skipped, "the default records every commit");
        uids.push(committed.version_uid());
    }
    Chain {
        ehr_id,
        vo_id,
        uids,
    }
}

/// `(sys_version, node rows, content_sys_version, content_digest)` per
/// version of `vo_id`, in order.
async fn stored_versions(
    pool: &PgPool,
    vo_id: VoId,
) -> Vec<(i32, i64, Option<i32>, Option<Vec<u8>>)> {
    sqlx::query_as(
        "SELECT v.sys_version, \
                (SELECT count(*) FROM node n WHERE n.vo_id = v.vo_id AND n.sys_version = v.sys_version), \
                v.content_sys_version, v.content_digest \
           FROM version v WHERE v.vo_id = $1 ORDER BY v.sys_version",
    )
    .bind(vo_id)
    .fetch_all(pool)
    .await
    .expect("read the stored versions")
}

/// The head's content key of `vo_id` (`vo_head.trunk_head_content_key`,
/// #4944), after checking that every head row of the store names its trunk
/// head's `content_key`.
async fn head_content_key(pool: &PgPool, vo_id: VoId) -> Option<i32> {
    let mismatched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM vo_head h \
         LEFT JOIN version v ON v.vo_id = h.vo_id AND v.sys_version = h.trunk_head_sys_version \
         WHERE h.trunk_head_content_key IS DISTINCT FROM v.content_key",
    )
    .fetch_one(pool)
    .await
    .expect("compare every head with its trunk head");
    assert_eq!(
        mismatched, 0,
        "every head names the rows its trunk head reads"
    );
    sqlx::query_scalar("SELECT trunk_head_content_key FROM vo_head WHERE vo_id = $1")
        .bind(vo_id)
        .fetch_one(pool)
        .await
        .expect("read the head's content key")
}

/// Every version's body read by uid, its node rows reassembled, and the AQL
/// answers, serialized.
#[derive(Debug, PartialEq, Eq)]
struct Served {
    bodies: Vec<String>,
    nodes: Vec<String>,
    aql: Vec<Vec<String>>,
}

async fn served(pool: &PgPool, svc: &FerroEhrService, chain: &Chain) -> Served {
    let mut bodies = Vec::new();
    let mut nodes = Vec::new();
    for (i, uid) in chain.uids.iter().enumerate() {
        let body = svc
            .composition_at_version(chain.ehr_id, uid.parse().expect("OBJECT_VERSION_ID"))
            .await
            .expect("the version reads")
            .body
            .into_value()
            .expect("the stored COMPOSITION decodes");
        assert_eq!(
            body["uid"]["value"],
            uid.as_str(),
            "the body of version {}",
            i + 1
        );
        bodies.push(serde_json::to_string(&body).expect("serializable"));
        let sys_version = i32::try_from(i + 1).expect("a small ordinal");
        let mut conn = pool.acquire().await.expect("a connection");
        let rows = ferroehr::storage::node_repo::read_version_canonical(
            &mut conn,
            chain.vo_id,
            sys_version,
        )
        .await
        .expect("the node rows reassemble");
        assert_eq!(
            rows,
            body,
            "the node rows of version {} are its body",
            i + 1
        );
        nodes.push(serde_json::to_string(&rows).expect("serializable"));
    }
    let aql = aql_answers(svc, &QUERIES).await;
    Served { bodies, nodes, aql }
}

#[tokio::test]
async fn identical_content_is_stored_once_and_every_version_reads_back_whole() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let chain = commit_chain(&svc).await;

    let stored = stored_versions(&pool, chain.vo_id).await;
    assert_eq!(stored.len(), 4);
    let own = stored[0].1;
    assert!(
        own > 1,
        "the composition decomposes into more than its root"
    );
    assert_eq!(stored[0].1, own);
    assert_eq!(stored[1].1, 0, "v2 writes no rows of its own");
    assert_eq!(stored[2].1, own, "v3 has its own rows");
    assert_eq!(stored[3].1, 0, "v4 writes no rows of its own");
    assert_eq!(stored[0].2, None);
    assert_eq!(stored[1].2, Some(1), "v2 reads v1's rows");
    assert_eq!(stored[2].2, None);
    assert_eq!(stored[3].2, Some(3), "v4 reads v3's rows");
    assert_eq!(
        head_content_key(&pool, chain.vo_id).await,
        Some(3),
        "the head names the rows its trunk head v4 reads"
    );
    assert!(
        stored
            .iter()
            .all(|v| v.3.as_ref().is_some_and(|d| d.len() == 32))
    );
    assert_eq!(stored[0].3, stored[1].3);
    assert_eq!(stored[2].3, stored[3].3);
    assert_ne!(stored[0].3, stored[2].3);

    // Every commit is recorded: its own audit and contribution.
    let (audits, contributions): (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT v.commit_audit_id), count(DISTINCT v.contribution_id) \
           FROM version v WHERE v.vo_id = $1",
    )
    .bind(chain.vo_id)
    .fetch_one(&pool)
    .await
    .expect("count the audits");
    assert_eq!((audits, contributions), (4, 4));

    let now = served(&pool, &svc, &chain).await;
    let all_versions: Vec<&String> = now.aql.first().expect("the first query").iter().collect();
    assert_eq!(
        all_versions.len(),
        4,
        "one row per version: {all_versions:?}"
    );
    for (i, uid) in chain.uids.iter().enumerate() {
        assert!(
            all_versions.iter().any(|row| row.contains(uid.as_str())),
            "version {} ({uid}) answers under its own uid",
            i + 1
        );
    }
    let latest = &now.aql[2];
    assert_eq!(latest.len(), 1);
    assert!(latest[0].contains(chain.uids[3].as_str()), "{latest:?}");
    assert_eq!(
        now.aql[3].len(),
        1,
        "the latest version's content answers the filter"
    );
    assert_eq!(now.aql[4].len(), 2, "two versions are named `same`");

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(report.is_clean(), "{report:?}");
}

#[tokio::test]
async fn a_shared_tree_answers_the_same_as_its_own_copy() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let chain = commit_chain(&svc).await;
    let now = served(&pool, &svc, &chain).await;

    // The shape the store wrote before: every version with its own rows.
    let columns: String = sqlx::query_scalar(
        "SELECT string_agg(CASE WHEN column_name = 'sys_version' THEN '$2' \
                                ELSE quote_ident(column_name) END, ', ' ORDER BY ordinal_position) \
           FROM information_schema.columns \
          WHERE table_schema = 'clinical' AND table_name = 'node'",
    )
    .fetch_one(&pool)
    .await
    .expect("the node columns");
    for (sharer, owner) in [(2, 1), (4, 3)] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO node SELECT {columns} FROM node WHERE vo_id = $1 AND sys_version = $3"
        )))
        .bind(chain.vo_id)
        .bind(sharer)
        .bind(owner)
        .execute(&pool)
        .await
        .expect("copy the rows under the sharer's key");
        sqlx::query(
            "UPDATE version SET content_sys_version = NULL WHERE vo_id = $1 AND sys_version = $2",
        )
        .bind(chain.vo_id)
        .bind(sharer)
        .execute(&pool)
        .await
        .expect("the version reads its own rows");
    }
    let stored = stored_versions(&pool, chain.vo_id).await;
    assert!(
        stored.iter().all(|v| v.1 > 1 && v.2.is_none()),
        "{stored:?}"
    );

    let before = served(&pool, &svc, &chain).await;
    assert_eq!(now.bodies, before.bodies, "the bodies");
    assert_eq!(now.nodes, before.nodes, "the node rows");
    for ((query, now), before) in QUERIES.iter().zip(&now.aql).zip(&before.aql) {
        assert_eq!(now, before, "{query}");
    }
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(report.is_clean(), "{report:?}");
}

#[tokio::test]
async fn a_delete_an_archive_and_a_restore_keep_the_shared_tree() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let chain = commit_chain(&svc).await;
    let before = served(&pool, &svc, &chain).await;

    let deleted = svc
        .delete_composition(
            chain.ehr_id,
            &chain.uids[3].parse().expect("OBJECT_VERSION_ID"),
            crate::fixtures::deleting(Some(chain.uids[3].parse().expect("OBJECT_VERSION_ID"))),
        )
        .await
        .expect("the delete commits");
    assert_eq!(deleted.sys_version, 5);
    let stored = stored_versions(&pool, chain.vo_id).await;
    assert_eq!(stored.len(), 5);
    assert_eq!(
        (stored[4].1, stored[4].2, stored[4].3.as_ref()),
        (0, None, None)
    );
    assert_eq!(
        head_content_key(&pool, chain.vo_id).await,
        Some(5),
        "a logical delete is its own content: no rows"
    );

    svc.archive_ehrs(&[chain.ehr_id]).await.expect("archive");
    svc.restore_archived_ehrs(&[chain.ehr_id])
        .await
        .expect("restore");
    assert_eq!(
        head_content_key(&pool, chain.vo_id).await,
        Some(5),
        "the archive and the restore keep the head's content key"
    );
    let mut conn = pool.acquire().await.expect("a connection");
    for (i, body) in before.nodes.iter().enumerate() {
        let sys_version = i32::try_from(i + 1).expect("a small ordinal");
        let rows = ferroehr::storage::node_repo::read_version_canonical(
            &mut conn,
            chain.vo_id,
            sys_version,
        )
        .await
        .expect("the node rows reassemble");
        assert_eq!(
            serde_json::to_string(&rows).expect("serializable"),
            *body,
            "version {} after the archive and restore",
            i + 1
        );
    }
    drop(conn);
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(report.is_clean(), "{report:?}");

    svc.admin_ehr_delete(chain.ehr_id)
        .await
        .expect("erase the EHR");
    let (versions, nodes): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM version WHERE vo_id = $1), \
                (SELECT count(*) FROM node WHERE vo_id = $1)",
    )
    .bind(chain.vo_id)
    .fetch_one(&pool)
    .await
    .expect("count what is left");
    assert_eq!(
        (versions, nodes),
        (0, 0),
        "an erasure takes the object whole"
    );
}

#[tokio::test]
async fn a_rebuilt_sharing_version_reads_its_own_rows_again() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let chain = commit_chain(&svc).await;
    let before = served(&pool, &svc, &chain).await;

    // Damage the tree v1 and v2 read, so the sweep finds both and the rebuild
    // writes each its own rows.
    sqlx::query(
        "UPDATE node SET name = 'damaged' WHERE vo_id = $1 AND sys_version = 1 AND num = 0",
    )
    .bind(chain.vo_id)
    .execute(&pool)
    .await
    .expect("damage the shared tree");
    let damaged = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert_eq!(damaged.mismatch_count, 2, "{damaged:?}");

    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");
    assert_eq!(report.versions_rebuilt, 2, "{report:?}");
    let stored = stored_versions(&pool, chain.vo_id).await;
    assert_eq!(stored[1].2, None, "v2 reads its own rows again");
    assert_eq!(stored[1].1, stored[0].1, "v2 holds a full tree");
    assert_eq!(stored[3].2, Some(3), "v4 still shares v3's untouched rows");
    assert_eq!(
        head_content_key(&pool, chain.vo_id).await,
        Some(3),
        "the head still reads v3's rows"
    );
    let after = served(&pool, &svc, &chain).await;
    assert_eq!(before, after);
}

/// The rebuild of the head when it shares a tree points the head's content
/// key at the head's own new rows (#4944), so a `LATEST_VERSION` read follows
/// them.
#[tokio::test]
async fn a_rebuilt_sharing_head_names_its_own_rows() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let chain = commit_chain(&svc).await;
    let before = served(&pool, &svc, &chain).await;

    // Damage the tree v3 and v4, the head, read.
    sqlx::query(
        "UPDATE node SET name = 'damaged' WHERE vo_id = $1 AND sys_version = 3 AND num = 0",
    )
    .bind(chain.vo_id)
    .execute(&pool)
    .await
    .expect("damage the shared tree");
    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");
    assert_eq!(report.versions_rebuilt, 2, "{report:?}");
    let stored = stored_versions(&pool, chain.vo_id).await;
    assert_eq!(stored[3].2, None, "v4 reads its own rows again");
    assert_eq!(
        head_content_key(&pool, chain.vo_id).await,
        Some(4),
        "the head names v4's own rows"
    );
    let after = served(&pool, &svc, &chain).await;
    assert_eq!(before, after);
}

/// A composition bound in an `OR` branch under `LATEST_VERSION` is answered
/// once with the head's uid when its head shares its predecessor's tree: the
/// branch's version row is the head's, never every version reading the tree
/// (#4944).
#[tokio::test]
async fn an_or_branch_answers_a_shared_head_once() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let chain = commit_chain(&svc).await;
    let answers = aql_answers(
        &svc,
        &["SELECT c1/uid/value FROM EHR e CONTAINS \
           (COMPOSITION c1[openEHR-EHR-COMPOSITION.minimal.v1] \
            OR COMPOSITION c2[openEHR-EHR-COMPOSITION.referral.v1])"],
    )
    .await;
    assert_eq!(
        answers,
        vec![vec![serde_json::json!([chain.uids[3]]).to_string()]],
        "one row, the head v4's uid"
    );
}

/// The `resend` records on the access log, as `(resource_id,
/// resource_class)`, polled until `expected` of them have landed or the
/// deadline passes: the sender drains in the background.
async fn resend_records(pool: &PgPool, expected: usize) -> Vec<(String, String)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT resource_id, resource_class FROM audit.audit_event \
             WHERE operation = $1 ORDER BY recorded_at",
        )
        .bind(RESEND_OPERATION)
        .fetch_all(pool)
        .await
        .expect("read the access log");
        if rows.len() >= expected || std::time::Instant::now() > deadline {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// With `[versioning] skip_identical_resend` on, an update whose content
/// equals the current version's commits nothing and answers with that version;
/// the access log keeps exactly one `resend` record naming the version matched
/// and its content digest (the entry Cadasto writes for a skipped send,
/// FerroHEALTH-backlog#23); a changed update commits as before; and content
/// equal to an EARLIER version's is matched against the current version only,
/// so it commits a new version.
#[tokio::test]
async fn the_opt_in_rule_answers_a_resend_with_the_existing_version() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
            full_verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (audit, _handle) = sender::start(config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(audit)
        .with_skip_identical_resend(true);
    let ehr_id = create_ehr(&svc).await;
    let v1 = svc
        .create_composition(
            ehr_id,
            uv(&observation_composition("same", 120.0), "249", None),
        )
        .await
        .expect("v1 commits");
    let uid = v1.version_uid();
    let vo_id: VoId = uid
        .split("::")
        .next()
        .expect("object id")
        .parse()
        .expect("uuid");

    let resend = svc
        .update_composition(
            ehr_id,
            vo_id,
            uv(&observation_composition("same", 120.0), "251", Some(&uid)),
        )
        .await
        .expect("the re-send is answered");
    assert!(resend.skipped);
    assert_eq!(resend.version_uid(), uid, "the existing version's uid");
    assert_eq!(resend.tree, v1.tree);
    assert_eq!(resend.time_committed, v1.time_committed);
    let (versions, audits): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT commit_audit_id) FROM version WHERE vo_id = $1",
    )
    .bind(vo_id)
    .fetch_one(&pool)
    .await
    .expect("count the versions");
    assert_eq!((versions, audits), (1, 1), "nothing was committed");
    let (digest, hex): (Vec<u8>, String) = sqlx::query_as(
        "SELECT content_digest, encode(content_digest, 'hex') FROM version \
         WHERE vo_id = $1 AND sys_version = 1",
    )
    .bind(vo_id)
    .fetch_one(&pool)
    .await
    .expect("the stored digest");
    assert_eq!(
        resend.content_digest.map(Vec::from),
        Some(digest),
        "the skipped send carries the digest the content and the version share"
    );
    let records = resend_records(&pool, 1).await;
    assert_eq!(
        records,
        vec![(format!("{uid}#sha256:{hex}"), "composition".to_owned())],
        "one resend record names the version matched and the digest"
    );

    let changed = svc
        .update_composition(
            ehr_id,
            vo_id,
            uv(&observation_composition("other", 130.0), "251", Some(&uid)),
        )
        .await
        .expect("a changed update commits");
    assert!(!changed.skipped);
    assert_eq!(changed.sys_version, 2);
    let stored = stored_versions(&pool, vo_id).await;
    assert_eq!(stored.len(), 2);
    assert!(stored[1].1 > 1 && stored[1].2.is_none());

    // The content of v1 again, against the current version v2: the rule
    // matches the current version only, so this commits v3 (sharing nothing,
    // since v2 differs), and the log gains no resend record.
    let earlier = svc
        .update_composition(
            ehr_id,
            vo_id,
            uv(
                &observation_composition("same", 120.0),
                "251",
                Some(&changed.version_uid()),
            ),
        )
        .await
        .expect("content equal to an earlier version's commits");
    assert!(!earlier.skipped);
    assert_eq!(earlier.sys_version, 3);
    let stored = stored_versions(&pool, vo_id).await;
    assert_eq!(stored.len(), 3);
    assert!(
        stored[2].1 > 1 && stored[2].2.is_none(),
        "v3 holds its own rows"
    );
    assert_eq!(
        resend_records(&pool, 2).await.len(),
        1,
        "still the one resend record"
    );
}
