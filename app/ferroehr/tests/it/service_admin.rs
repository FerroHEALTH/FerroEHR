// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for the ADMIN API (physical EHR delete) against a
//! real `PostgreSQL` 18 (shared testkit harness).
//!
//! Spec: SM `I_ADMIN_SERVICE.physical_ehr_delete`
//! (`docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`) — precondition
//! `has_ehr`, error `ehr_id_does_not_exist`. The cascade contract is the CNF
//! Robot prior art
//! (`docs/specs/openehr/CNF/tests/platform/robot/I_ADMIN_SERVICE/001-EHR.robot`):
//! after delete, every backing table returns to its pre-EHR baseline count. We
//! assert **zero rows remain** for the deleted EHR across `ehr`, `version`,
//! `node`, `contribution`, `audit`, and `item_tag`, while a second EHR is left
//! entirely untouched.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]
#![expect(
    clippy::too_many_lines,
    reason = "an end-to-end suite drives one long lifecycle per test on purpose: \
              splitting a case would hide the order its assertions depend on"
)]

use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use ferroehr::service::FerroEhrService;

use crate::admin_fixture::{count_for_ehr, party_relationship, repository, seed_full_ehr};
use crate::fixtures::uid;
use crate::fixtures::uv;
use crate::typed_body::typed;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::platform_service::PlatformService;
use ferroehr::service::status::{CallStatusType, SmError};

/// Row counts scoped to one EHR across every table a physical delete must clear.
/// `audit` has no `ehr_id`, so it is counted by the audit ids the EHR's
/// versions/contributions reference (the same set the delete captures).
#[derive(Debug, Default, PartialEq, Eq)]
struct EhrRows {
    ehr: i64,
    version: i64,
    node: i64,
    contribution: i64,
    item_tag: i64,
    audit: i64,
}

impl EhrRows {
    fn is_empty(&self) -> bool {
        *self == EhrRows::default()
    }
}

async fn ehr_rows(pool: &PgPool, ehr_id: Uuid) -> EhrRows {
    let commit_audit_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT commit_audit_id FROM version WHERE ehr_id = $1 \
         UNION SELECT commit_audit_id FROM contribution WHERE ehr_id = $1",
    )
    .bind(ehr_id)
    .fetch_all(pool)
    .await
    .expect("audit ids");
    let audit: i64 = sqlx::query_scalar("SELECT count(*) FROM commit_audit WHERE id = ANY($1)")
        .bind(&commit_audit_ids)
        .fetch_one(pool)
        .await
        .expect("audit count");
    EhrRows {
        ehr: count_for_ehr(pool, "SELECT count(*) FROM ehr WHERE id = $1", ehr_id).await,
        version: count_for_ehr(
            pool,
            "SELECT count(*) FROM version WHERE ehr_id = $1",
            ehr_id,
        )
        .await,
        node: count_for_ehr(pool, "SELECT count(*) FROM node WHERE ehr_id = $1", ehr_id).await,
        contribution: count_for_ehr(
            pool,
            "SELECT count(*) FROM contribution WHERE ehr_id = $1",
            ehr_id,
        )
        .await,
        item_tag: count_for_ehr(
            pool,
            "SELECT count(*) FROM item_tag WHERE ehr_id = $1",
            ehr_id,
        )
        .await,
        audit,
    }
}

#[tokio::test]
async fn admin_delete_cascades_and_leaves_other_ehr_untouched() {
    // One database, two handles: the service owns one clone, the test queries
    // the other directly to assert the cascade.
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let ehr1 = seed_full_ehr(&svc).await;
    let ehr2 = seed_full_ehr(&svc).await;

    // Both EHRs have content in every table before the delete.
    let before1 = ehr_rows(pool, ehr1.into()).await;
    let before2 = ehr_rows(pool, ehr2.into()).await;
    assert!(!before1.is_empty(), "ehr1 must be populated: {before1:?}");
    // EHR_STATUS v1+v2, EHR_ACCESS v1, FOLDER v1 → ≥4 versions; ≥3 contributions
    // (ehr create, status update, directory create); 1 item tag.
    assert!(before1.ehr == 1 && before1.version >= 4 && before1.node >= 4);
    assert!(before1.contribution >= 3 && before1.item_tag == 1 && before1.audit >= 3);

    // Physical delete via the ADMIN seam (SM physical_ehr_delete).
    svc.admin_ehr_delete(ehr1.to_string())
        .await
        .expect("admin delete");

    // Every trace of ehr1 is physically gone (CNF cascade contract).
    let after1 = ehr_rows(pool, ehr1.into()).await;
    assert!(
        after1.is_empty(),
        "physical delete must clear every table for the EHR, got {after1:?}"
    );

    // ehr2 is entirely untouched.
    let after2 = ehr_rows(pool, ehr2.into()).await;
    assert_eq!(after2, before2, "the other EHR must be untouched");
}

/// A physical EHR delete reaches the cross-reference: neither the party
/// mapping nor the `EHR_INDEX` association naming that EHR survives it.
///
/// Both halves matter. A surviving row is the additional information of GDPR
/// Art. 4(5) outliving the data it was additional to
/// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>), and it would keep
/// asserting whose record an erased EHR was, in force, forever — the linkage
/// domain corrects forward everywhere else and would never close it. The role
/// still holds no `DELETE`: the removal runs through `linkage.erase_ehr`.
#[tokio::test]
async fn admin_delete_reaches_the_cross_reference() {
    let (_db, pool, svc) = repository().await;

    let erased = seed_full_ehr(&svc).await;
    let kept = seed_full_ehr(&svc).await;
    let party = ferroehr::ids::VoId(Uuid::now_v7());
    let other_party = ferroehr::ids::VoId(Uuid::now_v7());
    svc.link(party, erased).await.expect("the mapping opens");
    svc.link(other_party, kept)
        .await
        .expect("the other mapping opens");
    svc.add_ehr_subject(
        erased.to_string(),
        ferroehr::service::ehr_index::types::SubjectRef::person("PID-ERASE", "mpi"),
        None,
        None,
    )
    .await
    .expect("the index association is recorded");

    let rows = |ehr: ferroehr::ids::EhrId| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM linkage.subject_ehr WHERE ehr_id = $1",
            )
            .bind(ehr.0)
            .fetch_one(&pool)
            .await
            .expect("count the cross-reference rows")
        }
    };
    assert_eq!(rows(erased).await, 2, "a mapping and an association");

    svc.admin_ehr_delete(erased.to_string())
        .await
        .expect("admin delete");

    assert_eq!(
        rows(erased).await,
        0,
        "no cross-reference row may name an erased EHR, in force or historical"
    );
    assert_eq!(
        rows(kept).await,
        1,
        "the other EHR's mapping is untouched: erase_ehr takes one EHR id"
    );
}

