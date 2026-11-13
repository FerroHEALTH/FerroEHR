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
use ferroehr::config::management::{AccessLevel, EndpointLevels, ManagementConfig};
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
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};

pub(crate) const ISSUER: &str = "https://login.example";
pub(crate) const AUDIENCE: &str = "ferroehr";
/// The `identifier_issuer` of the issuer's `[[auth.party]]` table.
pub(crate) const LOGIN: &str = "https://login.example/tenant";
/// The `group_identifier_issuer` of that table.
const GROUPS: &str = "https://login.example/groups";
const PERSON_TEMPLATE: &str = "FerroEHR professional";
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";

pub(crate) fn mapping() -> BindingMapping {
    professionals(BindingMapping::default())
}

/// `mapping` binding the professional `PERSON`s of [`professional_of`] by
/// their key.
pub(crate) fn professionals(mapping: BindingMapping) -> BindingMapping {
    let key = IdentifierSelector::new(
        format!("/details[at0001]/items[{CLUSTER}]/items[at0001]"),
        "guid",
    )
    .expect("the key selector compiles");
    mapping
        .with_person(PERSON_TEMPLATE, key)
        .with_professional_role(ROLE_TEMPLATE)
}

/// The `[auth]` table trusting `ISSUER`, whose logins the `[[auth.party]]`
/// table binds by `person_key`, reading groups from `groups`.
pub(crate) fn config(person_key: Option<&str>) -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            person_key: person_key.map(str::to_owned),
            ..common::entry(ISSUER, AUDIENCE)
        }],
        party: vec![PartySection {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
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
    router_with(svc, config(person_key), source).await
}

/// The router over `svc` under `config`, looking every caller up in `source`.
pub(crate) async fn router_with(
    svc: Arc<FerroEhrService>,
    config: AppConfig,
    source: Arc<dyn BindingSource>,
) -> Router {
    router_observed(svc, config, source, Observability::default()).await
}

/// The router over `svc` under `config` and `observability`, looking every
/// caller up in `source`.
async fn router_observed(
    svc: Arc<FerroEhrService>,
    config: AppConfig,
    source: Arc<dyn BindingSource>,
    observability: Observability,
) -> Router {
    common::first_party(&svc, &config).await;
    let authenticator = Authenticator::new(
        &config,
        None,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
        common::standing(&svc),
    )
    .await
    .expect("the binding builds");
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, svc, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// A bearer of `ISSUER` whose `sub` is `subject`, with `extra` claims.
pub(crate) fn token(subject: &str, extra: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut claims = json!({ "sub": subject, "iss": ISSUER, "aud": AUDIENCE, "exp": exp });
    if let (Some(claims), Some(extra)) = (claims.as_object_mut(), extra.as_object()) {
        claims.extend(extra.clone());
    }
    common::bearer(&claims)
}

/// The status and body of a template listing as `credential`.
pub(crate) async fn list(app: &Router, credential: &str) -> (StatusCode, String) {
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

pub(crate) fn person(identifier_issuer: &str, key: &str) -> Value {
    json!({
        "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", PERSON_TEMPLATE),
        "name": text("professional"), "identities": identities(),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": CLUSTER, "name": text("ids"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0001", "name": text("key"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": identifier_issuer, "id": key,
                               "type": "guid" } }] }] }
    })
}

pub(crate) fn write<T: serde::de::DeserializeOwned>(
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

pub(crate) fn uid(response: &ferroehr::service::response::ServiceResponse) -> String {
    response.meta.as_ref().expect("a version").uid.clone()
}

pub(crate) fn vo(uid: &str) -> VoId {
    VoId(
        uid.split("::")
            .next()
            .and_then(|id| id.parse().ok())
            .expect("a container id"),
    )
}

/// A professional `PERSON` carrying `key`, its head in `lifecycle`; returns
/// its last version uid and body.
pub(crate) async fn professional(
    svc: &FerroEhrService,
    key: &str,
    lifecycle: &str,
) -> (String, Value) {
    professional_of(svc, LOGIN, key, lifecycle).await
}

/// A professional `PERSON` carrying `key` issued by `identifier_issuer`, its
/// head in `lifecycle`; returns its last version uid and body.
pub(crate) async fn professional_of(
    svc: &FerroEhrService,
    identifier_issuer: &str,
    key: &str,
    lifecycle: &str,
) -> (String, Value) {
    let body = person(identifier_issuer, key);
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

pub(crate) async fn service() -> (testkit::TestDb, Arc<FerroEhrService>) {
    let (db, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_binding_mapping(mapping());
    (db, Arc::new(svc))
}

/// The real platform service over a fresh database, recording its access
/// records into the audit store; the handle keeps the audit drain running.
pub(crate) async fn audited_service() -> (
    testkit::TestDb,
    ferroehr::system_log::sender::AuditHandle,
    Arc<FerroEhrService>,
) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, handle) =
        ferroehr::system_log::sender::start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_binding_mapping(mapping())
        .with_audit(sender)
        .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool));
    (db, handle, Arc::new(svc))
}

