// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login binding on every request (#5004): a bearer token of a `persons`
//! issuer with an `[[auth.party]]` table is looked up in the local party
//! source, read from the binding side table with no cache, and the request is
//! refused with `403` and its binding code for every outcome that binds no
//! usable party. Driven through the assembled router over a real
//! `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! The states are RM common `master06-change_control_package.adoc` §Abandoned
//! and Inactive States; resolving a login to a party is governed by no openEHR
//! spec: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::ids::VoId;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::party_binding::{BindingMapping, IdentifierSelector};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry, IssuerKey};
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};

const ISSUER: &str = "https://login.example";
const AUDIENCE: &str = "ferroehr";
/// The `identifier_issuer` of the issuer's `[[auth.party]]` table.
const LOGIN: &str = "https://login.example/tenant";
/// The `group_identifier_issuer` of that table.
const GROUPS: &str = "https://login.example/groups";
const PERSON_TEMPLATE: &str = "FerroEHR professional";
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";

fn mapping() -> BindingMapping {
    let key = IdentifierSelector::new(
        format!("/details[at0001]/items[{CLUSTER}]/items[at0001]"),
        "guid",
    )
    .expect("the key selector compiles");
    BindingMapping::default()
        .with_person(PERSON_TEMPLATE, key)
        .with_professional_role(ROLE_TEMPLATE)
}

/// The `[auth]` table trusting `ISSUER`, whose logins the `[[auth.party]]`
/// table binds by `person_key`, reading groups from `groups`.
fn config(person_key: Option<&str>) -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(vec![IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            person_key: person_key.map(str::to_owned),
            ..common::entry(ISSUER, AUDIENCE)
        }]),
        party: vec![PartySection {
            issuer: ISSUER.to_owned(),
            identifier_issuer: LOGIN.to_owned(),
            identifier_type: "guid".to_owned(),
            group_claim: "groups".to_owned(),
            group_identifier_issuer: GROUPS.to_owned(),
            ..PartySection::default()
        }],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    }
}

/// The router over `svc`, looking every caller up in its party source.
async fn router(svc: Arc<FerroEhrService>, person_key: Option<&str>) -> Router {
    let source = Arc::new(svc.party_source("demographic"));
    router_over(svc, person_key, source).await
}

/// The router over `svc`, looking every caller up in `source`.
async fn router_over(
    svc: Arc<FerroEhrService>,
    person_key: Option<&str>,
    source: Arc<dyn BindingSource>,
) -> Router {
    let config = config(person_key);
    let authenticator = Authenticator::new(
        &config,
        None,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await
    .expect("the binding builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, svc, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// A bearer of `ISSUER` whose `sub` is `subject`, with `extra` claims.
fn token(subject: &str, extra: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut claims = json!({ "sub": subject, "iss": ISSUER, "aud": AUDIENCE, "exp": exp });
    if let (Some(claims), Some(extra)) = (claims.as_object_mut(), extra.as_object()) {
        claims.extend(extra.clone());
    }
    common::bearer(&claims)
}

/// The status and body of a template listing as `credential`.
async fn list(app: &Router, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req("GET", format!("{BASE}/definition/template/adl1.4"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn party_ref(class: &str, id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": class,
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

fn archetyped(archetype: &str, template: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "template_id": { "_type": "TEMPLATE_ID", "value": template },
            "rm_version": "1.1.0" })
}

fn identities() -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0002", "name": text("name"),
             "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0003",
                          "name": text("structure"),
                          "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0004",
                                      "name": text("name"), "value": text("professional") }] } }])
}

fn person(key: &str) -> Value {
    json!({
        "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", PERSON_TEMPLATE),
        "name": text("professional"), "identities": identities(),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": CLUSTER, "name": text("ids"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0001", "name": text("key"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": LOGIN, "id": key,
                               "type": "guid" } }] }] }
    })
}

