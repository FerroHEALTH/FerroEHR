// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A patient portal's representatives, provisioned over SCIM by the portal's
//! own client (#5622), in a deployment that also binds its professionals
//! through a `persons` issuer's `[auth.issuer.<name>.party]` table (#5652).
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
use http::StatusCode;
use serde_json::{Value, json};

use super::default_definitions::{seeded, zero_config};
use super::{CLIENT, LOGIN, MACHINES, SCOPE, call, patch_op};
use crate::common::{self, BASE, EXT};
use crate::policy_fixture::{capability, grant, scope};
use crate::portal_representatives_http::{PORTAL, child, representation, token};

/// The portal's provisioning client.
const PORTAL_CLIENT: &str = "portal-sync";
/// The client that records representations for the hospital: it holds
/// `write_representation` for [`crate::portal_representatives_http::PROVIDER`].
const WRITER: &str = "registry";
/// A client holding `write_representation` for another provider only.
const CLINIC: &str = "clinic-registry";
/// Frank's portal login, the binding key of his `PERSON`.
const FRANK: &str = "frank-login";

/// The SCIM harness configuration with the portal trusted as a `patients`
/// issuer beside the professionals' `persons` issuer and its table.
fn config() -> ferroehr_rest::config::AppConfig {
    let mut config = zero_config();
    common::add_issuer(
        &mut config,
        IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            ..common::entry(PORTAL, super::AUDIENCE)
        },
    );
    config.party_binding.provider = Some(crate::portal_representatives_http::provider());
    config
}

/// The router over a fresh database holding the default definitions, with
/// the person mapping compiled as the server installs it, the portal's
/// client granted `scim_provision` under the portal issuer and
/// `access_demographic`, and the hospital's client `scim_provision` under
/// [`LOGIN`].
async fn portal() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let config = config();
    let (db, pool) = common::migrated_pool().await;
    let resolved = ferroehr::config::auth::resolve(&config.auth.shared, &config.audience_bases())
        .expect("[auth] resolves");
    let mapping = ferroehr::config::binding::installed_mapping(
        &config.party_binding.sections(),
        &resolved,
        "ADMIN",
    )
    .expect("the mapping compiles");
    let service = Arc::new(seeded(&config, &pool).await.with_binding_mapping(mapping));
    // The replica boots on the configuration that adds the portal issuer, as
    // the server does: its lease records the key space and its first tend
    // fills it.
    service
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the replica boots")
        .expect("a lease")
        .tend()
        .await
        .expect("the portal issuer's key space fills");
    let provisioning = capability(
        "scim_provision",
        &[scope("binding_issuer", PORTAL), scope("token_scope", SCOPE)],
    );
    let demographic = capability("access_demographic", &[]);
    grant(
        &service,
        MACHINES,
        PORTAL_CLIENT,
        &[provisioning, demographic],
    )
    .await;
    let hospital = capability(
        "scim_provision",
        &[scope("binding_issuer", LOGIN), scope("token_scope", SCOPE)],
    );
    grant(&service, MACHINES, CLIENT, &[hospital]).await;
    let writes = |provider: (&str, &str)| {
        capability(
            "write_representation",
            &[crate::policy_fixture::identifier("provider", provider)],
        )
    };
    let demographic = capability("access_demographic", &[]);
    let hospital = crate::portal_representatives_http::PROVIDER;
    grant(
        &service,
        MACHINES,
        WRITER,
        &[demographic.clone(), writes(hospital)],
    )
    .await;
    let clinic = writes(("urn:example:providers", "clinic"));
    grant(&service, MACHINES, CLINIC, &[demographic, clinic]).await;
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
    let address =
        ferroehr::versioning::object_version_id::parse_uid_based_id(id).expect("a container id");
    let read = service
        .party_get(PartyKind::Person, address, None)
        .await
        .expect("the PERSON reads")
        .into_value_response()
        .expect("the stored body parses");
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

/// A standalone representation from the `PERSON` `source` to the subject
/// `subject`, of a deployment's own archetype, as the patient authorises a
/// proxy or an administrator records a guardian.
fn standalone(source: ferroehr::ids::VoId, subject: ferroehr::ids::VoId) -> Value {
    representation(source, subject, "253", None)
}

