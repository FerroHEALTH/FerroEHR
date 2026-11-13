// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants at the wire (#5435): a system client's request carries
//! the operations granted to the `ACTOR` its token's issuer and `client_id`
//! name, on the deployment's own archetypes,
//! a grant an administrator changes through the Demographic API decides the
//! caller's next request on every replica with no restart, a capability
//! above `[authz.grants] allowed` is refused at its commit, and a party store
//! that cannot answer is `503`. A client's `query_system_wide` grant makes
//! its `system/aql-*` scope count and its `access_demographic` grant admits
//! it to the DEMOGRAPHIC API, each read per request (#5437). No openEHR spec
//! gives a capability an authorization meaning: our own design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferroehr::party_binding::grants::capabilities::Capabilities;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferrohealth_smart::profile::Preset;
use http::{StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;

use crate::caller_kinds_http::{audience, bearer, machines};
use crate::common::{self, BASE};

/// The machines issuer the system clients' tokens come from.
const MACHINES: &str = "https://grants.machines.example";

/// One replica over `pool`: the policy definitions seeded and the policy
/// parties guarded as the server installs them, `root` an administrator over
/// Basic, and the system clients of [`MACHINES`] admitted, honouring the
/// grants `ceiling` allows. Its party pool is `party` when given, and it
/// listens for grant changes when `listen`.
async fn replica(
    pool: &PgPool,
    party: Option<PgPool>,
    listen: bool,
    ceiling: GrantCeiling,
) -> Router {
    let mut svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool));
    svc.seed_default_definitions()
        .await
        .expect("the policy definitions are seeded");
    let config = ferroehr::config::FerroEhrConfig::default();
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], "ADMIN")
        .expect("the mapping compiles");
    svc = svc
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(crate::party_binding_http::professionals(mapping))
        .with_grant_ceiling(ceiling);
    if let Some(party) = party {
        svc = svc.with_demographic_pool(party);
    }
    let svc = Arc::new(svc);
    if listen {
        let listened = svc.grants().listened();
        listened.spawn_listener();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !listened.is_listening() {
            assert!(
                std::time::Instant::now() < deadline,
                "the grant listener connects"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let mut app = common::api_config(true);
    common::with_basic_users(&mut app, &svc, pool, &[("root", &["ADMIN"])]).await;
    app.auth
        .shared
        .issuer
        .push(machines(MACHINES, Preset::Rfc9068));
    common::router_with(app, svc).await
}

/// The app-only token of the client `client_id`.
fn system(client_id: &str) -> String {
    bearer(
        MACHINES,
        &audience(MACHINES),
        &json!({ "sub": client_id, "client_id": client_id,
                 "scope": "system/composition-*.cruds system/template-*.r" }),
    )
}

/// The status and the capabilities the request of `credential` ran under.
async fn granted(app: &Router, credential: &str) -> (StatusCode, Option<Arc<Capabilities>>) {
    let resp = app
        .clone()
        .oneshot(
            common::req("GET", format!("{BASE}/definition/template/adl1.4"))
                .auth(credential)
                .accept("application/json")
                .build(),
        )
        .await
        .expect("a response");
    (
        resp.status(),
        resp.extensions().get::<Arc<Capabilities>>().cloned(),
    )
}

/// The capabilities of the request of `credential`, which is served.
async fn capabilities(app: &Router, credential: &str) -> Arc<Capabilities> {
    let (status, capabilities) = granted(app, credential).await;
    assert_eq!(status, StatusCode::OK);
    capabilities.expect("an authenticated request carries its capabilities")
}

/// The templates the grants of `capabilities` write, sorted.
fn written(capabilities: &Capabilities) -> Vec<String> {
    let mut out: Vec<String> = capabilities
        .grants()
        .iter()
        .flat_map(|grant| grant.credentials().texts("template"))
        .map(str::to_owned)
        .collect();
    out.sort();
    out
}

/// `method` of `body` on the demographic `path` as the administrator, over
/// `if_match` when given; the status, the version uid of the `ETag` and the
/// body.
async fn write(
    app: &Router,
    method: &str,
    path: &str,
    if_match: Option<&str>,
    body: &Value,
) -> (StatusCode, String, String) {
    let mut req = common::req(method, format!("{BASE}/demographic/{path}"))
        .auth(common::basic_credential("root"))
        .json(body.to_string());
    if let Some(uid) = if_match {
        req = req.if_match(format!("\"{uid}\""));
    }
    let (status, headers, text) = common::send(app, req.build()).await;
    let uid = headers
        .get(header::ETAG)
        .and_then(|etag| etag.to_str().ok())
        .map(|etag| etag.trim_start_matches("W/").trim_matches('"').to_owned())
        .unwrap_or_default();
    (status, uid, text)
}

/// The container of the version uid `uid`.
fn vo(uid: &str) -> &str {
    uid.split_once("::").map_or(uid, |(vo, _)| vo)
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A `DV_CODED_TEXT` of the operation terminology.
fn operation_code(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

/// An `AGENT` of the deployment's own archetype for the client `client_id`.
fn agent(client_id: &str) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-AGENT.lab_system.v1";
    json!({
        "_type": "AGENT", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Laboratory system"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("client"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": MACHINES, "id": client_id,
                           "type": "client_id" } }] },
        "identities": identities(client_id)
    })
}

