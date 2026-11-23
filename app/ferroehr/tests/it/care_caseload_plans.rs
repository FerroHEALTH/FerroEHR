// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The plans of the care projection at the scale a GP practice and a ward
//! produce (#6074), against a real `PostgreSQL` 18 (shared testkit harness):
//! three principal sets of 100, 1 600 and 50 000 patients in a store of
//! 151 700 EHRs, each with two compositions cloned from one committed
//! through the service (its version row and its root node, which a
//! population query over compositions reads).
//!
//! Each is read three ways: the check of one EHR, a caseload page and a
//! population AQL query. The check is one probe of `ix_access_care_subject`,
//! read with `EXPLAIN (ANALYZE, FORMAT JSON)`
//! (<https://www.postgresql.org/docs/18/sql-explain.html>). The queries run
//! through the service under a policy that requires a care relationship,
//! and their access paths are read from the cumulative statistics of the
//! test's own database (PostgreSQL 18 §27.2, `pg_stat_user_tables` and
//! `pg_stat_user_indexes`): a query starts from the caller's rows of
//! `ix_access_care_caseload` while its caseload is small, and never scans
//! the projection whole at any size. No openEHR spec governs storage access
//! paths: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::{Executor as _, PgPool};

use ferroehr::access_eval::condition::care::{Care, ProjectedCare};
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess, with_request_access};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::access_eval::scope::CategoryIndex;
use ferroehr::party_binding::grants::caller::facts::CallerFacts;
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::query::request::{AqlQueryRequest, QueryAccess};
use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, OperationClass};

use crate::fixtures::{composition, create_ehr, uv};

/// The principal sets, each a care team, with the patients in its care.
const SCALES: [(&str, i64); 3] = [("practice-a", 100), ("ward", 1_600), ("practice-c", 50_000)];
/// The EHRs in the care of other teams, spread over [`OTHER_TEAMS`].
const OTHERS: i64 = 100_000;
/// The teams the other EHRs are spread over.
const OTHER_TEAMS: i64 = 500;
/// The compositions of each EHR.
const COMPOSITIONS: i64 = 2;
/// The scale up to which a query must start from the caller's caseload.
const CASELOAD_LED: i64 = 1_600;
/// The provider every relationship is in the care of.
const PROVIDER: (&str, &str) = ("urn:example:providers", "hospital");
/// The day every seeded relationship holds on.
const DAY: &str = "2026-11-21";

/// A caseload page: the first EHRs of the caller's caseload.
const PAGE: &str = "SELECT e/ehr_id/value FROM EHR e";
/// A population query over the caseload's compositions.
const POPULATION: &str = "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c";

/// The relations whose sequential scans the queries are held to.
const SCANNED: &str = "SELECT relname, seq_scan FROM pg_stat_user_tables \
     WHERE schemaname = 'clinical' \
     AND relname IN ('access_care', 'ehr', 'vo_head', 'version_hot', 'node_hot')";
/// The indexes of the projection whose scans tell which side a query
/// started from.
const PROBED: &str = "SELECT indexrelname, idx_scan FROM pg_stat_user_indexes \
     WHERE schemaname = 'clinical' \
     AND indexrelname IN ('ix_access_care_caseload', 'ix_access_care_subject')";

