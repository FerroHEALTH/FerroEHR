// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `POST /ehr` across two replicas over one database (#5109).
//!
//! The Kubernetes probe's local-index row once answered an EHR create with
//! `500` on a two-replica release, right after a `helm upgrade` that changed
//! no value. The tests run the probe's request on two replicas of one
//! deployment, each with its own pools, copies and audit sender, as the
//! Basic user bound to a party: creates naming subjects in one namespace on
//! both replicas at once, the second replica starting cold while the first
//! serves; one subject on both; creates while the upgrade's migration hook
//! runs again; creates while the database refuses new connections; and
//! creates while it cuts open sessions. A create is answered `201`, `409`
//! for a subject another create took (ITS-REST EHR API `409_EHR`), or `503`
//! while the database cannot serve it, and never `500`. No openEHR spec
//! governs replicas: our own design.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::db::pool::IDLE_PROBE;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::{ResolvedSubject, SubjectLookup, SubjectResolver, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};

use crate::common::{self, BASE};

const USERS: &[(&str, &[&str])] = &[("root", &["ADMIN"]), ("clinician", &["USER"])];

/// The creates each replica sends at once.
const CREATES: usize = 24;

/// The subject resolver the server wires: one read of the clinical pool.
fn resolver(pool: PgPool) -> SubjectResolver {
    Arc::new(move |ehr_id: String| {
        let pool = pool.clone();
        Box::pin(async move {
            let Ok(id) = uuid::Uuid::parse_str(&ehr_id) else {
                return SubjectLookup::NoSubject;
            };
            match ferroehr::storage::ehr_repo::ehr_subject_columns(&pool, EhrId(id)).await {
                Ok(Some((Some(id), namespace))) => {
                    SubjectLookup::Found(ResolvedSubject { id, namespace })
                }
                Ok(_) => SubjectLookup::NoSubject,
                Err(_) => SubjectLookup::Unavailable,
            }
        })
    })
}

/// A replica over pools of its own, its copies cold and the access log on,
/// booted as the server boots it: the default definitions seeded first.
async fn replica(db: &testkit::TestDb) -> Router {
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(8);
    settings.min_connections = Some(0);
    settings.connection_budget = 32;
    settings.acquire_timeout = ferrohealth_config::units::Duration::from_secs(10);
    let pools: DomainPools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the pools connect");
    let clinical = pools.get(Domain::Clinical).clone();
    let audit = pools.get(Domain::Audit).clone();
    let (sender, _handle) = start(
        common::local_audit(),
        Some(resolver(clinical)),
        audit.clone(),
    )
    .await
    .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&pools)
            .with_binding_mapping(crate::party_binding_http::mapping())
            .with_audit(sender)
            .with_audit_store(AuditStore::new(audit)),
    );
    svc.seed_default_definitions()
        .await
        .expect("the default definitions seed");
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &svc, &db.pool(), USERS).await;
    common::build_full(config, svc, None, Observability::default()).await
}

/// The probe's `EHR_STATUS`, naming `subject` in the namespace `probe`.
fn status(subject: &str) -> String {
    json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": {
            "_type": "PARTY_REF", "namespace": "probe", "type": "PERSON",
            "id": { "_type": "GENERIC_ID", "value": subject, "scheme": "probe" } } },
        "is_queryable": true,
        "is_modifiable": true
    })
    .to_string()
}

