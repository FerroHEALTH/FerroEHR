// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The local party source over the binding side table (#5004), over a real
//! PostgreSQL 18 (shared testkit harness).
//!
//! Parties are committed through the demographic service calls the REST
//! routes make, so every side-table row is the one a commit derives. A
//! professional is a `PERSON` of a mapped template whose `roles` list a `ROLE`
//! of the mapped role template that names it as `performer` (RM demographic
//! `org.openehr.rm.demographic.actor.adoc`, `org.openehr.rm.demographic.role.adoc`);
//! a membership and a representation are inline `PARTY_RELATIONSHIP`s of
//! their source (`master02-demographic_package.adoc` §Party Relationships).
//! The states are the `version_lifecycle_state` codes of RM common
//! `master06-change_control_package.adoc` §Version Lifecycle. Resolving a
//! login to a party is governed by no openEHR spec: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::source::Lookup;
use ferrohealth_smart::party::{Markers, Outcome, PartyRef, PartyState};
use ferrohealth_smart::resource::Caller;
use serde_json::{Value, json};
use sqlx::Row;

use crate::fixtures::{delete_precondition, if_match, object, written_vo};
use ferroehr::ids::VoId;
use ferroehr::party_binding::source::LocalPartySource;
use ferroehr::party_binding::{BindingMapping, IdentifierSelector};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::party_set::NewParty;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The template of a professional's `PERSON`.
const PERSON_TEMPLATE: &str = "FerroEHR professional";
/// A `PERSON` template the mapping does not name.
const OTHER_TEMPLATE: &str = "FerroEHR patient";
/// The template of the professional `ROLE`.
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
/// The template of a provisioned `GROUP`.
const GROUP_TEMPLATE: &str = "FerroEHR care team";
/// The archetype of a membership `PARTY_RELATIONSHIP`.
const MEMBERSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1";
/// The archetype of a representation `PARTY_RELATIONSHIP`.
const REPRESENTATION: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1";
/// The identifier cluster the key selector reads.
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";
/// The `identifier_issuer` of the login's `[[auth.party]]` table.
const LOGIN: &str = "https://login.example/tenant";
/// The `group_identifier_issuer` of that table.
const GROUPS: &str = "https://login.example/groups";
/// The `identifier_type` of that table.
const GUID: &str = "guid";
/// The date every lookup is judged on.
const TODAY: &str = "2026-06-01";
/// The namespace a reported party is named under.
const NAMESPACE: &str = "demographic";

/// The mapping the tests derive the side table under.
fn mapping() -> BindingMapping {
    let key = IdentifierSelector::new(
        format!("/details[at0001]/items[{CLUSTER}]/items[at0001]"),
        GUID,
    )
    .expect("the key selector compiles");
    let external_id = IdentifierSelector::new("/details[at0001]/items[at0001]", "external_id")
        .expect("the group selector compiles");
    BindingMapping::default()
        .with_person(PERSON_TEMPLATE, key)
        .with_professional_role(ROLE_TEMPLATE)
        .with_group(GROUP_TEMPLATE, external_id)
        .with_membership(MEMBERSHIP)
        .with_representation(
            REPRESENTATION,
            BTreeMap::from([("at0010".to_owned(), "264".to_owned())]),
        )
}

/// A service over a fresh database with the mapping installed.
async fn service() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    (db, svc.with_binding_mapping(mapping()))
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

/// The login key `id` at the key selector.
fn key(id: &str) -> Value {
    identifier("at0001", LOGIN, GUID, id)
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

/// A `DV_INTERVAL<DV_DATE>` from `lower` to `upper`, each unbounded when
/// absent.
fn interval(lower: Option<&str>, upper: Option<&str>) -> Value {
    let mut value = json!({
        "_type": "DV_INTERVAL",
        "lower_unbounded": lower.is_none(), "upper_unbounded": upper.is_none(),
        "lower_included": lower.is_some(), "upper_included": upper.is_some()
    });
    if let Some(lower) = lower {
        value["lower"] = json!({ "_type": "DV_DATE", "value": lower });
    }
    if let Some(upper) = upper {
        value["upper"] = json!({ "_type": "DV_DATE", "value": upper });
    }
    value
}

/// A `PERSON` of `template` with `identifiers` in its identifier cluster,
/// and no `details` when it carries none.
fn person(template: &str, identifiers: &[Value]) -> Value {
    let mut body = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", template),
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

/// A `ROLE` of `template` performed by `performer`.
fn role(template: &str, performer: VoId, validity: Option<Value>) -> Value {
    let mut body = json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", template),
        "name": text("professional"),
        "identities": identities("professional"),
        "performer": party_ref("PERSON", performer)
    });
    if let Some(validity) = validity {
        body["time_validity"] = validity;
    }
    body
}