/// A `ROLE` of the deployment's own archetype the `AGENT` `performer`
/// performs, granting `operation` for `template`, relayed, valid until
/// `until` when given.
fn role(performer: &str, operation: &str, template: &str, until: Option<&str>) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.lab_feed.v1";
    let mut role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Laboratory feed"),
        "identities": identities("laboratory feed"),
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
                       "id": { "_type": "HIER_OBJECT_ID", "value": performer } },
        "capabilities": [{
            "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0021",
                             "name": text("terms"), "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0022", "name": text("operation"),
                  "value": operation_code(operation) },
                { "_type": "ELEMENT", "archetype_node_id": "at0023",
                  "name": operation_code("template"), "value": text(template) },
                { "_type": "ELEMENT", "archetype_node_id": "at0024",
                  "name": operation_code("origin"), "value": operation_code("relay") }
            ] }
        }]
    });
    if let Some(until) = until {
        role["time_validity"] = json!({ "_type": "DV_INTERVAL",
            "upper": { "_type": "DV_DATE", "value": until },
            "lower_unbounded": true, "upper_unbounded": false,
            "lower_included": false, "upper_included": true });
    }
    role
}

/// Commits the `AGENT` of `client_id` and a `ROLE` granting it to commit
/// `template` through `app`; the `ROLE`'s version uid and the `AGENT`'s
/// container.
async fn granted_client(app: &Router, client_id: &str, template: &str) -> (String, String) {
    let (status, agent_uid, body) = write(app, "POST", "agent", None, &agent(client_id)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let lab = vo(&agent_uid).to_owned();
    let grant = role(&lab, "commit_composition", template, None);
    let (status, role_uid, body) = write(app, "POST", "role", None, &grant).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (role_uid, lab)
}

/// Yesterday in UTC, as `YYYY-MM-DD`.
fn yesterday() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .yesterday()
        .expect("a date")
        .to_string()
}

#[tokio::test]
async fn a_grant_changed_through_the_demographic_api_decides_the_next_request() {
    let (_db, pool) = common::migrated_pool().await;
    let app = Box::pin(replica(&pool, None, true, GrantCeiling::default())).await;
    let (role_uid, lab) = granted_client(&app, "lab", "Lab.v1").await;
    let lab_request = capabilities(&app, &system("lab")).await;
    assert_eq!(
        lab_request.client().map(|client| client.party.to_string()),
        Some(lab.clone())
    );
    assert!(lab_request.holds(Operation::CommitComposition));
    assert_eq!(written(&lab_request), vec!["Lab.v1"]);
    assert_eq!(
        *capabilities(&app, &system("other")).await,
        Capabilities::default(),
        "a client no ACTOR names has none"
    );
    assert_eq!(
        *capabilities(&app, &common::basic_credential("root")).await,
        Capabilities::default(),
        "a caller who is no system client and binds to no party has none"
    );
    let ended = role(&lab, "commit_composition", "Lab.v1", Some(&yesterday()));
    let (status, _, body) = write(
        &app,
        "PUT",
        &format!("role/{}", vo(&role_uid)),
        Some(&role_uid),
        &ended,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let after = capabilities(&app, &system("lab")).await;
    assert!(after.client().is_some());
    assert_eq!(
        after.grants(),
        [],
        "the ended grant no longer holds on the next request"
    );
}

#[tokio::test]
async fn a_capability_above_the_ceiling_is_refused_at_the_wire() {
    let (_db, pool) = common::migrated_pool().await;
    let app = Box::pin(replica(&pool, None, false, GrantCeiling::default())).await;
    let (_, lab) = granted_client(&app, "lab", "Lab.v1").await;
    let privilege = role(&lab, "read_subject_audit", "Lab.v1", None);
    let (status, _, body) = write(&app, "POST", "role", None, &privilege).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("read_subject_audit") && body.contains("[authz.grants] allowed"),
        "{body}"
    );
    assert!(
        !capabilities(&app, &system("lab"))
            .await
            .holds(Operation::ReadSubjectAudit)
    );
}