#[tokio::test]
async fn admin_delete_unknown_ehr_is_not_found() {
    let (_db, _pool, svc) = repository().await;

    // `has_ehr` is false → `ehr_id_does_not_exist` (→ HTTP 404), preserved
    // through the ServiceError round-trip.
    let missing = Uuid::now_v7().to_string();
    let res = svc.admin_ehr_delete(missing).await;
    assert!(
        matches!(
            res,
            Err(SmError {
                status: CallStatusType::EhrIdDoesNotExist,
                ..
            })
        ),
        "unknown EHR must be ehr_id_does_not_exist, got {res:?}"
    );

    // A malformed id is a 400.
    let bad = svc.admin_ehr_delete("not-a-uuid".to_owned()).await;
    assert!(
        matches!(
            bad,
            Err(SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            })
        ),
        "malformed id must be BadRequest, got {bad:?}"
    );
}

#[tokio::test]
async fn admin_delete_all_deletes_present_and_skips_missing() {
    let (_db, _pool, svc) = repository().await;

    let a = seed_full_ehr(&svc).await;
    let b = seed_full_ehr(&svc).await;

    // A two-id list of existing EHRs deletes both.
    let deleted = svc
        .admin_ehr_delete_all(vec![a.to_string(), b.to_string()])
        .await
        .expect("delete all");
    assert_eq!(deleted, 2, "both existing EHRs deleted");

    // A list mixing one existing and one missing id deletes only the existing
    // (idempotent bulk: missing ids are skipped).
    let c = seed_full_ehr(&svc).await;
    let missing = Uuid::now_v7().to_string();
    let deleted = svc
        .admin_ehr_delete_all(vec![c.to_string(), missing])
        .await
        .expect("delete all with a bogus id");
    assert_eq!(
        deleted, 1,
        "only the existing EHR is deleted; missing skipped"
    );

    // A malformed id in the list rejects the whole request (400), no deletion.
    let d = seed_full_ehr(&svc).await;
    let res = svc
        .admin_ehr_delete_all(vec![d.to_string(), "not-a-uuid".to_owned()])
        .await;
    assert!(
        matches!(
            res,
            Err(SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            })
        ),
        "a malformed id must be BadRequest, got {res:?}"
    );
}

/// An **empty** `ehr_id` selector deletes ALL EHRs
/// (`operations/admin_ehr_delete_all.yaml:5` — "Deletes **all** or multiple
/// EHRs"; `parameters/query/ehr_id_Admin.yaml` — `ehr_id` is an OPTIONAL subset
/// selector, so an absent/empty list denotes the full set). This supersedes the
/// former delete-nothing safety posture.
#[tokio::test]
async fn admin_delete_all_with_empty_list_deletes_every_ehr() {
    let (_db, _pool, svc) = repository().await;

    seed_full_ehr(&svc).await;
    seed_full_ehr(&svc).await;
    seed_full_ehr(&svc).await;

    // Empty selector → every seeded EHR deleted.
    let deleted = svc
        .admin_ehr_delete_all(vec![])
        .await
        .expect("delete all (empty selector)");
    assert_eq!(deleted, 3, "an empty list deletes ALL EHRs");

    // Idempotent: a second empty delete now finds nothing.
    let again = svc
        .admin_ehr_delete_all(vec![])
        .await
        .expect("delete all again");
    assert_eq!(again, 0, "no EHRs remain after the all-delete");

    // A freshly seeded EHR is again the whole set for an empty selector.
    seed_full_ehr(&svc).await;
    let after = svc
        .admin_ehr_delete_all(vec![])
        .await
        .expect("delete all after reseed");
    assert_eq!(after, 1, "empty selector deletes the one remaining EHR");
}

// ─── Admin extensions: template + stored-query deletes (our own design) ───────
// No openEHR spec governs these (the ITS-REST Admin API defines only EHR
// deletes); they mirror the EHR-delete surface. See
// `app/ferroehr/src/service/admin/delete.rs`.

const OPT_FIXTURE_REL: &str = "knowledge/IDCR Allergies List.v0.opt";
const OPT_TEMPLATE_ID: &str = "IDCR Allergies List.v0";