/// A `PARTY_RELATIONSHIP` of `archetype` from `source` to `target`, of the
/// local type `code`.
fn relationship(
    archetype: &str,
    source: (&str, VoId),
    target: (&str, VoId),
    code: &str,
    validity: Option<Value>,
) -> Value {
    let mut body = json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": archetype,
        "name": {
            "_type": "DV_CODED_TEXT", "value": "relationship",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code }
        },
        "source": party_ref(source.0, source.1),
        "target": party_ref(target.0, target.1)
    });
    if let Some(validity) = validity {
        body["time_validity"] = validity;
    }
    body
}

/// A `GROUP` of `template` named by `external_id` under the group issuer.
fn group(template: &str, external_id: &str) -> Value {
    json!({
        "_type": "GROUP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.care_team.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-GROUP.care_team.v1", template),
        "name": text("care team"),
        "identities": identities("care team"),
        "details": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [identifier("at0001", GROUPS, "external_id", external_id)]
        }
    })
}

/// The write of `data` in the lifecycle state `lifecycle`, after `preceding`.
fn version<T: serde::de::DeserializeOwned>(
    data: &Value,
    lifecycle: &str,
    preceding: Option<&str>,
) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.lifecycle_state = Some(lifecycle.to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Creates the party `body` of `kind`, returning its container and version
/// uid.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> (VoId, String) {
    create_in(svc, kind, body, "532").await
}

/// Creates the party `body` of `kind` in `lifecycle`, returning its container
/// and version uid.
async fn create_in(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
    lifecycle: &str,
) -> (VoId, String) {
    let created = Box::pin(svc.party_create(kind, version(body, lifecycle, None)))
        .await
        .expect("the party commits");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    (written_vo(&created), uid)
}

/// Commits `body` as the next version of `vo` after `uid` in `lifecycle`,
/// returning the new version uid.
async fn update(
    svc: &FerroEhrService,
    kind: PartyKind,
    vo: VoId,
    uid: &str,
    body: &Value,
    lifecycle: &str,
) -> String {
    Box::pin(svc.party_update(
        kind,
        object(vo.to_string()),
        if_match(uid),
        version(body, lifecycle, Some(uid)),
    ))
    .await
    .expect("the party updates")
    .meta
    .expect("a version")
    .uid
}

/// Deletes the party `vo` of `kind` at `uid`.
async fn delete(svc: &FerroEhrService, kind: PartyKind, uid: &str) {
    Box::pin(svc.party_delete(
        kind,
        object(uid),
        delete_precondition(uid, None),
        WriteRequest::new(()),
    ))
    .await
    .expect("the party deletes");
}

/// A `PERSON` of `template` carrying `identifiers` that performs a `ROLE` of
/// `role_template` valid over `validity`, in `lifecycle`; returns the
/// `PERSON`, its last version uid and the `ROLE`'s container and version uid.
async fn performer(
    svc: &FerroEhrService,
    template: &str,
    identifiers: &[Value],
    role_template: &str,
    validity: Option<Value>,
    lifecycle: &str,
) -> (VoId, String, VoId, String) {
    let body = person(template, identifiers);
    // NOTE: RM common master06 §Version Lifecycle: only an incomplete version
    // may be abandoned.
    let first = if lifecycle == "801" { "553" } else { "532" };
    let (vo, uid) = create_in(svc, PartyKind::Person, &body, first).await;
    let (role_vo, role_uid) =
        create(svc, PartyKind::Role, &role(role_template, vo, validity)).await;
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", role_vo)]);
    let uid = update(svc, PartyKind::Person, vo, &uid, &listed, lifecycle).await;
    (vo, uid, role_vo, role_uid)
}

/// A professional `PERSON` carrying the login key `id`.
async fn professional(svc: &FerroEhrService, id: &str) -> (VoId, String) {
    let (vo, uid, _, _) =
        performer(svc, PERSON_TEMPLATE, &[key(id)], ROLE_TEMPLATE, None, "532").await;
    (vo, uid)
}

fn source(svc: &FerroEhrService) -> LocalPartySource {
    svc.party_source(NAMESPACE)
}

/// What the source holds for the login key `id`, with the memberships.
async fn lookup(svc: &FerroEhrService, id: &str) -> Lookup {
    source(svc)
        .read(LOGIN, GUID, id, Some(GROUPS), TODAY)
        .await
        .expect("the source answers")
}

/// The parties a lookup names, or a panic naming what it answered.
fn parties(lookup: &Lookup) -> Vec<PartyRef> {
    match lookup {
        Lookup::Parties(records) => records.iter().map(|r| r.party().clone()).collect(),
        other => panic!("expected parties, got {other:?}"),
    }
}

fn person_ref(vo: VoId) -> PartyRef {
    PartyRef::new(NAMESPACE, "PERSON", vo.to_string())
}

fn group_ref(vo: VoId) -> PartyRef {
    PartyRef::new(NAMESPACE, "GROUP", vo.to_string())
}

/// A key stored in upper case binds a lower-case GUID claim: the side table
/// holds the canonical form the `[[auth.party]]` type declares.
const UPPER_KEY: &str = "5BA552D6-0F3C-4D0B-9C11-6A5B0C2E7F10";

