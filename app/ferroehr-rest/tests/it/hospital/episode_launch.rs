// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A SMART on openEHR session launched in an episode of care (#6417):
//! `[auth.smart] episode_launch` reads the token's `episodeId` as
//! `issuer|id` and narrows every request of the session to that episode,
//! the unplaced data and the shared data, as `caseload_episodes` of that one
//! episode; an episode no live relationship of the caller names is refused
//! `400` (ITS-REST SMART App Launch, Experimental: Episode Context; draft
//! ITS-REST page, Queries and the Caseload). Our own design over both.

use std::sync::Arc;

use axum::Router;
use http::StatusCode;
use serde_json::json;

use crate::common::{self, BASE};
use crate::hospital::abac::answered;
use crate::hospital::build::{audit, world};
use crate::hospital::episodes::{placed, relate_in};
use crate::hospital::world::{
    AUDIENCE, Answer, Doc, PERSONS, PURPOSE, STAFF, World, external_id, replica, router, served,
};

/// A replica of the world's deployment with `[auth.smart] episode_launch`
/// on, with its service and its audit drain.
pub(crate) async fn launching(
    world: &World,
) -> (
    Arc<ferroehr::service::FerroEhrService>,
    Router,
    ferroehr::system_log::sender::AuditHandle,
) {
    let (sender, drain) = ferroehr::system_log::sender::start(audit(), None, world.pool.clone())
        .await
        .expect("the audit sender");
    let mut config = world.config.clone();
    config.auth.smart.episode_launch = true;
    let svc = Arc::new(replica(&config, &world.pool, Some(sender)).await);
    let app = router(&config, &svc).await;
    (svc, app, drain)
}

/// Dr. Jansen's access token, launched in the episode `episode` when one is
/// given.
pub(crate) fn jansen(episode: Option<&str>) -> String {
    let staff = STAFF
        .iter()
        .find(|staff| staff.name == "jansen")
        .expect("Dr. Jansen");
    let groups: Vec<String> = staff.groups.iter().map(|team| external_id(team)).collect();
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut claims = json!({ "iss": PERSONS, "aud": AUDIENCE, "exp": exp, "sub": "jansen",
        "roles": staff.roles, "groups": groups });
    if let Some(episode) = episode {
        claims["episodeId"] = json!(episode);
    }
    common::bearer(&claims)
}

/// `method uri` on `app` with `token`, declaring treatment.
async fn send(
    app: &Router,
    token: &str,
    (method, uri): (&str, &str),
    body: Option<&str>,
) -> Answer {
    let mut req = common::req(method, uri)
        .auth(token.to_owned())
        .accept("application/json")
        .prefer_representation()
        .header(PURPOSE, "TREAT");
    if let Some(body) = body {
        req = req.json(body.to_owned());
    }
    let (status, headers, body) = served(app, req.build()).await;
    Answer {
        status,
        headers,
        body,
    }
}

/// The read of `doc` in Eva's EHR with `token`.
async fn read(world: &World, app: &Router, token: &str, doc: &Doc) -> Answer {
    let uri = format!("{BASE}/ehr/{}/composition/{}", world.ehr, doc.vo);
    send(app, token, ("GET", &uri), None).await
}

/// The versioned compositions a query over every EHR answers `token`.
async fn queried(app: &Router, token: &str) -> Vec<String> {
    let body = json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" });
    let uri = format!("{BASE}/query/aql");
    let answer = send(app, token, ("POST", &uri), Some(&body.to_string())).await;
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

/// Launched in the cardiology episode, Dr. Jansen reads that episode and the
/// unplaced data and is refused the psychiatry episode on a named read and
/// in a query, though a relationship of hers names no episode; launched in
/// an episode none of her relationships names, every request is refused
/// `400`; with no launch she reads both.
#[tokio::test]
async fn a_launched_episode_narrows_every_request_of_the_session() {
    let world = Box::pin(world()).await;
    relate_in(&world, "Cardiology", "C-17").await;
    relate_in(&world, "Psychiatry", "D-4").await;
    let cardiology = placed(&world, "jansen", "C-17").await;
    let psychiatry = placed(&world, "smit", "D-4").await;
    let (_svc, app, _drain) = launching(&world).await;
    let unlaunched = jansen(None);
    let whole = queried(&app, &unlaunched).await;
    assert!(whole.contains(&psychiatry.vo), "{whole:?}");
    answered(
        &read(&world, &app, &unlaunched, &psychiatry).await,
        StatusCode::OK,
        None,
    );

    let launched = jansen(Some("urn:sint-example:episodes|C-17"));
    answered(
        &read(&world, &app, &launched, &psychiatry).await,
        StatusCode::FORBIDDEN,
        Some("refused-outside-episode"),
    );
    answered(
        &read(&world, &app, &launched, &cardiology).await,
        StatusCode::OK,
        None,
    );
    answered(
        &read(&world, &app, &launched, &world.encounter).await,
        StatusCode::OK,
        None,
    );
    let expected: Vec<String> = whole
        .iter()
        .filter(|vo| **vo != psychiatry.vo)
        .cloned()
        .collect();
    assert_eq!(queried(&app, &launched).await, expected);

    let elsewhere = jansen(Some("urn:sint-example:episodes|D-4"));
    let refused = read(&world, &app, &elsewhere, &cardiology).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert!(refused.body.contains("episodeId"), "{}", refused.body);
}
