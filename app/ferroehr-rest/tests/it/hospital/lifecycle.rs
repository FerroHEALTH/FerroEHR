// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RBAC rules and the SCIM lifecycle of the hospital scenario, one test
//! each: a nurse reads the vital signs and commits only the templates her
//! role allows; a role SCIM removes, a deactivation, a reactivation, a
//! deletion and a Group membership change each decide the next request,
//! with no restart (RFC 7644 §3.5.2, §3.6). Each access's entry in the access
//! log is checked. Who may do what is our own design (#5433, #5487).

use http::StatusCode;
use serde_json::{Value, json};

use crate::common::BASE;
use crate::hospital::abac::{answered, read};
use crate::hospital::build::{
    composition, directive, eva_directives, lab_result, naming, settle, world,
};
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::world::{
    ROOT, SCIM, STAFF, VITALS, World, call, capability, day, named, patch_op, read_party, roster,
    scim, text, update_party,
};

/// SCIM sets the User of `name` `active` or not.
async fn set_active(world: &World, name: &str, active: bool) {
    let path = format!("/Users/{}", world.people[name].person);
    let body = patch_op(json!([{ "op": "replace", "path": "active", "value": active }]));
    let (status, answer) = scim(&world.app, SCIM, "PATCH", &path, Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "{name} active = {active}: {answer}");
}

/// The answers of `name` on the vital signs, a query of Eva's record and
/// her EHR, each refused: what "refused everywhere" means for a person.
async fn refused_everywhere(world: &World, name: &str, why: &str) {
    for (method, uri) in [
        (
            "GET",
            format!("{BASE}/ehr/{}/composition/{}", world.ehr, world.vitals.vo),
        ),
        ("GET", format!("{BASE}/ehr/{}", world.ehr)),
        ("GET", format!("{BASE}/ehr/{}", world.other_ehr)),
        ("GET", format!("{BASE}/demographic/person/{}", world.eva)),
    ] {
        let answer = call(&world.app, name, method, &uri, Some("TREAT"), None).await;
        assert_eq!(
            answer.status,
            StatusCode::FORBIDDEN,
            "{name} {why} on {uri}: {}",
            answer.body
        );
    }
    let body = json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" });
    let answer = call(
        &world.app,
        name,
        "POST",
        &format!("{BASE}/query/aql"),
        Some("TREAT"),
        Some(&body),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::FORBIDDEN,
        "{name} {why} on a query: {}",
        answer.body
    );
}

const TREATMENT: Expect = Expect::granted(&["treatment relationship"]);

/// A nurse reads the vital signs of a patient in her ward's care, on her
/// role and the care relationship.
#[tokio::test]
async fn a_nurse_reads_the_vital_signs() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        &world,
        "the vitals",
        &answer,
        Expect::granted(&["role", "treatment relationship"]),
    );
    ledger.verify(&world).await;
}

/// The nurse's role lets her commit the vital signs template and no other:
/// her commit of a vital signs record is accepted and her commit of the
/// diagnosis summary is refused `403` (#5624).
#[tokio::test]
async fn a_nurse_cannot_commit_a_diagnosis_template_her_role_does_not_allow() {
    let world = Box::pin(world()).await;
    let role = &world.people["devries"].role;
    let (version, mut body) = read_party(&world.app, "role", role).await;
    let mut capabilities = body["capabilities"].as_array().cloned().unwrap_or_default();
    capabilities.push(capability(
        "commit_composition",
        &[named("template", &text(VITALS))],
    ));
    body["capabilities"] = Value::Array(capabilities);
    let answer = update_party(&world.app, ROOT, ("role", role, &version), &body).await;
    assert_eq!(answer.status, StatusCode::OK, "her role: {}", answer.body);
    let commit = |body: Value| {
        let world = &world;
        async move {
            call(
                &world.app,
                "devries",
                "POST",
                &format!("{BASE}/ehr/{}/composition", world.ehr),
                Some("TREAT"),
                Some(&body),
            )
            .await
        }
    };
    let vitals = commit(composition(VITALS, "Vital signs", "Bram de Vries")).await;
    assert_eq!(
        vitals.status,
        StatusCode::CREATED,
        "her vital signs: {}",
        vitals.body
    );
    let mut diagnosis = lab_result();
    diagnosis["composer"] = json!({ "_type": "PARTY_IDENTIFIED", "name": "Bram de Vries" });
    let refused = commit(diagnosis).await;
    assert_eq!(
        refused.status,
        StatusCode::FORBIDDEN,
        "a diagnosis: {}",
        refused.body
    );
}

