// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one `PERSON` mapping SCIM, just-in-time creation and the login binding
//! share (#5013), over a real PostgreSQL 18 (shared testkit harness).
//!
//! It covers the selectors the side table holds to one live party, the fold
//! a `userName` is compared by (RFC 8265 §3.3, RFC 7643 §4.1.1), the first
//! fill and the refill under the mapping's digest, the protection of the
//! parties the binding relies on (RFC 7643 §2.2 `mutability: "immutable"`
//! for the key), the administrator re-key, the boot rules that read the
//! templates, and a mapped write that keeps what it does not map. Parties
//! are committed through the demographic service calls the REST routes make.
//! No openEHR spec governs the mapping: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::Outcome;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::source::Lookup;
use ferrohealth_smart::resource::Caller;
use serde_json::{Value, json};

use crate::fixtures::{delete_precondition, if_match, object, written_vo};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::fill::{FillError, LEASE_TTL};
use ferroehr::party_binding::{BindingMapping, IdentifierSelector, NameSelector, PersonSelectors};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::rekey::BindingKey;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The template of a professional's `PERSON`.
const PERSON_TEMPLATE: &str = "FerroEHR professional";
/// The template of the professional `ROLE`.
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
/// The template of a provisioned `GROUP`.
const GROUP_TEMPLATE: &str = "FerroEHR care team";
/// The archetype of a membership `PARTY_RELATIONSHIP`.
const MEMBERSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1";
/// The archetype of a representation `PARTY_RELATIONSHIP`.
const REPRESENTATION: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1";
/// The identifier cluster every person selector reads.
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";
/// The `identifier_issuer` of the login's `[[auth.party]]` table.
const LOGIN: &str = "https://login.example/tenant";
/// The issuer a SCIM client writes `userName` and `externalId` under.
const SCIM: &str = "https://login.example/tenant#scim";
/// The issuer of a group's `externalId`.
const GROUPS: &str = "https://login.example/groups";
/// The `identifier_type` of the login key.
const GUID: &str = "guid";
/// The type of a SCIM `userName`.
const USER_NAME: &str = "scim_user_name";
/// The type of a SCIM `externalId`.
const EXTERNAL_ID: &str = "scim_external_id";
/// The administrator role of the tests.
const ADMIN: &str = "ADMIN";
/// The namespace a reported party is named under.
const NAMESPACE: &str = "demographic";

/// The key, `userName` and `externalId` selectors: one path, told apart by
/// type and issuer.
fn selectors() -> PersonSelectors {
    let path = format!("/details[at0001]/items[{CLUSTER}]/items[at0001]");
    let key = IdentifierSelector::new(path.clone(), GUID)
        .expect("compiles")
        .with_issuers([LOGIN]);
    PersonSelectors::new(key)
        .with_user_name(IdentifierSelector::new(path.clone(), USER_NAME).expect("compiles"))
        .with_external_id(IdentifierSelector::new(path, EXTERNAL_ID).expect("compiles"))
}

/// The mapping the tests derive the side table under.
fn mapping() -> BindingMapping {
    let external_id = IdentifierSelector::new("/details[at0001]/items[at0001]", "external_id")
        .expect("the group selector compiles");
    BindingMapping::default()
        .with_person_selectors(PERSON_TEMPLATE, selectors())
        .with_professional_role(ROLE_TEMPLATE)
        .with_group(GROUP_TEMPLATE, external_id)
        .with_group_display_name(
            NameSelector::new("/identities[at0002]/details[at0003]/items[at0004]")
                .expect("compiles"),
        )
        .with_membership(MEMBERSHIP)
        .with_representation(
            REPRESENTATION,
            BTreeMap::from([("at0010".to_owned(), "254".to_owned())]),
        )
        .with_admin_role(ADMIN)
}

/// A service over a fresh database with `mapping` installed.
async fn service_with(mapping: BindingMapping) -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    (db, svc.with_binding_mapping(mapping))
}

/// Another service over `db` with `mapping` installed: another replica.
fn replica(db: &testkit::TestDb, mapping: BindingMapping) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_binding_mapping(mapping)
}

