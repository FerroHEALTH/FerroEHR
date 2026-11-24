// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A care relationship scoped to an episode of care (#6412; draft access
//! control package, Episodes of Care): its episode is kept in the caller
//! facts, the care copy and the care projection; a malformed episode, and
//! none where the policy requires one, are refused `422`; and a composition
//! placed in an episode no relationship of its EHR's subject names is
//! refused `422` on every write path. Our own design over our own model.

use serde_json::{Value, json};

use crate::care_bounds::{policy, relate};
use crate::episode_placement::{
    care_in, cared, episode, head, keys, patient_and_team, placed, unprocessable,
};
use crate::fixtures::{commit_audit, committer, composition, uv};
use crate::policy_fixtures::{as_admin, granting};
use ferroehr::db::domain::DomainPools;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::episode::EpisodeKey;

/// The episode of the one care relationship the `PARTY_RELATIONSHIP` `vo`
/// holds, as the caller facts keep it.
async fn stored(pool: &sqlx::PgPool, vo: ferroehr::ids::VoId) -> Option<(String, String)> {
    sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT episode_issuer, episode_id FROM party.binding_caller_fact \
         WHERE holder_id = $1 AND kind = 'under_care_of'",
    )
    .bind(vo.0)
    .fetch_one(pool)
    .await
    .map(|(issuer, id)| issuer.zip(id))
    .expect("the caller fact reads")
}

/// The episode digests the care projection holds for the subject of `ehr`.
async fn projected(pool: &sqlx::PgPool, ehr: ferroehr::ids::EhrId) -> Vec<Option<Vec<u8>>> {
    sqlx::query_scalar(
        "SELECT c.episode_digest FROM ehr e JOIN access_care c \
         ON c.subject_digest = e.subject_digest WHERE e.id = $1 ORDER BY c.episode_digest",
    )
    .bind(ehr.0)
    .fetch_all(pool)
    .await
    .expect("the projection reads")
}

/// A relationship's episode reaches the caller facts, the care copy and the
/// projection; one without an episode holds none.
#[tokio::test]
async fn a_relationships_episode_is_kept_in_the_facts_the_copy_and_the_projection() {
    let (db, svc, ehr_id, (patient, team)) = cared(&[("H", "C-17")]).await;
    let pool = db.pool();
    let (plain, _) = relate(&svc, &care_in(patient, team, None))
        .await
        .expect("a relationship without an episode commits");
    assert_eq!(stored(&pool, plain).await, None);
    let held = svc
        .grants()
        .under_care_of(patient.0)
        .await
        .expect("the care copy reads");
    let mut episodes: Vec<Option<(String, String)>> = held
        .iter()
        .map(|care| {
            care.episode()
                .map(|episode| (episode.issuer.clone(), episode.id.clone()))
        })
        .collect();
    episodes.sort();
    assert_eq!(
        episodes,
        vec![None, Some(("H".to_owned(), "C-17".to_owned()))]
    );
    svc.grants().refresh().await.expect("the copy reads");
    svc.project_care().await.expect("the projection is written");
    assert_eq!(
        projected(&pool, ehr_id).await,
        vec![Some(EpisodeKey::of("H", "C-17").as_bytes().to_vec()), None]
    );
}

/// An episode with no issuer and id is refused `422`, and so is a
/// relationship without one that the policy's covering entry marks
/// `episode_required`.
#[tokio::test]
async fn a_malformed_episode_and_a_missing_required_one_are_refused() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = granting(FerroEhrService::new(&DomainPools::from_shared(&pool))).await;
    let (patient, team) = patient_and_team(&svc).await;
    let mut blank = care_in(patient, team, Some(("H", "C-17")));
    blank["details"]["items"][2]["value"]["issuer"] = json!(" ");
    let refused = relate(&svc, &blank)
        .await
        .expect_err("an episode with a blank issuer");
    assert!(unprocessable(&refused), "{refused:?}");

    let mut required = policy("P30D", &json!({}));
    required["relationship_validity"][0]["episode_required"] = json!(true);
    svc.put_access_policy(&PolicyPrecondition::Create, &required)
        .await
        .expect("the policy");
    let refused = relate(&svc, &care_in(patient, team, None))
        .await
        .expect_err("a relationship the policy requires an episode of");
    assert!(unprocessable(&refused), "{refused:?}");
    assert!(
        format!("{refused:?}").contains("episode_required"),
        "{refused:?}"
    );
    let (scoped, _) = relate(&svc, &care_in(patient, team, Some(("H", "C-17"))))
        .await
        .expect("one naming an episode commits");
    assert_eq!(
        stored(&pool, scoped).await,
        Some(("H".to_owned(), "C-17".to_owned()))
    );
}