/// The container UUID the seed names `label` by.
fn id(label: &str) -> uuid::Uuid {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(label.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(digest.get(..16).expect("a SHA-256 digest is 32 octets"));
    uuid::Uuid::from_bytes(bytes)
}

/// The namespace this CDR names its parties under.
fn own() -> String {
    ferroehr::config::binding::PartyBindingConfig::default().namespace
}

/// The EHRs one seeding statement writes, with their compositions and their
/// care, so that no statement's work grows with the store and each stays far
/// inside the pool's statement timeout on a loaded host (#6227).
const CHUNK: i64 = 10_000;

/// The object id of the seed's `m`-th composition: ordered as it is written,
/// as UUID v7 object ids are, so each index on the object id grows at its
/// right edge instead of touching a random page per row (#6227).
const CLONE_ID: &str = "('00000000-0000-4000-8000-' || lpad(to_hex(m), 12, '0'))::uuid";

/// The EHR of the `m`-th composition, each EHR holding [`COMPOSITIONS`].
const CLONE_EHR: &str = "md5('ehr-' || ((m + $3 - 1) / $3))::uuid";

/// The compositions of the EHRs `$1` to `$2`, `$3` to an EHR, each a clone of
/// the prototype `$4` under [`CLONE_ID`]: its head, its version row and its
/// root node, every other column the prototype's.
fn clone_statements() -> [String; 3] {
    let series = "generate_series(($1 - 1) * $3 + 1, $2 * $3) AS m";
    [
        format!(
            "INSERT INTO vo_head (vo_id, kind, ehr_id, tier, head_sys_version, \
                 trunk_head_sys_version, lifecycle_state, template_id, committed_at, \
                 trunk_head_content_key, root_archetype) \
             SELECT {CLONE_ID}, h.kind, {CLONE_EHR}, h.tier, h.head_sys_version, \
                    h.trunk_head_sys_version, h.lifecycle_state, h.template_id, \
                    h.committed_at, h.trunk_head_content_key, h.root_archetype \
             FROM vo_head h, {series} WHERE h.vo_id = $4"
        ),
        format!(
            "INSERT INTO version_hot (vo_id, kind, ehr_id, sys_version, trunk_version, \
                 lifecycle_state, creating_system_id, contribution_id, commit_audit_id, \
                 template_id, committed_at, body, content_digest) \
             SELECT {CLONE_ID}, v.kind, {CLONE_EHR}, v.sys_version, v.trunk_version, \
                    v.lifecycle_state, v.creating_system_id, v.contribution_id, \
                    v.commit_audit_id, v.template_id, v.committed_at, v.body, v.content_digest \
             FROM version_hot v, {series} WHERE v.vo_id = $4"
        ),
        format!(
            "INSERT INTO node_hot (vo_id, sys_version, num, num_cap, parent_num, ehr_id, \
                 rm_type, archetype, arch_entity, arch_concept, arch_major, name, path, data, \
                 context_start, rm_version, language) \
             SELECT {CLONE_ID}, d.sys_version, d.num, d.num_cap, d.parent_num, {CLONE_EHR}, \
                    d.rm_type, d.archetype, d.arch_entity, d.arch_concept, d.arch_major, \
                    d.name, d.path, d.data, d.context_start, d.rm_version, d.language \
             FROM node_hot d, {series} WHERE d.vo_id = $4 AND d.num = 0"
        ),
    ]
}

/// Seeds the EHRs, their compositions and the care projection, and takes
/// the statistics.
async fn seed(svc: &FerroEhrService, pool: &PgPool) {
    let prototype_ehr = create_ehr(svc).await;
    let prototype = svc
        .create_composition(prototype_ehr, uv(&composition("prototype"), "249", None))
        .await
        .expect("the prototype composition commits")
        .vo_id;
    let mut first = 1_i64;
    let (mut starts, mut ends, mut teams) = (Vec::new(), Vec::new(), Vec::new());
    for (team, patients) in SCALES {
        starts.push(first);
        ends.push(first + patients - 1);
        teams.push(id(team));
        first += patients;
    }
    let last = first + OTHERS - 1;
    let clones = clone_statements();
    let mut from = 1_i64;
    while from <= last {
        let to = (from + CHUNK - 1).min(last);
        sqlx::query(
            "INSERT INTO ehr (id, system_id, subject_id, subject_namespace, subject_digest) \
             SELECT md5('ehr-' || n)::uuid, 'plans.test', md5('person-' || n)::uuid::text, $3, \
                    sha256(convert_to($3 || chr(31) || md5('person-' || n)::uuid::text, 'UTF8')) \
             FROM generate_series($1::bigint, $2::bigint) AS n",
        )
        .bind(from)
        .bind(to)
        .bind(own())
        .execute(pool)
        .await
        .expect("seed the EHRs");
        for statement in &clones {
            sqlx::query(sqlx::AssertSqlSafe(statement.as_str()))
                .bind(from)
                .bind(to)
                .bind(COMPOSITIONS)
                .bind(prototype.0)
                .execute(pool)
                .await
                .expect("seed the compositions");
        }
        sqlx::query(
            "INSERT INTO access_care (subject_digest, person_id, carer_id, carer_kind, \
                 care_from, care_until, provider_issuer, provider_id, writer_id, self_declared, \
                 role_terminology, role_code) \
             SELECT e.subject_digest, md5('person-' || n)::uuid, \
                    COALESCE((SELECT r.team FROM unnest($3::bigint[], $4::bigint[], $5::uuid[]) \
                                  AS r(first, last, team) WHERE n BETWEEN r.first AND r.last), \
                             md5('team-' || (n % $6))::uuid), \
                    'G', date '2026-01-01', NULL, $7, $8, NULL, false, \
                    'HL7_ParticipationType', 'PART' \
             FROM generate_series($1::bigint, $2::bigint) AS n \
             JOIN ehr e ON e.id = md5('ehr-' || n)::uuid",
        )
        .bind(from)
        .bind(to)
        .bind(&starts)
        .bind(&ends)
        .bind(&teams)
        .bind(OTHER_TEAMS)
        .bind(PROVIDER.0)
        .bind(PROVIDER.1)
        .execute(pool)
        .await
        .expect("seed the care projection");
        from = to + 1;
    }
    for relation in ["ehr", "vo_head", "version_hot", "node_hot", "access_care"] {
        pool.execute(sqlx::AssertSqlSafe(format!("VACUUM {relation}")))
            .await
            .expect("vacuum a seeded relation");
    }
    pool.execute("ANALYZE")
        .await
        .expect("take the seed's statistics");
    ferroehr::db::parent_statistics::refresh_pools(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    )
    .await
    .expect("analyze the seeded partitioned parents");
}

/// The care a caller of `team` reads from the projection, on [`DAY`].
fn projected(team: uuid::Uuid) -> Care {
    Care::Projected(ProjectedCare {
        principals: vec![team].into(),
        direct: Vec::new().into(),
        provider_issuers: vec![PROVIDER.0.to_owned()].into(),
        provider_ids: vec![PROVIDER.1.to_owned()].into(),
        day: DAY.parse().expect("a date"),
        grace: "P14D".to_owned(),
        self_declared: false,
    })
}

/// The evaluator of a professional of `team` under a policy that requires a
/// care relationship.
fn access(team: uuid::Uuid) -> Arc<RequestAccess> {
    let policy =
        crate::access_eval_examples::policy(&json!({ "care_relationship_required": true }));
    Arc::new(RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: Some(team.to_string()),
            own_ehrs: Vec::new(),
            patient_grants_only: false,
            acts_for: Vec::new(),
            facts: CallerFacts::default(),
            administrator: false,
            settings: SettingsCaller::new(false, false, Some(team.to_string())),
            lacks_consent_reference: false,
        },
        Asking {
            operation: OperationClass::Query,
            purpose: Some("TREAT".to_owned()),
            emergency: Emergency::default(),
        },
        InForce {
            policy: Some(Arc::new(policy)),
            categories: CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now: jiff::Timestamp::now(),
            grants: None,
        },
    ))
}