fn read_fixture(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/service")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

async fn template_rows(pool: &PgPool, template_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM template_store WHERE template_id = $1")
        .bind(template_id)
        .fetch_one(pool)
        .await
        .expect("template count")
}

/// Whether the call failed with the expected granular does-not-exist status
/// (a wire `404`; the `ServiceError` round-trip preserves the status the
/// construction site named — `master03-common_package.adoc` §Representing
/// Call Status).
fn is_not_found(res: &Result<(), SmError>, status: CallStatusType) -> bool {
    matches!(res, Err(SmError { status: got, .. }) if *got == status)
}

#[tokio::test]
async fn admin_template_delete_happy_unknown_and_referenced() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    // Unknown id → NotFound (→ 404).
    let res = svc
        .admin_template_delete("no-such-template.v9".to_owned())
        .await;
    assert!(
        is_not_found(&res, CallStatusType::TemplateDoesNotExist),
        "unknown template → template_does_not_exist, got {res:?}"
    );

    // Upload a template; it is deletable while unreferenced, case-insensitively.
    svc.template_adl14_upload(read_fixture(OPT_FIXTURE_REL))
        .await
        .expect("upload opt");
    assert_eq!(template_rows(pool, OPT_TEMPLATE_ID).await, 1);
    svc.admin_template_delete(OPT_TEMPLATE_ID.to_ascii_uppercase())
        .await
        .expect("delete template (case-insensitive)");
    assert_eq!(
        template_rows(pool, OPT_TEMPLATE_ID).await,
        0,
        "template physically deleted"
    );

    // Re-upload, then reference it from a committed version: the delete must be
    // refused (409; the generic SM `conflict` — a referenced-template conflict
    // is not a COMPOSITION conflict and the SM names nothing more precise,
    // #2151) so a physical delete never
    // orphans clinical data. Pointing an existing version row at the template
    // exercises the `version.template_id` FK-reference guard directly (lighter
    // than a full validated composition commit, which the guard does not need).
    svc.template_adl14_upload(read_fixture(OPT_FIXTURE_REL))
        .await
        .expect("re-upload opt");
    let ehr: Uuid = svc.create_ehr(None).await.expect("ehr").into();
    let referenced = sqlx::query("UPDATE version SET template_id = $1 WHERE ehr_id = $2")
        .bind(OPT_TEMPLATE_ID)
        .bind(ehr)
        .execute(pool)
        .await
        .expect("reference template")
        .rows_affected();
    assert!(
        referenced >= 1,
        "a version row must now reference the template"
    );

    let res = svc.admin_template_delete(OPT_TEMPLATE_ID.to_owned()).await;
    assert!(
        matches!(
            res,
            Err(SmError {
                status: CallStatusType::Conflict,
                ..
            })
        ),
        "referenced template must be refused (409), got {res:?}"
    );
    assert_eq!(
        template_rows(pool, OPT_TEMPLATE_ID).await,
        1,
        "a refused delete leaves the template in place"
    );
}

#[tokio::test]
async fn admin_query_delete_exact_version_and_unknown() {
    let (_db, _pool, svc) = repository().await;

    let name = "org.example::my_query";
    // Store the query at an explicit version (the PUT-with-version path).
    svc.query_store(
        name.to_owned(),
        Some("1.0.0".to_owned()),
        "AQL".to_owned(),
        "SELECT c FROM EHR e CONTAINS COMPOSITION c".to_owned(),
    )
    .await
    .expect("store query 1.0.0");

    // Unknown (name, version) → NotFound.
    let res = svc
        .admin_query_delete(name.to_owned(), "9.9.9".to_owned())
        .await;
    assert!(
        is_not_found(&res, CallStatusType::ArtefactDoesNotExist),
        "unknown version → artefact_does_not_exist, got {res:?}"
    );

    // Exact-version delete succeeds (case-insensitive on the name); the row is
    // gone, so a second delete is NotFound.
    svc.admin_query_delete("ORG.EXAMPLE::MY_QUERY".to_owned(), "1.0.0".to_owned())
        .await
        .expect("delete version 1.0.0 (case-insensitive name)");
    let again = svc
        .admin_query_delete(name.to_owned(), "1.0.0".to_owned())
        .await;
    assert!(
        is_not_found(&again, CallStatusType::ArtefactDoesNotExist),
        "already-deleted version → artefact_does_not_exist, got {again:?}"
    );
}

// ─── SM-4: statistics / physical_party_delete / archive ───────────────────────

/// A minimal valid demographic PERSON (PARTY invariant `Identities_valid`),
/// with no structured identity detail beyond the mandatory legal name.
fn person(name: &str) -> Value {
    crate::admin_fixture::person(name, &json!([]))
}

/// Create a PERSON and return its bare versioned-object UUID string.
async fn make_person(svc: &FerroEhrService, name: &str) -> String {
    let created = svc
        .party_create(
            PartyKind::Person,
            openehr_its::json::from_canonical_value(&person(name)).expect("the PERSON decodes"),
            None,
        )
        .await
        .expect("create person");
    created.body["uid"]["value"]
        .as_str()
        .expect("uid")
        .split("::")
        .next()
        .expect("vo uuid")
        .to_owned()
}

/// The number of `version` rows for one versioned object (0 = physically gone).
/// Version rows of a DEMOGRAPHIC versioned object (a party or a party
/// relationship), named in the schema the pseudonymisation domain keeps them
/// in — the pool's own search path serves the clinical schema.
async fn version_rows(pool: &PgPool, vo: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM party.version WHERE vo_id = $1::uuid")
        .bind(vo)
        .fetch_one(pool)
        .await
        .expect("version count")
}

