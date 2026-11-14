// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Just-in-time creation of a provisional `PERSON` at a first login (#5016),
//! through the assembled router with the server's creating party source,
//! admitted by `create_at_login` grants held in the CDR (#5441).
//!
//! A first login that a grant in force for its issuer admits commits one
//! `553|incomplete|` `PERSON` (RM common
//! `master06-change_control_package.adoc` §Incomplete Content) with its
//! professional `ROLE` and is served bound to it; the cases it never runs for
//! are refused; a grant changed in the CDR decides the next login with no
//! restart; concurrent first requests commit one `PERSON`; and the SCIM
//! client of the key's issuer completes it. When a login creates a party is
//! our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::binding::jit::IssuerBinding;
use ferroehr::config::binding::{ProviderId, Providers};
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::admission::{ADMIT_GUESTS, ADMIT_IDP, LOGIN_ISSUER};
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::profile::Marker;
use http::StatusCode;
use serde_json::{Value, json};

use super::{AUDIENCE, CLIENT, EXTENSION, PERSONS, app_of, call, config, created, patch_op};
use crate::common;

/// No `PERSON`.
const NONE: [uuid::Uuid; 0] = [];

/// The identity provider the admission admits.
const IDP: &str = "https://sts.example/hospital";

/// The SCIM harness's configuration with the name claims mapped, so a
/// provisional `PERSON` can be written.
fn jit_config() -> AppConfig {
    let mut config = config();
    let attributes = &mut config
        .party_binding
        .person
        .get_or_insert_default()
        .attribute;
    for (attribute, claim) in
        attributes
            .iter_mut()
            .zip(["name", "given_name", "family_name", "email"])
    {
        attribute.oidc = Some(claim.to_owned());
    }
    config
}

/// The router of `config` over a fresh audited database, whose party source
/// creates a provisional `PERSON` through the service, with the store's pool
/// and the service; `[authz.grants] allowed` admits `create_at_login` and the
/// harness clients' `scim_provision`, and the CDR holds no admission.
async fn bare_app(
    config: AppConfig,
) -> (testkit::TestDb, sqlx::PgPool, Arc<FerroEhrService>, Router) {
    let (db, pool, service, _) = Box::pin(app_of(config.clone(), true)).await;
    service.grants().set_ceiling(GrantCeiling::new([
        Operation::CommitComposition,
        Operation::CreateAtLogin,
        Operation::ScimProvision,
    ]));
    let source = Arc::new(
        service
            .party_source("demographic")
            .with_creator(Arc::clone(&service)),
    );
    let app = crate::party_binding_http::router_with(Arc::clone(&service), config, source).await;
    (db, pool, service, app)
}

/// [`bare_app`] whose CDR admits the logins of `PERSONS` from `IDP`.
async fn jit_app(
    config: AppConfig,
) -> (testkit::TestDb, sqlx::PgPool, Arc<FerroEhrService>, Router) {
    let (db, pool, service, app) = Box::pin(bare_app(config)).await;
    admit(&service, &[], None).await;
    (db, pool, service, app)
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

/// A credentials element named by the terminology's `code`, holding `value`.
fn element(code: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": operation_code(code),
            "value": value })
}

/// The `ARCHETYPED` of the deployment's own `archetype`, with no template.
fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

/// A name identity of `name`.
fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