/// The rows `aql` answers a professional of `team`, at most `fetch`, its
/// care read from the projection.
async fn query(
    svc: &FerroEhrService,
    team: uuid::Uuid,
    aql: &str,
    fetch: Option<i64>,
) -> Vec<Value> {
    let access = access(team);
    let run = async {
        let classes = svc.ehr_access_classes().await.expect("the classes read");
        let request = AqlQueryRequest {
            ehr_access: QueryAccess::Given(access.condition(&classes, projected(team))),
            fetch,
            ..AqlQueryRequest::default()
        };
        Box::pin(svc.execute_aql(aql, None, &request))
            .await
            .expect("the query runs")
    };
    let outcome = Box::pin(with_request_access(Arc::clone(&access), run)).await;
    outcome
        .result_set
        .to_value()
        .expect("the result set renders")["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The sequential scans of each relation [`SCANNED`] names and the scans of
/// each index [`PROBED`] names, so far, once the one connection of `pool` has
/// published what it counted: it runs `pg_stat_force_next_flush()`, whose
/// transaction end flushes its pending counters (PostgreSQL 18 §27.2.25).
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the statistics on raw pool connections, outside any request"
)]
async fn counters(pool: &PgPool) -> BTreeMap<String, i64> {
    let mut conn = pool.acquire().await.expect("the pool's connection");
    sqlx::query("SELECT pg_stat_force_next_flush()")
        .execute(&mut *conn)
        .await
        .expect("publish the connection's counters");
    let mut read: BTreeMap<String, i64> = BTreeMap::new();
    for statement in [SCANNED, PROBED] {
        let rows: Vec<(String, i64)> = sqlx::query_as(statement)
            .fetch_all(&mut *conn)
            .await
            .expect("read the statistics");
        read.extend(rows);
    }
    read
}