#[tokio::test]
async fn admin_statistics_per_service_and_time_range() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let ehr = seed_full_ehr(&svc).await; // several EHR-scoped contributions
    make_person(&svc, "P").await; // one demographic (ehr-less) contribution

    // Directly seed COMPOSITIONs (the service fixtures here avoid COMPOSITION):
    // one vo with two versions + one vo with a single version → 2 versioned
    // compositions across 3 version rows. Reuse an existing audit/contribution.
    let (cid, aid): (Uuid, Uuid) =
        sqlx::query_as("SELECT id, commit_audit_id FROM contribution WHERE ehr_id = $1 LIMIT 1")
            .bind(ehr)
            .fetch_one(pool)
            .await
            .expect("an EHR contribution");
    let vo_x = Uuid::now_v7();
    let vo_y = Uuid::now_v7();
    for (vo, ver, period) in [
        (vo_x, 1, "now() - interval '2 seconds'"),
        (vo_x, 2, "now() - interval '1 second'"),
        (vo_y, 1, "now() - interval '1 second'"),
    ] {
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO version (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, contribution_id, commit_audit_id, creating_system_id) \
             VALUES ($1, 'COMPOSITION', $2, $3, $3, {period}, $4, $5, 'ferroehr.test')"
        )))
        .bind(vo)
        .bind(ehr)
        .bind(ver)
        .bind(cid)
        .bind(aid)
        .execute(pool)
        .await
        .expect("seed composition version");
    }

    // Contribution counts: Ehr scope has the seeded EHR's contributions;
    // Demographic scope has the party create; a non-content service has none.
    let ehr_count = svc
        .admin_contribution_count(PlatformService::Ehr, None)
        .await
        .expect("ehr count");
    assert!(
        ehr_count >= 3,
        "expected the EHR's contributions, got {ehr_count}"
    );
    let demo_count = svc
        .admin_contribution_count(PlatformService::Demographic, None)
        .await
        .expect("demo count");
    assert_eq!(demo_count, 1, "one demographic contribution (the party)");
    let query_count = svc
        .admin_contribution_count(PlatformService::Query, None)
        .await
        .expect("query count");
    assert_eq!(query_count, 0, "Query is not a versioned-content service");

    // list_contributions agrees with the count for the Ehr scope.
    let listed = svc
        .admin_list_contributions(PlatformService::Ehr, None)
        .await
        .expect("list contributions");
    assert_eq!(i64::try_from(listed.len()).unwrap(), ehr_count);

    // Composition statistics: 2 versioned compositions, 3 version rows (Ehr);
    // a non-Ehr service sees neither.
    assert_eq!(
        svc.versioned_composition_count(PlatformService::Ehr, None)
            .await
            .expect("versioned comp count"),
        2
    );
    assert_eq!(
        svc.composition_version_count(PlatformService::Ehr, None)
            .await
            .expect("comp version count"),
        3
    );
    assert_eq!(
        svc.versioned_composition_count(PlatformService::Demographic, None)
            .await
            .expect("demo versioned comp"),
        0
    );

    // Time range: an upper bound before everything → 0; a lower bound in the
    // future → 0; open bounds → all.
    let past = Some((None, Some("2000-01-01T00:00:00Z".to_owned())));
    assert_eq!(
        svc.admin_contribution_count(PlatformService::Ehr, past)
            .await
            .expect("past range"),
        0
    );
    let future = Some((Some("2999-01-01T00:00:00Z".to_owned()), None));
    assert_eq!(
        svc.admin_contribution_count(PlatformService::Ehr, future)
            .await
            .expect("future range"),
        0
    );

    // An invalid ISO bound → 400 (validated at the adapter before the query).
    let bad = svc
        .admin_contribution_count(
            PlatformService::Ehr,
            Some((Some("not-a-date".to_owned()), None)),
        )
        .await;
    assert!(
        matches!(
            bad,
            Err(SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            })
        ),
        "invalid ISO bound must be BadRequest, got {bad:?}"
    );
}

/// A physically deleted party leaves one erasure tombstone on the party
/// outbox, ahead of every registered reader.
///
/// The party domain carries its own outbox, so a consumer of the party stream
/// learns nothing from the clinical tombstone: every party event it received
/// announced a commit, and without this row the identity simply stops being
/// mentioned. GDPR Art. 19 (`docs/law/eu/gdpr/text.html`) makes the controller
/// communicate the erasure to each recipient the data reached. The row carries
/// no contribution, because the party's contributions went with it, and it
/// names the relationships erased alongside so a consumer that tracked only
/// those is told as well. No openEHR spec governs eventing.
#[tokio::test]
async fn physical_party_delete_tombstones_the_party_outbox() {
    let (_db, pool, svc) = repository().await;

    let p1 = make_person(&svc, "T1").await;
    let p2 = make_person(&svc, "T2").await;
    let rel = svc
        .party_relationship_create(typed(&party_relationship("tr", &p1, &p2)), None)
        .await
        .expect("the relationship commits");
    let rel_id = rel.body["uid"]["value"]
        .as_str()
        .expect("uid")
        .split("::")
        .next()
        .expect("versioned-object id")
        .to_owned();

    sqlx::query(
        "INSERT INTO party.event_outbox_reader (reader, last_seq, active) \
         VALUES ('omop', 0, true)",
    )
    .execute(&pool)
    .await
    .expect("register a reader over the party outbox");

    svc.physical_party_delete(p1.clone())
        .await
        .expect("physical party delete");

    let (seq, envelope): (i64, Value) = sqlx::query_as(
        "SELECT seq, envelope FROM party.event_outbox WHERE contribution_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("exactly one erasure tombstone");
    assert_eq!(envelope["event"], json!("erase"));
    assert_eq!(envelope["ehr_id"], Value::Null);
    assert_eq!(envelope["vo_id"], json!(p1));
    let erased: Vec<(String, String)> = envelope["versions"]
        .as_array()
        .expect("the tombstone names what it erased")
        .iter()
        .map(|v| {
            (
                v["vo_id"].as_str().unwrap_or_default().to_owned(),
                v["kind"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert!(
        erased.contains(&(p1.clone(), "PERSON".to_owned())),
        "the party itself is named under its RM type: {erased:?}"
    );
    assert!(
        erased.contains(&(rel_id, "PARTY_RELATIONSHIP".to_owned())),
        "the relationship erased with it is named too: {erased:?}"
    );
    assert!(
        envelope["versions"]
            .as_array()
            .is_some_and(|v| v.iter().all(|e| e["change_type"] == json!("erase"))),
        "every entry announces an erasure: {envelope}"
    );

    let behind: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.event_outbox_reader WHERE active AND last_seq < $1",
    )
    .bind(seq)
    .fetch_one(&pool)
    .await
    .expect("count the readers still to reach the tombstone");
    assert_eq!(
        behind, 1,
        "every registered reader still receives the tombstone"
    );

    // A party that still exists is never announced as erased.
    let surviving: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.event_outbox \
         WHERE contribution_id IS NULL AND envelope ->> 'vo_id' = $1",
    )
    .bind(&p2)
    .fetch_one(&pool)
    .await
    .expect("count the tombstones naming the surviving party");
    assert_eq!(surviving, 0, "no tombstone for a party that still exists");
}

#[tokio::test]
async fn physical_party_delete_cascades_relationships_and_spares_partner() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let p1 = make_person(&svc, "P1").await;
    let p2 = make_person(&svc, "P2").await;
    let p3 = make_person(&svc, "P3").await;

    // R1: p1 → p2 (references p1 as source). R2: p2 → p1 (references p1 as
    // target). R3: p2 → p3 (does NOT reference p1).
    let r1 = svc
        .party_relationship_create(typed(&party_relationship("r1", &p1, &p2)), None)
        .await
        .expect("r1");
    let r1 = r1.body["uid"]["value"]
        .as_str()
        .unwrap()
        .split("::")
        .next()
        .unwrap()
        .to_owned();
    let r2 = svc
        .party_relationship_create(typed(&party_relationship("r2", &p2, &p1)), None)
        .await
        .expect("r2");
    let r2 = r2.body["uid"]["value"]
        .as_str()
        .unwrap()
        .split("::")
        .next()
        .unwrap()
        .to_owned();
    let r3 = svc
        .party_relationship_create(typed(&party_relationship("r3", &p2, &p3)), None)
        .await
        .expect("r3");
    let r3 = r3.body["uid"]["value"]
        .as_str()
        .unwrap()
        .split("::")
        .next()
        .unwrap()
        .to_owned();

    // No orphaned audits before the delete: every audit row is referenced by a
    // version or contribution.
    let orphan_audits_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.commit_audit a \
         WHERE NOT EXISTS (SELECT 1 FROM party.version v WHERE v.commit_audit_id = a.id) \
           AND NOT EXISTS (SELECT 1 FROM party.contribution c WHERE c.commit_audit_id = a.id)",
    )
    .fetch_one(pool)
    .await
    .expect("orphan audits before");
    assert_eq!(orphan_audits_before, 0);

    // Delete p1 physically (SM physical_party_delete).
    svc.physical_party_delete(p1.clone())
        .await
        .expect("physical party delete");

    // p1 and both relationships referencing it are physically gone.
    assert_eq!(version_rows(pool, &p1).await, 0, "p1 gone");
    assert_eq!(version_rows(pool, &r1).await, 0, "r1 (p1 as source) gone");
    assert_eq!(version_rows(pool, &r2).await, 0, "r2 (p1 as target) gone");

    // The partner party p2 and the unrelated relationship r3 survive.
    assert!(version_rows(pool, &p2).await > 0, "partner p2 survives");
    assert!(version_rows(pool, &r3).await > 0, "unrelated r3 survives");
    assert_eq!(
        svc.party_get(PartyKind::Person, p2.clone(), None)
            .await
            .expect("p2 still readable")
            .body["_type"],
        "PERSON"
    );

    // No orphaned audit rows were left behind (audits swept in the cascade).
    let orphan_audits_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.commit_audit a \
         WHERE NOT EXISTS (SELECT 1 FROM party.version v WHERE v.commit_audit_id = a.id) \
           AND NOT EXISTS (SELECT 1 FROM party.contribution c WHERE c.commit_audit_id = a.id)",
    )
    .fetch_one(pool)
    .await
    .expect("orphan audits after");
    assert_eq!(orphan_audits_after, 0, "no orphaned audits after cascade");

    // An unknown party id → 404 (party_id_does_not_exist), preserved through
    // the ServiceError round-trip.
    let unknown = svc.physical_party_delete(Uuid::now_v7().to_string()).await;
    assert!(
        matches!(
            unknown,
            Err(SmError {
                status: CallStatusType::PartyIdDoesNotExist,
                ..
            })
        ),
        "unknown party must be party_id_does_not_exist, got {unknown:?}"
    );

    // A malformed id → 400.
    let bad = svc.physical_party_delete("not-a-uuid".to_owned()).await;
    assert!(matches!(
        bad,
        Err(SmError {
            status: CallStatusType::PreconditionViolation,
            ..
        })
    ));
}

