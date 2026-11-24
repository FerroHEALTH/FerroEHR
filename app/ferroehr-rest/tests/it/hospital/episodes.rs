// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care decided per datum by the episode of the relationship it rests on
//! (#6414, #6415; draft access control package, Episodes of Care): Dr.
//! Jansen, once her only live relationship to Eva is scoped to the
//! cardiology episode, reads that episode and the unplaced data and nothing
//! of the psychiatry episode, by a named read and by a query in every form
//! the statement reads care in. A replica that hears no notification
//! decides her very next request so, with no window in which the other
//! episode shows. Our own design over the draft package.

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferroehr::policy_register::listen::Listeners;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::{AuditHandle, start};
use http::StatusCode;
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;

use crate::common::{BASE, EXT};
use crate::hospital::abac::answered;
use crate::hospital::build::{audit, composition, world};
use crate::hospital::world::{
    Ask, Doc, ENCOUNTER, STAFF, World, admit, ask, call, care, day, replica, router, vo_of,
};

/// The issuer of the hospital's episodes of care.
const EPISODES: &str = "urn:sint-example:episodes";

/// The placement or relationship element naming the episode `id`.
fn episode(at: &str, id: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": at,
        "name": { "_type": "DV_CODED_TEXT", "value": "episode",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
                "code_string": "episode" } },
        "value": { "_type": "DV_IDENTIFIER", "issuer": EPISODES, "id": id } })
}

/// The ADT system writes a relationship of Eva with `team` in the role
/// `PART` from two days ago, scoped to the episode `id`.
pub(crate) async fn relate_in(world: &World, team: &str, id: &str) {
    let mut body = care("PART", &world.eva, &world.teams[team], (&day(-2), None));
    body["details"]["items"]
        .as_array_mut()
        .expect("the details")
        .push(episode("at0005", id));
    let answer = call(
        &world.app,
        "client:adt",
        "POST",
        &format!("{EXT}/demographic/party_relationship"),
        None,
        Some(&body),
    )
    .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
}

/// `caller` commits a note into Eva's EHR placed in the episode `id`.
pub(crate) async fn placed(world: &World, caller: &str, id: &str) -> Doc {
    let display = STAFF
        .iter()
        .find(|staff| staff.name == caller)
        .expect("a member of staff")
        .display;
    let mut body = composition(ENCOUNTER, &format!("Note in {id}"), display);
    body["context"] = json!({ "_type": "EVENT_CONTEXT",
        "start_time": { "_type": "DV_DATE_TIME", "value": "2026-11-23T10:00:00Z" },
        "setting": { "_type": "DV_CODED_TEXT", "value": "secondary medical care",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "232" } },
        "other_context": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "Tree" },
            "items": [episode("at9001", id)] } });
    let answer = call(
        &world.app,
        caller,
        "POST",
        &format!("{BASE}/ehr/{}/composition", world.ehr),
        Some("TREAT"),
        Some(&body),
    )
    .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    let version = answer.etag();
    Doc {
        vo: vo_of(&version),
        version,
    }
}