#[tokio::test]
async fn the_source_returns_the_one_professional_carrying_the_key() {
    let (_db, svc) = service().await;
    let (vo, _) = professional(&svc, UPPER_KEY).await;

    let found = lookup(&svc, &UPPER_KEY.to_ascii_lowercase()).await;
    let Lookup::Parties(records) = &found else {
        panic!("expected one party, got {found:?}");
    };
    assert_eq!(records.len(), 1, "{found:?}");
    let record = &records[0];
    assert_eq!(record.party(), &person_ref(vo));
    assert_eq!(record.state(), PartyState::Active);
    assert_eq!(record.markers(), Markers::PROFESSIONAL);

    // A clinician who is also a patient is one PERSON: an EHR whose subject
    // names the PERSON changes nothing the lookup reads.
    Box::pin(
        svc.create_ehr_meta(WriteRequest::new(Some(crate::fixtures::status_naming(
            &vo.to_string(),
            NAMESPACE,
            "PERSON",
        )))),
    )
    .await
    .expect("the EHR is created");
    assert_eq!(
        parties(&lookup(&svc, &UPPER_KEY.to_ascii_lowercase()).await),
        [person_ref(vo)]
    );
}

#[tokio::test]
async fn a_key_of_another_issuer_type_or_path_does_not_match() {
    let (_db, svc) = service().await;
    let value = "0b9e6c2a-5d4f-4e8a-9b1c-2d3e4f5a6b7c";
    professional(&svc, value).await;

    // Another issuer and another type of the same value.
    let other_issuer = source(&svc)
        .read("https://other.example", GUID, value, None, TODAY)
        .await
        .expect("answers");
    assert_eq!(parties(&other_issuer), []);
    let other_type = source(&svc)
        .read(LOGIN, "uuid", value, None, TODAY)
        .await
        .expect("answers");
    assert_eq!(parties(&other_type), []);

    // The same value at another node of the cluster, and under another type
    // at the key node, is no key.
    let elsewhere = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
    performer(
        &svc,
        PERSON_TEMPLATE,
        &[
            identifier("at0002", LOGIN, GUID, elsewhere),
            identifier("at0001", LOGIN, "other", elsewhere),
        ],
        ROLE_TEMPLATE,
        None,
        "532",
    )
    .await;
    assert_eq!(parties(&lookup(&svc, elsewhere).await), []);
}

#[tokio::test]
async fn a_person_without_a_live_professional_role_does_not_match() {
    let (_db, svc) = service().await;

    // A PERSON of a template the mapping does not name.
    let unmapped = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f6a";
    performer(
        &svc,
        OTHER_TEMPLATE,
        &[key(unmapped)],
        ROLE_TEMPLATE,
        None,
        "532",
    )
    .await;
    assert_eq!(
        parties(&lookup(&svc, unmapped).await),
        [],
        "unmapped template"
    );

    // A PERSON with no ROLE at all.
    let roleless = "3e4f5a6b-7c8d-4e9f-8a1b-2c3d4e5f6a7b";
    create(
        &svc,
        PartyKind::Person,
        &person(PERSON_TEMPLATE, &[key(roleless)]),
    )
    .await;
    assert_eq!(parties(&lookup(&svc, roleless).await), [], "no ROLE");

    // A ROLE of another template.
    let other_role = "4f5a6b7c-8d9e-4f0a-9b2c-3d4e5f6a7b8c";
    performer(
        &svc,
        PERSON_TEMPLATE,
        &[key(other_role)],
        "FerroEHR volunteer",
        None,
        "532",
    )
    .await;
    assert_eq!(parties(&lookup(&svc, other_role).await), [], "another ROLE");

    // A ROLE whose time_validity ended before the lookup's date, and one that
    // starts after it.
    let ended = "5a6b7c8d-9e0f-4a1b-8c3d-4e5f6a7b8c9d";
    performer(
        &svc,
        PERSON_TEMPLATE,
        &[key(ended)],
        ROLE_TEMPLATE,
        Some(interval(Some("2020-01-01"), Some("2026-05-31"))),
        "532",
    )
    .await;
    assert_eq!(parties(&lookup(&svc, ended).await), [], "ended ROLE");
    let future = "6b7c8d9e-0f1a-4b2c-9d4e-5f6a7b8c9d0e";
    performer(
        &svc,
        PERSON_TEMPLATE,
        &[key(future)],
        ROLE_TEMPLATE,
        Some(interval(Some("2026-06-02"), None)),
        "532",
    )
    .await;
    assert_eq!(parties(&lookup(&svc, future).await), [], "future ROLE");

    // A reduced date reads as its whole year: a ROLE valid through 2026 is
    // live on the lookup's date.
    let this_year = "7c8d9e0f-1a2b-4c3d-8e5f-6a7b8c9d0e1f";
    let (vo, _, _, _) = performer(
        &svc,
        PERSON_TEMPLATE,
        &[key(this_year)],
        ROLE_TEMPLATE,
        Some(interval(Some("2026"), Some("2026"))),
        "532",
    )
    .await;
    assert_eq!(parties(&lookup(&svc, this_year).await), [person_ref(vo)]);

    // A retired ROLE: its head inactive.
    let retired = "8d9e0f1a-2b3c-4d4e-9f6a-7b8c9d0e1f2a";
    let (vo, _, role_vo, role_uid) = performer(
        &svc,
        PERSON_TEMPLATE,
        &[key(retired)],
        ROLE_TEMPLATE,
        None,
        "532",
    )
    .await;
    assert_eq!(
        parties(&lookup(&svc, retired).await),
        [person_ref(vo)],
        "live before"
    );
    update(
        &svc,
        PartyKind::Role,
        role_vo,
        &role_uid,
        &role(ROLE_TEMPLATE, vo, None),
        "800",
    )
    .await;
    assert_eq!(parties(&lookup(&svc, retired).await), [], "retired ROLE");
}