#[tokio::test]
async fn archive_marks_vos_idempotently_and_reads_stay_unchanged() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let ehr = seed_full_ehr(&svc).await;
    let person = make_person(&svc, "Archie").await;

    // The live VO count BEFORE archiving: the marker set must match it, and
    // reading it afterwards would be vacuous once the rows have moved tiers.
    let ehr_vo_count: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT vo_id) FROM version WHERE ehr_id = $1")
            .bind(ehr)
            .fetch_one(pool)
            .await
            .expect("ehr vo count");
    assert!(ehr_vo_count > 0, "the seeded EHR holds versioned objects");

    // archive_ehrs marks every VO of the EHR.
    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("archive ehr");
    let archived: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM vo_head va \
         WHERE EXISTS (SELECT 1 FROM version v \
                       WHERE v.vo_id = va.vo_id AND v.ehr_id = $1)",
    )
    .bind(ehr)
    .fetch_one(pool)
    .await
    .expect("archived count");
    assert_eq!(archived, ehr_vo_count, "every EHR VO is marked archived");

    // Idempotent: a second archive_ehrs adds nothing and does not error.
    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("re-archive ehr");
    let archived_again: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vo_head WHERE archived_at IS NOT NULL")
            .fetch_one(pool)
            .await
            .expect("total archived");
    assert_eq!(archived_again, ehr_vo_count, "re-archive is idempotent");

    // archive_parties marks the party VO.
    svc.archive_parties(vec![person.clone()])
        .await
        .expect("archive party");
    let party_marked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.vo_head WHERE vo_id = $1::uuid AND archived_at IS NOT NULL",
    )
    .bind(&person)
    .fetch_one(pool)
    .await
    .expect("party archived");
    assert_eq!(party_marked, 1);

    // Reads are UNCHANGED after archival (zero wire drift): the EHR_STATUS and
    // the party still serve their content.
    let status = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status still readable after archive");
    assert_eq!(status["_type"], "EHR_STATUS");
    let party = svc
        .party_get(PartyKind::Person, person.clone(), None)
        .await
        .expect("party still readable after archive");
    assert_eq!(party.body["_type"], "PERSON");

    // All-or-nothing: a batch with one unknown EHR → 404 and nothing new is
    // archived (a fresh EHR paired with a bogus id stays unmarked).
    let fresh = seed_full_ehr(&svc).await;
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vo_head WHERE archived_at IS NOT NULL")
            .fetch_one(pool)
            .await
            .expect("before");
    let res = svc
        .archive_ehrs(vec![fresh.to_string(), Uuid::now_v7().to_string()])
        .await;
    assert!(
        matches!(
            res,
            Err(SmError {
                status: CallStatusType::EhrIdDoesNotExist,
                ..
            })
        ),
        "an unknown EHR aborts the batch, got {res:?}"
    );
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vo_head WHERE archived_at IS NOT NULL")
            .fetch_one(pool)
            .await
            .expect("after");
    assert_eq!(before, after, "all-or-nothing: nothing archived on failure");

    // Unknown party → 404 (`party_id_does_not_exist`).
    let bad_party = svc.archive_parties(vec![Uuid::now_v7().to_string()]).await;
    assert!(matches!(
        bad_party,
        Err(SmError {
            status: CallStatusType::PartyIdDoesNotExist,
            ..
        })
    ));
}

