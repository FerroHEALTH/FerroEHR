// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The default demographic definitions of `deploy/definitions/` (#5428)
//! under the default party mapping (#5462) and the `[authz.party_binding]`
//! example the book documents, read from the book page itself so the files,
//! the defaults and the documentation cannot drift.
//!
//! The set is seeded as boot seeds it, the mapping is judged against it as
//! boot judges it, and a SCIM User, a Group with that User as its member, and
//! a representation relationship are then written and read through it: the User's `PERSON` and `ROLE`, the member's inline
//! membership `PARTY_RELATIONSHIP` to the `GROUP`, and a representation
//! `PARTY_RELATIONSHIP` whose coded name maps to a `subject_relationship`
//! code (RM demographic `master02-demographic_package.adoc` §Party
//! Relationships). Which definitions ship by default is our own design.

use std::sync::Arc;

use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::template::TemplateFit;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::object_version_id::parse_uid_based_id;
use ferrohealth_smart::party::source::Lookup;
use ferrohealth_smart::party::{Markers, PartyRef};
use http::StatusCode;
use serde_json::{Value, json};

use super::{CLIENT, GROUPS, LOGIN, OTHER_LOGIN, call, config, created};
use crate::common;

/// The interface form of the representation archetype's id, which each
/// representation's `archetype_node_id` carries.
const REPRESENTATION: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1";
/// The ADL 2 id of the representation archetype.
const REPRESENTATION_HRID: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0";
/// The ADL 2 id of the membership archetype.
const MEMBERSHIP_HRID: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0";

/// A file of the repository.
fn repo_file(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

/// The TOML block of the book's authentication page that holds `marker`.
fn documented_block(marker: &str) -> String {
    let page = repo_file("website/book/src/installation/config-auth.md");
    page.split("```toml\n")
        .skip(1)
        .filter_map(|block| block.split("```").next())
        .find(|block| block.contains(marker))
        .unwrap_or_else(|| panic!("the page documents a block holding {marker:?}"))
        .to_owned()
}

/// The `[authz.party_binding]` table of `mapping`, with a provider.
fn party_binding(mapping: &str) -> ferroehr::config::binding::PartyBindingConfig {
    use assert_fs::prelude::FileWriteStr as _;

    let toml = format!(
        "[authz.party_binding]\nprovider = {{ issuer = \"urn:oid:2.999\", id = \"1\" }}\n\n{mapping}"
    );
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("a temp file");
    file.write_str(&toml).expect("the mapping is written");
    ferroehr::config::assemble(Some(file.path()), &std::collections::HashMap::new(), &[])
        .expect("the mapping assembles")
        .authz
        .party_binding
}

/// The harness configuration with its party mapping replaced by the
/// documented Microsoft Entra example, which keys the binding on the object
/// id and keeps every other section's default.
fn documented_config() -> ferroehr_rest::config::AppConfig {
    let mut config = config();
    config.party_binding = party_binding(&documented_block("oidc = \"name\""));
    config
}

/// The harness configuration with no mapping key and no group issuer: its
/// persons log in under the default key type, and its clients' Groups take
/// the group issuer derived from their `binding_issuer`.
fn zero_config() -> ferroehr_rest::config::AppConfig {
    let mut config = config();
    config.party_binding = party_binding("");
    for table in &mut config.auth.shared.party {
        "login".clone_into(&mut table.identifier_type);
        table.group_claim.clear();
        table.group_identifier_issuer.clear();
    }
    config
}

/// A service over `pool` with the default definitions seeded and the
/// mapping of `config` installed and judged, as `serve` does at boot.
async fn booted(
    config: &ferroehr_rest::config::AppConfig,
    pool: &sqlx::PgPool,
) -> Arc<FerroEhrService> {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .seed_default_definitions()
        .await
        .expect("the boot seeds the default definitions");
    let sections = config.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[LOGIN.to_owned(), OTHER_LOGIN.to_owned()], "ADMIN")
        .expect("the mapping compiles");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(
            ferroehr::party_binding::grants::operation::GrantCeiling::new(
                ferroehr::party_binding::grants::operation::Operation::ALL,
            ),
        );
    let errors = service
        .party_mapping_errors(&sections)
        .await
        .expect("the store answers");
    assert!(errors.is_empty(), "boot refuses the mapping: {errors:?}");
    service
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the side table is prepared")
        .expect("the mapping derives rows");
    super::grant_clients(&service).await;
    Arc::new(service)
}

/// The current version uid and body of the `PERSON` `id`.
async fn head(service: &FerroEhrService, id: &str) -> (String, Value) {
    let address = parse_uid_based_id(id).expect("a container id");
    let read = service
        .party_get(PartyKind::Person, address, None)
        .await
        .expect("the PERSON reads");
    let uid = read.meta.as_ref().expect("a version").uid.clone();
    (uid, read.body)
}