/// A second replica none of whose copies hears a notification, with its
/// service, its audit drain and the database its listeners connect to.
async fn deaf(world: &World) -> (Arc<FerroEhrService>, Router, AuditHandle, testkit::TestDb) {
    let (sender, drain) = start(audit(), None, world.pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(replica(&world.config, &world.pool, Some(sender)).await);
    let elsewhere = testkit::db().await.expect("a second database");
    let listened: Vec<_> = svc
        .policy_listeners()
        .into_iter()
        .map(|(_, listened)| listened)
        .collect();
    Listeners::on(
        PgConnectOptions::from_str(elsewhere.url()).expect("the test DSN parses"),
        &listened,
    )
    .spawn();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !listened.iter().all(|listened| listened.is_listening()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the listeners connect"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let app = router(&world.config, &svc).await;
    (svc, app, drain, elsewhere)
}

/// The versioned compositions `caller` reads on `app` by a query over every
/// EHR, or scoped to Eva's, sorted.
async fn queried(world: &World, app: &Router, caller: &str, scoped: bool) -> Vec<String> {
    let body = json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" });
    let answer = ask(
        app,
        caller,
        ("POST", &format!("{BASE}/query/aql")),
        Ask {
            purpose: Some("TREAT"),
            body: Some(&body),
            scope: scoped.then_some(world.ehr.as_str()),
        },
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let mut ids: Vec<String> = answer.json()["rows"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    ids.sort();
    ids
}

/// The read of `doc` by `caller` on `app`.
async fn read_on(
    world: &World,
    app: &Router,
    caller: &str,
    doc: &Doc,
) -> crate::hospital::world::Answer {
    call(
        app,
        caller,
        "GET",
        &format!("{BASE}/ehr/{}/composition/{}", world.ehr, doc.vo),
        Some("TREAT"),
        None,
    )
    .await
}

/// Dr. Jansen reads both episodes while a relationship of hers names none;
/// once it ends, the replica that heard nothing refuses her the psychiatry
/// episode on its very next named read and leaves it out of its very next
/// query, scoped or not, read from the census and then from the care
/// projection, and she keeps the cardiology episode and the unplaced data.
#[tokio::test]
async fn an_episode_relationship_covers_its_episode_on_every_read_with_no_window() {
    let mut world = Box::pin(world()).await;
    relate_in(&world, "Cardiology", "C-17").await;
    relate_in(&world, "Psychiatry", "D-4").await;
    let cardiology = placed(&world, "jansen", "C-17").await;
    let psychiatry = placed(&world, "smit", "D-4").await;
    let (other_svc, other, _drain, _elsewhere) = deaf(&world).await;

    let before = queried(&world, &other, "jansen", false).await;
    assert!(before.contains(&cardiology.vo), "{before:?}");
    assert!(
        before.contains(&psychiatry.vo),
        "a whole-record relationship"
    );
    assert_eq!(queried(&world, &other, "jansen", true).await, before);
    answered(
        &read_on(&world, &other, "jansen", &psychiatry).await,
        StatusCode::OK,
        None,
    );

    let (ward, old_from, old_until) = (day(-2), day(-40), day(-20));
    admit(
        &mut world,
        &[
            ("Cardiology", &old_from, Some(&old_until)),
            ("Ward 3", &ward, None),
            ("Psychiatry", &ward, None),
        ],
    )
    .await;
    assert!(
        !other_svc.care_projection_current(),
        "the census, which the replica reads before its projection"
    );
    let expected: Vec<String> = before
        .iter()
        .filter(|vo| **vo != psychiatry.vo)
        .cloned()
        .collect();
    answered(
        &read_on(&world, &other, "jansen", &psychiatry).await,
        StatusCode::FORBIDDEN,
        Some("refused-outside-episode"),
    );
    answered(
        &read_on(&world, &other, "jansen", &cardiology).await,
        StatusCode::OK,
        None,
    );
    assert_eq!(
        queried(&world, &other, "jansen", false).await,
        expected,
        "the census"
    );
    assert_eq!(
        queried(&world, &other, "jansen", true).await,
        expected,
        "the pinned EHR"
    );

    world
        .svc
        .project_care()
        .await
        .expect("the projection is written");
    other_svc
        .project_care()
        .await
        .expect("the projection is read");
    assert!(
        other_svc.care_projection_current(),
        "the projection is read"
    );
    assert_eq!(
        queried(&world, &other, "jansen", false).await,
        expected,
        "the projection"
    );
}

/// The versioned compositions of the answer of Dr. Jansen's query over every
/// EHR carrying the caseload members `members`, and its status.
async fn in_caseload(world: &World, members: &Value) -> (StatusCode, Vec<String>, String) {
    let mut body = json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" });
    if let (Some(body), Some(members)) = (body.as_object_mut(), members.as_object()) {
        body.extend(members.clone());
    }
    let answer = ask(
        &world.app,
        "jansen",
        ("POST", &format!("{BASE}/query/aql")),
        Ask {
            purpose: Some("TREAT"),
            body: Some(&body),
            scope: None,
        },
    )
    .await;
    if answer.status != StatusCode::OK {
        return (answer.status, Vec::new(), answer.body);
    }
    let mut ids: Vec<String> = answer.json()["rows"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    ids.sort();
    (answer.status, ids, answer.body)
}

/// `caseload_episodes` narrows each EHR of Dr. Jansen's caseload to the
/// cardiology episode, the unplaced data and nothing of the psychiatry
/// episode, though a relationship of hers names no episode; an episode none
/// of her live relationships names, and the member with no caseload, are
/// refused `400` naming it (draft ITS-REST page, Queries and the Caseload).
#[tokio::test]
async fn caseload_episodes_narrows_the_caseload_to_the_named_episodes() {
    let world = Box::pin(world()).await;
    relate_in(&world, "Cardiology", "C-17").await;
    relate_in(&world, "Psychiatry", "D-4").await;
    let cardiology = placed(&world, "jansen", "C-17").await;
    let psychiatry = placed(&world, "smit", "D-4").await;
    let (status, whole, body) = in_caseload(&world, &json!({ "caseload": "team" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(whole.contains(&psychiatry.vo), "{whole:?}");
    let named = format!("{EPISODES}|C-17");
    let (status, narrowed, body) = in_caseload(
        &world,
        &json!({ "caseload": "team", "caseload_episodes": [named] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let expected: Vec<String> = whole
        .iter()
        .filter(|vo| **vo != psychiatry.vo)
        .cloned()
        .collect();
    assert_eq!(narrowed, expected);
    assert!(narrowed.contains(&cardiology.vo), "{narrowed:?}");
    for members in [
        json!({ "caseload": "team", "caseload_episodes": [format!("{EPISODES}|D-4")] }),
        json!({ "caseload": "none", "caseload_episodes": [named] }),
        json!({ "caseload": "team", "caseload_episodes": ["C-17"] }),
    ] {
        let (status, _, body) = in_caseload(&world, &members).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{members}: {body}");
        assert!(body.contains("caseload_episodes"), "{members}: {body}");
    }
}
