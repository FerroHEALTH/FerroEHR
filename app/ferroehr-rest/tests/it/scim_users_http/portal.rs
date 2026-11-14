// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A patient portal's representatives, provisioned over SCIM by the portal's
//! own client (#5622), in a deployment that also binds its professionals
//! through a `persons` issuer's `[[auth.party]]` table (#5652).
//!
//! The portal client's `binding_issuer` is the `patients` issuer, one of the
//! key issuers the server compiles its person mapping from. Its Users are
//! `PERSON`s of the patient template with no professional `ROLE`. A portal
//! login a `PERSON` carries is held to that `PERSON`: the patient only when it
//! is the EHR's subject, a representative only through a live
//! representation, and refused otherwise. SCIM writes the identity only; the
//! representation is the patient's or an administrator's (Regulation (EU)
//! 2025/327 Art. 4(2)). No openEHR spec governs provisioning or resolving a
//! login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::kind::IssuerKind;
use http::StatusCode;
use serde_json::{Value, json};

use super::default_definitions::{seeded, zero_config};
use super::{MACHINES, SCOPE, call, patch_op};
use crate::common::{self, BASE, EXT};
use crate::policy_fixture::{capability, grant, scope};
use crate::portal_representatives_http::{PORTAL, child, representation, token};

/// The portal's provisioning client.
const PORTAL_CLIENT: &str = "portal-sync";
/// Frank's portal login, the binding key of his `PERSON`.
const FRANK: &str = "frank-login";

/// The SCIM harness configuration with the portal trusted as a `patients`
/// issuer beside the professionals' `persons` issuer and its table.
fn config() -> ferroehr_rest::config::AppConfig {
    let mut config = zero_config();
    config.auth.shared.issuer.push(IssuerEntry {
        kind: Some(IssuerKind::Patients),
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(PORTAL, super::AUDIENCE)
    });
    config.auth.smart.patient_issuers = vec![ferrohealth_secret::secret_url::SecretUrl::new(PORTAL)];
    config
}

/// The router over a fresh database holding the default definitions, with
/// the person mapping compiled as the server installs it and the portal's
/// client granted `scim_provision` under the portal issuer and
/// `access_demographic`.
async fn portal() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let config = config();
    let (db, pool) = common::migrated_pool().await;
    let resolved = ferroehr::config::auth::resolve(&config.auth.shared).expect("[auth] resolves");
    let mapping = ferroehr::config::binding::installed_mapping(
        &config.party_binding.sections(),
        &resolved,
        "ADMIN",
    )
    .expect("the mapping compiles");
    let service = Arc::new(seeded(&config, &pool).await.with_binding_mapping(mapping));
    let provisioning = capability(
        "scim_provision",
        &[scope("binding_issuer", PORTAL), scope("token_scope", SCOPE)],
    );
    let demographic = capability("access_demographic", &[]);
    grant(&service, MACHINES, PORTAL_CLIENT, &[provisioning, demographic]).await;
    let app = common::router_with(config, Arc::clone(&service)).await;
    (db, service, app)
}

/// The SCIM User Frank of the portal's client; its id.
async fn frank(app: &Router) -> String {
    let body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User", super::EXTENSION],
        "userName": "frank",
        "name": { "formatted": "Frank Visser" },
        super::EXTENSION: { "loginIdentifier": FRANK }
    });
    let (status, _, created) = call(app, "POST", "/Users", PORTAL_CLIENT, Some(&body), &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["id"].as_str().expect("an id").to_owned()
}

/// The status and body of `GET /ehr/{ehr}` under Frank's own portal login.
async fn read_as_frank(app: &Router, ehr: &str) -> (StatusCode, String) {
    let request = common::req("GET", format!("{BASE}/ehr/{ehr}"))
        .auth(&token(FRANK, ehr, &json!({})))
        .accept("application/json")
        .build();
    let (status, _, body) = common::send(app, request).await;
    (status, body)
}

/// The current uid and body of the `PERSON` `id`, read by the server.
async fn head(service: &FerroEhrService, id: &str) -> (String, Value) {
    let address = ferroehr::versioning::object_version_id::parse_uid_based_id(id)
        .expect("a container id");
    let read = service
        .party_get(PartyKind::Person, address, None)
        .await
        .expect("the PERSON reads");
    (read.meta.expect("a version").uid, read.body)
}

