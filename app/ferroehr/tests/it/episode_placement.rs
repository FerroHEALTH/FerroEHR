// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A COMPOSITION is placed in episodes of care by the `episode` elements of
//! its `context.other_context`, and its head row keeps them (draft access
//! control, Episodes of Care; #6413). Each episode it names is the episode
//! of a care relationship of its EHR's subject (#6412).

use ferroehr::db::domain::DomainPools;
use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::versioning::episode::EpisodeKey;
use serde_json::{Value, json};

use crate::care_bounds::{care_of, coded, interval, party, relate};
use crate::fixtures::{commit_audit, committer, composition, uv};
use crate::policy_fixtures::{CARE_PROVIDER, care_provider, create, granting};

/// Every episode the tests of this module place a composition in.
pub(crate) const EPISODES: [(&str, &str); 5] = [
    ("H", "C-17"),
    ("H", "D-4"),
    ("H", "D-9"),
    ("G", "1"),
    ("Hé", "ü\"1"),
];

/// The care relationship of `patient` to `team` in the role `ATND` until the
/// end of 2026, scoped to the episode `(issuer, id)` when one is given.
pub(crate) fn care_in(patient: VoId, team: VoId, episode: Option<(&str, &str)>) -> Value {
    let mut body = care_of(
        patient,
        ("GROUP", &team.to_string()),
        "ATND",
        &interval("2026-11-01", Some("2026-12-31")),
    );
    if let Some((issuer, id)) = episode {
        body["details"]["items"]
            .as_array_mut()
            .expect("the details")
            .push(json!({ "_type": "ELEMENT", "archetype_node_id": "at0005",
                "name": coded(TERMINOLOGY, "episode"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id } }));
    }
    body
}

/// A patient and a care team of [`CARE_PROVIDER`] in `svc`.
pub(crate) async fn patient_and_team(svc: &FerroEhrService) -> (VoId, VoId) {
    let (patient, _) = create(svc, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    care_provider(svc, CARE_PROVIDER).await;
    let (team, _) = create(svc, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    (patient, team)
}

/// A service and the EHR of a patient under the care of a team in each of
/// `episodes`, by one relationship each, with the patient and the team.
pub(crate) async fn cared(
    episodes: &[(&str, &str)],
) -> (testkit::TestDb, FerroEhrService, EhrId, (VoId, VoId)) {
    let db = testkit::db().await.expect("testkit database");
    let svc = granting(FerroEhrService::new(&DomainPools::from_shared(&db.pool()))).await;
    let (patient, team) = patient_and_team(&svc).await;
    for episode in episodes {
        relate(&svc, &care_in(patient, team, Some(*episode)))
            .await
            .expect("the relationship in its episode commits");
    }
    let ehr_id = crate::fixtures::create_ehr_for_party(&svc, patient).await;
    (db, svc, ehr_id, (patient, team))
}

/// [`cared`] in every episode of [`EPISODES`].
async fn setup() -> (testkit::TestDb, FerroEhrService, EhrId) {
    let (db, svc, ehr_id, _) = cared(&EPISODES).await;
    (db, svc, ehr_id)
}

/// An `episode` element naming `issuer` and `id`.
pub(crate) fn episode(issuer: &str, id: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at9001",
        "name": { "_type": "DV_CODED_TEXT", "value": "episode",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
                "code_string": "episode" } },
        "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id } })
}

/// The fixture composition named `name` of `category`, with a context whose
/// `other_context` holds `items` beside a ward element.
pub(crate) fn placed(name: &str, category: &str, items: &[Value]) -> Value {
    let ward = json!({ "_type": "ELEMENT", "archetype_node_id": "at0003",
        "name": { "_type": "DV_TEXT", "value": "Ward" },
        "value": { "_type": "DV_TEXT", "value": "B2" } });
    let items: Vec<Value> = std::iter::once(ward).chain(items.iter().cloned()).collect();
    let mut body = composition(name);
    body["category"]["defining_code"]["code_string"] = json!(category);
    body["context"] = json!({ "_type": "EVENT_CONTEXT",
        "start_time": { "_type": "DV_DATE_TIME", "value": "2026-11-23T10:00:00Z" },
        "setting": { "_type": "DV_CODED_TEXT", "value": "secondary medical care",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "232" } },
        "other_context": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "Tree" },
            "items": [{ "_type": "CLUSTER", "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "Admission" },
                "items": items }] } });
    body
}

/// The keys of `episodes`, sorted as the head row keeps them.
pub(crate) fn keys(episodes: &[(&str, &str)]) -> Vec<Vec<u8>> {
    let mut keys: Vec<Vec<u8>> = episodes
        .iter()
        .map(|(issuer, id)| EpisodeKey::of(issuer, id).as_bytes().to_vec())
        .collect();
    keys.sort_unstable();
    keys
}

/// The `episodes` of the head row of `vo`.
pub(crate) async fn head(db: &testkit::TestDb, vo: VoId) -> Option<Vec<Vec<u8>>> {
    sqlx::query_scalar("SELECT episodes FROM vo_head WHERE vo_id = $1")
        .bind(vo.0)
        .fetch_one(&db.pool())
        .await
        .expect("the head row")
}