/// A COMPOSITION member of a CONTRIBUTION.
fn member(data: &Value) -> Value {
    json!({ "data": data,
        "lifecycle_state": crate::fixtures::lifecycle("532"),
        "commit_audit": { "change_type": { "value": "creation",
            "defining_code": { "code_string": "249",
                "terminology_id": { "value": "openehr" } } },
            "committer": committer("T") } })
}

/// A composition placed in an episode no care relationship of the subject
/// names is refused `422` on create, update and CONTRIBUTION, and one
/// placed in a declared episode, or unplaced, commits.
#[tokio::test]
async fn an_undeclared_episode_is_refused_on_every_commit() {
    let (db, svc, ehr_id, _) = cared(&[("H", "C-17")]).await;
    let undeclared = placed("note", "433", &[episode("H", "C-17"), episode("H", "X-1")]);
    let refused = svc
        .create_composition(ehr_id, uv(&undeclared, "249", None))
        .await
        .expect_err("an undeclared episode");
    assert!(unprocessable(&refused), "{refused:?}");

    let declared = placed("note", "433", &[episode("H", "C-17")]);
    let created = svc
        .create_composition(ehr_id, uv(&declared, "249", None))
        .await
        .expect("a declared episode commits");
    assert_eq!(head(&db, created.vo_id).await, Some(keys(&[("H", "C-17")])));
    let refused = svc
        .update_composition(
            ehr_id,
            created.vo_id,
            uv(&undeclared, "251", Some(&created.version_uid())),
        )
        .await
        .expect_err("an update into an undeclared episode");
    assert!(unprocessable(&refused), "{refused:?}");
    assert_eq!(head(&db, created.vo_id).await, Some(keys(&[("H", "C-17")])));

    let refused = svc
        .create_ehr_contribution(
            ehr_id,
            json!({ "versions": [member(&composition("a")), member(&undeclared)],
                "audit": commit_audit("249", "creation", "T") }),
        )
        .await
        .expect_err("a member in an undeclared episode");
    assert!(unprocessable(&refused), "{refused:?}");
    svc.create_ehr_contribution(
        ehr_id,
        json!({ "versions": [member(&composition("b")), member(&declared)],
            "audit": commit_audit("249", "creation", "T") }),
    )
    .await
    .expect("declared and unplaced members commit");
}

/// An EHR Extract import into a CDR that holds no relationship of the
/// subject in the episode, and an archive whose relationship no longer names
/// it, are refused `422` before anything is written.
#[tokio::test]
async fn an_import_and_a_load_of_an_undeclared_episode_are_refused() {
    let (_db, source, ehr_id, (patient, team)) = cared(&[]).await;
    let (relationship, version) = relate(&source, &care_in(patient, team, Some(("H", "C-17"))))
        .await
        .expect("the relationship in its episode commits");
    source
        .create_composition(
            ehr_id,
            uv(&placed("note", "433", &[episode("H", "C-17")]), "249", None),
        )
        .await
        .expect("a declared episode commits");

    let mut extracts = source.extract_ehrs(ehr_id).await.expect("the extract");
    let extract =
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("the typed extract");
    let (_target_db, target) = crate::fixtures::service().await;
    let refused = target
        .import_ehr(None, extract)
        .await
        .expect_err("an import whose episode the target holds no relationship in");
    assert!(unprocessable(&refused), "{refused:?}");

    let mut write = WriteRequest::new(crate::typed_body::typed(&care_in(patient, team, None)));
    write.lifecycle_state = Some("532".to_owned());
    as_admin(async {
        Box::pin(source.party_relationship_update(
            relationship,
            crate::fixtures::if_match(&version),
            write,
        ))
        .await
    })
    .await
    .expect("the relationship no longer names the episode");
    let dir = crate::admin_fixture::archive_dir();
    source
        .export_ehrs(
            dir.clone(),
            ferroehr::service::admin::types::ExportSpec::canonical_json(1),
        )
        .await
        .expect("the export");
    let (_loaded_db, _pool, loaded) = crate::admin_fixture::repository().await;
    let refused = loaded
        .load_ehrs(dir.clone())
        .await
        .expect_err("an archive whose relationship names no episode the record is placed in");
    std::fs::remove_dir_all(&dir).expect("remove the archive");
    assert!(unprocessable(&refused), "{refused:?}");
}