/// A service over a fresh database with [`mapping`] installed.
async fn service() -> (testkit::TestDb, FerroEhrService) {
    service_with(mapping()).await
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// An `ELEMENT` `at` holding the `DV_IDENTIFIER` `id` of `kind` under
/// `issuer`.
fn identifier(at: &str, issuer: &str, kind: &str, id: &str) -> Value {
    json!({
        "_type": "ELEMENT", "archetype_node_id": at, "name": text("identifier"),
        "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind }
    })
}

/// The login key `id`.
fn key(id: &str) -> Value {
    identifier("at0001", LOGIN, GUID, id)
}

/// The SCIM `userName` `name`.
fn user_name(name: &str) -> Value {
    identifier("at0001", SCIM, USER_NAME, name)
}

fn identities(name: &str) -> Value {
    json!([{
        "_type": "PARTY_IDENTITY", "archetype_node_id": "at0002", "name": text("name"),
        "details": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0003", "name": text("structure"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0004",
                        "name": text("name"), "value": text(name) }]
        }
    }])
}

fn archetyped(archetype: &str, template: &str) -> Value {
    json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
        "template_id": { "_type": "TEMPLATE_ID", "value": template },
        "rm_version": "1.1.0"
    })
}

fn party_ref(class: &str, id: VoId) -> Value {
    json!({
        "_type": "PARTY_REF", "namespace": NAMESPACE, "type": class,
        "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() }
    })
}

/// A `PERSON` of the mapped template with `identifiers` in its cluster.
fn person(identifiers: &[Value]) -> Value {
    let mut body = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", PERSON_TEMPLATE),
        "name": text("professional"),
        "identities": identities("professional")
    });
    if !identifiers.is_empty() {
        body["details"] = json!({
            "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": CLUSTER,
                        "name": text("identifiers"), "items": identifiers }]
        });
    }
    body
}

/// A `ROLE` of the mapped role template performed by `performer`, ended on
/// `until` when given.
fn role(performer: VoId, until: Option<&str>) -> Value {
    let mut body = json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", ROLE_TEMPLATE),
        "name": text("professional"),
        "identities": identities("professional"),
        "performer": party_ref("PERSON", performer)
    });
    if let Some(until) = until {
        body["time_validity"] = json!({
            "_type": "DV_INTERVAL", "lower_unbounded": true, "upper_unbounded": false,
            "lower_included": false, "upper_included": true,
            "upper": { "_type": "DV_DATE", "value": until }
        });
    }
    body
}

/// A `GROUP` of the mapped group template with `external_id` and the display
/// name `name`.
fn group(external_id: &str, name: &str) -> Value {
    json!({
        "_type": "GROUP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.care_team.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-GROUP.care_team.v1", GROUP_TEMPLATE),
        "name": text("care team"),
        "identities": identities(name),
        "details": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [identifier("at0001", GROUPS, "external_id", external_id)]
        }
    })
}

/// The write of `data` after `preceding`.
fn version<T: serde::de::DeserializeOwned>(
    data: &Value,
    preceding: Option<&str>,
) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Creates `body` as a party of `kind`.
async fn try_create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
) -> Result<(VoId, String), ServiceError> {
    let created = Box::pin(svc.party_create(kind, version(body, None))).await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((written_vo(&created), uid))
}

/// Creates `body` as a party of `kind`, which must commit.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> (VoId, String) {
    try_create(svc, kind, body)
        .await
        .expect("the party commits")
}

/// Commits `body` as the next version of `vo` after `uid`.
async fn try_update(
    svc: &FerroEhrService,
    kind: PartyKind,
    vo: VoId,
    uid: &str,
    body: &Value,
) -> Result<String, ServiceError> {
    let updated = Box::pin(svc.party_update(
        kind,
        object(vo.to_string()),
        if_match(uid),
        version(body, Some(uid)),
    ))
    .await?;
    Ok(updated.meta.expect("a version").uid)
}