/// Row counts in one relation of one storage tier, for one versioned object.
async fn vo_rows(pool: &PgPool, relation: &str, vo_id: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM {relation} WHERE vo_id = $1::uuid");
    sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(vo_id)
        .fetch_one(pool)
        .await
        .expect("vo row count")
}

/// Row counts in one relation of one storage tier, for an EHR.
async fn tier_rows(pool: &PgPool, relation: &str, ehr: ferroehr::ids::EhrId) -> i64 {
    let sql = format!("SELECT count(*) FROM {relation} WHERE ehr_id = $1");
    sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(ehr)
        .fetch_one(pool)
        .await
        .expect("tier row count")
}

/// SM `I_ADMIN_ARCHIVE` "Move … to archival storage" is a PHYSICAL move: the
/// primary tier shrinks to zero rows for the archived EHR, the cold tier gains
/// exactly those rows, reads keep serving them, and the move reverses.
///
/// No openEHR spec governs storage tiering — our own design/extension; what the
/// SM does fix is the word "Move" and the archived objects' continued
/// existence, which is what the read assertions below pin.
#[tokio::test]
async fn archive_physically_moves_rows_to_the_cold_tier_and_back() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let ehr = seed_full_ehr(&svc).await;
    let other = seed_full_ehr(&svc).await;

    let hot_versions = tier_rows(pool, "version_hot", ehr).await;
    let hot_nodes = tier_rows(pool, "node_hot", ehr).await;
    let other_versions = tier_rows(pool, "version_hot", other).await;
    let other_nodes = tier_rows(pool, "node_hot", other).await;
    assert!(hot_versions > 0 && hot_nodes > 0, "the EHR has stored rows");

    // The composition/EHR_STATUS content served before archiving, to compare
    // against the post-archive read.
    let status_before = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status before archive");

    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("archive ehr");

    // 1. The primary tier shrank to nothing for this EHR; the cold tier holds
    //    exactly what left.
    assert_eq!(tier_rows(pool, "version_hot", ehr).await, 0);
    assert_eq!(tier_rows(pool, "node_hot", ehr).await, 0);
    assert_eq!(tier_rows(pool, "version_cold", ehr).await, hot_versions);
    assert_eq!(tier_rows(pool, "node_cold", ehr).await, hot_nodes);

    // 2. An UNARCHIVED EHR is untouched in both directions.
    assert_eq!(tier_rows(pool, "version_hot", other).await, other_versions);
    assert_eq!(tier_rows(pool, "node_hot", other).await, other_nodes);
    assert_eq!(tier_rows(pool, "version_cold", other).await, 0);
    assert_eq!(tier_rows(pool, "node_cold", other).await, 0);
    svc.get_ehr_status_at_time(other, None)
        .await
        .expect("the unarchived EHR still reads");

    // 3. The archived EHR still serves the same content, now from the cold
    //    tier (SM: archiving MOVES storage, it does not remove the object).
    let status_after = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status after archive");
    assert_eq!(status_before, status_after, "archived read is unchanged");
    let revisions = svc
        .ehr_status_revision_history(ehr)
        .await
        .expect("revision history after archive");
    assert_eq!(revisions["_type"], "REVISION_HISTORY");
    assert!(
        !revisions["items"]
            .as_array()
            .expect("items array")
            .is_empty(),
        "the archived EHR_STATUS keeps its revision history"
    );

    // 4. Re-archiving is still a no-op (nothing left to move).
    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("re-archive");
    assert_eq!(tier_rows(pool, "version_cold", ehr).await, hot_versions);

    // 5. The move reverses exactly, markers included.
    svc.restore_archived_ehrs(vec![ehr.to_string()])
        .await
        .expect("restore ehr");
    assert_eq!(tier_rows(pool, "version_hot", ehr).await, hot_versions);
    assert_eq!(tier_rows(pool, "node_hot", ehr).await, hot_nodes);
    assert_eq!(tier_rows(pool, "version_cold", ehr).await, 0);
    assert_eq!(tier_rows(pool, "node_cold", ehr).await, 0);
    let markers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM vo_head WHERE ehr_id = $1 AND archived_at IS NOT NULL",
    )
    .bind(ehr)
    .fetch_one(pool)
    .await
    .expect("marker count");
    assert_eq!(markers, 0, "restore drops the archive markers");
    assert_eq!(
        svc.get_ehr_status_at_time(ehr, None)
            .await
            .expect("status after restore"),
        status_before
    );
}

/// A physical EHR delete reaches the cold tier: the tier is a partition of the
/// relation the delete names, so the same cascade carries archived rows away.
#[tokio::test]
async fn physical_delete_removes_archived_rows_from_the_cold_tier() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let ehr = seed_full_ehr(&svc).await;
    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("archive ehr");
    assert!(tier_rows(pool, "version_cold", ehr).await > 0);

    svc.admin_ehr_delete(ehr.to_string())
        .await
        .expect("delete archived ehr");

    assert_eq!(tier_rows(pool, "version_cold", ehr).await, 0);
    assert_eq!(tier_rows(pool, "node_cold", ehr).await, 0);
    let markers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM vo_head WHERE ehr_id = $1 AND archived_at IS NOT NULL",
    )
    .bind(Uuid::from(ehr))
    .fetch_one(pool)
    .await
    .expect("marker count");
    assert_eq!(markers, 0);
}