#[tokio::test]
async fn a_grant_written_through_one_replica_decides_the_next_request_on_another() {
    let (_db, pool) = common::migrated_pool().await;
    let writer = Box::pin(replica(&pool, None, true, GrantCeiling::default())).await;
    let reader = Box::pin(replica(&pool, None, true, GrantCeiling::default())).await;
    assert_eq!(capabilities(&reader, &system("lab")).await.grants(), []);
    granted_client(&writer, "lab", "Lab.v1").await;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let read = capabilities(&reader, &system("lab")).await;
        if written(&read) == vec!["Lab.v1"] {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the other replica's notification arrives"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_party_store_that_cannot_answer_is_503() {
    let (db, pool) = common::migrated_pool().await;
    let healthy = Box::pin(replica(&pool, None, false, GrantCeiling::default())).await;
    assert_eq!(granted(&healthy, &system("lab")).await.0, StatusCode::OK);
    let closed = sqlx::postgres::PgPoolOptions::new()
        .connect(db.url())
        .await
        .expect("a party pool");
    closed.close().await;
    let unavailable = Box::pin(replica(&pool, Some(closed), false, GrantCeiling::default())).await;
    let (status, capabilities) = granted(&unavailable, &system("lab")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(capabilities.is_none());
}

/// The app-only token of the client `client_id`, carrying `scope`.
fn system_with(client_id: &str, scope: &str) -> String {
    bearer(
        MACHINES,
        &audience(MACHINES),
        &json!({ "sub": client_id, "client_id": client_id, "scope": scope }),
    )
}

/// Commits the `AGENT` of `client_id` and a `ROLE` granting it `operation`
/// through `app`; the `ROLE`'s version uid and the `AGENT`'s container.
async fn client_granted(app: &Router, client_id: &str, operation: &str) -> (String, String) {
    let (status, agent_uid, body) = write(app, "POST", "agent", None, &agent(client_id)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let party = vo(&agent_uid).to_owned();
    let grant = role(&party, operation, "Any.v1", None);
    let (status, role_uid, body) = write(app, "POST", "role", None, &grant).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (role_uid, party)
}

/// Ends the grant of the `ROLE` `role_uid` performed by `party`, which
/// granted `operation`, as of yesterday.
async fn end_grant(app: &Router, role_uid: &str, party: &str, operation: &str) {
    let ended = role(party, operation, "Any.v1", Some(&yesterday()));
    let (status, _, body) = write(
        app,
        "PUT",
        &format!("role/{}", vo(role_uid)),
        Some(role_uid),
        &ended,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// The status of an ad-hoc AQL query over every EHR by `credential`.
async fn system_wide_query(app: &Router, credential: &str) -> StatusCode {
    common::send_status(
        app,
        common::req("POST", format!("{BASE}/query/aql"))
            .auth(credential)
            .accept("application/json")
            .json(json!({ "q": "SELECT e/ehr_id/value FROM EHR e" }).to_string())
            .build(),
    )
    .await
}

/// The status of a DEMOGRAPHIC API read by `credential` of a party no
/// version names, so a caller the API admits is answered `404`.
async fn demographic_read(app: &Router, credential: &str) -> StatusCode {
    common::send_status(
        app,
        common::req(
            "GET",
            format!("{BASE}/demographic/person/{}", uuid::Uuid::now_v7()),
        )
        .auth(credential)
        .accept("application/json")
        .build(),
    )
    .await
}

#[tokio::test]
async fn a_system_wide_aql_grant_counts_for_a_client_granted_query_system_wide() {
    let (_db, pool) = common::migrated_pool().await;
    let ceiling = GrantCeiling::new([Operation::QuerySystemWide]);
    let app = Box::pin(replica(&pool, None, true, ceiling)).await;
    let etl = system_with("etl", "system/aql-*.s");
    assert_eq!(
        system_wide_query(&app, &etl).await,
        StatusCode::FORBIDDEN,
        "a system/aql-* grant counts for no client the CDR grants nothing"
    );
    let (role_uid, party) = client_granted(&app, "etl", "query_system_wide").await;
    assert_eq!(system_wide_query(&app, &etl).await, StatusCode::OK);
    assert_eq!(
        system_wide_query(&app, &system_with("other", "system/aql-*.s")).await,
        StatusCode::FORBIDDEN,
        "another client's grant counts for no one else"
    );
    end_grant(&app, &role_uid, &party, "query_system_wide").await;
    assert_eq!(
        system_wide_query(&app, &etl).await,
        StatusCode::FORBIDDEN,
        "the ended grant no longer counts on the next request"
    );
}

#[tokio::test]
async fn the_demographic_api_admits_a_client_granted_access_demographic() {
    let (_db, pool) = common::migrated_pool().await;
    let ceiling = GrantCeiling::new([Operation::AccessDemographic]);
    let app = Box::pin(replica(&pool, None, true, ceiling)).await;
    let mpi = system_with("mpi", "system/composition-*.r");
    assert_eq!(
        demographic_read(&app, &mpi).await,
        StatusCode::FORBIDDEN,
        "a client the CDR grants nothing reaches no demographic data"
    );
    let (role_uid, party) = client_granted(&app, "mpi", "access_demographic").await;
    assert_eq!(demographic_read(&app, &mpi).await, StatusCode::NOT_FOUND);
    assert_eq!(
        demographic_read(&app, &system_with("other", "system/composition-*.r")).await,
        StatusCode::FORBIDDEN,
        "another client's grant admits no one else"
    );
    end_grant(&app, &role_uid, &party, "access_demographic").await;
    assert_eq!(
        demographic_read(&app, &mpi).await,
        StatusCode::FORBIDDEN,
        "the ended grant no longer admits the client on the next request"
    );
}