/// Deletes the party at `uid`.
async fn try_delete(svc: &FerroEhrService, kind: PartyKind, uid: &str) -> Result<(), ServiceError> {
    Box::pin(svc.party_delete(
        kind,
        object(uid),
        delete_precondition(uid, None),
        WriteRequest::new(()),
    ))
    .await
    .map(|_| ())
}

/// A professional carrying `identifiers`: the `PERSON`, its last version and
/// its `ROLE`'s container and version.
async fn professional(
    svc: &FerroEhrService,
    identifiers: &[Value],
) -> (VoId, String, VoId, String) {
    let body = person(identifiers);
    let (vo, uid) = create(svc, PartyKind::Person, &body).await;
    let (role_vo, role_uid) = create(svc, PartyKind::Role, &role(vo, None)).await;
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", role_vo)]);
    let uid = try_update(svc, PartyKind::Person, vo, &uid, &listed)
        .await
        .expect("the professional binds");
    (vo, uid, role_vo, role_uid)
}

/// The identity of a request by `subject` holding `roles`.
fn caller(subject: &str) -> CommitterIdentity {
    CommitterIdentity {
        subject: subject.to_owned(),
        id_type: "oauth2",
        issuer: Some("https://login.example".to_owned()),
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
    }
}

/// Runs `fut` as a request of `identity` holding `roles`.
async fn as_caller<F: Future>(identity: CommitterIdentity, roles: &[&str], fut: F) -> F::Output {
    ferroehr::system_log::access_context::with_roles(
        roles.iter().map(|r| (*r).to_owned()).collect(),
        with_committer(Some(identity), fut),
    )
    .await
}

/// The parties the lookup finds for the login key `value`.
async fn bound(svc: &FerroEhrService, value: &str) -> Vec<String> {
    let lookup = svc
        .party_source(NAMESPACE)
        .read(LOGIN, GUID, value, None, "2026-06-01")
        .await
        .expect("the source answers");
    match lookup {
        Lookup::Parties(records) => records.iter().map(|r| r.party().id().to_owned()).collect(),
        other => vec![format!("{other:?}")],
    }
}

/// The live selector rows of `party`, as `selector issuer type folded`.
async fn rows(pool: &sqlx::PgPool, party: VoId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT selector || ' ' || issuer || ' ' || identifier_type || ' ' || folded \
         FROM party.binding_identifier WHERE party_id = $1 AND retired_at IS NULL ORDER BY 1",
    )
    .bind(party)
    .fetch_all(pool)
    .await
    .expect("the rows")
}

/// Asserts `refused` is a `409` naming `selector`.
fn assert_in_use(refused: ServiceError, selector: &str) {
    let refused = refused.classified();
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
    assert!(refused.to_string().contains(selector), "{refused}");
}