/// Asserts `credential` is refused with `403` naming `code`.
pub(crate) async fn refused(app: &Router, credential: &str, code: &str) {
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
    let app = router_with(svc, config(None), Arc::new(Twice)).await;
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

/// An overage token whose groups the identity provider cannot give for now is
/// the server's failure, never the person's: with the `idp-entra` adapter
/// registered and its Graph answering `429`, the request is answered `503`
/// with the Graph's `Retry-After` (#5086). The adapter reaches a Graph on
/// loopback over plain `http`, as RFC 8414 §2's loopback exception admits.
#[tokio::test]
async fn an_overage_the_identity_provider_throttles_is_unavailable_with_its_retry_after() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let graph = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "graph-access", "token_type": "Bearer", "expires_in": 3600
        })))
        .mount(&graph)
        .await;
    let oid = "6c7d8e9f-0a1b-4c2d-8e3f-4a5b6c7d8e9f";
    Mock::given(method("POST"))
        .and(path(format!("/v1.0/users/{oid}/getMemberObjects")))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "7"))
        .mount(&graph)
        .await;

    let (_db, svc) = service().await;
    let key = "7d8e9f0a-1b2c-4d3e-9f4a-5b6c7d8e9f0a";
    professional(&svc, key, "532").await;
    let mut config = config(None);
    if let Some(entry) = config.auth.shared.issuer.first_mut() {
        entry.adapter = Some(ferrohealth_smart::idp::entra::NAME.to_owned());
    }
    config.auth.shared.idp.entra = Some(Box::new(ferrohealth_smart::idp::entra::GraphSection {
        base_url: format!("{}/v1.0", graph.uri()),
        credentials: ferrohealth_smart::config::credentials::OauthSection {
            token_endpoint: ferrohealth_secret::secret_url::SecretUrl::new(format!(
                "{}/token",
                graph.uri()
            )),
            client_id: "ferroehr-graph".to_owned(),
            client_secret: Some(secrecy::SecretString::from("graph-test-secret")),
            scope: "https://graph.microsoft.com/.default".to_owned(),
            ..ferrohealth_smart::config::credentials::OauthSection::default()
        },
        ..ferrohealth_smart::idp::entra::GraphSection::default()
    }));
    common::first_party(&svc, &config).await;
    let authenticator = Authenticator::new(
        &config,
        None,
        Some(Arc::new(svc.party_source("demographic"))),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
        common::standing(&svc),
    )
    .await
    .expect("the binding with its adapter builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let app = ferroehr_rest::router::router(
        AppState::with_parts(config, svc, None, observability),
        authenticator,
    );

    let overage = token(
        key,
        &json!({ "oid": oid,
                 "_claim_names": { "groups": "src1" },
                 "_claim_sources": { "src1": { "endpoint": "https://graph.example" } } }),
    );
    let (status, headers, body) = common::send(
        &app,
        common::req("GET", format!("{BASE}/definition/template/adl1.4"))
            .auth(&overage)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        headers
            .get(http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("7"),
        "{body}"
    );
}

/// The stored failed-authentication records carrying `binding`, as
/// `(event_code, party id)`, waiting for the drain to write `count` of them.
pub(crate) async fn refusal_rows(
    pool: &sqlx::PgPool,
    binding: &str,
    count: usize,
) -> Vec<(String, Option<String>)> {
    for _ in 0..50 {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT event_code, party ->> 'id' FROM audit.audit_event \
             WHERE binding = $1 AND summary IS NULL ORDER BY recorded_at",
        )
        .bind(binding)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} {binding} records were stored");
}