/// The write of `body` as the next version after `uid`.
fn next<T: serde::de::DeserializeOwned>(body: &Value, uid: Option<&str>) -> WriteRequest<T> {
    let mut write =
        WriteRequest::new(openehr_its::json::from_canonical_value(body).expect("the body decodes"));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = uid.map(|uid| uid.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// A `PARTY_REF` to the `PERSON` `id`.
fn person_ref(id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// A representation of the type `code` (`at10` parent, `at11` guardian) from
/// `representative` to `subject`, as the default representation archetype
/// describes it: its name is its type (RM `PARTY_RELATIONSHIP`
/// `Type_validity`), and its details carry the representation code and the
/// openEHR `subject_relationship` code `254|parent|`.
fn representation(code: &str, label: &str, representative: &str, subject: &str) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": REPRESENTATION,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": REPRESENTATION },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_CODED_TEXT", "value": label,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code } },
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id3",
            "name": { "_type": "DV_TEXT", "value": "Details" },
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "id4",
                    "name": { "_type": "DV_TEXT", "value": "Representation code" },
                    "value": coded("ferroehr-operations", "represents", "represents") },
                { "_type": "ELEMENT", "archetype_node_id": "id6",
                    "name": { "_type": "DV_TEXT", "value": "Relationship" },
                    "value": coded("openehr", "254", "parent") }
            ] },
        "source": person_ref(representative),
        "target": person_ref(subject)
    })
}

/// A `DV_CODED_TEXT` of `code` in `terminology`, reading `value`.
fn coded(terminology: &str, code: &str, value: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
            "code_string": code } })
}