#[tokio::test]
async fn a_deleted_person_leaves_a_tombstone() {
    let (_db, svc) = service().await;
    let value = "9e0f1a2b-3c4d-4e5f-8a7b-8c9d0e1f2a3b";
    let (_, uid) = professional(&svc, value).await;
    delete(&svc, PartyKind::Person, &uid).await;
    assert_eq!(lookup(&svc, value).await, Lookup::Deprovisioned);

    // A live PERSON binding the key again clears the tombstone.
    let (vo, _) = professional(&svc, value).await;
    assert_eq!(parties(&lookup(&svc, value).await), [person_ref(vo)]);
}

#[tokio::test]
async fn a_second_person_binding_one_key_is_refused_at_its_commit() {
    let (_db, svc) = service().await;
    let value = "0f1a2b3c-4d5e-4f6a-9b8c-9d0e1f2a3b4c";
    let (first, _) = professional(&svc, value).await;

    // The second PERSON binds the key the moment it lists its professional
    // ROLE, and the unique index on live keys refuses that commit.
    let body = person(PERSON_TEMPLATE, &[key(&value.to_ascii_uppercase())]);
    let (second, uid) = create(&svc, PartyKind::Person, &body).await;
    let (role_vo, _) = create(&svc, PartyKind::Role, &role(ROLE_TEMPLATE, second, None)).await;
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", role_vo)]);
    let refused = Box::pin(svc.party_update(
        PartyKind::Person,
        object(second.to_string()),
        if_match(&uid),
        version(&listed, "532", Some(&uid)),
    ))
    .await
    .expect_err("a key another PERSON binds is refused")
    .classified();
    assert!(
        matches!(refused, ferroehr::error::ServiceError::Conflict(_)),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("binding key"), "{refused}");
    assert_eq!(parties(&lookup(&svc, value).await), [person_ref(first)]);
}

/// A `PERSON` and the professional `ROLE` naming it are created by one
/// demographic CONTRIBUTION, each body carrying the other's allocated id.
#[tokio::test]
async fn one_contribution_creates_a_person_and_the_role_naming_it() {
    let (_db, svc) = service().await;
    let value = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    let (person_id, role_id) = (svc.allocate_party_id(), svc.allocate_party_id());
    let (person_vo, role_vo) = (person_id.id(), role_id.id());
    assert_ne!(person_vo, role_vo);
    let mut body = person(PERSON_TEMPLATE, &[key(value)]);
    body["roles"] = json!([party_ref("ROLE", role_vo)]);
    let parties_of_set = vec![
        NewParty {
            id: person_id,
            kind: PartyKind::Person,
            body,
            lifecycle_state: None,
        },
        NewParty {
            id: role_id,
            kind: PartyKind::Role,
            body: role(ROLE_TEMPLATE, person_vo, None),
            lifecycle_state: None,
        },
    ];
    let committed = Box::pin(svc.create_parties(parties_of_set, "provision a professional"))
        .await
        .expect("the set commits");
    let written: Vec<VoId> = committed.versions.iter().map(|c| c.vo_id).collect();
    assert_eq!(written, [person_vo, role_vo]);

    // RM common master06 §Contributions: one CONTRIBUTION lists both versions.
    let contribution = svc
        .demographic_contribution_get(committed.id)
        .await
        .expect("the contribution reads")
        .body;
    let listed: Vec<&str> = contribution["versions"]
        .as_array()
        .expect("the versions")
        .iter()
        .filter_map(|v| v.pointer("/id/value").and_then(Value::as_str))
        .collect();
    assert_eq!(
        listed,
        [
            committed.versions[0].version_uid(),
            committed.versions[1].version_uid()
        ],
        "{contribution}"
    );

    // The PERSON binds its key: it lists the ROLE, which names it.
    assert_eq!(parties(&lookup(&svc, value).await), [person_ref(person_vo)]);
}

#[tokio::test]
async fn the_head_state_is_reported() {
    let (_db, svc) = service().await;
    for (lifecycle, state) in [
        ("553", PartyState::Incomplete),
        ("800", PartyState::Inactive),
        ("801", PartyState::Abandoned),
    ] {
        let value = format!("1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c{lifecycle}");
        let (vo, _, _, _) = performer(
            &svc,
            PERSON_TEMPLATE,
            &[key(&value)],
            ROLE_TEMPLATE,
            None,
            lifecycle,
        )
        .await;
        let found = lookup(&svc, &value).await;
        let Lookup::Parties(records) = &found else {
            panic!("expected one party, got {found:?}");
        };
        assert_eq!(records.len(), 1, "{lifecycle}: {found:?}");
        assert_eq!(records[0].party(), &person_ref(vo));
        assert_eq!(records[0].state(), state, "{lifecycle}");
    }
}