/// The write of `body` after `preceding`.
fn write<T: serde::de::DeserializeOwned>(body: &Value, preceding: Option<&str>) -> WriteRequest<T> {
    let mut write =
        WriteRequest::new(openehr_its::json::from_canonical_value(body).expect("the body decodes"));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// The admission `ROLE` the identity provider's `ORGANISATION` `performer`
/// performs: a capability granting `create_at_login` for `PERSONS` from
/// `IDP`, with the further credentials `extra`, valid until `until` when
/// given.
fn admission(performer: VoId, extra: &[Value], until: Option<&str>) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.login_admission.v1";
    let mut items = vec![
        json!({ "_type": "ELEMENT", "archetype_node_id": "at0021", "name": text("operation"),
                "value": operation_code(Operation::CreateAtLogin.code()) }),
        element(
            LOGIN_ISSUER,
            &json!({ "_type": "DV_URI", "value": PERSONS }),
        ),
        element(ADMIT_IDP, &text(IDP)),
    ];
    items.extend_from_slice(extra);
    let mut role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Login admission"),
        "identities": identities("login admission"),
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                       "id": { "_type": "HIER_OBJECT_ID", "value": performer.to_string() } },
        "capabilities": [{
            "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("admission"),
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022",
                             "name": text("terms"), "items": items }
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

/// Commits the identity provider's `ORGANISATION` and its admission `ROLE`
/// with the further credentials `extra` through `service`, outside a
/// request, as the server's own administration does; the `ROLE`'s version
/// uid and the `ORGANISATION`'s container.
async fn admit(service: &FerroEhrService, extra: &[Value], until: Option<&str>) -> (String, VoId) {
    let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.identity_provider.v1";
    let idp = json!({
        "_type": "ORGANISATION", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Hospital identity provider"),
        "identities": identities("hospital identity provider")
    });
    let created = Box::pin(service.party_create(PartyKind::Organisation, write(&idp, None)))
        .await
        .expect("the ORGANISATION commits");
    let performer = crate::party_binding_http::vo(&created.meta.expect("a version").uid);
    let role = Box::pin(service.party_create(
        PartyKind::Role,
        write(&admission(performer, extra, until), None),
    ))
    .await
    .expect("the admission ROLE commits");
    (role.meta.expect("a version").uid, performer)
}

/// A bearer of `PERSONS` whose key (`sub`) is `key`, with `extra` claims.
fn login(key: &str, extra: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut claims = json!({ "iss": PERSONS, "aud": AUDIENCE, "exp": exp, "sub": key });
    if let (Some(claims), Some(extra)) = (claims.as_object_mut(), extra.as_object()) {
        claims.extend(extra.clone());
    }
    common::bearer(&claims)
}

/// The claims of an admitted login with its names and a work email.
fn named() -> Value {
    json!({ "idp": IDP, "name": "Dr. A. Jansen", "given_name": "Anna",
            "family_name": "Jansen", "email": "a.jansen@example.org" })
}

/// The status and problem body of a request as `token`.
async fn request(app: &Router, token: &str) -> (StatusCode, String) {
    crate::party_binding_http::list(app, token).await
}

/// The live `PERSON`s the side table binds to `key`.
async fn persons_of(pool: &sqlx::PgPool, key: &str) -> Vec<uuid::Uuid> {
    sqlx::query_scalar(
        "SELECT DISTINCT party_id FROM party.binding_identifier \
         WHERE selector = 'key' AND value = $1 AND retired_at IS NULL",
    )
    .bind(key)
    .fetch_all(pool)
    .await
    .expect("read the side table")
}

/// How many `PERSON` containers the party store holds.
async fn person_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM party.vo_head WHERE kind = 'PERSON'")
        .fetch_one(pool)
        .await
        .expect("count the persons")
}

/// The `(lifecycle_state, change_type, description, committer)` of each
/// version of `party`, oldest first.
async fn versions(pool: &sqlx::PgPool, party: uuid::Uuid) -> Vec<(String, String, Value, Value)> {
    sqlx::query_as(
        "SELECT v.lifecycle_state, a.change_type, a.description, a.committer \
         FROM party.version v JOIN party.commit_audit a ON a.id = v.commit_audit_id \
         WHERE v.vo_id = $1 ORDER BY v.sys_version",
    )
    .bind(party)
    .fetch_all(pool)
    .await
    .expect("read the versions")
}