/// `POST /ehr` naming `subject`, as the bound Basic user.
fn create(subject: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header(header::AUTHORIZATION, common::basic_credential("clinician"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
        .body(Body::from(status(subject)))
        .expect("a request")
}

/// `GET /ehr` by `subject` in the namespace `probe`: the EHR id it finds.
async fn found(app: &Router, subject: &str) -> (StatusCode, Option<String>) {
    let request = Request::builder()
        .uri(format!(
            "{BASE}/ehr?subject_id={subject}&subject_namespace=probe"
        ))
        .header(header::AUTHORIZATION, common::basic_credential("clinician"))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .expect("a request");
    let (status, body) = common::send_body(app, request).await;
    let id = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|ehr| ehr["ehr_id"]["value"].as_str().map(str::to_owned));
    (status, id)
}

/// A subject of letters alone, unique to `n` and `round`.
fn subject(round: &str, n: usize) -> String {
    let letters: String = n
        .to_string()
        .bytes()
        .map(|digit| char::from(b'a' + (digit - b'0')))
        .collect();
    format!("local-{round}-{letters}")
}

/// Every create the two replicas answer at once, the second replica
/// starting while the first serves: `201` with a `Location`, then found by
/// its subject on the other replica.
#[tokio::test]
async fn creates_on_two_replicas_are_answered_201_and_found_on_the_other() {
    let db = testkit::db().await.expect("testkit database");
    let first = replica(&db).await;
    let (status, body) = common::send_body(
        &first,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .header(header::AUTHORIZATION, common::basic_credential("clinician"))
            .body(Body::empty())
            .expect("a request"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the serve row's create: {body}"
    );

    let mut answers = tokio::task::JoinSet::new();
    for n in 0..CREATES {
        let app = first.clone();
        answers.spawn(async move {
            let subject = subject("a", n);
            let (status, headers, body) = common::send(&app, create(&subject)).await;
            (
                subject,
                status,
                headers.contains_key(header::LOCATION),
                body,
            )
        });
    }
    let second = replica(&db).await;
    for n in 0..CREATES {
        let app = second.clone();
        answers.spawn(async move {
            let subject = subject("b", n);
            let (status, headers, body) = common::send(&app, create(&subject)).await;
            (
                subject,
                status,
                headers.contains_key(header::LOCATION),
                body,
            )
        });
    }
    let answered = tokio::time::timeout(Duration::from_secs(120), answers.join_all())
        .await
        .expect("the creates are answered");
    let refused: Vec<String> = answered
        .iter()
        .filter(|(_, status, located, _)| *status != StatusCode::CREATED || !located)
        .map(|(subject, status, _, body)| format!("{subject}: {status} {body}"))
        .collect();
    assert!(
        refused.is_empty(),
        "every create is answered 201: {refused:#?}"
    );
    for (subject, ..) in &answered {
        let other = if subject.starts_with("local-a") {
            &second
        } else {
            &first
        };
        let (status, id) = found(other, subject).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{subject} is found on the other replica"
        );
        assert!(id.is_some(), "{subject} names an EHR");
    }
}

/// A create naming a subject in another namespace, as the bound user: the
/// access decision reads the subject's `PERSON` on the party pool before the
/// create takes its audit and clinical connections, so it never waits for
/// the party pool inside its write, which a debug build refuses (#6186).
#[tokio::test]
async fn a_create_reads_the_party_store_before_it_takes_its_write_connections() {
    let db = testkit::db().await.expect("testkit database");
    let app = replica(&db).await;
    for n in 0..4 {
        let subject = subject("p", n);
        let (status, body) = common::send_body(&app, create(&subject)).await;
        assert_eq!(status, StatusCode::CREATED, "{subject}: {body}");
    }
}

/// Two creates naming one subject, one on each replica at once: one is
/// answered `201` and the other `409`, never `500`.
#[tokio::test]
async fn one_subject_on_two_replicas_is_one_201_and_one_409() {
    let db = testkit::db().await.expect("testkit database");
    let first = replica(&db).await;
    let second = replica(&db).await;
    for n in 0..CREATES {
        let subject = subject("c", n);
        let (a, b) = tokio::join!(
            common::send_body(&first, create(&subject)),
            common::send_body(&second, create(&subject)),
        );
        let mut statuses = [a.0, b.0];
        statuses.sort();
        assert_eq!(
            statuses,
            [StatusCode::CREATED, StatusCode::CONFLICT],
            "{subject}: {} {} / {} {}",
            a.0,
            a.1,
            b.0,
            b.1
        );
    }
}

/// Creates on both replicas while the migration step a `helm upgrade` runs
/// as its pre-upgrade hook applies again over the serving database: each is
/// answered `201`.
#[tokio::test]
async fn creates_while_the_upgrade_hook_migrates_again_are_answered_201() {
    let db = testkit::db().await.expect("testkit database");
    let first = replica(&db).await;
    let second = replica(&db).await;
    let pool = db.pool();
    let stop = AtomicBool::new(false);
    let migrations = async {
        let mut runs = 0_u32;
        while !stop.load(Ordering::Relaxed) {
            ferroehr::db::run_migrations(&pool)
                .await
                .expect("the migrations apply again");
            runs += 1;
        }
        runs
    };
    let creates = async {
        let mut answers = tokio::task::JoinSet::new();
        for n in 0..4 * CREATES {
            let app = if n % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            answers.spawn(async move {
                let subject = subject("m", n);
                let (status, body) = common::send_body(&app, create(&subject)).await;
                (subject, status, body)
            });
        }
        let answered = answers.join_all().await;
        stop.store(true, Ordering::Relaxed);
        answered
    };
    let (runs, answered) = tokio::join!(migrations, creates);
    assert!(runs > 0, "the migrations ran beside the creates");
    let refused: Vec<String> = answered
        .iter()
        .filter(|(_, status, _)| *status != StatusCode::CREATED)
        .map(|(subject, status, body)| format!("{subject}: {status} {body}"))
        .collect();
    assert!(
        refused.is_empty(),
        "every create is answered 201: {refused:#?}"
    );
}

/// Sets whether the database `name` takes new connections, on a session of
/// the server's maintenance database, and cuts every session of `name` when
/// it stops taking them.
async fn connections(admin: &mut PgConnection, name: &str, allowed: bool) {
    sqlx::query("SELECT set_config('ferroehr_test.database', $1, false)")
        .bind(name)
        .execute(&mut *admin)
        .await
        .expect("the database name is set");
    let statement = if allowed {
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I ALLOW_CONNECTIONS true', \
         current_setting('ferroehr_test.database')); END $$"
    } else {
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I ALLOW_CONNECTIONS false', \
         current_setting('ferroehr_test.database')); END $$"
    };
    sqlx::query(statement)
        .execute(&mut *admin)
        .await
        .expect("the database's connections change");
    if !allowed {
        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE datname = $1 AND backend_type = 'client backend'",
        )
        .bind(name)
        .execute(&mut *admin)
        .await
        .expect("the sessions are cut");
    }
}