fn write<T: serde::de::DeserializeOwned>(
    body: &Value,
    lifecycle: &str,
    preceding: Option<&str>,
) -> WriteRequest<T> {
    let mut write =
        WriteRequest::new(openehr_its::json::from_canonical_value(body).expect("the body decodes"));
    write.lifecycle_state = Some(lifecycle.to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

fn uid(response: &ferroehr::service::response::ServiceResponse) -> String {
    response.meta.as_ref().expect("a version").uid.clone()
}

fn vo(uid: &str) -> VoId {
    VoId(
        uid.split("::")
            .next()
            .and_then(|id| id.parse().ok())
            .expect("a container id"),
    )
}

/// A professional `PERSON` carrying `key`, its head in `lifecycle`; returns
/// its last version uid and body.
async fn professional(svc: &FerroEhrService, key: &str, lifecycle: &str) -> (String, Value) {
    let body = person(key);
    // NOTE: RM common master06 §Version Lifecycle: only an incomplete version
    // may be abandoned.
    let first = if lifecycle == "801" { "553" } else { "532" };
    let created = Box::pin(svc.party_create(PartyKind::Person, write(&body, first, None)))
        .await
        .expect("the PERSON commits");
    let person_uid = uid(&created);
    let role = json!({
        "_type": "ROLE", "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", ROLE_TEMPLATE),
        "name": text("professional"), "identities": identities(),
        "performer": party_ref("PERSON", vo(&person_uid))
    });
    let role_uid = uid(
        &Box::pin(svc.party_create(PartyKind::Role, write(&role, "532", None)))
            .await
            .expect("the ROLE commits"),
    );
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", vo(&role_uid))]);
    (update(svc, &person_uid, &listed, lifecycle).await, listed)
}

/// Commits `body` as the next version of the `PERSON` at `uid`.
async fn update(svc: &FerroEhrService, uid: &str, body: &Value, lifecycle: &str) -> String {
    let updated = Box::pin(svc.party_update(
        PartyKind::Person,
        vo(uid),
        Precondition::parse(Some(uid), None).expect("an If-Match"),
        write(body, lifecycle, Some(uid)),
    ))
    .await
    .expect("the PERSON updates");
    self::uid(&updated)
}

async fn service() -> (testkit::TestDb, Arc<FerroEhrService>) {
    let (db, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_binding_mapping(mapping());
    (db, Arc::new(svc))
}

/// Asserts `credential` is refused with `403` naming `code`.
async fn refused(app: &Router, credential: &str, code: &str) {
    let (status, body) = list(app, credential).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{code}: {body}");
    assert!(body.contains(code), "{code}: {body}");
}

impl BindingSource for Twice {}

/// A party source that reports two parties for every login: the unique index
/// on live keys keeps the local source from holding one key twice, so the
/// shared crate's ambiguous outcome is driven through this one.
struct Twice;

impl PartySource for Twice {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        let record = |id: &str| {
            PartyRecord::new(
                PartyRef::new("demographic", "PERSON", id),
                PartyState::Active,
                Markers {
                    professional: true,
                    representative: false,
                },
            )
        };
        Box::pin(async move {
            Ok(Lookup::Parties(vec![
                record("5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c8d"),
                record("6b7c8d9e-0f1a-4b2c-9d3e-4f5a6b7c8d9e"),
            ]))
        })
    }
}

#[tokio::test]
async fn a_login_matching_two_parties_is_refused_naming_the_caller() {
    let (_db, svc) = service().await;
    let app = router_over(svc, None, Arc::new(Twice)).await;
    let twice = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
    refused(&app, &token(twice, &json!({})), "refused-ambiguous").await;
    // The refusal carries the refused caller, which the failed-authentication
    // record names.
    let response = tower::ServiceExt::oneshot(
        app.clone(),
        common::req("GET", format!("{BASE}/definition/template/adl1.4"))
            .auth(token(twice, &json!({})))
            .accept("application/json")
            .build(),
    )
    .await
    .expect("the router answers");
    let principal = response
        .extensions()
        .get::<ferroehr_rest::extensions::access::authn::Principal>()
        .expect("the refusal names its caller");
    assert_eq!(principal.subject, twice);
}

#[tokio::test]
async fn a_bound_professional_is_served_and_the_unusable_outcomes_are_refused() {
    let (_db, svc) = service().await;
    let bound = "0b9e6c2a-5d4f-4e8a-9b1c-2d3e4f5a6b7c";
    professional(&svc, &bound.to_ascii_uppercase(), "532").await;
    let inactive = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f6a";
    professional(&svc, inactive, "800").await;
    let abandoned = "3e4f5a6b-7c8d-4e9f-8a1b-2c3d4e5f6a7b";
    professional(&svc, abandoned, "801").await;
    let deleted = "4f5a6b7c-8d9e-4f0a-9b2c-3d4e5f6a7b8c";
    let (deleted_uid, _) = professional(&svc, deleted, "532").await;
    Box::pin(
        svc.party_delete(
            PartyKind::Person,
            vo(&deleted_uid),
            Precondition::parse(
                None,
                ferroehr::versioning::object_version_id::parse_uid_based_id(&deleted_uid)
                    .expect("a version uid")
                    .tree(),
            )
            .expect("a precondition"),
            WriteRequest::new(()),
        ),
    )
    .await
    .expect("the PERSON deletes");
    let app = router(Arc::clone(&svc), None).await;

    let (status, body) = list(&app, &token(bound, &json!({}))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a bound professional is served: {body}"
    );
    refused(&app, &token(inactive, &json!({})), "refused-inactive").await;
    refused(&app, &token(abandoned, &json!({})), "refused-abandoned").await;
    refused(&app, &token(deleted, &json!({})), "refused-deprovisioned").await;
    // A groups overage marker with no adapter on the issuer.
    refused(
        &app,
        &token(
            bound,
            &json!({ "_claim_names": { "groups": "src1" },
                     "_claim_sources": { "src1": { "endpoint": "https://graph.example" } } }),
        ),
        "refused-groups-unavailable",
    )
    .await;
}

#[tokio::test]
async fn a_token_without_the_key_claim_is_refused() {
    let (_db, svc) = service().await;
    let app = router(svc, Some("oid")).await;
    refused(
        &app,
        &token("someone", &json!({})),
        "refused-key-claim-missing",
    )
    .await;
}

#[tokio::test]
async fn a_deactivation_on_one_instance_refuses_the_next_request_on_another() {
    let (db, pool) = common::migrated_pool().await;
    let first = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_binding_mapping(mapping()),
    );
    let second = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
            .with_binding_mapping(mapping()),
    );
    let key = "5a6b7c8d-9e0f-4a1b-8c3d-4e5f6a7b8c9d";
    let (uid, body) = professional(&first, key, "532").await;
    let on_second = router(Arc::clone(&second), None).await;
    let credential = token(key, &json!({}));
    assert_eq!(list(&on_second, &credential).await.0, StatusCode::OK);

    // The deactivation commits on the first instance; the second refuses the
    // very next request, since every request reads the side table.
    update(&first, &uid, &body, "800").await;
    refused(&on_second, &credential, "refused-inactive").await;
}
