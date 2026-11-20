// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care relationship resources at the wire (#6076): written and read by
//! party and by subject reference, versioned with their `AUDIT_DETAILS`
//! under `If-Match` and `ETag` (ITS-REST overview `Requests_and_responses.md`
//! §"If-Match and accidental overwrites"), each read decided per subject it
//! reveals and logged once per subject (draft access control package,
//! ITS-REST Resources, Care Relationships; GDPR Art 4(15), Art 9). No openEHR
//! spec defines these resources: our own design over the draft package.

use std::time::{Duration, Instant};

use http::StatusCode;
use serde_json::{Value, json};

use crate::common::EXT;
use crate::hospital::build::world;
use crate::hospital::world::{
    Answer, World, call, call_matching, care, coded_in, day, party_ref, vo_of,
};

/// A care relationship of Eva in the role `role`, from today, naming Dr.
/// Jansen's `PERSON` directly.
fn direct(world: &World, role: &str) -> Value {
    let mut body = care(
        role,
        &world.eva,
        &world.teams["Cardiology"],
        (&day(0), None),
    );
    body["target"] = party_ref("PERSON", &world.people["jansen"].person);
    body
}

/// The `care_role` code of the relationship `body`.
fn role_of(body: &Value) -> Value {
    body["details"]["items"][0]["value"]["defining_code"]["code_string"].clone()
}

/// The containers of the relationships a list answered, sorted.
fn containers(answer: &Answer) -> Vec<String> {
    let mut ids: Vec<String> = answer
        .json()
        .as_array()
        .unwrap_or_else(|| panic!("a list: {}", answer.body))
        .iter()
        .map(|body| vo_of(body["uid"]["value"].as_str().expect("a uid")))
        .collect();
    ids.sort();
    ids
}

/// The containers of Eva's three ward stays, sorted.
fn stays(world: &World) -> Vec<String> {
    let mut ids: Vec<String> = world.stays.values().map(|(id, _)| id.clone()).collect();
    ids.sort();
    ids
}

/// The access-log entries the request `request_id` wrote, by EHR id, once
/// a marker request made after it has landed.
async fn entries_of(world: &World, request_id: &str) -> Vec<String> {
    let marker = call(
        &world.app,
        "root",
        "GET",
        &format!("{}/ehr/{}", crate::common::BASE, world.other_ehr),
        Some("HOPERAT"),
        None,
    )
    .await
    .request_id();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT s.request_id, r.ehr_id::text FROM audit.access_log_record r \
             JOIN audit.access_log_search s ON s.chain_seq = r.chain_seq \
             WHERE r.record_type = 'ACCESS_LOG_ENTRY' ORDER BY r.chain_seq",
        )
        .fetch_all(&world.pool)
        .await
        .expect("read the access log");
        if rows
            .iter()
            .any(|(id, _)| id.as_deref() == Some(marker.as_str()))
        {
            return rows
                .into_iter()
                .filter(|(id, _)| id.as_deref() == Some(request_id))
                .filter_map(|(_, ehr)| ehr)
                .collect();
        }
        assert!(Instant::now() < deadline, "the marker access never landed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `GET` of `uri` as `caller`, declaring treatment.
async fn read(world: &World, caller: &str, uri: &str) -> Answer {
    call(&world.app, caller, "GET", uri, Some("TREAT"), None).await
}

#[tokio::test]
async fn a_care_relationship_is_written_read_and_versioned_by_party_and_by_subject() {
    let world = Box::pin(world()).await;
    let by_party = format!("{EXT}/demographic/person/{}/care_relationship", world.eva);
    let created = call(
        &world.app,
        "client:adt",
        "POST",
        &by_party,
        None,
        Some(&direct(&world, "ATND")),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let first = created.etag();
    let id = vo_of(&first);
    assert!(first.ends_with("::1"), "{first}");
    assert_eq!(
        crate::common::header(&created.headers, "location"),
        Some(format!("{EXT}/care_relationship/{first}").as_str())
    );
    let one = format!("{EXT}/care_relationship/{id}");
    let got = read(&world, "client:adt", &one).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.body);
    assert_eq!(got.etag(), first);
    assert_eq!(role_of(&got.json()), json!("ATND"));

    let second = updates(&world, &one, &first).await;
    refuses(&world, (&by_party, &one), &second).await;
    lists(&world, &by_party, &id).await;
    versions(&world, &id, &first).await;
    deletes(&world, (&by_party, &one), (&id, &second)).await;
}

/// Changes the relationship `one` over `first` with `If-Match` alone, and
/// refuses a stale one; the second version.
async fn updates(world: &World, one: &str, first: &str) -> String {
    let mut changed = direct(world, "RESP");
    changed["time_validity"]["upper"] = json!({ "_type": "DV_DATE", "value": day(10) });
    changed["time_validity"]["upper_unbounded"] = json!(false);
    let unconditional = call(&world.app, "client:adt", "PUT", one, None, Some(&changed)).await;
    assert_eq!(
        unconditional.status,
        StatusCode::BAD_REQUEST,
        "If-Match is required: {}",
        unconditional.body
    );
    let updated = call_matching(&world.app, "client:adt", ("PUT", one), first, &changed).await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    let second = updated.etag();
    assert!(second.ends_with("::2"), "{second}");
    assert_eq!(role_of(&updated.json()), json!("RESP"));
    let stale = call_matching(&world.app, "client:adt", ("PUT", one), first, &changed).await;
    assert_eq!(
        stale.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        stale.body
    );
    assert_eq!(
        stale.etag(),
        second,
        "the refusal names the current version"
    );

    second
}

/// The relationship over `second` moved to another subject, and a body that
/// is no care relationship, are refused `422`.
async fn refuses(world: &World, (by_party, one): (&str, &str), second: &str) {
    let changed = direct(world, "RESP");
    let mut elsewhere = changed.clone();
    elsewhere["source"] = party_ref("PERSON", &world.people["frank"].person);
    let moved = call_matching(&world.app, "client:adt", ("PUT", one), second, &elsewhere).await;
    assert_eq!(
        moved.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        moved.body
    );
    let mut not_care = changed.clone();
    not_care["name"] = coded_in("local", "at0001");
    let wrong = call(
        &world.app,
        "client:adt",
        "POST",
        by_party,
        None,
        Some(&not_care),
    )
    .await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.body
    );
}