/// The scans `run` made, and the rows it answered.
async fn scans_of(
    pool: &PgPool,
    run: impl Future<Output = Vec<Value>>,
) -> (BTreeMap<String, i64>, Vec<Value>) {
    let before = counters(pool).await;
    let rows = run.await;
    let after = counters(pool).await;
    let moved = after
        .iter()
        .map(|(name, value)| (name.clone(), value - before.get(name).copied().unwrap_or(0)))
        .collect();
    (moved, rows)
}

/// A query over a caseload of `patients` holds its shape: the projection is
/// never scanned whole, and while the caseload is small the query starts
/// from the caller's rows of the caseload index and scans none of the EHRs,
/// the heads, the versions or the nodes whole.
fn assert_shape(what: &str, patients: i64, scans: &BTreeMap<String, i64>) {
    let moved = |name: &str| scans.get(name).copied().unwrap_or(0);
    assert!(
        moved("ix_access_care_subject") > 0,
        "{what} over {patients} patients: the statistics never showed the run: {scans:?}"
    );
    assert_eq!(
        moved("access_care"),
        0,
        "{what} over {patients} patients scans the projection whole: {scans:?}"
    );
    if patients <= CASELOAD_LED {
        assert!(
            moved("ix_access_care_caseload") > 0,
            "{what} over {patients} patients does not start from the caseload index: {scans:?}"
        );
        for relation in ["ehr", "vo_head", "version_hot", "node_hot"] {
            assert_eq!(
                moved(relation),
                0,
                "{what} over {patients} patients scans {relation} whole: {scans:?}"
            );
        }
    }
}

/// The plan of the projection's check of the EHR of patient `n` for `team`,
/// as a query's probe reads it.
async fn check_plan(pool: &PgPool, n: i64, team: uuid::Uuid) -> Value {
    sqlx::query_scalar(
        "EXPLAIN (ANALYZE, FORMAT JSON) SELECT EXISTS (SELECT 1 FROM ehr e \
         JOIN access_care c ON c.subject_digest = e.subject_digest \
         WHERE e.id = md5('ehr-' || $1)::uuid AND c.carer_id = ANY($2) \
         AND (c.provider_issuer, c.provider_id) IN (SELECT * FROM unnest($3::text[], $4::text[])) \
         AND (c.care_from IS NULL OR c.care_from <= $5::date) \
         AND (c.care_until IS NULL OR $5::date < c.care_until))",
    )
    .bind(n)
    .bind(vec![team])
    .bind(vec![PROVIDER.0])
    .bind(vec![PROVIDER.1])
    .bind(DAY)
    .fetch_one(pool)
    .await
    .expect("explain the check")
}

/// Every index and relation the plan tree `node` reads, with the node's
/// type.
fn reads(node: &Value, out: &mut Vec<String>) {
    let text = |key: &str| node.get(key).and_then(Value::as_str).unwrap_or("");
    out.push(format!(
        "{} {} {}",
        text("Node Type"),
        text("Relation Name"),
        text("Index Name")
    ));
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        reads(child, out);
    }
}