/// Whether `error` is a `422`.
pub(crate) fn unprocessable(error: &ServiceError) -> bool {
    matches!(error, ServiceError::Unprocessable { .. })
}

/// A placed commit keeps its episodes, each once; a new version moves them,
/// one without the element clears them, and an unplaced commit holds none.
#[tokio::test]
async fn the_head_row_follows_the_placement_of_the_current_version() {
    let (db, svc, ehr_id) = setup().await;
    let first = placed(
        "note",
        "433",
        &[
            episode("H", "C-17"),
            episode("H", "D-4"),
            episode("H", "C-17"),
        ],
    );
    let created = svc
        .create_composition(ehr_id, uv(&first, "249", None))
        .await
        .expect("a placed commit");
    let vo = created.vo_id;
    assert_eq!(
        head(&db, vo).await,
        Some(keys(&[("H", "C-17"), ("H", "D-4")]))
    );

    let moved = placed("note", "433", &[episode("H", "D-9")]);
    let updated = svc
        .update_composition(ehr_id, vo, uv(&moved, "251", Some(&created.version_uid())))
        .await
        .expect("a moved version");
    assert_eq!(head(&db, vo).await, Some(keys(&[("H", "D-9")])));

    let unplaced = placed("note", "433", &[]);
    svc.update_composition(
        ehr_id,
        vo,
        uv(&unplaced, "251", Some(&updated.version_uid())),
    )
    .await
    .expect("an unplaced version");
    assert_eq!(head(&db, vo).await, None);

    let plain = svc
        .create_composition(ehr_id, uv(&composition("plain"), "249", None))
        .await
        .expect("an unplaced commit");
    assert_eq!(head(&db, plain.vo_id).await, None);
}

/// A persistent composition named in an episode and an episode element that
/// carries no `DV_IDENTIFIER` with an issuer and an id are refused `422`.
#[tokio::test]
async fn a_placed_persistent_composition_and_a_malformed_episode_are_refused() {
    let (_db, svc, ehr_id) = setup().await;
    let persistent = placed("medication list", "431", &[episode("H", "C-17")]);
    let refused = svc
        .create_composition(ehr_id, uv(&persistent, "249", None))
        .await
        .expect_err("a placed persistent composition");
    assert!(unprocessable(&refused), "{refused:?}");

    let mut blank = episode("H", "C-17");
    blank["value"]["id"] = json!(" ");
    let refused = svc
        .create_composition(ehr_id, uv(&placed("note", "433", &[blank]), "249", None))
        .await
        .expect_err("an episode with a blank id");
    assert!(unprocessable(&refused), "{refused:?}");

    svc.create_composition(
        ehr_id,
        uv(&placed("medication list", "431", &[]), "249", None),
    )
    .await
    .expect("an unplaced persistent composition");
}

/// Each member of a CONTRIBUTION keeps its own placement, whichever members
/// around it are placed, and a placed persistent member refuses the whole
/// CONTRIBUTION.
#[tokio::test]
async fn each_member_of_a_contribution_keeps_its_own_placement() {
    let (db, svc, ehr_id) = setup().await;
    let member = |data: Value| {
        json!({ "data": data,
            "lifecycle_state": crate::fixtures::lifecycle("532"),
            "commit_audit": { "change_type": { "value": "creation",
                "defining_code": { "code_string": "249",
                    "terminology_id": { "value": "openehr" } } },
                "committer": committer("T") } })
    };
    let body = json!({ "versions": [
            member(placed("a", "433", &[episode("H", "C-17")])),
            member(composition("b")),
            member(placed("c", "433", &[episode("H", "D-4"), episode("G", "1")])),
        ],
        "audit": commit_audit("249", "creation", "T") });
    let committed = svc
        .create_ehr_contribution(ehr_id, body)
        .await
        .expect("a CONTRIBUTION with placed members");
    let versions = committed.body["versions"]
        .as_array()
        .expect("the versions")
        .clone();
    let vo = |i: usize| -> VoId {
        let id = versions[i]["id"]["value"]
            .as_str()
            .expect("a version uid")
            .split("::")
            .next()
            .expect("the object id");
        VoId(id.parse().expect("a uuid"))
    };
    assert_eq!(head(&db, vo(0)).await, Some(keys(&[("H", "C-17")])));
    assert_eq!(head(&db, vo(1)).await, None);
    assert_eq!(
        head(&db, vo(2)).await,
        Some(keys(&[("H", "D-4"), ("G", "1")]))
    );

    let refused = svc
        .create_ehr_contribution(
            ehr_id,
            json!({ "versions": [
                member(composition("d")),
                member(placed("e", "431", &[episode("H", "C-17")])),
            ],
            "audit": commit_audit("249", "creation", "T") }),
        )
        .await
        .expect_err("a placed persistent member");
    assert!(unprocessable(&refused), "{refused:?}");
}