/// Gives the `PERSON` `member` exactly the memberships of `groups`: each an
/// inline relationship from the `PERSON` to the `GROUP`, the member holding
/// it (#5220).
async fn memberships(svc: &FerroEhrService, member: VoId, groups: &[VoId]) {
    let read = Box::pin(svc.party_get(
        PartyKind::Person,
        crate::fixtures::at(member.to_string()),
        None,
    ))
    .await
    .expect("the PERSON reads");
    let uid = read.meta.as_ref().expect("a version").uid.clone();
    let mut body = read.body;
    let object = body.as_object_mut().expect("a PERSON object");
    object.remove("uid");
    let mut kept: Vec<Value> = object
        .remove("relationships")
        .and_then(|relationships| relationships.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|relationship| relationship["archetype_node_id"] != MEMBERSHIP)
        .collect();
    kept.extend(groups.iter().map(|group| {
        relationship(
            MEMBERSHIP,
            ("PERSON", member),
            ("GROUP", *group),
            "at0001",
            None,
        )
    }));
    if !kept.is_empty() {
        object.insert("relationships".to_owned(), Value::Array(kept));
    }
    update(svc, PartyKind::Person, member, &uid, &body, "532").await;
}

/// Adds to the `PERSON` `member` a membership of `group`, keeping its others.
async fn join(svc: &FerroEhrService, member: VoId, group: VoId) {
    let mut groups = held_groups(svc, member).await;
    groups.push(group);
    memberships(svc, member, &groups).await;
}

/// The `GROUP`s the `PERSON` `member` holds a membership of.
async fn held_groups(svc: &FerroEhrService, member: VoId) -> Vec<VoId> {
    let read = Box::pin(svc.party_get(
        PartyKind::Person,
        crate::fixtures::at(member.to_string()),
        None,
    ))
    .await
    .expect("the PERSON reads");
    read.body["relationships"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|relationship| relationship["archetype_node_id"] == MEMBERSHIP)
        .filter_map(|relationship| relationship["target"]["id"]["value"].as_str())
        .map(|id| VoId(id.parse().expect("a container id")))
        .collect()
}

/// Creates a group of `template` named `external_id` in `lifecycle`, and a
/// membership of it in each of `members`.
async fn team(
    svc: &FerroEhrService,
    template: &str,
    external_id: &str,
    members: &[VoId],
    lifecycle: &str,
) -> VoId {
    let body = group(template, external_id);
    let (vo, uid) = create(svc, PartyKind::Group, &body).await;
    if lifecycle != "532" {
        update(svc, PartyKind::Group, vo, &uid, &body, lifecycle).await;
    }
    for member in members {
        join(svc, *member, vo).await;
    }
    vo
}

#[tokio::test]
async fn the_source_reports_the_live_groups_a_person_belongs_to() {
    let (_db, svc) = service().await;
    let value = "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e";
    let (vo, _) = professional(&svc, value).await;
    let ward = team(&svc, GROUP_TEMPLATE, "ward", &[vo], "532").await;
    let theatre = team(&svc, GROUP_TEMPLATE, "theatre", &[vo], "532").await;
    let clinic = team(&svc, GROUP_TEMPLATE, "clinic", &[vo], "553").await;
    // An inactive GROUP lists no member, nor does a GROUP of another template.
    team(&svc, GROUP_TEMPLATE, "closed", &[vo], "800").await;
    team(&svc, OTHER_TEMPLATE, "other", &[vo], "532").await;
    // A standalone relationship container to a GROUP never counts.
    let (loose, _) = create(&svc, PartyKind::Group, &group(GROUP_TEMPLATE, "loose")).await;
    Box::pin(crate::fixtures::create_relationship(
        &svc,
        version(
            &relationship(MEMBERSHIP, ("GROUP", loose), ("PERSON", vo), "at0001", None),
            "532",
            None,
        ),
    ))
    .await
    .expect("the container commits");

    let found = lookup(&svc, value).await;
    let Lookup::Parties(records) = &found else {
        panic!("expected one party, got {found:?}");
    };
    let mut groups: Vec<(PartyRef, String)> = records[0]
        .memberships()
        .iter()
        .map(|m| (m.group().clone(), m.identifier().to_owned()))
        .collect();
    groups.sort();
    let mut expected = vec![
        (group_ref(ward), "ward".to_owned()),
        (group_ref(theatre), "theatre".to_owned()),
        (group_ref(clinic), "clinic".to_owned()),
    ];
    expected.sort();
    assert_eq!(groups, expected);

    // Through the shared binding, a token naming two of the three groups
    // binds the PERSON with exactly those two.
    let table = PartySection {
        issuer: "https://login.example".to_owned(),
        identifier_issuer: LOGIN.to_owned(),
        identifier_type: GUID.to_owned(),
        group_claim: "groups".to_owned(),
        group_identifier_issuer: GROUPS.to_owned(),
        ..PartySection::default()
    };
    let profile = ferrohealth_smart::profile::Profile::default();
    let binding = PartyBinding::new(Arc::new(source(&svc)))
        .with_issuer(
            table,
            ferrohealth_smart::kind::IssuerKind::Persons,
            &profile,
        )
        .expect("the table binds");
    let mut claims = serde_json::Map::new();
    claims.insert("groups".to_owned(), json!(["ward", "clinic", "unknown"]));
    let caller = Caller::new(
        "subject".to_owned(),
        Some("app".to_owned()),
        "user/*.read",
        None,
        false,
        CallerKind::Person,
    )
    .with_issuer("https://login.example".to_owned())
    .with_person_key(Some(value.to_owned()), None)
    .with_claims(claims);
    let outcome = binding.resolve(&caller).await.expect("the source answers");
    let Some(Outcome::Party(bound)) = outcome else {
        panic!("expected a bound party, got {outcome:?}");
    };
    assert_eq!(bound.party, person_ref(vo));
    let mut bound_groups = bound.groups.clone();
    bound_groups.sort();
    let mut two = vec![group_ref(ward), group_ref(clinic)];
    two.sort();
    assert_eq!(bound_groups, two);
}