/// Every refusal of the login binding becomes an access record through the
/// gated path, carrying its binding code and the party it names (#5119).
#[tokio::test]
async fn a_login_binding_refusal_is_recorded_with_its_code_and_party() {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, ferroehr::system_log::sender::AuditHandle) =
        ferroehr::system_log::sender::start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_binding_mapping(mapping())
            .with_audit(sender)
            .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool.clone())),
    );
    let inactive = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
    let (person_uid, _) = professional(&svc, inactive, "800").await;
    let app = router(Arc::clone(&svc), None).await;
    refused(&app, &token(inactive, &json!({})), "refused-inactive").await;
    refused(&app, &token(inactive, &json!({})), "refused-inactive").await;

    let rows = refusal_rows(&db.pool(), "refused-inactive", 2).await;
    let party = vo(&person_uid).0.to_string();
    assert_eq!(
        rows,
        vec![
            ("110114".to_owned(), Some(party.clone())),
            ("110114".to_owned(), Some(party))
        ],
        "one failed-authentication record per refusal, naming the inactive party"
    );
}

/// A token without the key claim is recorded with the name of the claim the
/// issuer's binding reads, never a claim value (#5139).
#[tokio::test]
async fn a_key_claim_refusal_is_recorded_with_the_claim_name() {
    let (db, _audit, svc) = audited_service().await;
    let app = router(svc, Some("oid")).await;
    refused(
        &app,
        &token("someone", &json!({"name": "A Person"})),
        "refused-key-claim-missing",
    )
    .await;

    refusal_rows(&db.pool(), "refused-key-claim-missing", 1).await;
    let (detail,): (Option<Value>,) = sqlx::query_as(
        "SELECT binding_detail FROM audit.audit_event \
         WHERE binding = 'refused-key-claim-missing' AND summary IS NULL",
    )
    .fetch_one(&db.pool())
    .await
    .expect("query");
    assert_eq!(
        detail,
        Some(json!({"claim": ["oid"]})),
        "the record names the key claim and carries no claim value"
    );
}

/// A tracing layer recording the fields of every event it sees.
#[derive(Clone, Default)]
pub(crate) struct Capture(pub(crate) Arc<std::sync::Mutex<Vec<String>>>);

#[derive(Default)]
struct Fields(Vec<String>);

impl tracing::field::Visit for Fields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0.join(" "));
    }
}

