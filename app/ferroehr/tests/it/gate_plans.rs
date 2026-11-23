// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gate read of a guarded write settles on a generic plan (#6392): on one
//! connection, every form of the statement takes at most five custom plans
//! before the plan cache keeps the generic one (`PostgreSQL` docs, `PREPARE`,
//! "Notes"), so a write does not plan its gate again on every call.
//!
//! No openEHR spec governs the planning of a statement: our own design.

use ferroehr::db::domain::DomainPools;
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::storage::version_repo::gate::{self, Ask, Persistent, Shape};
use sqlx::PgConnection;

use crate::fixtures::{composition, create_ehr, uv};

/// The executions of each form, past the five custom plans.
const RUNS: usize = 8;

/// The custom plans `PostgreSQL` makes before it weighs the generic plan.
const CUSTOM: i64 = 5;

/// The custom and generic plans of each prepared statement on `conn` whose
/// text starts with `prefix`.
async fn plans(conn: &mut PgConnection, prefix: &str) -> Vec<(i64, i64, String)> {
    sqlx::query_as(
        "SELECT custom_plans, generic_plans, statement FROM pg_prepared_statements \
         WHERE starts_with(statement, $1) ORDER BY statement",
    )
    .bind(prefix)
    .fetch_all(conn)
    .await
    .expect("the prepared statements read")
}

/// Every form of the composition gate and the party gate settles on a generic
/// plan within five executions on one connection.
#[tokio::test]
async fn every_gate_read_settles_on_a_generic_plan() {
    let db = testkit::db().await.expect("testkit database");
    let pools = DomainPools::from_shared(&db.pool());
    let svc = FerroEhrService::new(&pools);
    let ehr_id = create_ehr(&svc).await;
    let mut vo_ids = Vec::new();
    for name in ["first", "second"] {
        let created = svc
            .create_composition(ehr_id, uv(&composition(name), "249", None))
            .await
            .expect("the composition commits");
        vo_ids.push(created.vo_id);
    }
    let one = &vo_ids[..1];
    let persistent = Persistent {
        template_id: "ferroehr.test.v1",
        category_code: "431",
        deleted_state: "523",
    };
    let ask = |targets, shape| Ask {
        ehr_id,
        targets,
        shape,
        persistent: None,
        addressed: None,
        items: &[],
    };
    let asks = [
        ask(&[], Shape::Plain),
        Ask {
            persistent: Some(persistent),
            ..ask(&[], Shape::Plain)
        },
        ask(one, Shape::Update),
        Ask {
            addressed: Some((1, 0, 0)),
            ..ask(one, Shape::Delete)
        },
        ask(&[], Shape::Members),
        ask(one, Shape::Members),
        ask(&vo_ids, Shape::Members),
        ask(&[], Shape::Status),
        Ask {
            items: &[vo_ids[0].0],
            ..ask(&[], Shape::Directory)
        },
    ];
    let mut clinical = ferroehr::db::acquire::acquire(&pools.clinical)
        .await
        .expect("a clinical connection");
    for ask in asks {
        for _ in 0..RUNS {
            gate::read(&mut clinical, ask).await.expect("the gate read");
        }
    }
    let gates = plans(&mut clinical, "SELECT e.is_modifiable").await;
    assert_eq!(gates.len(), 8, "one statement per form: {gates:?}");
    let mut party = ferroehr::db::acquire::acquire(&pools.party)
        .await
        .expect("a party connection");
    for target in [None, Some(VoId(uuid::Uuid::now_v7()))] {
        for _ in 0..RUNS {
            gate::party(&mut party, target)
                .await
                .expect("the party gate read");
        }
    }
    let parties = plans(&mut party, "SELECT now() AS ts, t.vo_id").await;
    assert_eq!(parties.len(), 1, "one party gate statement: {parties:?}");
    let unsettled: Vec<_> = gates
        .iter()
        .chain(&parties)
        .filter(|(custom, generic, _)| *custom > CUSTOM || *generic == 0)
        .collect();
    assert!(
        unsettled.is_empty(),
        "custom plans, generic plans and statement of each form that keeps planning: \
         {unsettled:#?}"
    );
}

/// The tier of every version and of the head row of `vo_id`.
async fn tiers(conn: &mut PgConnection, vo_id: VoId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT tier::text FROM version WHERE vo_id = $1 \
         UNION ALL SELECT tier::text FROM vo_head WHERE vo_id = $1",
    )
    .bind(vo_id.0)
    .fetch_all(conn)
    .await
    .expect("the tiers read")
}

/// A gate read that finds an archived tip returns the facts it found and
/// brings the object back to the hot tier before the write lands on it.
#[tokio::test]
async fn a_gate_read_brings_an_archived_target_back_to_the_hot_tier() {
    let db = testkit::db().await.expect("testkit database");
    let pools = DomainPools::from_shared(&db.pool());
    let svc = FerroEhrService::new(&pools);
    let ehr_id = create_ehr(&svc).await;
    let created = svc
        .create_composition(ehr_id, uv(&composition("archived"), "249", None))
        .await
        .expect("the composition commits");
    let mut conn = ferroehr::db::acquire::acquire(&pools.clinical)
        .await
        .expect("a clinical connection");
    gate::read(
        &mut conn,
        Ask {
            ehr_id,
            targets: &[created.vo_id],
            shape: Shape::Update,
            persistent: None,
            addressed: None,
            items: &[],
        },
    )
    .await
    .expect("the hot gate read");
    ferroehr::storage::version_repo::tier::freeze(&mut conn, &[created.vo_id], Some("test"))
        .await
        .expect("the object freezes");
    assert!(
        tiers(&mut conn, created.vo_id)
            .await
            .iter()
            .all(|t| t == "cold"),
        "the object is archived"
    );
    let gate = gate::read(
        &mut conn,
        Ask {
            ehr_id,
            targets: &[created.vo_id],
            shape: Shape::Update,
            persistent: None,
            addressed: None,
            items: &[],
        },
    )
    .await
    .expect("the gate read of the archived object");
    let target = gate.targets.first().expect("the target's facts");
    assert_eq!(target.vo_id, created.vo_id);
    assert_eq!(
        target.placement.tip.as_ref().map(|tip| tip.sys_version),
        Some(1),
        "the archived tip is the one found"
    );
    assert!(
        tiers(&mut conn, created.vo_id)
            .await
            .iter()
            .all(|t| t == "hot"),
        "the object is back in the hot tier"
    );
}