#[tokio::test]
async fn a_membership_change_writes_one_row() {
    let (db, svc) = service().await;
    let (first, _) = professional(&svc, "3c4d5e6f-7a8b-4c9d-8e0f-2a3b4c5d6e7f").await;
    let (second, _) = professional(&svc, "4d5e6f7a-8b9c-4d0e-9f1a-3b4c5d6e7f8a").await;
    let (vo, _) = create(&svc, PartyKind::Group, &group(GROUP_TEMPLATE, "ward")).await;
    join(&svc, first, vo).await;
    join(&svc, second, vo).await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM party.binding_membership WHERE group_id = $1")
            .bind(vo)
            .fetch_one(&db.pool())
            .await
            .expect("count");
    assert_eq!(
        rows, 2,
        "adding the second member keeps the first member's row"
    );
}

/// The live memberships of `group` and the sources the reverse index names
/// as pointing at it, each sorted.
async fn group_rows(pool: &sqlx::PgPool, group: VoId) -> (Vec<VoId>, Vec<VoId>) {
    let members: Vec<VoId> = sqlx::query_scalar(
        "SELECT member_id FROM party.binding_membership \
         WHERE group_id = $1 AND retired_at IS NULL ORDER BY 1",
    )
    .bind(group)
    .fetch_all(pool)
    .await
    .expect("the memberships read");
    let sources: Vec<VoId> = sqlx::query_scalar(
        "SELECT source_vo_id FROM party.party_relationship_target \
         WHERE target_party_id = $1 ORDER BY 1",
    )
    .bind(group)
    .fetch_all(pool)
    .await
    .expect("the reverse index reads");
    (members, sources)
}