/// #5006: a person whose login no demographic party carries is refused with
/// `403` on a read, an AQL query, a commit and a demographic write, each an
/// access record of `refused-unbound`; the problem names the identifier
/// issuer to provision the person under, and no fallback serves the login.
#[tokio::test]
async fn an_unbound_person_is_refused_on_every_kind_of_request() {
    let (db, _audit, svc) = audited_service().await;
    let app = router(svc, None).await;
    let credential = token("8d9e0f1a-2b3c-4d4e-9f5a-6b7c8d9e0f1a", &json!({}));
    let ehr = "9e0f1a2b-3c4d-4e5f-8a6b-7c8d9e0f1a2b";
    let requests = [
        common::req("GET", format!("{BASE}/definition/template/adl1.4"))
            .auth(&credential)
            .accept("application/json")
            .build(),
        common::req("POST", format!("{BASE}/query/aql"))
            .auth(&credential)
            .accept("application/json")
            .json(r#"{"q":"SELECT e/ehr_id/value FROM EHR e"}"#)
            .build(),
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .auth(&credential)
            .accept("application/json")
            .json(r#"{"_type":"COMPOSITION"}"#)
            .build(),
        common::req("POST", format!("{BASE}/demographic/person"))
            .auth(&credential)
            .accept("application/json")
            .json(r#"{"_type":"PERSON"}"#)
            .build(),
    ];
    for request in requests {
        let target = format!("{} {}", request.method(), request.uri());
        let (status, body) = common::send_body(&app, request).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{target}: {body}");
        assert!(body.contains("refused-unbound"), "{target}: {body}");
        assert!(
            body.contains(LOGIN),
            "the problem names the identifier issuer: {target}: {body}"
        );
    }
    let rows = refusal_rows(&db.pool(), "refused-unbound", 4).await;
    assert_eq!(rows.len(), 4, "one access record per refused request");
}

/// #5008: a `persons` token is looked up whatever its scopes, so one with
/// `user/` and `patient/` scopes and one with `patient/` scopes alone are
/// each refused when no party carries the login.
#[tokio::test]
async fn an_unbound_person_is_refused_under_patient_scopes() {
    let (_db, svc) = service().await;
    let app = router(svc, None).await;
    let subject = "1a2b3c4d-5e6f-4a7b-9c8d-9e0f1a2b3c4d";
    for scope in [
        "user/composition-*.cruds patient/composition-*.r",
        "patient/composition-*.r",
    ] {
        let claims = json!({ "scope": scope, "patient": "9e0f1a2b-3c4d-4e5f-8a6b-7c8d9e0f1a2b" });
        refused(&app, &token(subject, &claims), "refused-unbound").await;
    }
}

/// #5006: the refusal of a token without the key claim names the claim the
/// issuer's binding reads, which "provision this person" would misdiagnose.
#[tokio::test]
async fn a_key_claim_refusal_names_the_missing_claim() {
    let (_db, svc) = service().await;
    let app = router(svc, Some("oid")).await;
    let (status, body) = list(&app, &token("someone", &json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-key-claim-missing"), "{body}");
    assert!(
        body.contains(r#"claim \"oid\""#),
        "the claim is named: {body}"
    );
    assert!(!body.contains("provision"), "{body}");
}

/// #5006: each refusal writes one `warn` line naming the issuer and the
/// outcome and never the claim value, and counts into `binding_refusals` by
/// issuer and outcome.
#[tokio::test]
async fn a_refusal_logs_no_claim_value_and_is_counted_by_issuer_and_outcome() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let telemetry = common::telemetry();
    let (_db, svc) = service().await;
    let app = router(svc, None).await;
    let subject = "0f1a2b3c-4d5e-4f6a-9b7c-8d9e0f1a2b3c";
    let capture = Capture::default();
    let lines = Arc::clone(&capture.0);
    {
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(capture));
        refused(&app, &token(subject, &json!({})), "refused-unbound").await;
    }
    let lines = lines.lock().unwrap().clone();
    let warned: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("the login binding refused the request"))
        .collect();
    assert_eq!(warned.len(), 1, "one warn line: {lines:?}");
    assert!(
        warned[0].contains(ISSUER) && warned[0].contains("unbound"),
        "{warned:?}"
    );
    for line in &lines {
        assert!(!line.contains(subject), "a claim value was logged: {line}");
    }
    let exposition = telemetry
        .render_prometheus()
        .expect("the exposition renders");
    let counted = exposition.lines().any(|line| {
        line.starts_with("binding_refusals_total{")
            && line.contains(&format!("issuer=\"{ISSUER}\""))
            && line.contains("outcome=\"unbound\"")
            && line.ends_with(" 1")
    });
    assert!(counted, "{exposition}");
}

/// #5250: the management surface answers a person the login binding refuses
/// with `403` and the `refused-unbound` refusal and access record every other
/// route gives (RFC 9110 §15.5.4), and a request with no credential with
/// `401` and its challenge (§15.5.2).
#[tokio::test]
async fn the_management_surface_refuses_an_unbound_person_with_403() {
    let (db, _audit, svc) = audited_service().await;
    let source = Arc::new(svc.party_source("demographic"));
    let observability = Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints: EndpointLevels {
                info: AccessLevel::Private,
                ..EndpointLevels::default()
            },
            ..ManagementConfig::default()
        },
        ..Observability::default()
    };
    let app = router_observed(svc, config(None), source, observability).await;
    let credential = token("1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", &json!({}));
    let (status, body) = common::send_body(
        &app,
        common::req("GET", "/management/info")
            .auth(&credential)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-unbound"), "{body}");
    // DCM 110114 is the failed user authentication; the management surface
    // records its own 110100 system access beside it (#3244).
    let rows = refusal_rows(&db.pool(), "refused-unbound", 2).await;
    let mut codes: Vec<&str> = rows.iter().map(|(code, _)| code.as_str()).collect();
    codes.sort_unstable();
    assert_eq!(codes, ["110100", "110114"], "{rows:?}");

    let (status, headers, body) = common::send(
        &app,
        common::req("GET", "/management/info")
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(
        headers.contains_key(http::header::WWW_AUTHENTICATE),
        "the 401 carries its challenge"
    );
}