#[tokio::test]
async fn two_selectors_at_one_path_address_two_identifiers_of_one_person() {
    let (db, svc) = service().await;
    let value = "8a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    let (vo, ..) = professional(
        &svc,
        &[
            key(value),
            user_name("Jane.Doe"),
            identifier("at0001", SCIM, EXTERNAL_ID, "entra-1"),
            // The key's type under another issuer is no key.
            identifier(
                "at0001",
                "https://other.example",
                GUID,
                "9b1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
            ),
        ],
    )
    .await;
    assert_eq!(
        rows(&db.pool(), vo).await,
        [
            format!("external_id {SCIM} {EXTERNAL_ID} entra-1"),
            format!("key {LOGIN} {GUID} {value}"),
            format!("user_name {SCIM} {USER_NAME} jane.doe"),
        ]
    );
    assert_eq!(bound(&svc, value).await, [vo.to_string()]);
    assert_eq!(
        bound(&svc, "9b1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d").await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn user_names_differing_in_case_width_or_normal_form_are_one_user_name() {
    let (_db, svc) = service().await;
    create(&svc, PartyKind::Person, &person(&[user_name("José.Doe")])).await;
    for variant in [
        "JOSÉ.DOE",
        // Fullwidth J and O (RFC 8265 §3.3.2 rule 1).
        "\u{FF2A}\u{FF2F}SÉ.DOE",
        // The decomposed é (rule 3).
        "Jose\u{301}.Doe",
    ] {
        let refused = try_create(&svc, PartyKind::Person, &person(&[user_name(variant)]))
            .await
            .expect_err("one userName per service provider");
        assert_in_use(refused, "userName");
    }
    create(&svc, PartyKind::Person, &person(&[user_name("John.Doe")])).await;
}

#[tokio::test]
async fn a_second_external_id_or_display_name_is_refused() {
    let (_db, svc) = service().await;
    let external = |id: &str| person(&[identifier("at0001", SCIM, EXTERNAL_ID, id)]);
    create(&svc, PartyKind::Person, &external("entra-7")).await;
    let refused = try_create(&svc, PartyKind::Person, &external("entra-7"))
        .await
        .expect_err("one externalId per client");
    assert_in_use(refused, "externalId");

    create(&svc, PartyKind::Group, &group("ward-1", "Ward 3")).await;
    let refused = try_create(&svc, PartyKind::Group, &group("ward-2", "WARD 3"))
        .await
        .expect_err("one displayName");
    assert_in_use(refused, "displayName");
    let refused = try_create(&svc, PartyKind::Group, &group("ward-1", "Ward 4"))
        .await
        .expect_err("one group externalId");
    assert_in_use(refused, "externalId");
}

#[tokio::test]
async fn concurrent_creations_binding_one_key_commit_once() {
    let (_db, svc) = service().await;
    let value = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
    let (first, second) = tokio::join!(
        professional_or_refusal(&svc, value),
        professional_or_refusal(&svc, value)
    );
    let outcomes = [first, second];
    assert_eq!(
        outcomes.iter().filter(|o| o.is_ok()).count(),
        1,
        "{outcomes:?}"
    );
    assert_eq!(bound(&svc, value).await.len(), 1);
}

/// A professional binding `value`, or the refusal of its binding commit.
async fn professional_or_refusal(svc: &FerroEhrService, value: &str) -> Result<VoId, String> {
    let body = person(&[key(value)]);
    let (vo, uid) = create(svc, PartyKind::Person, &body).await;
    let (role_vo, _) = create(svc, PartyKind::Role, &role(vo, None)).await;
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", role_vo)]);
    try_update(svc, PartyKind::Person, vo, &uid, &listed)
        .await
        .map(|_| vo)
        .map_err(|e| e.classified().to_string())
}

#[tokio::test]
async fn the_first_fill_stores_the_digest_and_a_replica_with_another_mapping_refuses() {
    let (db, svc) = service().await;
    let value = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f6a";
    let (vo, ..) = professional(&svc, &[key(value)]).await;
    let pool = db.pool();
    sqlx::query("DELETE FROM party.binding_identifier")
        .execute(&pool)
        .await
        .expect("empty the table");

    // Two replicas booting together: one fills, both run.
    let (one, two) = (replica(&db, mapping()), replica(&db, mapping()));
    let (a, b) = tokio::join!(
        one.take_binding_lease(uuid::Uuid::now_v7()),
        two.take_binding_lease(uuid::Uuid::now_v7())
    );
    let a = a.expect("the first replica").expect("a lease");
    let b = b.expect("the second replica").expect("a lease");
    assert_eq!(bound(&svc, value).await, [vo.to_string()]);
    let states: i64 = sqlx::query_scalar("SELECT count(*) FROM party.binding_state")
        .fetch_one(&pool)
        .await
        .expect("the state");
    assert_eq!(states, 1);

    let other = mapping().with_professional_role("Another role");
    let changed = replica(&db, other.clone());
    let refused = changed
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect_err("another mapping refuses to start");
    let FillError::Digest { stored, running } = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(stored, &mapping().digest());
    assert_eq!(running, &other.digest());
    assert!(
        refused.to_string().contains(&mapping().digest()),
        "{refused}"
    );

    // A refill waits for every replica to stop.
    let refill = changed
        .refill_binding(LEASE_TTL)
        .await
        .expect_err("leases are live");
    assert!(
        matches!(refill, FillError::Leased { ref replicas } if replicas.len() == 2),
        "{refill:?}"
    );
    a.release().await.expect("released");
    b.release().await.expect("released");
    changed
        .refill_binding(LEASE_TTL)
        .await
        .expect("no replica runs");
    changed
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the new mapping starts");
    assert!(matches!(
        one.take_binding_lease(uuid::Uuid::now_v7()).await,
        Err(FillError::Digest { .. })
    ));
}

#[tokio::test]
async fn a_refill_rebuilds_every_key_row_from_the_versions() {
    let (db, svc) = service().await;
    let value = "3e4f5a6b-7c8d-4e9f-8a0b-1c2d3e4f5a6b";
    let (vo, ..) = professional(&svc, &[key(value), user_name("refill")]).await;
    let pool = db.pool();
    let before = rows(&pool, vo).await;
    sqlx::query("DELETE FROM party.binding_identifier")
        .execute(&pool)
        .await
        .expect("empty the table");
    svc.refill_binding(LEASE_TTL).await.expect("refilled");
    assert_eq!(rows(&pool, vo).await, before);
    assert_eq!(bound(&svc, value).await, [vo.to_string()]);
}

#[tokio::test]
async fn the_fill_refuses_a_store_carrying_one_key_twice_naming_both_parties() {
    let (db, unmapped) = crate::fixtures::service().await;
    let value = "4f5a6b7c-8d9e-4f0a-9b1c-2d3e4f5a6b7c";
    let (first, _) = create(&unmapped, PartyKind::Person, &person(&[key(value)])).await;
    let (second, _) = create(&unmapped, PartyKind::Person, &person(&[key(value)])).await;
    let refused = replica(&db, mapping())
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect_err("a key carried twice");
    let text = refused.to_string();
    assert!(
        text.contains(&first.to_string()) && text.contains(&second.to_string()),
        "{text}"
    );
    assert!(text.contains("one binding key"), "{text}");
}

#[tokio::test]
async fn only_an_administrator_writes_a_protected_party() {
    let (_db, svc) = service().await;
    let clinician = || caller("clinician");
    let admin = || caller("admin");
    let value = "5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c8d";

    // A key-bearing PERSON is never planted by a non-administrator.
    let refused = as_caller(
        clinician(),
        &["CLINICIAN"],
        try_create(&svc, PartyKind::Person, &person(&[key(value)])),
    )
    .await
    .expect_err("a key-bearing PERSON is protected");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    // A PERSON carrying no selector is anyone's to write.
    let (plain, plain_uid) = as_caller(
        clinician(),
        &["CLINICIAN"],
        create(&svc, PartyKind::Person, &person(&[])),
    )
    .await;

    // Nobody but an administrator grants the professional ROLE.
    let refused = as_caller(
        clinician(),
        &["CLINICIAN"],
        try_create(&svc, PartyKind::Role, &role(plain, None)),
    )
    .await
    .expect_err("the professional ROLE is protected");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");

    // Nor creates a GROUP of the group template.
    let refused = as_caller(
        clinician(),
        &["CLINICIAN"],
        try_create(&svc, PartyKind::Group, &group("ward-9", "Ward 9")),
    )
    .await
    .expect_err("a provisioned GROUP is protected");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");

    // Nor adds a representation relationship.
    let mut represents = person(&[]);
    represents["relationships"] = json!([{
        "_type": "PARTY_RELATIONSHIP", "archetype_node_id": REPRESENTATION,
        "name": { "_type": "DV_CODED_TEXT", "value": "parent",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": "at0010" } },
        "source": party_ref("PERSON", plain),
        "target": party_ref("PERSON", plain)
    }]);
    let refused = as_caller(
        clinician(),
        &["CLINICIAN"],
        try_update(&svc, PartyKind::Person, plain, &plain_uid, &represents),
    )
    .await
    .expect_err("a representation relationship is protected");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");

    // An administrator creates the professional and grants the ROLE.
    let (vo, uid, role_vo, role_uid) = as_caller(
        admin(),
        &["admin"],
        professional(&svc, &[key(value), user_name("jane")]),
    )
    .await;
    assert_eq!(bound(&svc, value).await, [vo.to_string()]);

    // A non-administrator neither versions nor deletes it.
    let refused = as_caller(
        clinician(),
        &["CLINICIAN"],
        try_update(
            &svc,
            PartyKind::Person,
            vo,
            &uid,
            &person(&[key(value), user_name("jane")]),
        ),
    )
    .await
    .expect_err("a protected PERSON");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let refused = as_caller(
        clinician(),
        &["CLINICIAN"],
        try_delete(&svc, PartyKind::Person, &uid),
    )
    .await
    .expect_err("a protected PERSON");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");

    // The administrator retires the ROLE, and the PERSON stops binding.
    as_caller(
        admin(),
        &[ADMIN],
        try_update(
            &svc,
            PartyKind::Role,
            role_vo,
            &role_uid,
            &role(vo, Some("2020-01-01")),
        ),
    )
    .await
    .expect("an administrator retires the ROLE");
    assert_eq!(bound(&svc, value).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_present_key_is_immutable_and_a_protected_identifier_is_never_dropped() {
    let (_db, svc) = service().await;
    let admin = || caller("admin");
    let value = "6b7c8d9e-0f1a-4b2c-9d3e-4f5a6b7c8d9e";
    let (vo, uid, role_vo, _) = professional(&svc, &[key(value), user_name("jane")]).await;
    let listed = |identifiers: &[Value]| {
        let mut body = person(identifiers);
        body["roles"] = json!([party_ref("ROLE", role_vo)]);
        body
    };

    let refused = as_caller(
        admin(),
        &[ADMIN],
        try_update(
            &svc,
            PartyKind::Person,
            vo,
            &uid,
            &listed(&[
                key("7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f"),
                user_name("jane"),
            ]),
        ),
    )
    .await
    .expect_err("a present key is immutable");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    assert!(refused.to_string().contains("immutable"), "{refused}");

    let refused = as_caller(
        admin(),
        &[ADMIN],
        try_update(&svc, PartyKind::Person, vo, &uid, &listed(&[key(value)])),
    )
    .await
    .expect_err("a protected identifier is never dropped");
    assert!(
        matches!(refused, ServiceError::Unprocessable { .. }),
        "{refused:?}"
    );
    assert!(format!("{refused:?}").contains("userName"), "{refused:?}");
}

#[tokio::test]
async fn a_deleted_key_leaves_a_tombstone_an_administrator_clears() {
    let (_db, svc) = service().await;
    let value = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
    let (_, uid, ..) = professional(&svc, &[key(value)]).await;
    as_caller(
        caller("admin"),
        &[ADMIN],
        try_delete(&svc, PartyKind::Person, &uid),
    )
    .await
    .expect("an administrator deletes");
    let lookup = svc
        .party_source(NAMESPACE)
        .read(LOGIN, GUID, value, None, "2026-06-01")
        .await
        .expect("answers");
    assert_eq!(lookup, Lookup::Deprovisioned);
    let (again, ..) = as_caller(caller("admin"), &[ADMIN], professional(&svc, &[key(value)])).await;
    assert_eq!(bound(&svc, value).await, [again.to_string()]);
}

#[tokio::test]
async fn an_administrator_rekeys_a_person_and_never_to_an_own_or_bound_key() {
    let (_db, svc) = service().await;
    let old = "8d9e0f1a-2b3c-4d4e-9f5a-6b7c8d9e0f1a";
    let new = "9E0F1A2B-3C4D-4E5F-8A6B-7C8D9E0F1A2B";
    let taken = "0f1a2b3c-4d5e-4f6a-9b7c-8d9e0f1a2b3c";
    let mine = "1a2b3c4d-5e6f-4a7b-8c8d-9e0f1a2b3c4d";
    let (vo, ..) = professional(&svc, &[key(old)]).await;
    let (other, ..) = professional(&svc, &[key(taken)]).await;
    let (own, ..) = professional(&svc, &[key(mine)]).await;
    let admin = admin_bound_to(&svc, mine).await;
    assert_eq!(
        admin.party.as_ref().map(|p| p.party.id().to_owned()),
        Some(own.to_string())
    );
    let rekey = |value: &str| BindingKey {
        issuer: LOGIN.to_owned(),
        value: value.to_owned(),
    };

    let refused = as_caller(admin.clone(), &[ADMIN], svc.rekey_party(vo, rekey(mine)))
        .await
        .expect_err("never to the administrator's own login");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let refused = as_caller(admin.clone(), &[ADMIN], svc.rekey_party(vo, rekey(taken)))
        .await
        .expect_err("never to a bound key");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
    assert!(refused.to_string().contains("already bound"), "{refused}");

    as_caller(admin, &[ADMIN], svc.rekey_party(vo, rekey(new)))
        .await
        .expect("re-keyed");
    assert_eq!(bound(&svc, old).await, Vec::<String>::new());
    assert_eq!(
        bound(&svc, &new.to_ascii_lowercase()).await,
        [vo.to_string()]
    );
    assert_eq!(bound(&svc, taken).await, [other.to_string()]);
}

/// The committer identity of an administrator whose login binds the
/// professional carrying `value`, resolved through the shared binding.
async fn admin_bound_to(svc: &FerroEhrService, value: &str) -> CommitterIdentity {
    let table = PartySection {
        issuer: "https://login.example".to_owned(),
        identifier_issuer: LOGIN.to_owned(),
        identifier_type: GUID.to_owned(),
        ..PartySection::default()
    };
    let binding = PartyBinding::new(Arc::new(svc.party_source(NAMESPACE)))
        .with_issuer(
            table,
            ferrohealth_smart::kind::IssuerKind::Persons,
            &ferrohealth_smart::profile::Profile::default(),
        )
        .expect("the table binds");
    let login = Caller::new(
        "admin".to_owned(),
        Some("app".to_owned()),
        "user/*.cruds",
        None,
        false,
        CallerKind::Person,
    )
    .with_issuer("https://login.example".to_owned())
    .with_person_key(Some(value.to_owned()), None);
    let outcome = binding.resolve(&login).await.expect("answers");
    let bound = outcome.as_ref().and_then(Outcome::bound).cloned();
    assert!(
        bound.is_some(),
        "the administrator's login binds: {outcome:?}"
    );
    CommitterIdentity {
        party: bound,
        ..caller("admin")
    }
}

#[tokio::test]
async fn a_membership_change_on_a_five_thousand_member_group_writes_one_row() {
    let (db, svc) = service().await;
    let body = group("ward-5000", "Ward 5000");
    let (vo, uid) = create(&svc, PartyKind::Group, &body).await;
    let with = |members: &[VoId]| {
        let mut b = body.clone();
        b["relationships"] = members
            .iter()
            .map(|member| {
                json!({
                    "_type": "PARTY_RELATIONSHIP", "archetype_node_id": MEMBERSHIP,
                    "name": text("member"),
                    "source": party_ref("GROUP", vo),
                    "target": party_ref("PERSON", *member)
                })
            })
            .collect();
        b
    };
    let mut members: Vec<VoId> = (0..5000).map(|_| VoId(uuid::Uuid::now_v7())).collect();
    let uid = as_caller(
        caller("admin"),
        &[ADMIN],
        try_update(&svc, PartyKind::Group, vo, &uid, &with(&members)),
    )
    .await
    .expect("five thousand members");
    members.push(VoId(uuid::Uuid::now_v7()));
    as_caller(
        caller("admin"),
        &[ADMIN],
        try_update(&svc, PartyKind::Group, vo, &uid, &with(&members)),
    )
    .await
    .expect("one more member");
    let (rows, newest): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE written_at = \
                (SELECT max(written_at) FROM party.binding_membership WHERE group_id = $1)) \
         FROM party.binding_membership WHERE group_id = $1 AND retired_at IS NULL",
    )
    .bind(vo)
    .fetch_one(&db.pool())
    .await
    .expect("the rows");
    assert_eq!((rows, newest), (5001, 1));
}