#[tokio::test]
async fn a_large_group_writes_only_the_rows_a_change_changes() {
    let (db, svc) = service().await;
    let pool = db.pool();
    let mut people = Vec::new();
    for _ in 0..60 {
        people.push(
            create(&svc, PartyKind::Person, &person(OTHER_TEMPLATE, &[]))
                .await
                .0,
        );
    }
    let (vo, _) = create(&svc, PartyKind::Group, &group(GROUP_TEMPLATE, "ward")).await;
    let sorted = |members: &[VoId]| {
        let mut members = members.to_vec();
        members.sort();
        members
    };
    let all = people.get(..50).expect("fifty people").to_vec();
    for member in &all {
        join(&svc, *member, vo).await;
    }
    assert_eq!(group_rows(&pool, vo).await, (sorted(&all), sorted(&all)));

    // Ten members leave and five join, each by a version of its own PERSON:
    // the forty kept rows stay as they were, the ten are retired and the
    // five inserted.
    let changed = people.get(10..55).expect("forty-five people").to_vec();
    let kept: Vec<String> = people
        .get(10..50)
        .expect("the kept members")
        .iter()
        .map(ToString::to_string)
        .collect();
    let kept_written: Vec<String> = sqlx::query_scalar(
        "SELECT member_id::text || written_at::text FROM party.binding_membership \
         WHERE group_id = $1 AND member_id::text = ANY($2) ORDER BY 1",
    )
    .bind(vo)
    .bind(&kept)
    .fetch_all(&pool)
    .await
    .expect("the kept rows read");
    for member in people.get(..10).expect("the leavers") {
        memberships(&svc, *member, &[]).await;
    }
    for member in people.get(50..55).expect("the joiners") {
        join(&svc, *member, vo).await;
    }
    assert_eq!(
        group_rows(&pool, vo).await,
        (sorted(&changed), sorted(&changed))
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM party.binding_membership WHERE group_id = $1")
            .bind(vo)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(
        rows, 55,
        "fifty first rows and five added ones, none written twice"
    );
    let kept_after: Vec<String> = sqlx::query_scalar(
        "SELECT member_id::text || written_at::text FROM party.binding_membership \
         WHERE group_id = $1 AND member_id::text = ANY($2) AND retired_at IS NULL ORDER BY 1",
    )
    .bind(vo)
    .bind(&kept)
    .fetch_all(&pool)
    .await
    .expect("the kept rows read");
    assert_eq!(kept_after, kept_written, "a kept membership keeps its row");
    let revision: i64 =
        sqlx::query_scalar("SELECT revision FROM party.binding_group_revision WHERE group_id = $1")
            .bind(vo)
            .fetch_one(&pool)
            .await
            .expect("the revision");
    assert_eq!(
        revision, 65,
        "every membership change moves the Group's revision"
    );

    // A membership whose target is no container of this store is neither a
    // member nor indexed, and a refill rebuilds the same live rows.
    let foreign = *people.get(56).expect("a person");
    let read = Box::pin(svc.party_get(
        PartyKind::Person,
        crate::fixtures::at(foreign.to_string()),
        None,
    ))
    .await
    .expect("the PERSON reads");
    let uid = read.meta.as_ref().expect("a version").uid.clone();
    let mut body = read.body;
    body.as_object_mut().expect("an object").remove("uid");
    body["relationships"] = json!([{
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": MEMBERSHIP,
        "name": text("elsewhere"),
        "source": party_ref("PERSON", foreign),
        "target": { "_type": "PARTY_REF", "namespace": NAMESPACE, "type": "GROUP",
                    "id": { "_type": "GENERIC_ID", "value": "1.2.3", "scheme": "oid" } }
    }]);
    update(&svc, PartyKind::Person, foreign, &uid, &body, "532").await;
    let before = group_rows(&pool, vo).await;
    assert_eq!(before, (sorted(&changed), sorted(&changed)));
    svc.refill_binding_table().await.expect("the refill runs");
    assert_eq!(group_rows(&pool, vo).await, before);
}

#[tokio::test]
async fn a_representative_reports_only_its_live_subjects() {
    let (_db, svc) = service().await;
    let (subject, _) = create(&svc, PartyKind::Person, &person(OTHER_TEMPLATE, &[])).await;
    let (former, _) = create(&svc, PartyKind::Person, &person(OTHER_TEMPLATE, &[])).await;
    let value = "5e6f7a8b-9c0d-4e1f-8a2b-4c5d6e7f8a9b";
    let body = person(PERSON_TEMPLATE, &[key(value)]);
    let (vo, uid) = create(&svc, PartyKind::Person, &body).await;
    let mut representing = body;
    representing["relationships"] = json!([
        relationship(
            REPRESENTATION,
            ("PERSON", vo),
            ("PERSON", subject),
            "at0010",
            None
        ),
        relationship(
            REPRESENTATION,
            ("PERSON", vo),
            ("PERSON", former),
            "at0010",
            Some(interval(Some("2020-01-01"), Some("2025-12-31"))),
        ),
    ]);
    update(&svc, PartyKind::Person, vo, &uid, &representing, "532").await;

    let found = lookup(&svc, value).await;
    let Lookup::Parties(records) = &found else {
        panic!("expected one party, got {found:?}");
    };
    assert_eq!(records.len(), 1, "{found:?}");
    assert_eq!(records[0].markers(), Markers::REPRESENTATIVE);
    let subjects: Vec<(PartyRef, Option<String>)> = records[0]
        .represents()
        .iter()
        .map(|r| (r.subject().clone(), r.relationship().map(str::to_owned)))
        .collect();
    assert_eq!(subjects, [(person_ref(subject), Some("264".to_owned()))]);
}

/// The live key rows, as `(party, issuer, type, value, lifecycle,
/// professional, representative)` text, in a stable order.
async fn key_rows(pool: &sqlx::PgPool) -> Vec<String> {
    sqlx::query(
        "SELECT party_id::text || '|' || issuer || '|' || identifier_type || '|' || value \
                || '|' || lifecycle_state || '|' || professional::text || '|' \
                || representative::text || '|' || (deprovisioned_at IS NOT NULL)::text AS row \
         FROM party.binding_identifier \
         WHERE retired_at IS NULL OR deprovisioned_at IS NOT NULL ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("the rows read")
    .into_iter()
    .map(|row| row.get::<String, _>("row"))
    .collect()
}

#[tokio::test]
async fn a_refill_rebuilds_every_row_from_the_versions() {
    let (db, svc) = service().await;
    let (vo, _) = professional(&svc, UPPER_KEY).await;
    let (_, gone) = professional(&svc, "6f7a8b9c-0d1e-4f2a-9b3c-5d6e7f8a9b0c").await;
    delete(&svc, PartyKind::Person, &gone).await;
    performer(
        &svc,
        PERSON_TEMPLATE,
        &[key("7a8b9c0d-1e2f-4a3b-8c4d-6e7f8a9b0c1d")],
        ROLE_TEMPLATE,
        Some(interval(Some("2020-01-01"), Some("2021-01-01"))),
        "800",
    )
    .await;
    // A PERSON with neither a ROLE nor a representation gets no row.
    create(
        &svc,
        PartyKind::Person,
        &person(
            PERSON_TEMPLATE,
            &[key("8b9c0d1e-2f3a-4b4c-9d5e-7f8a9b0c1d2e")],
        ),
    )
    .await;
    team(&svc, GROUP_TEMPLATE, "ward", &[vo], "532").await;

    let pool = db.pool();
    let before = key_rows(&pool).await;
    assert_eq!(before.len(), 4, "{before:#?}");
    let members = "SELECT group_id::text || member_id::text FROM party.binding_membership \
                   WHERE retired_at IS NULL ORDER BY 1";
    let members_before: Vec<String> = sqlx::query_scalar(members)
        .fetch_all(&pool)
        .await
        .expect("members");

    svc.refill_binding_table().await.expect("the refill runs");

    assert_eq!(key_rows(&pool).await, before);
    let members_after: Vec<String> = sqlx::query_scalar(members)
        .fetch_all(&pool)
        .await
        .expect("members");
    assert_eq!(members_after, members_before);
    assert_eq!(
        parties(&lookup(&svc, &UPPER_KEY.to_ascii_lowercase()).await),
        [person_ref(vo)]
    );
}

#[tokio::test]
async fn the_lookup_is_an_index_only_scan_at_fifty_thousand_persons() {
    let (db, svc) = service().await;
    let (vo, _) = professional(&svc, UPPER_KEY).await;
    let pool = db.pool();
    sqlx::query(
        "WITH k AS (SELECT gen_random_uuid()::text AS value FROM generate_series(1, 50000)) \
         INSERT INTO party.binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
              professional) \
         SELECT uuidv7(), 'key', $1, $2, k.value, k.value, '532', \
                datemultirange(daterange(NULL, NULL)) \
         FROM k",
    )
    .bind(LOGIN)
    .bind(GUID)
    .execute(&pool)
    .await
    .expect("the persons seed");
    sqlx::query(
        "INSERT INTO party.binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state) \
         SELECT uuidv7(), 'group_external_id', $1, 'external_id', 'group-' || n, 'group-' || n, \
                '532' \
         FROM generate_series(1, 5000) AS n",
    )
    .bind(GROUPS)
    .execute(&pool)
    .await
    .expect("the groups seed");
    sqlx::query("VACUUM (ANALYZE) party.binding_identifier")
        .execute(&pool)
        .await
        .expect("vacuum");

    let mut conn = pool.acquire().await.expect("a connection");
    sqlx::query("SET search_path = party, ext, public")
        .execute(&mut *conn)
        .await
        .expect("search path");
    let explain = format!(
        "EXPLAIN (FORMAT JSON) {}",
        ferroehr::party_binding::lookup::LOOKUP
    );
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(explain))
        .bind(LOGIN)
        .bind(GUID)
        .bind(UPPER_KEY.to_ascii_lowercase())
        .bind(TODAY)
        .fetch_one(&mut *conn)
        .await
        .expect("the plan");
    let text = plan.to_string();
    assert!(
        text.contains("\"Index Only Scan\"") && text.contains("ix_binding_identifier_lookup"),
        "the lookup is not an index-only scan of ix_binding_identifier_lookup: {text}"
    );
    drop(conn);
    assert_eq!(
        parties(&lookup(&svc, &UPPER_KEY.to_ascii_lowercase()).await),
        [person_ref(vo)]
    );
}