/// The check of one EHR, the caseload page and the population query hold
/// their plans at 100, 1 600 and 50 000 patients per principal set, and
/// answer exactly the caseload.
#[tokio::test]
async fn the_caseload_plans_hold_at_practice_scale() {
    let db = testkit::db().await.expect("testkit database");
    // NOTE: one clinical connection, so the counters it publishes are every scan a query made.
    let mut config = ferroehr::db::DbConfig::new(db.url().to_owned());
    config.max_connections = Some(1);
    config.min_connections = Some(0);
    let pool = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Clinical)
        .await
        .expect("a pool of one connection");
    let shared = ferroehr::db::domain::DomainPools::from_shared(&db.pool());
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools {
        clinical: pool.clone(),
        ..shared
    });
    Box::pin(seed(&svc, &pool)).await;
    let mut first = 1_i64;
    for (team, patients) in SCALES {
        let team = id(team);
        let mut read = Vec::new();
        reads(&check_plan(&pool, first, team).await[0]["Plan"], &mut read);
        assert!(
            read.iter()
                .any(|node| node.ends_with(" ix_access_care_subject"))
                && !read.iter().any(|node| node.starts_with("Seq Scan")),
            "the check of one EHR is one probe of the subject index: {read:?}"
        );
        first += patients;

        let (scans, rows) = scans_of(&pool, query(&svc, team, PAGE, Some(50))).await;
        assert_eq!(
            rows.len(),
            usize::try_from(50.min(patients)).expect("a page size"),
            "a caseload page of {patients}"
        );
        assert_shape("a caseload page", patients, &scans);

        let (scans, rows) = scans_of(&pool, query(&svc, team, POPULATION, None)).await;
        assert_shape("a population query", patients, &scans);
        assert_eq!(
            rows.first().and_then(|row| row[0].as_i64()),
            Some(patients * COMPOSITIONS),
            "the population query counts the caseload's compositions"
        );
    }
}

/// A query bounded to the caller's caseload still reaches an EHR outside the
/// caller's care whose settings class its condition does not know, and fails
/// on it (SQLSTATE `ZA002`) as the condition alone fails, so the service
/// judges the query again and never answers it without that EHR (#5794).
#[tokio::test]
async fn a_bounded_query_still_fails_on_a_class_its_condition_does_not_know() {
    use ferroehr::aql::error::StaleCopy;
    let (db, svc) = crate::fixtures::service().await;
    let pool = db.pool();
    let ehr = create_ehr(&svc).await;
    let team = id("ward");
    let access = access(team);
    let classes = svc.ehr_access_classes().await.expect("the classes read");
    let condition = Box::pin(with_request_access(Arc::clone(&access), async {
        access.condition(&classes, projected(team))
    }))
    .await;
    new_class_on(&pool, ehr).await;

    let ast = openehr_query::parser::parse_str(PAGE).expect("the AQL parses");
    let params = ferroehr::aql::ir::Params::default();
    let profile = ferroehr::config::profile::SpecProfile::default();
    let ir = ferroehr::aql::plan(&ast, &params, profile).expect("the AQL plans");
    let ctx = ferroehr::aql::sql::SqlCtx {
        system_id: "plans.test".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        ehr_access: condition,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ferroehr::aql::lineage::ArchetypeLineage::default()),
        policy_parties: ferroehr::aql::sql::readers::PolicyParties::Served,
        access_settings: ferroehr::aql::sql::readers::EhrAccessReaders::Every,
        own_records: None,
        caseload: None,
    };
    let refused = ferroehr::aql::exec::execute(&pool, &ir, None, &params, &ctx, profile)
        .await
        .expect_err("the EHR of the unknown class fails the statement");
    assert_eq!(
        refused.stale_copy(),
        Some(StaleCopy::Classes),
        "{refused:?}"
    );
}

/// Gives `ehr` settings of a class no copy has read yet, as another
/// replica's commit would.
async fn new_class_on(pool: &PgPool, ehr: ferroehr::ids::EhrId) {
    let class: i64 = sqlx::query_scalar(
        "INSERT INTO access_class (digest, settings) \
         VALUES (sha256('a class no copy has read'::bytea), '{}'::jsonb) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("the class is written");
    sqlx::query(
        "INSERT INTO access_settings (ehr_id, vo_id, sys_version, held_closed, default_access, \
             settings, class_id, facts) \
         SELECT $1, vo_id, 1, false, 'open', '{}'::jsonb, $2, '[]'::jsonb \
         FROM vo_head WHERE ehr_id = $1 AND kind = 'EHR_ACCESS'",
    )
    .bind(ehr.0)
    .bind(class)
    .execute(pool)
    .await
    .expect("the EHR's settings are written");
}