/// While the database refuses every new connection, a create and a lookup
/// on either replica are answered `503`; once it takes them again, the first
/// create on each replica is answered `201`, since no request is handed a
/// connection the outage cut while it sat idle.
#[tokio::test]
async fn a_refused_connect_is_503_on_both_replicas_and_then_201() {
    let db = testkit::db().await.expect("testkit database");
    let first = replica(&db).await;
    let second = replica(&db).await;
    for (app, subject) in [(&first, "local-warm-a"), (&second, "local-warm-b")] {
        let (status, body) = common::send_body(app, create(subject)).await;
        assert_eq!(status, StatusCode::CREATED, "{subject}: {body}");
    }
    let name = db.name().to_owned();
    let maintenance = db.url().replacen(&format!("/{name}"), "/postgres", 1);
    let mut admin = PgConnection::connect(&maintenance)
        .await
        .expect("the maintenance database connects");
    connections(&mut admin, &name, false).await;
    let mut answered = Vec::new();
    for (n, app) in [&first, &second, &first, &second].into_iter().enumerate() {
        let subject = subject("f", n);
        let (status, body) = common::send_body(app, create(&subject)).await;
        answered.push((subject, status, body));
        let (status, _) = found(app, "local-warm-a").await;
        answered.push(("the lookup".to_owned(), status, String::new()));
    }
    connections(&mut admin, &name, true).await;
    let other: Vec<String> = answered
        .iter()
        .filter(|(_, status, _)| *status != StatusCode::SERVICE_UNAVAILABLE)
        .map(|(subject, status, body)| format!("{subject}: {status} {body}"))
        .collect();
    assert!(
        other.is_empty(),
        "every request is answered 503: {other:#?}"
    );
    // The connections the outage cut sat idle in the pools throughout it.
    tokio::time::sleep(IDLE_PROBE + Duration::from_millis(100)).await;
    for (app, subject) in [(&first, "local-back-a"), (&second, "local-back-b")] {
        let (status, body) = common::send_body(app, create(subject)).await;
        assert_eq!(status, StatusCode::CREATED, "{subject}: {body}");
    }
}

/// Creates on both replicas while the database cuts their open sessions at
/// random: each is answered `201` or `503`, never `500`.
#[tokio::test]
async fn creates_while_sessions_are_cut_are_201_or_503() {
    let db = testkit::db().await.expect("testkit database");
    let first = replica(&db).await;
    let second = replica(&db).await;
    let pool = db.pool();
    let stop = AtomicBool::new(false);
    let cuts = async {
        while !stop.load(Ordering::Relaxed) {
            // A round may run on a session an earlier round cut, which the pool
            // then replaces, so a failed round is the cut working.
            let _round = sqlx::query(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                 WHERE datname = current_database() AND pid <> pg_backend_pid() \
                 AND backend_type = 'client backend' AND random() < 0.3",
            )
            .execute(&pool)
            .await;
            tokio::time::sleep(Duration::from_millis(7)).await;
        }
    };
    let creates = async {
        let mut answers = tokio::task::JoinSet::new();
        for n in 0..8 * CREATES {
            let app = if n % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            answers.spawn(async move {
                let subject = subject("k", n);
                let (status, body) = common::send_body(&app, create(&subject)).await;
                (subject, status, body)
            });
        }
        let answered = answers.join_all().await;
        stop.store(true, Ordering::Relaxed);
        answered
    };
    let ((), answered) = tokio::join!(cuts, creates);
    let other: Vec<String> = answered
        .iter()
        .filter(|(_, status, _)| {
            *status != StatusCode::CREATED && *status != StatusCode::SERVICE_UNAVAILABLE
        })
        .map(|(subject, status, body)| format!("{subject}: {status} {body}"))
        .collect();
    assert!(other.is_empty(), "every create is 201 or 503: {other:#?}");
}