/// The list by party and by subject reference holds `id` beside the stays.
async fn lists(world: &World, by_party: &str, id: &str) {
    let mut expected = stays(world);
    expected.push(id.to_owned());
    expected.sort();
    let listed = read(world, "client:adt", by_party).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(containers(&listed), expected);
    let by_subject = format!(
        "{EXT}/care_relationship?subject_id={}&subject_namespace=demographic",
        world.eva
    );
    assert_eq!(
        containers(&read(world, "client:adt", &by_subject).await),
        expected
    );
}

/// The versioned object, its history with an `AUDIT_DETAILS` per version,
/// and the first version of `id`.
async fn versions(world: &World, id: &str, first: &str) {
    let versioned = format!("{EXT}/versioned_care_relationship/{id}");
    let container = read(world, "client:adt", &versioned).await;
    assert_eq!(container.status, StatusCode::OK, "{}", container.body);
    let history = read(
        world,
        "client:adt",
        &format!("{versioned}/revision_history"),
    )
    .await;
    assert_eq!(history.status, StatusCode::OK, "{}", history.body);
    let items = history.json()["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(items.len(), 2, "{}", history.body);
    assert!(
        items
            .iter()
            .all(|item| item["audits"][0]["_type"] == "AUDIT_DETAILS"),
        "each version carries its AUDIT_DETAILS: {}",
        history.body
    );
    let version = read(world, "client:adt", &format!("{versioned}/version/{first}")).await;
    assert_eq!(version.status, StatusCode::OK, "{}", version.body);
    assert_eq!(role_of(&version.json()["data"]), json!("ATND"));
}

/// The deletion of `id` over `second` needs `If-Match`, leaves the list and
/// keeps its earlier versions.
async fn deletes(world: &World, (by_party, one): (&str, &str), (id, second): (&str, &str)) {
    let versioned = format!("{EXT}/versioned_care_relationship/{id}");
    let unconditional = call(&world.app, "client:adt", "DELETE", one, None, None).await;
    assert_eq!(
        unconditional.status,
        StatusCode::BAD_REQUEST,
        "{}",
        unconditional.body
    );
    let deleted = crate::hospital::world::served(
        &world.app,
        crate::common::req("DELETE", one)
            .auth(crate::hospital::world::credential("client:adt"))
            .if_match(format!("\"{second}\""))
            .build(),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::NO_CONTENT, "{}", deleted.2);
    assert_eq!(
        containers(&read(world, "client:adt", by_party).await),
        stays(world)
    );
    let gone = read(world, "client:adt", one).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    let kept = read(
        world,
        "client:adt",
        &format!("{versioned}/version/{second}"),
    )
    .await;
    assert_eq!(
        kept.status,
        StatusCode::OK,
        "its earlier versions keep it: {}",
        kept.body
    );
}

#[tokio::test]
async fn a_relationship_is_revealed_only_to_who_may_read_the_record_and_logged_per_subject() {
    let world = Box::pin(world()).await;
    let created = call(
        &world.app,
        "client:adt",
        "POST",
        &format!("{EXT}/demographic/person/{}/care_relationship", world.eva),
        None,
        Some(&direct(&world, "ATND")),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = vo_of(&created.etag());
    let by_party = format!("{EXT}/demographic/person/{}/care_relationship", world.eva);
    let mut all = stays(&world);
    all.push(id.clone());
    all.sort();

    // Dr. Jansen treats Eva in Cardiology and reads her record.
    let admitted = read(&world, "jansen", &by_party).await;
    assert_eq!(admitted.status, StatusCode::OK, "{}", admitted.body);
    assert_eq!(containers(&admitted), all);
    assert_eq!(
        entries_of(&world, &admitted.request_id()).await,
        vec![world.ehr.clone()],
        "one entry for the one subject revealed"
    );

    // Dr. Bakker has no relationship to Eva and may not read her record.
    let refused = read(&world, "bakker", &by_party).await;
    assert_eq!(refused.status, StatusCode::OK, "{}", refused.body);
    assert_eq!(refused.json(), json!([]), "left out of the list");
    assert_eq!(
        entries_of(&world, &refused.request_id()).await,
        Vec::<String>::new()
    );
    for uri in [
        format!("{EXT}/care_relationship/{id}"),
        format!("{EXT}/versioned_care_relationship/{id}"),
        format!("{EXT}/versioned_care_relationship/{id}/revision_history"),
    ] {
        let answer = read(&world, "bakker", &uri).await;
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{uri}: {}",
            answer.body
        );
        assert!(
            !answer.body.contains("ATND") && !answer.body.contains("HL7_ParticipationType"),
            "no role reaches a caller who may not read the record: {}",
            answer.body
        );
    }
    let reverse = format!(
        "{EXT}/care_relationship?carer={}",
        world.people["jansen"].person
    );
    assert_eq!(read(&world, "bakker", &reverse).await.json(), json!([]));

    // The ADT system writes the hospital's relationships, so it reads them
    // without reading the record, and each read is still logged.
    let writer = read(&world, "client:adt", &by_party).await;
    assert_eq!(containers(&writer), all);
    assert_eq!(
        entries_of(&world, &writer.request_id()).await,
        vec![world.ehr.clone()]
    );
}

#[tokio::test]
async fn the_reverse_reading_finds_the_relationships_that_reach_a_carer() {
    let world = Box::pin(world()).await;
    let created = call(
        &world.app,
        "client:adt",
        "POST",
        &format!("{EXT}/demographic/person/{}/care_relationship", world.eva),
        None,
        Some(&direct(&world, "ATND")),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = vo_of(&created.etag());
    let jansen = &world.people["jansen"].person;
    let reverse = |query: &str| format!("{EXT}/care_relationship?carer={jansen}{query}");
    assert_eq!(
        containers(&read(&world, "client:adt", &reverse("&path=direct")).await),
        vec![id.clone()]
    );
    let cardiology = world.stays["Cardiology"].0.clone();
    assert_eq!(
        containers(&read(&world, "client:adt", &reverse("&path=team")).await),
        vec![cardiology.clone()]
    );
    let mut both = vec![id.clone(), cardiology];
    both.sort();
    let all = read(&world, "client:adt", &reverse("")).await;
    assert_eq!(containers(&all), both);
    assert_eq!(
        entries_of(&world, &all.request_id()).await,
        vec![world.ehr.clone()],
        "one entry per subject, whatever the relationships revealed of it"
    );
    assert_eq!(
        containers(
            &read(
                &world,
                "client:adt",
                &reverse("&role=HL7_ParticipationType%7CATND")
            )
            .await
        ),
        vec![id]
    );
    let page = read(&world, "client:adt", &reverse("&offset=1&fetch=1")).await;
    assert_eq!(containers(&page).len(), 1, "{}", page.body);
    let ended = read(&world, "client:adt", &reverse(&format!("&at={}", day(-30)))).await;
    assert_eq!(ended.json(), json!([]), "nothing held thirty days ago");
    let bad = read(&world, "client:adt", &reverse("&path=household")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
}