/// A SCIM User of the default `PERSON` performing the default `ROLE`, with
/// the binding key `key`; its id.
async fn provision_user(app: &axum::Router, key: &str) -> String {
    let (user, _) = created(app, CLIENT, "jdoe", key).await;
    let (status, _, read) = call(app, "GET", &format!("/Users/{user}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["userName"], "jdoe");
    assert_eq!(read["name"]["formatted"], "Ms. Barbara J Jensen, III");
    user
}

/// A SCIM Group of the default `GROUP` with `user` as its member; its id.
async fn provision_group(app: &axum::Router, user: &str) -> String {
    let body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "displayName": "Ward 3 care team",
        "externalId": "ward-3",
        "members": [{ "value": user }],
    });
    let (status, _, group) = call(app, "POST", "/Groups", CLIENT, Some(&body), &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let id = group["id"].as_str().expect("an id").to_owned();
    let (status, _, read) = call(app, "GET", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["displayName"], "Ward 3 care team");
    assert_eq!(read["externalId"], "ward-3");
    assert_eq!(read["members"][0]["value"], user, "{read}");
    id
}

/// Commits a subject `PERSON` and adds to `user`'s `PERSON` a representation
/// of it as its parent, after checking the membership `user` holds and the
/// representation each fit their default archetype; the subject's id.
async fn represent(service: &FerroEhrService, user: &str) -> String {
    let (uid, body) = head(service, user).await;
    let memberships = body["relationships"]
        .as_array()
        .expect("the PERSON holds its membership");
    assert_eq!(memberships.len(), 1, "{body}");
    let fit = service
        .party_template_fit(&memberships[0], MEMBERSHIP_HRID)
        .await
        .expect("the membership is of the membership archetype");
    assert!(matches!(fit, TemplateFit::Complete), "{fit:?}");

    let subject = json!({
        "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "subject" },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "name" },
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": "A child" } }] } }]
    });
    let subject = Box::pin(service.party_create(PartyKind::Person, next(&subject, None)))
        .await
        .expect("the subject commits");
    let version = subject.meta.as_ref().expect("a version").uid.clone();
    let subject_id = version
        .split("::")
        .next()
        .expect("a container id")
        .to_owned();
    let parent = representation("at10", "Parent", user, &subject_id);
    let fit = service
        .party_template_fit(&parent, REPRESENTATION_HRID)
        .await
        .expect("the representation is of the representation archetype");
    assert!(matches!(fit, TemplateFit::Complete), "{fit:?}");
    let mut representing = body;
    representing["relationships"]
        .as_array_mut()
        .expect("the relationships")
        .push(parent);
    Box::pin(service.party_update(
        PartyKind::Person,
        ferroehr::ids::VoId(user.parse().expect("a container id")),
        Precondition::parse(Some(&uid), None).expect("an If-Match"),
        next(&representing, Some(&uid)),
    ))
    .await
    .expect("the representation commits");
    subject_id
}

#[tokio::test]
async fn the_documented_mapping_boots_on_the_default_definitions_and_serves_users_groups_and_representation()
 {
    let config = documented_config();
    let (_db, pool) = common::migrated_pool().await;
    let service = booted(&config, &pool).await;
    ferroehr_rest::scim::endpoint::check(&config).expect("the endpoint can be served");
    let app = common::router_with(config, Arc::clone(&service)).await;
    let key = "5e6f7a8b-9c0d-4e1f-8a2b-4c5d6e7f8a9b";
    let user = provision_user(&app, key).await;
    let group = provision_group(&app, &user).await;
    let subject = represent(&service, &user).await;

    // The login resolves to the User's PERSON: a professional, a member of the
    // Group under its externalId, and the parent of the subject.
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string();
    let found = service
        .party_source("demographic")
        .read(LOGIN, "guid", key, Some(GROUPS), &today)
        .await
        .expect("the source answers");
    let Lookup::Parties(records) = &found else {
        panic!("one party, got {found:?}");
    };
    assert_eq!(records.len(), 1, "{found:?}");
    let record = &records[0];
    assert_eq!(
        record.party(),
        &PartyRef::new("demographic", "PERSON", user.as_str())
    );
    assert_eq!(
        record.markers(),
        Markers {
            professional: true,
            representative: true,
        }
    );
    let groups: Vec<(&PartyRef, &str)> = record
        .memberships()
        .iter()
        .map(|m| (m.group(), m.identifier()))
        .collect();
    assert_eq!(
        groups,
        [(
            &PartyRef::new("demographic", "GROUP", group.as_str()),
            "ward-3"
        )]
    );
    let represents: Vec<(&PartyRef, Option<&str>)> = record
        .represents()
        .iter()
        .map(|r| (r.subject(), r.relationship()))
        .collect();
    assert_eq!(
        represents,
        [(
            &PartyRef::new("demographic", "PERSON", subject.as_str()),
            Some("254")
        )],
        "at10 maps to 254|parent| of the openEHR subject_relationship group"
    );
}

/// The default mapping the book prints is the one an empty configuration
/// has.
#[test]
fn the_book_prints_the_default_mapping() {
    let printed = party_binding(&documented_block("[authz.party_binding.patient]"));
    let empty = party_binding("");
    assert_eq!(
        (
            &printed.person,
            &printed.professional_role,
            &printed.group,
            &printed.membership,
            &printed.patient,
        ),
        (
            &empty.person,
            &empty.professional_role,
            &empty.group,
            &empty.membership,
            &empty.patient,
        )
    );
}

/// A configuration with no mapping key and no group issuer boots on the
/// seeded default definitions and serves a SCIM User, a Group with that User
/// as its member, and a representation, with the Group's `externalId` under
/// the issuer derived from the client's `binding_issuer`.
#[tokio::test]
async fn an_empty_configuration_boots_and_serves_users_groups_and_representation() {
    let config = zero_config();
    let (_db, pool) = common::migrated_pool().await;
    let service = booted(&config, &pool).await;
    ferroehr_rest::scim::endpoint::check(&config).expect("the endpoint can be served");
    let app = common::router_with(config, Arc::clone(&service)).await;
    let key = "clinician-7";
    let user = provision_user(&app, key).await;
    let group = provision_group(&app, &user).await;
    let subject = represent(&service, &user).await;

    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string();
    let derived = format!("{LOGIN}/groups");
    let found = service
        .party_source("demographic")
        .read(LOGIN, "login", key, Some(&derived), &today)
        .await
        .expect("the source answers");
    let Lookup::Parties(records) = &found else {
        panic!("one party, got {found:?}");
    };
    assert_eq!(records.len(), 1, "{found:?}");
    let record = &records[0];
    assert_eq!(
        record.party(),
        &PartyRef::new("demographic", "PERSON", user.as_str())
    );
    assert_eq!(
        record.markers(),
        Markers {
            professional: true,
            representative: true,
        }
    );
    let groups: Vec<(&PartyRef, &str)> = record
        .memberships()
        .iter()
        .map(|m| (m.group(), m.identifier()))
        .collect();
    assert_eq!(
        groups,
        [(
            &PartyRef::new("demographic", "GROUP", group.as_str()),
            "ward-3"
        )],
        "the Group's externalId is under {derived}"
    );
    let represents: Vec<(&PartyRef, Option<&str>)> = record
        .represents()
        .iter()
        .map(|r| (r.subject(), r.relationship()))
        .collect();
    assert_eq!(
        represents,
        [(
            &PartyRef::new("demographic", "PERSON", subject.as_str()),
            Some("254")
        )]
    );
}