/// A write of `body` as the version after `uid`.
fn next<T: serde::de::DeserializeOwned>(body: &Value, uid: &str) -> WriteRequest<T> {
    let mut write =
        WriteRequest::new(openehr_its::json::from_canonical_value(body).expect("the body decodes"));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// The portal's client provisions Frank in the patient template with no
/// `ROLE`. His portal login reaches nothing until a representation is
/// recorded; an administrator records it and he reads Eva's record; the
/// client deactivates him and he is refused on the next request.
#[tokio::test]
async fn the_portal_provisions_a_representative_who_binds_only_through_a_representation() {
    let (_db, service, app) = portal().await;
    let (eva, ehr) = child(&service).await;
    let user = frank(&app).await;
    let (uid, body) = head(&service, &user).await;
    assert_eq!(
        body["archetype_details"]["template_id"]["value"],
        ferroehr::party_binding::shipped::PATIENT,
        "{body}"
    );
    assert!(body.get("roles").is_none(), "no professional ROLE: {body}");

    let (status, refused) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(refused.contains("refused-representation-unrecorded"), "{refused}");

    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let mut represented = body.clone();
    let mut relationship = representation(frank_id, eva, "253", None);
    relationship["source"]["id"]["value"] = json!(user);
    represented["relationships"] = json!([relationship]);
    Box::pin(service.party_update(
        PartyKind::Person,
        frank_id,
        Precondition::parse(Some(&uid), None).expect("an If-Match"),
        next(&represented, &uid),
    ))
    .await
    .expect("the administrator records the representation");
    let (status, body) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let deactivate = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, _, deactivated) = call(
        &app,
        "PATCH",
        &format!("/Users/{user}"),
        PORTAL_CLIENT,
        Some(&deactivate),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deactivated}");
    let (status, refused) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

/// The owning portal client writes no representation: neither inline in
/// the `PERSON` it provisioned, through the Demographic API, nor as a
/// `PARTY_RELATIONSHIP` of its own.
#[tokio::test]
async fn the_portal_client_is_refused_every_representation_write() {
    let (_db, service, app) = portal().await;
    let (eva, ehr) = child(&service).await;
    let user = frank(&app).await;
    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let system = |client: &str| {
        let exp = jiff::Timestamp::now().as_second() + 3600;
        common::bearer(&json!({ "iss": MACHINES, "aud": super::AUDIENCE, "exp": exp,
            "sub": client, "client_id": client,
            "scope": "system/composition-*.cruds system/aql-*.s" }))
    };

    let (uid, body) = head(&service, &user).await;
    let mut represented = body;
    let mut relationship = representation(frank_id, eva, "253", None);
    relationship["source"]["id"]["value"] = json!(user);
    represented["relationships"] = json!([relationship.clone()]);
    let request = common::req("PUT", format!("{BASE}/demographic/person/{user}"))
        .auth(&system(PORTAL_CLIENT))
        .accept("application/json")
        .if_match(format!("\"{uid}\""))
        .json(represented.to_string())
        .build();
    let (status, _, refused) = common::send(&app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "inline: {refused}");

    let request = common::req("POST", format!("{EXT}/demographic/party_relationship"))
        .auth(&system(PORTAL_CLIENT))
        .accept("application/json")
        .json(relationship.to_string())
        .build();
    let (status, _, refused) = common::send(&app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "standalone: {refused}");

    let (status, refused) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

/// Inside a SCIM request of the portal client, a version of the `PERSON` it
/// owns that adds a representation is refused by the party protection,
/// whatever route carries it.
#[tokio::test]
async fn a_scim_version_that_adds_a_representation_is_refused() {
    let (_db, service, app) = portal().await;
    let (eva, _) = child(&service).await;
    let user = frank(&app).await;
    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let (uid, body) = head(&service, &user).await;
    let mut represented = body;
    let mut relationship = representation(frank_id, eva, "253", None);
    relationship["source"]["id"]["value"] = json!(user);
    represented["relationships"] = json!([relationship]);
    let refused = Box::pin(ferroehr::party_binding::protect::provisioning(
        PORTAL.to_owned(),
        service.party_update(
            PartyKind::Person,
            frank_id,
            Precondition::parse(Some(&uid), None).expect("an If-Match"),
            next(&represented, &uid),
        ),
    ))
    .await
    .expect_err("a SCIM client writes no representation");
    assert!(
        refused.to_string().contains("changes its representations"),
        "{refused}"
    );
}
