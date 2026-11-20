// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A person's own record, by the party binding (#6106): the caller is the
//! subject of an EHR whose `EHR_STATUS.subject` names the `PERSON` its
//! authenticated identity is bound to, whatever roles it holds and with no
//! purpose of use (EHDS Art. 3). A person holding no role reaches only those
//! records, and a lookup by subject finds no one else's EHR. The access log
//! records each access as the subject's own. Our own design.

use http::StatusCode;
use serde_json::json;

use crate::common::{self, BASE};
use crate::hospital::abac::answered;
use crate::hospital::build::world;
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::world::{AUDIENCE, Answer, PERSONS, World, call, served};

/// The token an authorization server's patient picker signs for `name`: the
/// person's login, a `user/` read scope, and no role.
fn picker(name: &str) -> String {
    common::bearer(&json!({
        "iss": PERSONS, "aud": AUDIENCE, "exp": jiff::Timestamp::now().as_second() + 3600,
        "sub": name, "scope": "user/composition-*.r",
    }))
}

/// `GET uri` with `token`, declaring no purpose.
async fn get(world: &World, token: &str, uri: &str) -> Answer {
    let req = common::req("GET", uri)
        .auth(token.to_owned())
        .accept("application/json")
        .prefer_representation()
        .build();
    let (status, headers, body) = served(&world.app, req).await;
    Answer {
        status,
        headers,
        body,
    }
}

/// The lookup of the EHR whose subject is the `PERSON` `person`.
fn lookup(person: &str) -> String {
    format!("{BASE}/ehr?subject_id={person}&subject_namespace=demographic")
}

/// Gina, holding no role on the picker's token, looks up her own EHR and
/// reads it as its subject.
#[tokio::test]
async fn a_person_with_no_role_looks_up_their_own_ehr() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let token = picker("gina");
    let answer = get(&world, &token, &lookup(&world.people["gina"].person)).await;
    answered(&answer, StatusCode::OK, None);
    assert_eq!(
        answer.json()["ehr_id"]["value"],
        json!(world.other_ehr),
        "the lookup finds her own EHR"
    );
    let answer = get(&world, &token, &format!("{BASE}/ehr/{}", world.other_ehr)).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        &world,
        "her own EHR",
        &answer,
        Expect::granted(&["subject of care"]),
    );
    ledger.verify(&world).await;
}

/// The same token finds no one else's EHR by subject, which answers as an
/// unknown subject does, and reads no one else's EHR by id.
#[tokio::test]
async fn a_person_with_no_role_reaches_no_one_elses_ehr() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let token = picker("gina");
    let answer = get(&world, &token, &lookup(&world.eva)).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    let unknown = get(&world, &token, &lookup(&uuid::Uuid::now_v7().to_string())).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.body);
    assert_eq!(
        answer.json()["message"],
        unknown.json()["message"],
        "another person's EHR answers as no EHR does"
    );
    let answer = get(&world, &token, &format!("{BASE}/ehr/{}", world.ehr)).await;
    answered(&answer, StatusCode::FORBIDDEN, Some("refused-not-subject"));
    ledger.note(
        &world,
        "Eva's EHR",
        &answer,
        Expect::refused("refused-not-subject", 0),
    );
    ledger.verify(&world).await;
}

/// Gina, a professional holding her roles, reads her own record as its
/// subject with no purpose of use, while Dr. Jansen, whose record Eva's is
/// not, reads it under the policy's rule for a purpose.
#[tokio::test]
async fn a_professional_reads_their_own_record_as_its_subject() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let own = format!("{BASE}/ehr/{}", world.other_ehr);
    let answer = call(&world.app, "gina", "GET", &own, None, None).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        &world,
        "her own EHR",
        &answer,
        Expect::granted(&["subject of care"]),
    );
    let evas = format!("{BASE}/ehr/{}", world.ehr);
    let answer = call(&world.app, "jansen", "GET", &evas, None, None).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-purpose-missing"),
    );
    ledger.verify(&world).await;
}