/// A write to an archived object thaws it first, so a versioned object is never
/// split across the two storage tiers (its version history would otherwise be
/// truncated on the next read).
#[tokio::test]
async fn writing_an_archived_object_thaws_it_back_to_the_primary_tier() {
    let (_db, pool, svc) = repository().await;
    let pool = &pool;

    let ehr = svc.create_ehr(None).await.expect("ehr");
    let before = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status get");
    let preceding = uid(&before).to_owned();
    let status_vo = preceding.split("::").next().expect("vo uuid").to_owned();

    // Creation mints an EHR_STATUS and an EHR_ACCESS; archiving moves both.
    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("archive ehr");
    assert_eq!(tier_rows(pool, "version_hot", ehr).await, 0);
    assert_eq!(tier_rows(pool, "version_cold", ehr).await, 2);

    let mut updated = before.clone();
    updated.as_object_mut().expect("status obj").remove("uid");
    svc.replace_ehr_status(ehr, uv(&updated, "251", Some(&preceding)))
        .await
        .expect("update the archived EHR_STATUS");

    // ONLY the written object thaws: the EHR_STATUS is whole in the primary
    // tier (its archived version plus the new one), while the untouched
    // EHR_ACCESS stays archived.
    assert_eq!(vo_rows(pool, "version_hot", &status_vo).await, 2);
    assert_eq!(vo_rows(pool, "version_cold", &status_vo).await, 0);
    assert_eq!(tier_rows(pool, "version_cold", ehr).await, 1);
    assert_eq!(tier_rows(pool, "version_hot", ehr).await, 2);
    let revisions = svc
        .ehr_status_revision_history(ehr)
        .await
        .expect("revision history");
    assert_eq!(
        revisions["items"].as_array().expect("items array").len(),
        2,
        "the thawed object keeps its whole version history"
    );
}

/// The archival tier is a PARTITION of the relation it archives, so a column
/// added to `version` / `node` / `vo_attestation` reaches the cold rows by
/// construction — the mirror relations that had to be kept in step by hand are
/// gone. This test is what pins that: two partitions per relation, one
/// attached for each tier, and no relation of those names outside the domain
/// schemas.
#[tokio::test]
async fn the_archival_tier_is_a_partition_of_the_relation_it_archives() {
    let (_db, pool, _svc) = repository().await;

    for relation in ["version", "node", "vo_attestation"] {
        let partitions: Vec<(String, String)> = sqlx::query_as(
            "SELECT c.relname::text, pg_get_expr(c.relpartbound, c.oid)::text \
             FROM pg_inherits i \
             JOIN pg_class p ON p.oid = i.inhparent \
             JOIN pg_class c ON c.oid = i.inhrelid \
             JOIN pg_namespace n ON n.oid = p.relnamespace \
             WHERE n.nspname = 'clinical' AND p.relname = $1 ORDER BY 1",
        )
        .bind(relation)
        .fetch_all(&pool)
        .await
        .expect("partition list");

        assert_eq!(
            partitions,
            vec![
                (
                    format!("{relation}_cold"),
                    "FOR VALUES IN ('cold')".to_owned()
                ),
                (
                    format!("{relation}_hot"),
                    "FOR VALUES IN ('hot')".to_owned()
                ),
            ],
            "{relation} must carry exactly the hot and cold partitions"
        );
    }

    // And no mirror schema survives to be kept in step by hand.
    let mirrors: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.schemata \
         WHERE schema_name IN ('cold', 'cold_demographic')",
    )
    .fetch_one(&pool)
    .await
    .expect("count the mirror schemas");
    assert_eq!(mirrors, 0, "the mirror schemas are gone");
}