/// A client-credentials token of `client` for the Demographic API.
fn system(client: &str) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(
        &json!({ "iss": MACHINES, "aud": super::AUDIENCE, "exp": exp,
        "sub": client, "client_id": client,
        "scope": "system/composition-*.cruds system/aql-*.s" }),
    )
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
    let (_, body) = head(&service, &user).await;
    assert_eq!(
        body["archetype_details"]["template_id"]["value"],
        ferroehr::party_binding::shipped::PATIENT,
        "the PERSON is of the patient template"
    );
    assert!(body.get("roles").is_none(), "no professional ROLE");

    let (status, refused) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        refused.contains("refused-representation-unrecorded"),
        "{refused}"
    );

    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let mut write = WriteRequest::<openehr_rm::prelude::PartyRelationship>::new(
        openehr_its::json::from_canonical_value(&standalone(frank_id, eva))
            .expect("a PARTY_RELATIONSHIP"),
    );
    write.lifecycle_state = Some("532".to_owned());
    Box::pin(service.party_relationship_create(write))
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

/// The owning portal client writes no representation of its own
/// representative: its `PARTY_RELATIONSHIP` naming Frank is refused, and
/// Frank still reaches nothing.
#[tokio::test]
async fn the_portal_client_is_refused_a_representation() {
    let (_db, service, app) = portal().await;
    let (eva, ehr) = child(&service).await;
    let user = frank(&app).await;
    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let request = common::req("POST", format!("{EXT}/demographic/party_relationship"))
        .auth(&system(PORTAL_CLIENT))
        .accept("application/json")
        .json(standalone(frank_id, eva).to_string())
        .build();
    let (status, _, refused) = common::send(&app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(refused.contains("write_representation"), "{refused}");
    let (status, refused) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

/// The `POST` of `relationship` at `/demographic/party_relationship` as the
/// client `client`; its status and body.
async fn post_as(app: &Router, client: &str, relationship: &Value) -> (StatusCode, String) {
    let request = common::req("POST", format!("{EXT}/demographic/party_relationship"))
        .auth(&system(client))
        .accept("application/json")
        .json(relationship.to_string())
        .build();
    let (status, _, body) = common::send(app, request).await;
    (status, body)
}

/// A representation is written under a `write_representation` grant whose
/// `provider` elements name its provider (draft access control decision
/// procedure, Writing Relationships): a grant for another provider is
/// refused `403`, the hospital's own records it, and Frank then reads Eva's
/// record.
#[tokio::test]
async fn a_representation_is_written_under_a_grant_for_its_provider() {
    let (_db, service, app) = portal().await;
    let (eva, ehr) = child(&service).await;
    let user = frank(&app).await;
    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let guardian = representation(frank_id, eva, "264", None);

    let (status, refused) = post_as(&app, CLINIC, &guardian).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        refused.contains("hospital"),
        "names the provider: {refused}"
    );
    let (status, refused) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

    let (status, body) = post_as(&app, WRITER, &guardian).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = read_as_frank(&app, &ehr).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // NOTE: EHDS Art. 4(2), the subject authorises a proxy; a representative writes no
    // authorisation of themself, so Frank's own write of one is refused.
    let request = common::req("POST", format!("{EXT}/demographic/party_relationship"))
        .auth(&token(FRANK, &ehr, &json!({})))
        .accept("application/json")
        .json(representation(frank_id, eva, "253", None).to_string())
        .build();
    let (status, _, refused) = common::send(&app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
}

/// Inside a SCIM request, a version of a `PERSON` the client owns that adds
/// a representation to its relationships is refused: a representation is a
/// `PARTY_RELATIONSHIP` of its own (draft access control package, The
/// Relationship Store), whatever route carries it.
#[tokio::test]
async fn a_scim_version_that_touches_a_representation_is_refused() {
    let (_db, service, app) = portal().await;
    let (eva, _) = child(&service).await;
    let (user, _) = super::created(&app, CLIENT, "anna", "anna-login").await;
    let anna = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let (uid, body) = head(&service, &user).await;
    let mut represented = body;
    represented["relationships"] = json!([representation(anna, eva, "254", None)]);
    let refused = Box::pin(ferroehr::party_binding::protect::provisioning(
        LOGIN.to_owned(),
        service.party_update(
            PartyKind::Person,
            anna,
            Precondition::parse(Some(&uid), None).expect("an If-Match"),
            next(&represented, &uid),
        ),
    ))
    .await
    .expect_err("a SCIM client writes no representation");
    assert!(
        format!("{refused:?}").contains("/demographic/party_relationship"),
        "{refused:?}"
    );
}

/// A patient portal's representative holds no role: the portal client's
/// User asking for one is refused, as a representative acts only through a
/// representation.
#[tokio::test]
async fn a_portal_representative_holds_no_role() {
    let (_db, _service, app) = portal().await;
    let body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User", super::EXTENSION],
        "userName": "frank",
        "roles": [{ "value": "nurse" }],
        super::EXTENSION: { "loginIdentifier": FRANK }
    });
    let (status, _, refused) = call(&app, "POST", "/Users", PORTAL_CLIENT, Some(&body), &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(refused.to_string().contains("hold no role"), "{refused}");
}
