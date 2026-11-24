// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Representations in a deployment that provisions its professionals over
//! SCIM (#5622) and binds them through a `[auth.issuer.<name>.party]` table
//! (#5652), with a second issuer whose logins bind persons' own `PERSON`s.
//!
//! A representative is a `PERSON` whose login binds them. Their login reaches
//! a subject only through a live representation recorded for the provider
//! under a `write_representation` grant naming it; SCIM writes the identity
//! only, never a representation (Regulation (EU) 2025/327 Art. 4(2)). No
//! openEHR spec governs provisioning or resolving a login to a party: our
//! own design.

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
use super::{CLIENT, LOGIN, MACHINES, SCOPE};
use crate::common::{self, BASE, EXT};
use crate::policy_fixture::{capability, grant, scope};
use crate::portal_representatives_http::{PORTAL, child, representation, token};

/// The client that records representations for the hospital: it holds
/// `write_representation` for [`crate::portal_representatives_http::PROVIDER`].
const WRITER: &str = "registry";
/// A client holding `write_representation` for another provider only.
const CLINIC: &str = "clinic-registry";
/// Frank's login, the binding key of his `PERSON`.
const FRANK: &str = "frank-login";

/// The SCIM harness configuration trusting [`PORTAL`] beside the
/// professionals' issuer, its logins bound under its own identifiers.
fn config() -> ferroehr_rest::config::AppConfig {
    let mut config = zero_config();
    common::add_issuer(
        &mut config,
        IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            party: Some(ferrohealth_smart::party::config::PartySection {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(PORTAL),
                identifier_issuer: PORTAL.to_owned(),
                identifier_type: "login".to_owned(),
                ..ferrohealth_smart::party::config::PartySection::default()
            }),
            ..common::entry(PORTAL, super::AUDIENCE)
        },
    );
    config.party_binding.provider = Some(crate::portal_representatives_http::provider());
    config
}

/// The router over a fresh database holding the default definitions, with
/// the person mapping compiled as the server installs it, the hospital's
/// client granted `scim_provision` under [`LOGIN`] and `access_demographic`,
/// and the representation writers granted.
async fn world() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
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
    // The replica boots on the configuration that adds the issuer, as the
    // server does: its lease records the key space and its first tend fills it.
    service
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the replica boots")
        .expect("a lease")
        .tend()
        .await
        .expect("the issuer's key space fills");
    let hospital = capability(
        "scim_provision",
        &[scope("binding_issuer", LOGIN), scope("token_scope", SCOPE)],
    );
    let demographic = capability("access_demographic", &[]);
    grant(&service, MACHINES, CLIENT, &[hospital, demographic]).await;
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

/// Frank's `PERSON`, bound by his login under [`PORTAL`]; its id.
async fn frank(service: &FerroEhrService) -> String {
    common::shipped_patient(service, (PORTAL, FRANK), "Frank Visser").await
}

/// The status and body of `GET /ehr/{ehr}` under Frank's own login.
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

/// A SCIM client writes no representation: its `PARTY_RELATIONSHIP` naming
/// Frank is refused, and Frank still reaches nothing.
#[tokio::test]
async fn a_scim_client_is_refused_a_representation() {
    let (_db, service, app) = world().await;
    let (eva, ehr) = child(&service).await;
    let user = frank(&service).await;
    let frank_id = ferroehr::ids::VoId(user.parse().expect("a container id"));
    let request = common::req("POST", format!("{EXT}/demographic/party_relationship"))
        .auth(&system(CLIENT))
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
    let (_db, service, app) = world().await;
    let (eva, ehr) = child(&service).await;
    let user = frank(&service).await;
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
    let (_db, service, app) = world().await;
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
