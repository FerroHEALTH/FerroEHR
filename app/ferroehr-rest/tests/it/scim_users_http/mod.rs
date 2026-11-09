// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `/scim/v2/Users` (#5014) through the assembled router: the RFC 7644 §3.3
//! create, the §3.4.1 read with its §3.14 entity tag, the §3.4.2 query with
//! the §3.4.2.2 `eq` and `and` filter this provider supports and an
//! unsupported one answered `invalidFilter`, and the §3.12 refusals. Each
//! client sees only its own Users. The definitions are synthetic ADL 2
//! archetypes. Who may provision, and the mapping to a `PERSON`, are our own
//! design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::binding::person::{
    GroupSection, MembershipSection, PersonAttribute, PersonSection, PersonTemplate, RoleSection,
    Selector,
};
use ferroehr::config::scim::{ScimClient, ScimConfig};
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerEntry, IssuerKey};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::Preset;
use http::{StatusCode, header};
use serde_json::{Value, json};

use crate::common;

mod entra;
mod groups;
mod groups_entra;
mod jit;
mod refusals;

/// The deployment's own authorization server, which issues the clients' tokens.
const MACHINES: &str = "https://as.example";
/// The issuers of the two clients' professionals' logins.
const PERSONS: &str = "https://login.example";
const OTHER_PERSONS: &str = "https://other-login.example";
/// The audience of the tokens.
const AUDIENCE: &str = "ferroehr";
/// The identifier issuer of the binding keys the first client provisions.
const LOGIN: &str = "urn:example:login";
/// The identifier issuer of the second client's binding keys.
const OTHER_LOGIN: &str = "urn:example:other-login";
/// The two clients and their scope.
const CLIENT: &str = "provisioning";
const OTHER: &str = "other-provisioning";
const SCOPE: &str = "ferroehr/scim";
/// The professional `PERSON` and `ROLE` archetypes.
const PERSON: &str = "openEHR-DEMOGRAPHIC-PERSON.professional.v1.0.0";
const ROLE: &str = "openEHR-DEMOGRAPHIC-ROLE.professional.v1.0.0";
/// The identifier element every selector shares, and the name element.
const KEY_PATH: &str = "/details[id2]/items[id3]";
const NAME_PATH: &str = "/identities[id4]/details[id5]/items[id6]";
const GIVEN_PATH: &str = "/identities[id4]/details[id5]/items[id15]";
const FAMILY_PATH: &str = "/identities[id4]/details[id5]/items[id16]";
const EMAIL_PATH: &str = "/contacts[id10]/addresses[id11]/details[id12]/items[id13]";
/// The group template, the membership archetype and the identifier issuers
/// of each client's Groups' `externalId`s.
const GROUP: &str = "openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0";
const MEMBERSHIP_HRID: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0";
/// The archetype id each membership's `archetype_node_id` carries, its
/// interface form (AM 2 Identification §Archetype Identifiers).
const MEMBERSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1";
const GROUPS: &str = "urn:example:groups";
const OTHER_GROUPS: &str = "urn:example:other-groups";
/// The custom extension FerroEHR's User attributes sit in.
const EXTENSION: &str = "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User";

