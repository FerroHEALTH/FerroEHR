// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A key issuer added and removed across two replicas of one deployment on
//! one PostgreSQL, with neither stopped (#5688): a rolling deploy that adds a
//! persons issuer serves the old issuer on both replicas throughout, answers
//! the new issuer's logins `503` until every live replica names it, and binds
//! them on both once the last old replica is replaced; a replica restarted
//! without the issuer stops its logins binding on the other replica at once.
//! Driven through the assembled router over real `FerroEhrService`s. No
//! openEHR spec governs the side table: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::party_binding::fill::Lease;
use ferroehr::party_binding::{BindingMapping, IdentifierSelector};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::party::config::PartySection;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{AUDIENCE, ISSUER, LOGIN, config, router_with, uid, vo, write};

/// The persons issuer the rolling deploy adds.
const ADDED: &str = "https://added.example";
/// The `identifier_issuer` of its `[[auth.party]]` table.
const ADDED_LOGIN: &str = "https://added.example/tenant";
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";

/// The professional mapping holding keys under `issuers`.
fn mapping(issuers: &[&str]) -> BindingMapping {
    let key = IdentifierSelector::new(
        format!("/details[at0001]/items[{CLUSTER}]/items[at0001]"),
        "guid",
    )
    .expect("the key selector compiles")
    .with_issuers(issuers.iter().copied());
    BindingMapping::default()
        .with_person("FerroEHR professional", key)
        .with_professional_role("FerroEHR professional role")
}

/// The `[auth]` table of a replica trusting `ISSUER` and, with `added`, the
/// added issuer too, each with its `[[auth.party]]` table.
fn trusting(added: bool) -> AppConfig {
    let mut config = config(None);
    if added {
        let shared = &mut config.auth.shared;
        shared.issuer.push(IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            ..common::entry(ADDED, AUDIENCE)
        });
        shared.party.push(PartySection {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(ADDED),
            identifier_issuer: ADDED_LOGIN.to_owned(),
            identifier_type: "guid".to_owned(),
            ..PartySection::default()
        });
    }
    config
}

/// A replica over `pool`: its service under `issuers`, its lease, and its
/// router trusting the added issuer when `issuers` names its key issuer.
async fn replica(pool: &sqlx::PgPool, issuers: &[&str]) -> (Arc<FerroEhrService>, Lease, Router) {
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
            .with_binding_mapping(mapping(issuers)),
    );
    let lease = svc
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the replica boots")
        .expect("a lease");
    let source = Arc::new(svc.party_source("demographic"));
    let app = router_with(
        Arc::clone(&svc),
        trusting(issuers.contains(&ADDED_LOGIN)),
        source,
    )
    .await;
    (svc, lease, app)
}

/// A bearer of `issuer` whose `sub` is `subject`.
fn token(issuer: &str, subject: &str) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({ "sub": subject, "iss": issuer, "aud": AUDIENCE, "exp": exp }))
}

/// The status and `Retry-After` of a template listing as `credential`.
async fn listed(app: &Router, credential: &str) -> (StatusCode, Option<String>) {
    let (status, headers, _) = common::send(
        app,
        common::req("GET", format!("{BASE}/definition/template/adl1.4"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await;
    let retry = headers
        .get(http::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    (status, retry)
}

/// Gives the professional `PERSON` at `uid`, whose body is `body`, the
/// second key `key` under the added issuer.
async fn add_key(svc: &FerroEhrService, uid: &str, body: &Value, key: &str) -> String {
    let mut body = body.clone();
    if let Some(items) = body["details"]["items"][0]["items"].as_array_mut() {
        items.push(json!({
            "_type": "ELEMENT", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "key" },
            "value": { "_type": "DV_IDENTIFIER", "issuer": ADDED_LOGIN, "id": key,
                       "type": "guid" }
        }));
    }
    let updated = Box::pin(svc.party_update(
        PartyKind::Person,
        vo(uid),
        Precondition::parse(Some(uid), None).expect("an If-Match"),
        write(&body, "532", Some(uid)),
    ))
    .await
    .expect("the PERSON updates");
    self::uid(&updated)
}

#[tokio::test]
async fn a_rolling_deploy_adds_and_removes_a_key_issuer_with_no_replica_stopped() {
    let (_db, pool) = common::migrated_pool().await;
    let (old, old_lease, on_old) = replica(&pool, &[LOGIN]).await;
    let (login, added) = (
        "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4e",
        "1b2c3d4e-5f6a-4b7c-9d8e-0f1a2b3c4d5f",
    );
    let (uid, body) = crate::party_binding_http::professional(&old, login, "532").await;
    add_key(&old, &uid, &body, added).await;
    let (persons, adds) = (token(ISSUER, login), token(ADDED, added));
    assert_eq!(listed(&on_old, &persons).await.0, StatusCode::OK);

    // The first replica restarts on the configuration that adds the issuer:
    // both serve the old issuer, and the new one answers 503 with a
    // Retry-After until the other replica names it too.
    let (new, new_lease, on_new) = replica(&pool, &[LOGIN, ADDED_LOGIN]).await;
    assert_eq!(listed(&on_new, &persons).await.0, StatusCode::OK);
    assert_eq!(listed(&on_old, &persons).await.0, StatusCode::OK);
    new_lease.tend().await.expect("tended");
    let (status, retry) = listed(&on_new, &adds).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        retry.is_some(),
        "a 503 of a pending key space names a Retry-After"
    );

    // The second replica restarts on it too: the key space fills on the next
    // tend and the added issuer's login binds on both.
    old_lease.release().await.expect("released");
    drop(on_old);
    let (_, replaced_lease, on_replaced) = replica(&pool, &[LOGIN, ADDED_LOGIN]).await;
    new_lease.tend().await.expect("tended");
    assert_eq!(listed(&on_new, &adds).await.0, StatusCode::OK);
    assert_eq!(listed(&on_replaced, &adds).await.0, StatusCode::OK);

    // One replica restarts without the issuer: its logins stop binding on the
    // other at once, which keeps serving the first issuer.
    replaced_lease.release().await.expect("released");
    drop(on_replaced);
    let (dropped, dropped_lease, on_dropped) = replica(&pool, &[LOGIN]).await;
    let (status, _) = listed(&on_new, &adds).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(listed(&on_new, &persons).await.0, StatusCode::OK);
    assert_eq!(listed(&on_dropped, &persons).await.0, StatusCode::OK);
    assert_eq!(
        listed(&on_dropped, &adds).await.0,
        StatusCode::UNAUTHORIZED,
        "the replica that dropped the issuer trusts its tokens no more"
    );

    // Once no replica names it, its key space goes.
    new_lease.release().await.expect("released");
    drop((new, on_new));
    dropped_lease.tend().await.expect("tended");
    let spaces = dropped.binding_key_spaces().await.expect("the key spaces");
    assert_eq!(
        spaces.iter().map(|s| s.issuer.as_str()).collect::<Vec<_>>(),
        [LOGIN]
    );
}