#[tokio::test]
async fn the_lookup_runs_on_the_party_role_only() {
    use ferroehr::db::DbConfig;
    use ferrohealth_secret::secret_url::SecretUrl;

    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: SecretUrl::new(crate::fixtures::dsn_as(&db, "bclin", "ferroehr_clinical").await),
        ..DbConfig::default()
    };
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(SecretUrl::new(
                crate::fixtures::dsn_as(&db, "bdemo", "ferroehr_party").await,
            )),
            url_file: None,
        },
        ..ferroehr::db::domain::StorageConfig::default()
    };
    let pools = ferroehr::db::connect_domains(&settings, &storage)
        .await
        .expect("each pool connects on its own credential");
    let svc = FerroEhrService::new(&pools).with_binding_mapping(mapping());
    let (vo, _) = professional(&svc, UPPER_KEY).await;

    assert_eq!(
        parties(&lookup(&svc, &UPPER_KEY.to_ascii_lowercase()).await),
        [person_ref(vo)]
    );
    let party_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&pools.party)
        .await
        .expect("the party role");
    assert_eq!(party_role, format!("{}_bdemo", db.name()));
    let clinical = sqlx::query("SELECT 1 FROM party.binding_identifier LIMIT 1")
        .fetch_optional(&pools.clinical)
        .await;
    assert!(
        clinical.is_err(),
        "the clinical role reads the binding side table: {clinical:?}"
    );
    pools.close().await;
}