/// A synthetic ADL 2 archetype `hrid` rooted at `root`, with the formatted,
/// given and family names in `identities`, a work email in `contacts` and repeatable identifiers in
/// `details`.
fn archetype(hrid: &str, root: &str) -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)
    {hrid}

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <[\"name\"] = <\"FerroEHR tests\">>
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A party SCIM provisioning writes.\">
        >
    >

definition
    {root}[id1] matches {{
        identities matches {{
            PARTY_IDENTITY[id4] matches {{
                details matches {{
                    ITEM_TREE[id5] matches {{
                        items matches {{
                            ELEMENT[id6] occurrences matches {{0..1}} matches {{
                                value matches {{ DV_TEXT[id8] }}
                            }}
                            ELEMENT[id15] occurrences matches {{0..1}} matches {{
                                value matches {{ DV_TEXT[id17] }}
                            }}
                            ELEMENT[id16] occurrences matches {{0..1}} matches {{
                                value matches {{ DV_TEXT[id18] }}
                            }}
                        }}
                    }}
                }}
            }}
        }}
        contacts matches {{
            CONTACT[id10] occurrences matches {{0..1}} matches {{
                addresses matches {{
                    ADDRESS[id11] matches {{
                        details matches {{
                            ITEM_TREE[id12] matches {{
                                items matches {{
                                    ELEMENT[id13] occurrences matches {{0..1}} matches {{
                                        value matches {{ DV_TEXT[id14] }}
                                    }}
                                }}
                            }}
                        }}
                    }}
                }}
            }}
        }}
        details matches {{
            ITEM_TREE[id2] matches {{
                items matches {{
                    ELEMENT[id3] occurrences matches {{0..*}} matches {{
                        value matches {{ DV_IDENTIFIER[id9] }}
                    }}
                }}
            }}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"The root.\">>
            [\"id2\"] = <text = <\"Details\"> description = <\"The details.\">>
            [\"id3\"] = <text = <\"Identifier\"> description = <\"An identifier.\">>
            [\"id4\"] = <text = <\"Identity\"> description = <\"The identity.\">>
            [\"id5\"] = <text = <\"Name\"> description = <\"The name structure.\">>
            [\"id6\"] = <text = <\"Name text\"> description = <\"The name.\">>
            [\"id10\"] = <text = <\"Contact\"> description = <\"The contact.\">>
            [\"id11\"] = <text = <\"Email\"> description = <\"The email address.\">>
            [\"id12\"] = <text = <\"Email details\"> description = <\"The address structure.\">>
            [\"id13\"] = <text = <\"Email text\"> description = <\"The work email.\">>
            [\"id15\"] = <text = <\"Given name\"> description = <\"The given name.\">>
            [\"id16\"] = <text = <\"Family name\"> description = <\"The family name.\">>
        >
    >
"
    )
}

/// The shared party mapping: one person template whose selectors share a
/// path, `name.formatted` mapped to its name with the given and family names
/// beside it, the work email to its contact,
/// and the professional role.
fn binding() -> PartyBindingConfig {
    let selector = |kind: &str| Selector {
        path: KEY_PATH.to_owned(),
        identifier_type: kind.to_owned(),
    };
    PartyBindingConfig {
        namespace: "demographic".to_owned(),
        person: PersonSection {
            template: vec![PersonTemplate {
                template_id: PERSON.to_owned(),
                key: selector("guid"),
                user_name: Some(selector("scim_user_name")),
                external_id: Some(selector("scim_external_id")),
                default: true,
            }],
            attribute: vec![
                PersonAttribute {
                    scim: Some("name.formatted".to_owned()),
                    name: true,
                    path: [(PERSON.to_owned(), NAME_PATH.to_owned())].into(),
                    ..PersonAttribute::default()
                },
                PersonAttribute {
                    scim: Some("name.givenName".to_owned()),
                    path: [(PERSON.to_owned(), GIVEN_PATH.to_owned())].into(),
                    ..PersonAttribute::default()
                },
                PersonAttribute {
                    scim: Some("name.familyName".to_owned()),
                    path: [(PERSON.to_owned(), FAMILY_PATH.to_owned())].into(),
                    ..PersonAttribute::default()
                },
                PersonAttribute {
                    scim: Some("emails[type eq \"work\"].value".to_owned()),
                    path: [(PERSON.to_owned(), EMAIL_PATH.to_owned())].into(),
                    ..PersonAttribute::default()
                },
            ],
        },
        professional_role: Some(RoleSection {
            template_id: ROLE.to_owned(),
        }),
        group: Some(GroupSection {
            template_id: GROUP.to_owned(),
            external_id: selector("group_external_id"),
            display_name: Some(NAME_PATH.to_owned()),
        }),
        membership: Some(MembershipSection {
            archetype_id: MEMBERSHIP.to_owned(),
        }),
        ..PartyBindingConfig::default()
    }
}

/// A synthetic ADL 2 membership archetype: a `PARTY_RELATIONSHIP` with
/// optional details.
fn membership_archetype() -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)
    {MEMBERSHIP_HRID}

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <[\"name\"] = <\"FerroEHR tests\">>
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A member of a group.\">
        >
    >

definition
    PARTY_RELATIONSHIP[id1] matches {{
        details matches {{
            ITEM_TREE[id2] occurrences matches {{0..1}}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Membership\"> description = <\"A membership.\">>
            [\"id2\"] = <text = <\"Details\"> description = <\"The details.\">>
        >
    >
"
    )
}

fn config() -> AppConfig {
    let machines = IssuerEntry {
        issuer: MACHINES.to_owned(),
        kind: Some(IssuerKind::Machines),
        profile: Preset::Rfc9068,
        audience: vec![AUDIENCE.to_owned()],
        jwks_file: common::jwks_file().display().to_string(),
        // NOTE: the test key signs RFC 9068 `at+jwt` tokens.
        token_types: Some(vec!["at+jwt".to_owned()]),
        ..IssuerEntry::default()
    };
    let persons = |issuer: &str| IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(issuer, AUDIENCE)
    };
    let party = |issuer: &str, identifier_issuer: &str, groups: &str| PartySection {
        issuer: issuer.to_owned(),
        identifier_issuer: identifier_issuer.to_owned(),
        identifier_type: "guid".to_owned(),
        group_claim: "groups".to_owned(),
        group_identifier_issuer: groups.to_owned(),
        ..PartySection::default()
    };
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(vec![machines, persons(PERSONS), persons(OTHER_PERSONS)]),
        party: vec![
            party(PERSONS, LOGIN, GROUPS),
            party(OTHER_PERSONS, OTHER_LOGIN, OTHER_GROUPS),
        ],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let client = |client_id: &str, binding_issuer: &str| ScimClient {
        issuer: MACHINES.to_owned(),
        client_id: client_id.to_owned(),
        scope: SCOPE.to_owned(),
        binding_issuer: binding_issuer.to_owned(),
    };
    AppConfig {
        server: common::server(),
        auth,
        party_binding: binding(),
        scim: ScimConfig {
            enabled: true,
            public_base_url: Some("https://cdr.example.com".to_owned()),
            client: vec![client(CLIENT, LOGIN), client(OTHER, OTHER_LOGIN)],
            ..ScimConfig::default()
        },
        ..AppConfig::default()
    }
}

/// The router over a fresh database holding the archetypes, with the
/// mapping installed as the binary installs it.
async fn app() -> (testkit::TestDb, Router) {
    let (db, _pool, app) = app_auditing(false).await;
    (db, app)
}

/// [`app`], with the service behind it.
async fn app_with_service() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let (db, _pool, service, app) = app_of(config(), false).await;
    (db, service, app)
}

/// [`app`] whose clients deactivate or delete at most `max` Users an hour.
async fn app_limited(max: u32) -> (testkit::TestDb, Router) {
    let mut config = config();
    config.scim.max_deactivations = max;
    let (db, _pool, _service, app) = app_of(config, false).await;
    (db, app)
}

/// [`app`], recording every access to the local audit store when
/// `audited`; with the store's pool.
async fn app_auditing(audited: bool) -> (testkit::TestDb, sqlx::PgPool, Router) {
    let (db, pool, _service, app) = app_of(config(), audited).await;
    (db, pool, app)
}

/// The router of `config` over a fresh database holding the archetypes,
/// auditing when `audited`; with the store's pool and the service.
async fn app_of(
    config: AppConfig,
    audited: bool,
) -> (testkit::TestDb, sqlx::PgPool, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let sections = config.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[LOGIN.to_owned(), OTHER_LOGIN.to_owned()], "ADMIN")
        .expect("the mapping compiles");
    let mut service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping);
    if audited {
        let (sender, _handle) =
            ferroehr::system_log::sender::start(common::local_audit(), None, Some(pool.clone()))
                .await
                .expect("the audit sender");
        service = service
            .with_audit(sender)
            .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool.clone()));
    }
    for adl in [
        archetype(PERSON, "PERSON"),
        archetype(ROLE, "ROLE"),
        archetype(GROUP, "GROUP"),
        membership_archetype(),
    ] {
        service
            .template_adl2_upload(&adl)
            .await
            .expect("the archetype uploads");
    }
    ferroehr_rest::scim::endpoint::check(&config).expect("the endpoint can be served");
    let service = Arc::new(service);
    let app = common::router_with(config, Arc::clone(&service)).await;
    (db, pool, service, app)
}

/// A client-credentials token of `client_id` holding the SCIM scope.
fn token(client_id: &str) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({ "iss": MACHINES, "aud": AUDIENCE, "exp": exp,
                            "sub": client_id, "client_id": client_id, "scope": SCOPE }))
}

/// `method path` under `/scim/v2` as `client_id`, with `body` and the extra
/// `headers`; the status, the headers and the JSON body.
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    client_id: &str,
    body: Option<&Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, http::HeaderMap, Value) {
    let mut request = common::req(method, format!("/scim/v2{path}")).auth(&token(client_id));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(body) = body {
        request = request
            .header(header::CONTENT_TYPE, "application/scim+json")
            .body(body.to_string());
    }
    let (status, headers, body) = common::send(app, request.build()).await;
    let body = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, headers, body)
}

/// The RFC 7644 §3.3 create request, reduced to the mapped attributes, with
/// the binding key in FerroEHR's extension.
fn create_body(user_name: &str, external_id: &str, key: &str) -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User", EXTENSION],
        "userName": user_name,
        "externalId": external_id,
        "name": { "formatted": "Ms. Barbara J Jensen, III" },
        EXTENSION: { "loginIdentifier": key }
    })
}

#[tokio::test]
async fn the_create_answers_201_with_the_representation_its_location_and_entity_tag() {
    let (_db, app) = app().await;
    let key = "2819c223-7f76-453a-919d-413861904646";
    let (status, headers, body) = call(
        &app,
        "POST",
        "/Users",
        CLIENT,
        Some(&create_body("bjensen", "bjensen", key)),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().expect("an id").to_owned();
    assert_eq!(body["userName"], "bjensen");
    assert_eq!(body["externalId"], "bjensen");
    assert_eq!(body["active"], true);
    assert_eq!(body["name"]["formatted"], "Ms. Barbara J Jensen, III");
    assert_eq!(body[EXTENSION]["loginIdentifier"], key);
    let location = format!("https://cdr.example.com/scim/v2/Users/{id}");
    assert_eq!(body["meta"]["resourceType"], "User");
    assert_eq!(body["meta"]["location"], location.as_str());
    assert_eq!(headers[header::LOCATION], location.as_str());
    let version = body["meta"]["version"].as_str().expect("a version");
    assert!(version.starts_with("W/\""), "{version}");
    assert_eq!(headers[header::ETAG], version);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/scim+json",
        "RFC 7644 §8.1"
    );

    // RFC 7644 §3.4.1 and §3.14: the read serves the same version, and an
    // If-None-Match naming it answers 304.
    let (status, headers, read) =
        call(&app, "GET", &format!("/Users/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["meta"]["version"], version);
    assert_eq!(headers[header::ETAG], version);
    let (status, _, _) = call(
        &app,
        "GET",
        &format!("/Users/{id}"),
        CLIENT,
        None,
        &[("if-none-match", version)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn a_duplicate_user_name_is_409_uniqueness_and_a_first_inactive_version_400() {
    let (_db, app) = app().await;
    let (status, _, body) = call(
        &app,
        "POST",
        "/Users",
        CLIENT,
        Some(&create_body(
            "bjensen",
            "bjensen",
            "2819c223-7f76-453a-919d-413861904646",
        )),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // RFC 7644 §3.3: "409 (Conflict) with a "scimType" error code of "uniqueness"".
    let (status, _, body) = call(
        &app,
        "POST",
        "/Users",
        OTHER,
        Some(&create_body(
            "BJensen",
            "other",
            "3819c223-7f76-453a-919d-413861904646",
        )),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness");
    assert_eq!(body["status"], "409");

    let mut inactive = create_body("jdoe", "jdoe", "4819c223-7f76-453a-919d-413861904646");
    inactive["active"] = json!(false);
    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&inactive), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "invalidValue");

    // A User without its binding key is refused.
    let mut keyless = create_body("jroe", "jroe", "5819c223-7f76-453a-919d-413861904646");
    keyless
        .as_object_mut()
        .expect("an object")
        .remove(EXTENSION);
    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&keyless), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "invalidValue");
}

#[tokio::test]
async fn a_query_finds_the_clients_users_and_another_clients_are_404() {
    let (_db, app) = app().await;
    let (_, _, jensen) = call(
        &app,
        "POST",
        "/Users",
        CLIENT,
        Some(&create_body(
            "bjensen",
            "ext-1",
            "2819c223-7f76-453a-919d-413861904646",
        )),
        &[],
    )
    .await;
    let (_, _, theirs) = call(
        &app,
        "POST",
        "/Users",
        OTHER,
        Some(&create_body(
            "ajones",
            "ext-1",
            "3819c223-7f76-453a-919d-413861904646",
        )),
        &[],
    )
    .await;
    let id = jensen["id"].as_str().expect("an id");
    let their_id = theirs["id"].as_str().expect("an id");
    let query = |filter: &str| format!("/Users?filter={}", urlencoding::encode(filter));
    let total = |body: &Value| body["totalResults"].as_u64().expect("a total");

    // RFC 7644 §3.4.2.2: attribute names and operators are case-insensitive.
    for filter in [
        "userName eq \"bjensen\"".to_owned(),
        "USERNAME EQ \"BJensen\"".to_owned(),
        "externalId eq \"ext-1\"".to_owned(),
        format!("id eq \"{id}\" and userName eq \"bjensen\""),
    ] {
        let (status, _, body) = call(&app, "GET", &query(&filter), CLIENT, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{filter}: {body}");
        assert_eq!(total(&body), 1, "{filter}: {body}");
        assert_eq!(body["Resources"][0]["id"], id, "{filter}");
        assert_eq!(
            body["schemas"],
            json!(["urn:ietf:params:scim:api:messages:2.0:ListResponse"])
        );
    }

    // A query matching nothing is an empty ListResponse with 200 (§3.4.2).
    for filter in [
        format!("externalId eq \"{}\"", uuid::Uuid::now_v7()),
        format!("id eq \"{their_id}\""),
        "id eq \"not-a-uuid\"".to_owned(),
        "externalId eq \"EXT-1\"".to_owned(),
    ] {
        let (status, _, body) = call(&app, "GET", &query(&filter), CLIENT, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{filter}: {body}");
        assert_eq!(total(&body), 0, "{filter}: {body}");
        assert_eq!(body["Resources"], json!([]), "{filter}");
    }

    // An operator this provider does not support is 400 invalidFilter.
    for filter in [
        "userName sw \"b\"",
        "userName eq \"a\" or userName eq \"b\"",
        "title eq \"x\"",
    ] {
        let (status, _, body) = call(&app, "GET", &query(filter), CLIENT, None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{filter}: {body}");
        assert_eq!(body["scimType"], "invalidFilter", "{filter}");
    }

    // Another client's User is 404 to a read.
    let (status, _, body) = call(
        &app,
        "GET",
        &format!("/Users/{their_id}"),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["status"], "404");

    // The unfiltered list holds the client's Users only.
    let (status, _, body) = call(&app, "GET", "/Users", CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(total(&body), 1, "{body}");
}

/// The principal and action of each access record whose object is `party`,
/// once the trail holds `expected` of them.
async fn records(pool: &sqlx::PgPool, party: &str, expected: usize) -> Vec<(String, String)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let rows: Vec<(Option<String>, String)> = sqlx::query_as(
            "SELECT principal, action FROM audit.audit_event WHERE resource_id = $1 \
             ORDER BY chain_seq",
        )
        .bind(party)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= expected {
            return rows
                .into_iter()
                .map(|(principal, action)| (principal.unwrap_or_default(), action))
                .collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{} access records of {party}",
            rows.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Every SCIM read and write is an access record of the client, with the
/// `PERSON` as its object.
#[tokio::test]
async fn each_request_is_an_access_record_of_the_client_naming_the_person() {
    let (_db, pool, app) = app_auditing(true).await;
    let (status, _, body) = call(
        &app,
        "POST",
        "/Users",
        CLIENT,
        Some(&create_body(
            "bjensen",
            "ext-1",
            "2819c223-7f76-453a-919d-413861904646",
        )),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().expect("an id").to_owned();
    let (status, _, _) = call(&app, "GET", &format!("/Users/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let filter = format!(
        "/Users?filter={}",
        urlencoding::encode("userName eq \"bjensen\"")
    );
    let (status, _, _) = call(&app, "GET", &filter, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        records(&pool, &id, 3).await,
        [
            (CLIENT.to_owned(), "C".to_owned()),
            (CLIENT.to_owned(), "R".to_owned()),
            (CLIENT.to_owned(), "E".to_owned()),
        ]
    );
}

/// Creates the User `user_name` for `client_id` and returns its id and
/// version.
async fn created(app: &Router, client_id: &str, user_name: &str, key: &str) -> (String, String) {
    let (status, _, body) = call(
        app,
        "POST",
        "/Users",
        client_id,
        Some(&create_body(user_name, user_name, key)),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["id"].as_str().expect("an id").to_owned(),
        body["meta"]["version"]
            .as_str()
            .expect("a version")
            .to_owned(),
    )
}

/// A `PatchOp` message of `operations` (RFC 7644 §3.5.2).
fn patch_op(operations: Value) -> Value {
    let mut message = json!({ "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"] });
    message["Operations"] = operations;
    message
}

/// The RFC 7644 §3.5.2.3 replace of a mapped attribute, the Entra ID
/// disable (`Replace` without a `path`, the operation in another case,
/// Microsoft's "SCIM protocol requests and responses"), and the lifecycle
/// that follows: an inactive User is still read, refuses an attribute change
/// until it is reactivated, and is reactivated by `active = true`.
#[tokio::test]
async fn patch_replaces_a_mapped_value_and_active_drives_the_lifecycle() {
    let (_db, app) = app().await;
    let (id, version) = created(
        &app,
        CLIENT,
        "bjensen",
        "2819c223-7f76-453a-919d-413861904646",
    )
    .await;
    let path = format!("/Users/{id}");

    let replace =
        patch_op(json!([{ "op": "replace", "path": "name.formatted", "value": "Babs Jensen" }]));
    let (status, headers, body) = call(
        &app,
        "PATCH",
        &path,
        CLIENT,
        Some(&replace),
        &[("if-match", &version)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"]["formatted"], "Babs Jensen");
    assert_eq!(body["active"], true);
    let patched = body["meta"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    assert_ne!(patched, version, "one new version");
    assert_eq!(headers[header::ETAG], patched.as_str());

    // RFC 7644 §3.14: an If-Match naming an older version is 412.
    let (status, _, body) = call(
        &app,
        "PATCH",
        &path,
        CLIENT,
        Some(&replace),
        &[("if-match", &version)],
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");

    let disable = patch_op(json!([{ "op": "Replace", "value": { "active": false } }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&disable), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], false);
    assert_eq!(body[EXTENSION]["lifecycleState"], "inactive");
    let (status, _, body) = call(&app, "GET", &path, CLIENT, None, &[]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an inactive User is still read: {body}"
    );

    // An attribute change to an inactive User is refused, and the state stays.
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&replace), &[]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the same value changes nothing: {body}"
    );
    let rename =
        patch_op(json!([{ "op": "replace", "path": "name.formatted", "value": "B. Jensen" }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&rename), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "invalidValue");
    let (_, _, body) = call(&app, "GET", &path, CLIENT, None, &[]).await;
    assert_eq!(body["active"], false);

    let enable = patch_op(json!([{ "op": "replace", "path": "active", "value": true }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&enable), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], true);
    assert!(body[EXTENSION].get("lifecycleState").is_none(), "{body}");
}

/// A PUT clears an omitted mapped attribute, keeps the binding key and
/// `externalId`, and keeps the lifecycle state when it omits `active`
/// (RFC 7644 §3.5.1); a changed key is 400 `mutability` (RFC 7643 §2.2).
#[tokio::test]
async fn put_clears_an_omitted_attribute_and_keeps_the_key_external_id_and_state() {
    let (_db, app) = app().await;
    let key = "2819c223-7f76-453a-919d-413861904646";
    let (id, _) = created(&app, CLIENT, "bjensen", key).await;
    let path = format!("/Users/{id}");
    let disable = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, _, _) = call(&app, "PATCH", &path, CLIENT, Some(&disable), &[]).await;
    assert_eq!(status, StatusCode::OK);

    let put = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
        "userName": "bjensen"
    });
    let (status, _, body) = call(&app, "PUT", &path, CLIENT, Some(&put), &[]).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "clearing the name changes an inactive User, which needs active = true: {body}"
    );
    let mut put = put;
    put["active"] = json!(true);
    let (status, _, body) = call(&app, "PUT", &path, CLIENT, Some(&put), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.get("name").is_none(),
        "the omitted name is cleared: {body}"
    );
    assert_eq!(body["externalId"], "bjensen");
    assert_eq!(body[EXTENSION]["loginIdentifier"], key);
    assert_eq!(body["active"], true);

    // The same PUT without active keeps the state.
    put.as_object_mut().expect("an object").remove("active");
    let (status, _, body) = call(&app, "PUT", &path, CLIENT, Some(&put), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], true);

    let rekey = patch_op(json!([{
        "op": "replace",
        "path": format!("{EXTENSION}:loginIdentifier"),
        "value": "3819c223-7f76-453a-919d-413861904646"
    }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&rekey), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "mutability");

    // Another client's User is 404 to a write.
    let (status, _, _) = call(&app, "PUT", &path, OTHER, Some(&put), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// RFC 7644 §3.6: a deleted User is 404 to every request and absent from
/// every list, and its `userName` may be reused.
#[tokio::test]
async fn a_deleted_user_is_404_and_its_user_name_is_free() {
    let (_db, app) = app().await;
    let (id, version) = created(
        &app,
        CLIENT,
        "bjensen",
        "2819c223-7f76-453a-919d-413861904646",
    )
    .await;
    let path = format!("/Users/{id}");
    let (status, _, _) = call(
        &app,
        "DELETE",
        &path,
        CLIENT,
        None,
        &[("if-match", "W/\"other\"")],
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "RFC 7644 §3.14");
    let (status, _, body) = call(
        &app,
        "DELETE",
        &path,
        CLIENT,
        None,
        &[("if-match", &version)],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let replace = patch_op(json!([{ "op": "replace", "path": "name.formatted", "value": "x" }]));
    for (method, body) in [("GET", None), ("PATCH", Some(&replace)), ("DELETE", None)] {
        let (status, _, answer) = call(&app, method, &path, CLIENT, body, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {answer}");
    }
    let (_, _, list) = call(&app, "GET", "/Users", CLIENT, None, &[]).await;
    assert_eq!(list["totalResults"], 0, "{list}");
    let (again, _) = created(
        &app,
        CLIENT,
        "bjensen",
        "3819c223-7f76-453a-919d-413861904646",
    )
    .await;
    assert_ne!(again, id, "a new PERSON");
}

/// Past `max_deactivations` a deactivation and a deletion are refused with
/// 429 and `Retry-After` (RFC 6585 §4), and nothing is written.
#[tokio::test]
async fn deactivations_past_the_bound_are_429() {
    let (_db, app) = app_limited(2).await;
    let mut ids = Vec::new();
    for (index, key) in [
        "2819c223-7f76-453a-919d-413861904646",
        "3819c223-7f76-453a-919d-413861904646",
        "4819c223-7f76-453a-919d-413861904646",
    ]
    .into_iter()
    .enumerate()
    {
        ids.push(created(&app, CLIENT, &format!("user{index}"), key).await.0);
    }
    let disable = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, _, _) = call(
        &app,
        "PATCH",
        &format!("/Users/{}", ids[0]),
        CLIENT,
        Some(&disable),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/Users/{}", ids[1]),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, headers, body) = call(
        &app,
        "PATCH",
        &format!("/Users/{}", ids[2]),
        CLIENT,
        Some(&disable),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(headers[header::RETRY_AFTER], "3600");
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/Users/{}", ids[2]),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let (_, _, body) = call(
        &app,
        "GET",
        &format!("/Users/{}", ids[2]),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(body["active"], true, "nothing was written: {body}");

    // The bound is the client's own: another client is not held by it, and
    // a write that deactivates nothing passes.
    let rename = patch_op(json!([{ "op": "replace", "path": "name.formatted", "value": "y" }]));
    let (status, _, _) = call(
        &app,
        "PATCH",
        &format!("/Users/{}", ids[2]),
        CLIENT,
        Some(&rename),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (theirs, _) = created(
        &app,
        OTHER,
        "ajones",
        "5819c223-7f76-453a-919d-413861904646",
    )
    .await;
    let (status, _, _) = call(
        &app,
        "DELETE",
        &format!("/Users/{theirs}"),
        OTHER,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