/// The binding codes of the access records, once `count` are stored.
async fn bindings(pool: &sqlx::PgPool, code: &str, count: usize) -> usize {
    for _ in 0..100 {
        let found: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit.access_trail WHERE binding = $1")
                .bind(code)
                .fetch_one(pool)
                .await
                .expect("read the trail");
        if usize::try_from(found).unwrap_or_default() >= count {
            return count;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    0
}

/// The SCIM User of `CLIENT` whose `userName` is `user_name`.
async fn user_named(app: &Router, user_name: &str) -> Value {
    let filter = format!(
        "/Users?filter={}",
        urlencoding::encode(&format!("userName eq \"{user_name}\""))
    );
    let (status, _, body) = call(app, "GET", &filter, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], 1, "{body}");
    body["Resources"][0].clone()
}

/// A first login creates one `incomplete` `PERSON` holding the key, the
/// `userName` and the names, performing a professional `ROLE`, committed as
/// a creation by the person with the just-in-time description; the request is
/// served and recorded `provisional`, the second request creates nothing, and
/// the email, whose mapping does not set `jit`, is not written.
#[tokio::test]
async fn a_first_login_creates_one_provisional_person_and_binds_to_it() {
    let (_db, pool, _service, app) = Box::pin(jit_app(jit_config())).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000a001";
    let token = login(key, &named());
    let (status, body) = request(&app, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let persons = persons_of(&pool, key).await;
    assert_eq!(persons.len(), 1, "one PERSON carries the key");
    let person = persons[0];
    let history = versions(&pool, person).await;
    assert_eq!(history.len(), 1);
    let (lifecycle, change_type, description, committer) = &history[0];
    assert_eq!(lifecycle, "553", "RM common §Incomplete Content");
    assert_eq!(change_type, "249", "openEHR audit change type creation");
    assert!(
        description["value"]
            .as_str()
            .is_some_and(|text| text.contains("just-in-time") && text.contains(PERSONS)),
        "{description}"
    );
    assert_eq!(committer["name"], "Dr. A. Jansen", "{committer}");
    assert_eq!(committer["external_ref"]["type"], "PERSON");
    assert_eq!(
        committer["external_ref"]["id"]["value"],
        person.to_string(),
        "the committer names the new PERSON"
    );
    assert_eq!(committer["identifiers"][0]["id"], key);
    assert_eq!(committer["identifiers"][0]["issuer"], PERSONS);

    let user = user_named(&app, key).await;
    assert_eq!(user["id"], person.to_string());
    assert_eq!(user[EXTENSION]["lifecycleState"], "incomplete");
    assert_eq!(user[EXTENSION]["loginIdentifier"], key);
    assert_eq!(user["name"]["formatted"], "Dr. A. Jansen");
    assert_eq!(user["name"]["givenName"], "Anna");
    assert!(user.get("externalId").is_none(), "RFC 7643 §3.1: {user}");
    assert!(
        user.get("emails").is_none(),
        "no jit = true on the email: {user}"
    );

    let (status, body) = request(&app, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        person_count(&pool).await,
        1,
        "the second request creates nothing"
    );
    assert_eq!(
        bindings(&pool, "provisional", 2).await,
        2,
        "EHDS Annex II 3.2(b)"
    );
}

/// The provisional records of a first login name the healthcare provider the
/// deployment declares, as a bound record does (EHDS Annex II 3.2(a); the
/// record's fields are our own design).
#[tokio::test]
async fn a_provisional_record_names_the_declared_provider() {
    let mut config = jit_config();
    config.party_binding.provider = Some(Providers::One(ProviderId {
        issuer: "urn:oid:2.16.528.1.1007.3.3".to_owned(),
        id: "90000013".to_owned(),
    }));
    let (_db, pool, _service, app) = Box::pin(jit_app(config)).await;
    let token = login("6a1f6c1e-0000-4000-8000-00000000a0f1", &named());
    for _ in 0..2 {
        let (status, body) = request(&app, &token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(bindings(&pool, "provisional", 2).await, 2);
    let providers: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT provider ->> 'issuer', provider ->> 'id' FROM audit.access_trail \
         WHERE binding = 'provisional'",
    )
    .fetch_all(&pool)
    .await
    .expect("read the trail");
    assert!(providers.len() >= 2, "{providers:?}");
    assert!(
        providers.iter().all(|provider| provider
            == &(
                Some("urn:oid:2.16.528.1.1007.3.3".to_owned()),
                Some("90000013".to_owned())
            )),
        "{providers:?}"
    );
}

/// An attribute whose mapping sets `jit = true` is written.
#[tokio::test]
async fn an_attribute_marked_jit_is_written() {
    let mut config = jit_config();
    if let Some(email) = config
        .party_binding
        .person
        .get_or_insert_default()
        .attribute
        .get_mut(3)
    {
        email.jit = true;
    }
    let (_db, _pool, _service, app) = Box::pin(jit_app(config)).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000a002";
    let (status, body) = request(&app, &login(key, &named())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let user = user_named(&app, key).await;
    assert_eq!(user["emails"][0]["value"], "a.jansen@example.org", "{user}");
}

/// A login that fails the admission condition, a token with no mapped name
/// claim, an issuer the CDR admits no login of, and a key a SCIM client deleted or
/// deactivated are each refused, and none creates a `PERSON`.
#[tokio::test]
async fn each_case_jit_never_runs_for_is_refused_and_creates_nothing() {
    let (_db, pool, _service, app) = Box::pin(jit_app(jit_config())).await;
    let mut other_idp = named();
    other_idp["idp"] = json!("https://elsewhere.example/");
    for (key, claims) in [
        ("6a1f6c1e-0000-4000-8000-00000000b001", other_idp),
        (
            "6a1f6c1e-0000-4000-8000-00000000b002",
            json!({ "idp": IDP }),
        ),
    ] {
        let (status, body) = request(&app, &login(key, &claims)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.contains("refused-unbound"), "{body}");
        assert_eq!(persons_of(&pool, key).await, NONE, "{key}");
    }

    let deleted = "6a1f6c1e-0000-4000-8000-00000000b003";
    let (id, _) = created(&app, CLIENT, "deleted", deleted).await;
    let (status, _, _) = call(&app, "DELETE", &format!("/Users/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let before = person_count(&pool).await;
    let (status, body) = request(&app, &login(deleted, &named())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-deprovisioned"), "{body}");
    assert_eq!(
        person_count(&pool).await,
        before,
        "a deleted key is never recreated"
    );

    let inactive = "6a1f6c1e-0000-4000-8000-00000000b004";
    let (id, _) = created(&app, CLIENT, "inactive", inactive).await;
    let disable = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, _, body) = call(
        &app,
        "PATCH",
        &format!("/Users/{id}"),
        CLIENT,
        Some(&disable),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = request(&app, &login(inactive, &named())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-inactive"), "{body}");
    assert_eq!(persons_of(&pool, inactive).await.len(), 1);

    let (_db, pool, _service, app) = Box::pin(bare_app(jit_config())).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000b005";
    let (status, body) = request(&app, &login(key, &named())).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "no admission in the CDR creates nobody: {body}"
    );
    assert_eq!(persons_of(&pool, key).await, NONE);
}

/// The admission is read from the CDR on each first login: with none a
/// login is refused, an admission committed while the server runs creates
/// the next login's `PERSON`, an ended one creates none, and one that admits
/// another identity provider refuses this one, each with no restart.
#[tokio::test]
async fn an_admission_changed_in_the_cdr_decides_the_next_login() {
    let (_db, pool, service, app) = Box::pin(bare_app(jit_config())).await;
    let before = "6a1f6c1e-0000-4000-8000-00000000b101";
    let (status, body) = request(&app, &login(before, &named())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(persons_of(&pool, before).await, NONE);

    let (role_uid, performer) = admit(&service, &[], None).await;
    let admitted = "6a1f6c1e-0000-4000-8000-00000000b102";
    let (status, body) = request(&app, &login(admitted, &named())).await;
    assert_eq!(status, StatusCode::OK, "the next login is created: {body}");
    assert_eq!(persons_of(&pool, admitted).await.len(), 1);

    let yesterday = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .yesterday()
        .expect("a date")
        .to_string();
    let ended = Box::pin(
        service.party_update(
            PartyKind::Role,
            crate::party_binding_http::vo(&role_uid),
            ferroehr::service::demographic::precondition::Precondition::parse(
                Some(&role_uid),
                None,
            )
            .expect("an If-Match"),
            write(
                &admission(performer, &[], Some(&yesterday)),
                Some(&role_uid),
            ),
        ),
    )
    .await
    .expect("the admission ends");
    let after = "6a1f6c1e-0000-4000-8000-00000000b103";
    let (status, body) = request(&app, &login(after, &named())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the ended admission: {body}");
    assert_eq!(persons_of(&pool, after).await, NONE);

    let ended_uid = ended.meta.expect("a version").uid;
    let mut elsewhere = admission(performer, &[], None);
    elsewhere["capabilities"][0]["credentials"]["items"][2] =
        element(ADMIT_IDP, &text("https://sts.example/elsewhere"));
    Box::pin(
        service.party_update(
            PartyKind::Role,
            crate::party_binding_http::vo(&ended_uid),
            ferroehr::service::demographic::precondition::Precondition::parse(
                Some(&ended_uid),
                None,
            )
            .expect("an If-Match"),
            write(&elsewhere, Some(&ended_uid)),
        ),
    )
    .await
    .expect("the admission names another identity provider");
    let other = "6a1f6c1e-0000-4000-8000-00000000b104";
    let (status, body) = request(&app, &login(other, &named())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "another idp: {body}");
    assert_eq!(persons_of(&pool, other).await, NONE);
}

/// Ten concurrent first requests of one login commit one `PERSON`, and a
/// login whose `userName` another party holds is refused and binds nothing.
#[tokio::test]
async fn concurrent_first_requests_commit_one_person_and_a_taken_user_name_is_refused() {
    let mut config = jit_config();
    config.party_binding.issuer = vec![IssuerBinding {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(PERSONS),
        user_name_claim: Some("login_name".to_owned()),
        ..IssuerBinding::default()
    }];
    let (_db, pool, _service, app) = Box::pin(jit_app(config)).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000c001";
    let mut claims = named();
    claims["login_name"] = json!("ajansen");
    let token = login(key, &claims);
    let requests = (0..10).map(|_| request(&app, &token));
    for (status, body) in futures::future::join_all(requests).await {
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(persons_of(&pool, key).await.len(), 1);
    assert_eq!(person_count(&pool).await, 1, "exactly one PERSON");

    let other = "6a1f6c1e-0000-4000-8000-00000000c002";
    let (status, body) = request(&app, &login(other, &claims)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-unbound"), "{body}");
    assert_eq!(persons_of(&pool, other).await, NONE);
    assert_eq!(person_count(&pool).await, 1);
}

/// A later login with changed claims changes nothing; the SCIM client finds
/// the `PERSON` by `userName` with its key, a `PATCH` writes the
/// `externalId`, and a complete `PUT` completes it. An AQL query over the
/// version's lifecycle state lists the provisional persons, and the check
/// reports the old ones.
#[tokio::test]
async fn scim_completes_a_provisional_person_that_a_later_login_leaves_alone() {
    let (_db, pool, service, app) = Box::pin(jit_app(jit_config())).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000d001";
    let (status, body) = request(&app, &login(key, &named())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let person = persons_of(&pool, key).await[0];

    let aql = "SELECT p/uid/value FROM VERSION v[lifecycle_state/value='incomplete'] \
               CONTAINS PERSON p";
    let rows = service
        .execute_aql(aql, None, &AqlQueryRequest::default())
        .await
        .expect("the query runs")
        .result_set["rows"]
        .clone();
    assert!(
        rows.to_string().contains(&person.to_string()),
        "the provisional PERSON is listed: {rows}"
    );
    let old = service
        .provisional_persons(0)
        .await
        .expect("the check runs");
    assert_eq!(
        old.iter().map(|(party, _)| party.0).collect::<Vec<_>>(),
        [person]
    );
    assert_eq!(
        service
            .provisional_persons(30)
            .await
            .expect("the check runs"),
        [],
        "a PERSON created today is no older than 30 days"
    );

    let mut changed = named();
    changed["name"] = json!("Dr. A. Jansen-de Vries");
    let (status, body) = request(&app, &login(key, &changed)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(versions(&pool, person).await.len(), 1, "JIT only creates");

    let user = user_named(&app, key).await;
    assert_eq!(user[EXTENSION]["loginIdentifier"], key, "the key is equal");
    let path = format!("/Users/{person}");
    let add = patch_op(json!([{ "op": "add", "path": "externalId", "value": "emp-0001" }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&add), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["externalId"], "emp-0001");

    let complete = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User", EXTENSION],
        "userName": key,
        "externalId": "emp-0001",
        "name": { "formatted": "Dr. A. Jansen", "givenName": "Anna", "familyName": "Jansen" },
        "emails": [{ "type": "work", "value": "a.jansen@example.org" }],
        EXTENSION: { "loginIdentifier": key }
    });
    let (status, _, body) = call(&app, "PUT", &path, CLIENT, Some(&complete), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body[EXTENSION].get("lifecycleState").is_none(),
        "complete: {body}"
    );
    let history = versions(&pool, person).await;
    assert_eq!(
        history.last().map(|(state, ..)| state.as_str()),
        Some("532")
    );
    assert_eq!(
        service
            .provisional_persons(0)
            .await
            .expect("the check runs"),
        [],
        "a complete PERSON is no longer provisional"
    );
}

/// The issuer of the patients' logins.
const PATIENTS: &str = "https://patients.example";

/// The subject of the service account the persons issuer declares app-only.
const SERVICE_ACCOUNT: &str = "service-account-lab";

/// [`jit_config`] beside a patients issuer, whose persons issuer marks a
/// guest by `acct = 1` and its service account app-only, its `[[auth.party]]` table admitting guests.
fn callers_config() -> AppConfig {
    let mut config = jit_config();
    if let Some(persons) = config
        .auth
        .shared
        .issuer
        .iter_mut()
        .find(|entry| entry.issuer.expose() == PERSONS)
    {
        persons.guest = Some(vec![Marker::Claim {
            claim: "acct".to_owned(),
            value: "1".to_owned(),
        }]);
        persons.app_only = Some(vec![Marker::Subjects(vec![SERVICE_ACCOUNT.to_owned()])]);
    }
    {
        let entries = &mut config.auth.shared.issuer;
        entries.push(IssuerEntry {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(PATIENTS),
            kind: Some(IssuerKind::Patients),
            audience: vec![AUDIENCE.to_owned()],
            jwks_file: common::jwks_file().display().to_string(),
            ..IssuerEntry::default()
        });
    }
    if let Some(party) = config
        .auth
        .shared
        .party
        .iter_mut()
        .find(|party| party.issuer.expose() == PERSONS)
    {
        party.guests = true;
    }
    config
}

/// A patient of the patients issuer and an app-only token of the persons
/// issuer pass the binding as before, the patient's scopes then deciding,
/// and create no `PERSON`, and a guest is refused and creates none while no
/// admission admits guests.
#[tokio::test]
async fn a_patient_a_system_caller_and_a_guest_without_admit_guests_create_nothing() {
    let (_db, pool, _service, app) = Box::pin(jit_app(callers_config())).await;
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let patient = common::bearer(&json!({ "iss": PATIENTS, "aud": AUDIENCE, "exp": exp,
        "sub": "6a1f6c1e-0000-4000-8000-00000000e001", "name": "A. Patient",
        "scope": "patient/template-*.r" }));
    let (status, body) = request(&app, &patient).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("the credential does not permit") && !body.contains("refused-"),
        "a patient is not bound, and its scopes decide: {body}"
    );

    let mut system = named();
    system["scope"] = json!("system/template-*.r");
    let (status, body) = request(&app, &login(SERVICE_ACCOUNT, &system)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a system caller is not bound: {body}"
    );

    let guest_key = "6a1f6c1e-0000-4000-8000-00000000e003";
    let mut guest = named();
    guest["acct"] = json!("1");
    let (status, body) = request(&app, &login(guest_key, &guest)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-unbound"), "{body}");
    assert_eq!(
        person_count(&pool).await,
        0,
        "no caller here creates a PERSON"
    );
}

/// Under an admission whose `admit_guests` is true an admitted guest's first
/// login creates its provisional `PERSON` and is served bound to it.
#[tokio::test]
async fn a_guest_under_admit_guests_is_created() {
    let (_db, pool, service, app) = Box::pin(bare_app(callers_config())).await;
    admit(
        &service,
        &[element(
            ADMIT_GUESTS,
            &json!({ "_type": "DV_BOOLEAN", "value": true }),
        )],
        None,
    )
    .await;
    let key = "6a1f6c1e-0000-4000-8000-00000000e004";
    let mut guest = named();
    guest["acct"] = json!("1");
    let (status, body) = request(&app, &login(key, &guest)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(persons_of(&pool, key).await.len(), 1);
}

/// The provisional `PERSON` performs a professional `ROLE` from today and
/// represents nobody, so it binds as a professional: the side table carries
/// the marker, and a representative login on it is refused.
#[tokio::test]
async fn the_provisional_person_carries_the_professional_marker() {
    let (_db, pool, _service, app) = Box::pin(jit_app(jit_config())).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000e005";
    let (status, body) = request(&app, &login(key, &named())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (professional_today, representative): (bool, bool) = sqlx::query_as(
        "SELECT professional @> current_date, representative FROM party.binding_identifier \
         WHERE selector = 'key' AND value = $1 AND retired_at IS NULL",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("read the side table");
    assert!(professional_today, "the professional marker");
    assert!(!representative, "not a representative");
}

/// A SCIM User whose `userName` a provisional `PERSON` already holds, with
/// another key, is refused, and the provisional `PERSON` keeps its key.
#[tokio::test]
async fn a_scim_user_taking_a_provisional_user_name_with_another_key_is_refused() {
    let (_db, pool, _service, app) = Box::pin(jit_app(jit_config())).await;
    let key = "6a1f6c1e-0000-4000-8000-00000000e006";
    let (status, body) = request(&app, &login(key, &named())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let squatter = persons_of(&pool, key).await;
    assert_eq!(squatter.len(), 1);

    let other = "6a1f6c1e-0000-4000-8000-00000000e007";
    let body = super::create_body(key, "emp-0002", other);
    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&body), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness", "RFC 7644 §3.12: {body}");
    assert_eq!(persons_of(&pool, other).await, NONE);
    assert_eq!(persons_of(&pool, key).await, squatter);
}