/// SCIM removes the nurse's role and her next request is refused (#5625).
#[tokio::test]
async fn a_role_removed_through_scim_stops_working_on_the_next_request() {
    let world = Box::pin(world()).await;
    let path = format!("/Users/{}", world.people["devries"].person);
    let grant =
        patch_op(json!([{ "op": "add", "path": "roles", "value": [{ "value": "nurse" }] }]));
    let (status, answer) = scim(&world.app, SCIM, "PATCH", &path, Some(&grant)).await;
    assert_eq!(status, StatusCode::OK, "the role: {answer}");
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    let remove = patch_op(json!([{ "op": "remove", "path": "roles[value eq \"nurse\"]" }]));
    let (status, answer) = scim(&world.app, SCIM, "PATCH", &path, Some(&remove)).await;
    assert_eq!(status, StatusCode::OK, "the removal: {answer}");
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    assert_eq!(
        answer.status,
        StatusCode::FORBIDDEN,
        "the next request: {}",
        answer.body
    );
}

/// A User SCIM deactivates is refused everywhere on the next request, and
/// reactivated reads again on the next.
#[tokio::test]
async fn a_deactivated_user_is_refused_everywhere_until_reactivated() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    set_active(&world, "devries", false).await;
    refused_everywhere(&world, "devries", "deactivated").await;
    set_active(&world, "devries", true).await;
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "reactivated", &answer, TREATMENT);
    ledger.verify(&world).await;
}

/// A User SCIM deletes is refused everywhere on the next request.
#[tokio::test]
async fn a_deleted_user_is_refused_everywhere() {
    let world = Box::pin(world()).await;
    let path = format!("/Users/{}", world.people["devries"].person);
    let (status, answer) = scim(&world.app, SCIM, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the deletion: {answer}");
    refused_everywhere(&world, "devries", "deleted").await;
}

/// A User SCIM creates signs in on the next request, and once the ADT system
/// rosters her on Ward 3 she reads the vital signs.
#[tokio::test]
async fn a_user_scim_creates_reads_on_the_next_request() {
    let mut world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let before = read(&world, "nieuw", &world.vitals, Some("TREAT")).await;
    assert_eq!(
        before.status,
        StatusCode::FORBIDDEN,
        "before: {}",
        before.body
    );
    let provisioned = crate::hospital::build::user(&world.app, SCIM, "nieuw", "Nina Nieuw").await;
    world.people.insert("nieuw", provisioned);
    let answer = read(&world, "nieuw", &world.vitals, Some("TREAT")).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-no-care-relationship"),
    );
    ledger.note(
        &world,
        "not rostered",
        &answer,
        Expect::refused("refused-no-care-relationship", 5),
    );
    let role = &world.people["nieuw"].role;
    let (version, mut body) = read_party(&world.app, "role", role).await;
    let nurse = STAFF
        .iter()
        .find(|staff| staff.name == "devries")
        .expect("a nurse");
    body["capabilities"] = json!([crate::hospital::build::facts(nurse)]);
    let cleared = update_party(&world.app, ROOT, ("role", role, &version), &body).await;
    assert_eq!(cleared.status, StatusCode::OK, "her role: {}", cleared.body);
    let rostered = roster(&world, "client:adt", "nieuw", &["Ward 3"], (&day(-1), None)).await;
    assert_eq!(
        rostered.status,
        StatusCode::OK,
        "her roster: {}",
        rostered.body
    );
    let answer = read(&world, "nieuw", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "rostered", &answer, TREATMENT);
    ledger.verify(&world).await;
}

/// A directive naming the SCIM Group Ward 3 binds its members: the nurse is
/// refused while SCIM keeps her in it, and reads again on the next request
/// after SCIM removes her.
#[tokio::test]
async fn a_scim_group_change_decides_the_next_request() {
    let mut world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let ward = world.teams["Ward 3"].clone();
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": naming("GROUP", &ward) }),
    );
    let settings = eva_directives(&world, None, &[deny]);
    settle(&mut world, &settings).await;
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-consent-denied"),
    );
    ledger.note(
        &world,
        "a member",
        &answer,
        Expect::refused("refused-consent-denied", 3),
    );
    let member = &world.people["devries"].person;
    let remove =
        patch_op(json!([{ "op": "remove", "path": format!("members[value eq \"{member}\"]") }]));
    let (status, body) = scim(
        &world.app,
        SCIM,
        "PATCH",
        &format!("/Groups/{ward}"),
        Some(&remove),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the removal: {body}");
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "no longer a member", &answer, TREATMENT);
    ledger.verify(&world).await;
}