/// The head recomputation of the import and archive-load paths reads the
/// same episodes off a stored body as the commit does.
#[tokio::test]
async fn the_head_recomputation_reads_the_episodes_the_commit_wrote() {
    let (db, svc, ehr_id) = setup().await;
    let body = placed(
        "note",
        "433",
        &[
            episode("H", "C-17"),
            episode("Hé", "ü\"1"),
            episode("H", "C-17"),
        ],
    );
    let vo = svc
        .create_composition(ehr_id, uv(&body, "249", None))
        .await
        .expect("a placed commit")
        .vo_id;
    let written = head(&db, vo).await;
    assert_eq!(written, Some(keys(&[("H", "C-17"), ("Hé", "ü\"1")])));
    recompute(&db, vo).await;
    assert_eq!(head(&db, vo).await, written);
}

/// Clears the head row's episodes of `vo` and recomputes the row from its
/// versions.
async fn recompute(db: &testkit::TestDb, vo: VoId) {
    let pool = db.pool();
    let mut conn = ferroehr::db::acquire::acquire(&pool)
        .await
        .expect("a connection");
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .expect("a transaction");
    sqlx::query("UPDATE vo_head SET episodes = NULL WHERE vo_id = $1")
        .bind(vo.0)
        .execute(&mut *tx)
        .await
        .expect("clear the episodes");
    ferroehr::storage::version_repo::import::sync_heads(&mut tx, &[vo])
        .await
        .expect("recompute the head");
    tx.commit().await.expect("commit");
}

/// A placed composition keeps its episodes through an EHR Extract import
/// into a CDR that holds its subject's relationship, and through an archive
/// load, which lands the relationship with the record.
#[tokio::test]
async fn an_import_and_an_archive_load_keep_the_placement() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = granting(FerroEhrService::new(&DomainPools::from_shared(
        &source_db.pool(),
    )))
    .await;
    let (patient, team) = patient_and_team(&source).await;
    relate(&source, &care_in(patient, team, Some(("H", "C-17"))))
        .await
        .expect("the relationship in its episode commits");
    let parties = crate::admin_fixture::archive_dir();
    source
        .export_ehrs(
            parties.clone(),
            ferroehr::service::admin::types::ExportSpec::canonical_json(1),
        )
        .await
        .expect("the export of the parties");
    let ehr_id = crate::fixtures::create_ehr_for_party(&source, patient).await;
    let body = placed("note", "433", &[episode("H", "C-17")]);
    let vo = source
        .create_composition(ehr_id, uv(&body, "249", None))
        .await
        .expect("a placed commit")
        .vo_id;
    let expected = Some(keys(&[("H", "C-17")]));

    let (imported_db, _imported_pool, imported) = crate::admin_fixture::repository().await;
    let imported = granting(imported).await;
    let reports = imported
        .load_ehrs(parties.clone())
        .await
        .expect("the parties load");
    assert!(reports.is_empty(), "every party loads: {reports:?}");
    std::fs::remove_dir_all(&parties).expect("remove the archive");
    let mut extracts = source.extract_ehrs(ehr_id).await.expect("the extract");
    let extract =
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("the typed extract");
    imported
        .import_ehr(None, extract)
        .await
        .expect("the import");
    assert_eq!(head(&imported_db, vo).await, expected);

    let (loaded_db, _pool, loaded) = crate::admin_fixture::repository().await;
    let dir = crate::admin_fixture::archive_dir();
    source
        .export_ehrs(
            dir.clone(),
            ferroehr::service::admin::types::ExportSpec::canonical_json(1),
        )
        .await
        .expect("the export");
    loaded.load_ehrs(dir.clone()).await.expect("the load");
    assert_eq!(head(&loaded_db, vo).await, expected);
    std::fs::remove_dir_all(&dir).expect("remove the archive");
}

/// An EHR Extract whose persistent composition names an episode is refused
/// `422`, and nothing of it is imported.
#[tokio::test]
async fn an_import_of_a_placed_persistent_composition_is_refused() {
    let (_source_db, source) = crate::fixtures::service().await;
    let ehr_id = crate::fixtures::create_ehr(&source).await;
    source
        .create_composition(
            ehr_id,
            uv(&placed("medication list", "431", &[]), "249", None),
        )
        .await
        .expect("an unplaced persistent commit");
    let mut extract = source
        .extract_ehrs(ehr_id)
        .await
        .expect("the extract")
        .remove(0);
    let items = extract["chapters"][0]["items"]
        .as_array_mut()
        .expect("the chapter items");
    let composition = items
        .iter_mut()
        .find(|it| it["item"]["_type"] == json!("X_VERSIONED_COMPOSITION"))
        .expect("the composition");
    composition["item"]["versions"][0]["data"]["context"]["other_context"]["items"][0]["items"]
        .as_array_mut()
        .expect("the cluster items")
        .push(episode("H", "C-17"));
    let extract = openehr_its::json::from_canonical_value(&extract).expect("the typed extract");

    let (_target_db, target) = crate::fixtures::service().await;
    let refused = target
        .import_ehr(None, extract)
        .await
        .expect_err("a placed persistent composition");
    assert!(unprocessable(&refused), "{refused:?}");
}