/// Physical erasure leaves no row naming the EHR in any domain, and leaves one
/// tombstone on the outbox for every registered reader.
///
/// The relations are named one by one rather than swept, because the point of
/// the assertion is that a relation ADDED to the schema has to be added here
/// too: the cascade is a property of the foreign-key graph, and a new table
/// that hangs off the EHR without one would pass a sweep that only counts what
/// it knows. The EHR is archived first, so the delete is asserted to reach the
/// cold partition as well as the hot one.
///
/// The tombstone is the GDPR Art. 19 notification: a consumer holds what it
/// derived from this EHR, and its cursor has not reached the tombstone yet, so
/// the row is still ahead of every registered reader.
#[tokio::test]
async fn erasure_leaves_no_row_in_any_domain_and_tombstones_the_outbox() {
    let (_db, pool, svc) = repository().await;

    let erased = seed_full_ehr(&svc).await;
    let kept = seed_full_ehr(&svc).await;

    // The marks, the cross-reference and a reader cursor: every relation the
    // erasure has to reach, populated before it runs.
    svc.restrict_processing(&erased.to_string(), None, "gdpr-18-1-c", None)
        .await
        .expect("record the restriction");
    svc.put_retention_anchor(&erased.to_string(), "nl", None, None)
        .await
        .expect("record the retention anchor");
    svc.link(ferroehr::ids::VoId(Uuid::now_v7()), erased)
        .await
        .expect("open the mapping");
    sqlx::query(
        "INSERT INTO event_outbox_reader (reader, last_seq, active) VALUES ('omop', 0, true)",
    )
    .execute(&pool)
    .await
    .expect("register a reader");

    // The versioned objects of the EHR, captured while they exist: the
    // attestation and blob-reference relations are keyed on them, not on the
    // EHR.
    let vo_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT DISTINCT vo_id FROM version WHERE ehr_id = $1")
            .bind(erased.0)
            .fetch_all(&pool)
            .await
            .expect("the EHR's versioned objects");
    assert!(!vo_ids.is_empty(), "the seed committed versioned objects");
    let commit_audit_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT commit_audit_id FROM version WHERE ehr_id = $1 \
         UNION SELECT commit_audit_id FROM contribution WHERE ehr_id = $1",
    )
    .bind(erased.0)
    .fetch_all(&pool)
    .await
    .expect("the EHR's audit rows");

    // Archived, so the delete has to reach the cold partition too.
    svc.archive_ehrs(vec![erased.to_string()])
        .await
        .expect("archive the EHR");
    let cold: i64 =
        sqlx::query_scalar("SELECT count(*) FROM version WHERE ehr_id = $1 AND tier = 'cold'")
            .bind(erased.0)
            .fetch_one(&pool)
            .await
            .expect("count the archived versions");
    assert!(cold > 0, "the versions really moved to the cold tier");

    svc.admin_ehr_delete(erased.to_string())
        .await
        .expect("admin delete");

    let by_ehr = |sql: &'static str| {
        let pool = pool.clone();
        async move { count_for_ehr(&pool, sql, erased.0).await }
    };
    for (relation, sql) in [
        ("ehr", "SELECT count(*) FROM ehr WHERE id = $1"),
        ("vo_head", "SELECT count(*) FROM vo_head WHERE ehr_id = $1"),
        ("version", "SELECT count(*) FROM version WHERE ehr_id = $1"),
        ("node", "SELECT count(*) FROM node WHERE ehr_id = $1"),
        (
            "contribution",
            "SELECT count(*) FROM contribution WHERE ehr_id = $1",
        ),
        (
            "item_tag",
            "SELECT count(*) FROM item_tag WHERE ehr_id = $1",
        ),
        (
            "ehr_folder",
            "SELECT count(*) FROM ehr_folder WHERE ehr_id = $1",
        ),
        (
            "restriction",
            "SELECT count(*) FROM restriction WHERE ehr_id = $1",
        ),
        (
            "retention_anchor",
            "SELECT count(*) FROM retention_anchor WHERE ehr_id = $1",
        ),
        (
            "linkage.subject_ehr",
            "SELECT count(*) FROM linkage.subject_ehr WHERE ehr_id = $1",
        ),
        (
            "event_outbox (commit events)",
            "SELECT count(*) FROM event_outbox WHERE ehr_id = $1 AND contribution_id IS NOT NULL",
        ),
    ] {
        assert_eq!(
            by_ehr(sql).await,
            0,
            "{relation} still names the erased EHR"
        );
    }
    for (relation, sql) in [
        (
            "vo_attestation",
            "SELECT count(*) FROM vo_attestation WHERE vo_id = ANY($1)",
        ),
        (
            "blob_ref",
            "SELECT count(*) FROM blob_ref WHERE vo_id = ANY($1)",
        ),
    ] {
        let remaining: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(&vo_ids)
            .fetch_one(&pool)
            .await
            .expect("count the rows keyed on the versioned objects");
        assert_eq!(remaining, 0, "{relation} still names an erased object");
    }
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM commit_audit WHERE id = ANY($1)")
        .bind(&commit_audit_ids)
        .fetch_one(&pool)
        .await
        .expect("count the audit rows");
    assert_eq!(audits, 0, "commit_audit still holds the erased EHR's rows");

    // One tombstone, contribution-less, ahead of every registered reader.
    let (seq, envelope): (i64, Value) = sqlx::query_as(
        "SELECT seq, envelope FROM event_outbox \
         WHERE ehr_id = $1 AND contribution_id IS NULL",
    )
    .bind(erased.0)
    .fetch_one(&pool)
    .await
    .expect("exactly one erasure tombstone");
    assert_eq!(envelope["event"], json!("erase"));
    assert_eq!(envelope["ehr_id"], json!(erased.0));
    assert_eq!(envelope["versions"][0]["kind"], json!("EHR"));
    let behind: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox_reader WHERE active AND last_seq < $1",
    )
    .bind(seq)
    .fetch_one(&pool)
    .await
    .expect("count the readers still to reach the tombstone");
    assert_eq!(
        behind, 1,
        "every registered reader still receives the tombstone"
    );

    // The other EHR keeps everything, tombstone included.
    let kept_rows = ehr_rows(&pool, kept.into()).await;
    assert!(!kept_rows.is_empty(), "the other EHR is untouched");
    let kept_tombstones: i64 = count_for_ehr(
        &pool,
        "SELECT count(*) FROM event_outbox WHERE ehr_id = $1 AND contribution_id IS NULL",
        kept.0,
    )
    .await;
    assert_eq!(
        kept_tombstones, 0,
        "no tombstone for an EHR that still exists"
    );
}

/// A blob reference follows its version into the cold tier and dies with the
/// EHR.
///
/// Both halves are the foreign key, not application code: `blob_ref` points at
/// `version (tier, vo_id, sys_version)` with `ON UPDATE CASCADE ON DELETE
/// CASCADE`, so archiving — one `UPDATE version SET tier = 'cold'`, which
/// PostgreSQL performs as a move between partitions
/// (<https://www.postgresql.org/docs/18/ddl-partitioning.html>) — carries the
/// reference across, and the erasure removes it. A reference stranded in the
/// hot partition would make the collector believe a blob is unreferenced while
/// an archived version still holds it.
#[tokio::test]
async fn a_blob_reference_follows_its_version_across_the_tier_move() {
    let (_db, pool, svc) = repository().await;

    let ehr = seed_full_ehr(&svc).await;
    let (vo_id, sys_version): (Uuid, i32) = sqlx::query_as(
        "SELECT vo_id, sys_version FROM version WHERE ehr_id = $1 ORDER BY committed_at LIMIT 1",
    )
    .bind(ehr.0)
    .fetch_one(&pool)
    .await
    .expect("a committed version");
    sqlx::query(
        "INSERT INTO blob_ref (vo_id, sys_version, uri) VALUES ($1, $2, 's3://blobs/cafe')",
    )
    .bind(vo_id)
    .bind(sys_version)
    .execute(&pool)
    .await
    .expect("record the reference");

    svc.archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("archive the EHR");
    let tier: String = sqlx::query_scalar("SELECT tier FROM blob_ref WHERE vo_id = $1")
        .bind(vo_id)
        .fetch_one(&pool)
        .await
        .expect("the reference survived the move");
    assert_eq!(tier, "cold", "the reference followed its version");

    svc.admin_ehr_delete(ehr.to_string())
        .await
        .expect("admin delete");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM blob_ref WHERE vo_id = $1")
        .bind(vo_id)
        .fetch_one(&pool)
        .await
        .expect("count the references");
    assert_eq!(left, 0, "the reference died with the version");
}
