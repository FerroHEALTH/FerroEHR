// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The node rows a commit stores are the rows its decomposition cut, column
//! for column, however the insert carries them: the columns most rows leave
//! `NULL` travel as sparse groups, each the positions of the rows that set
//! one of its columns, and land on the same rows as the dense columns
//! (#6244). Checked on the IPS composition (402 rows) and a small one, by
//! decomposing the stored body again as the node rebuild does. No openEHR
//! spec governs the `node` table: our own design.

use serde_json::Value;
use sqlx::PgPool;

use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::storage::compact::RootIdentity;
use ferroehr::storage::render::nodes::StoredNode;

use crate::fixtures::{composition, create_ehr, uv};

/// A file of the workspace, relative to this crate.
fn fixture(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// One stored node row: every column the decomposition cuts but the
/// fragment, the shared members and the promoted leaves, as text.
type Stored = (
    (
        i32,
        i32,
        i32,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    (
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ),
    (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    (Option<String>, Option<String>, Option<String>),
    (Option<bool>, Option<bool>, Option<bool>, Option<bool>),
);

/// The row `node` should hold for `row`, in the shape of [`Stored`].
fn expected(row: &StoredNode) -> Stored {
    let value = row.value.as_ref();
    (
        (
            row.num,
            row.num_cap,
            row.parent_num,
            row.rm_type.clone(),
            row.archetype.clone(),
            row.arch_entity.clone(),
            row.arch_concept.clone(),
        ),
        (
            row.arch_major,
            row.name.clone(),
            row.name_code.clone(),
            row.name_terminology.clone(),
            row.path.clone(),
        ),
        (
            value.map(|v| v.value_type.clone()),
            value.and_then(|v| v.num.clone()),
            value.and_then(|v| v.text.clone()),
            value.and_then(|v| v.code.clone()),
            value.and_then(|v| v.terminology.clone()),
        ),
        (
            row.header.rm_version.clone(),
            row.header.language.clone(),
            row.header.encoding.clone(),
        ),
        (
            row.compact.origin.then_some(true),
            row.compact.time.then_some(true),
            row.compact.party_self.then_some(true),
            row.compact.uid_derived.then_some(true),
        ),
    )
}

/// The rows `node` holds for the first version of `vo`, in `num` order.
async fn stored(pool: &PgPool, vo: VoId) -> Vec<Stored> {
    let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(
        "SELECT num, num_cap, parent_num, rm_type, archetype, arch_entity, arch_concept, \
         arch_major, name, name_code, name_terminology, path, value_type, value_num::text, \
         value_text, value_code, value_terminology, rm_version, language, encoding, \
         origin_compact, time_compact, party_self, uid_derived \
         FROM node WHERE vo_id = $1 AND sys_version = 1 ORDER BY num",
    )
    .bind(vo.0)
    .fetch_all(pool)
    .await
    .expect("the node rows");
    rows.iter()
        .map(|r| {
            use sqlx::Row as _;
            let g = |i: usize| r.get::<Option<String>, _>(i);
            let b = |i: usize| r.get::<Option<bool>, _>(i);
            (
                (r.get(0), r.get(1), r.get(2), r.get(3), g(4), g(5), g(6)),
                (r.get(7), g(8), g(9), g(10), r.get(11)),
                (g(12), g(13), g(14), g(15), g(16)),
                (g(17), g(18), g(19)),
                (b(20), b(21), b(22), b(23)),
            )
        })
        .collect()
}

/// The rows the decomposition of the stored first version of `vo` cuts, as
/// the node rebuild decomposes it.
async fn decomposed(pool: &PgPool, vo: VoId) -> Vec<Stored> {
    let (body, uid, template): (String, String, Option<String>) = sqlx::query_as(
        "SELECT v.body::text, ext.version_uid(v.vo_id, v.creating_system_id, v.trunk_version, \
         v.branch_number, v.branch_version), v.template_id \
         FROM version v WHERE v.vo_id = $1 AND v.sys_version = 1",
    )
    .bind(vo.0)
    .fetch_one(pool)
    .await
    .expect("the stored version");
    let body: Value = serde_json::from_str(&body).expect("the body parses");
    let root = RootIdentity {
        version_uid: &uid,
        template_id: template.as_deref(),
    };
    ferroehr::storage::render::decompose(&body, &root)
        .expect("the body decomposes")
        .rows
        .iter()
        .map(expected)
        .collect()
}

/// Asserts that the first version of `vo` stores exactly the rows its body
/// decomposes into; the rows.
async fn stores_its_decomposition(pool: &PgPool, vo: VoId) -> Vec<Stored> {
    let want = decomposed(pool, vo).await;
    let have = stored(pool, vo).await;
    assert_eq!(have.len(), want.len(), "one stored row per decomposed row");
    for (have, want) in have.iter().zip(&want) {
        assert_eq!(have, want, "node row {}", want.0.0);
    }
    want
}

/// The IPS composition's 402 rows are stored as its decomposition cut them.
#[tokio::test]
async fn the_ips_composition_stores_its_decomposition() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    svc.template_adl14_upload(&fixture(
        "../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt",
    ))
    .await
    .expect("upload the IPS template");
    let ips: Value = serde_json::from_str(&fixture(
        "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
    ))
    .expect("the IPS composition parses");
    let ehr = create_ehr(&svc).await;
    let created = Box::pin(svc.create_composition(ehr, uv(&ips, "249", None)))
        .await
        .expect("the IPS composition commits");
    let rows = stores_its_decomposition(&db.pool(), created.vo_id).await;
    assert_eq!(rows.len(), 402, "the IPS composition cuts 402 rows");
    for (group, sets) in [
        (
            "name code",
            rows.iter().filter(|row| row.1.2.is_some()).count(),
        ),
        ("value", rows.iter().filter(|row| row.2.0.is_some()).count()),
        (
            "header",
            rows.iter().filter(|row| row.3.0.is_some()).count(),
        ),
        (
            "marker",
            rows.iter()
                .filter(|row| row.4 != (None, None, None, None))
                .count(),
        ),
    ] {
        assert!(
            sets > 0 && sets < rows.len(),
            "some rows set the {group} group and some do not: {sets}"
        );
    }
}

/// A small composition's rows are stored as its decomposition cut them.
#[tokio::test]
async fn a_small_composition_stores_its_decomposition() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = create_ehr(&svc).await;
    let created = Box::pin(svc.create_composition(ehr, uv(&composition("Encounter"), "249", None)))
        .await
        .expect("the composition commits");
    stores_its_decomposition(&db.pool(), created.vo_id).await;
}
