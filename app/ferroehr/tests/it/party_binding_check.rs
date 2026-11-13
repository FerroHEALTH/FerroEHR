// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pre-flight of a migration to party binding (#5009) over a fixture
//! store on a real PostgreSQL 18 (shared testkit harness).
//!
//! The parties are committed through the demographic service with the
//! committer each history would carry: a SCIM client's client identifier
//! (`committer::bound::server_committer`), an administrator's reference to
//! its `PERSON`, a declared migration's name, and an unknown writer. The
//! lifecycle codes are RM common `master06-change_control_package.adoc`
//! §Version Lifecycle. No openEHR spec governs a deployment's migration: our
//! own design.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::fixtures::{change_type, if_match, object, written_vo};
use ferroehr::ids::VoId;
use ferroehr::party_binding::check::{
    Administrator, BasicUser, CheckError, CheckInput, Expected, Origin,
};
use ferroehr::party_binding::{BindingMapping, IdentifierSelector};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::jit;
use ferroehr::service::demographic::party_set::NewParty;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The template of a professional's `PERSON`.
const PERSON_TEMPLATE: &str = "FerroEHR professional";
/// The template of the professional `ROLE`.
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
/// The identifier cluster the key selector reads.
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";
/// The `identifier_issuer` of the login's `[[auth.party]]` table.
const LOGIN: &str = "https://login.example/tenant";
/// The token issuer of the SCIM client.
const MACHINES: &str = "https://auth.example";
/// The SCIM client.
const CLIENT: &str = "entra-provisioning";
/// The committer name of the declared migration.
const MIGRATION: &str = "Cadasto V1 migration";
/// The namespace the parties name each other in.
const NAMESPACE: &str = "demographic";

/// The mapping the fixture store is read under.
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

async fn service() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    (db, svc.with_binding_mapping(mapping()))
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn archetyped(archetype: &str, template: &str) -> Value {
    json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
        "template_id": { "_type": "TEMPLATE_ID", "value": template },
        "rm_version": "1.1.0"
    })
}

fn identities() -> Value {
    json!([{
        "_type": "PARTY_IDENTITY", "archetype_node_id": "at0002", "name": text("name"),
        "details": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0003", "name": text("structure"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0004",
                        "name": text("name"), "value": text("professional") }]
        }
    }])
}

fn party_ref(class: &str, id: VoId) -> Value {
    json!({
        "_type": "PARTY_REF", "namespace": NAMESPACE, "type": class,
        "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() }
    })
}

/// A `PERSON` of the mapped template carrying the login key `key`.
fn person(key: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", PERSON_TEMPLATE),
        "name": text("professional"),
        "identities": identities(),
        "details": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": CLUSTER,
                        "name": text("identifiers"), "items": [{
                "_type": "ELEMENT", "archetype_node_id": "at0001", "name": text("identifier"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": LOGIN, "id": key, "type": "guid" }
            }] }]
        }
    })
}

/// The professional `ROLE` performed by `performer`.
fn role(performer: VoId) -> Value {
    json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", ROLE_TEMPLATE),
        "name": text("professional"),
        "identities": identities(),
        "performer": party_ref("PERSON", performer)
    })
}

/// The committer a SCIM client's commit records.
fn scim() -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED", "name": CLIENT,
        "identifiers": [{ "issuer": MACHINES, "id": CLIENT, "type": "client" }]
    })
}

/// The committer an administrator bound to `admin` records.
fn administrator(admin: VoId) -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED", "name": "administrator",
        "external_ref": party_ref("PERSON", admin),
        "identifiers": [{ "issuer": MACHINES, "id": "admin", "type": "oauth2" }]
    })
}

/// A committer known only by its name.
fn named(name: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": name })
}

/// The write of `data` in `lifecycle` by `committer`, after `preceding`.
fn version<T: serde::de::DeserializeOwned>(
    data: &Value,
    lifecycle: &str,
    preceding: Option<&str>,
    committer: &Value,
) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.lifecycle_state = Some(lifecycle.to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    let code = if preceding.is_some() {
        change_type("251", "modification")
    } else {
        change_type("249", "creation")
    };
    write.audit = Some(
        openehr_its::json::from_canonical_value(&json!({
            "change_type": code, "committer": committer
        }))
        .expect("an UPDATE_AUDIT"),
    );
    write
}

async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
    lifecycle: &str,
    committer: &Value,
) -> (VoId, String) {
    let created = Box::pin(svc.party_create(kind, version(body, lifecycle, None, committer)))
        .await
        .expect("the party commits");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    (written_vo(&created), uid)
}

async fn update(
    svc: &FerroEhrService,
    vo: VoId,
    uid: &str,
    body: &Value,
    lifecycle: &str,
    committer: &Value,
) -> String {
    Box::pin(svc.party_update(
        PartyKind::Person,
        object(vo.to_string()),
        if_match(uid),
        version(body, lifecycle, Some(uid), committer),
    ))
    .await
    .expect("the party updates")
    .meta
    .expect("a version")
    .uid
}

/// A `PERSON` carrying `key` introduced by `committer`, performing the
/// professional `ROLE` when `professional`, its head left in `lifecycle`.
async fn person_with(
    svc: &FerroEhrService,
    key: &str,
    committer: &Value,
    professional: bool,
    lifecycle: &str,
) -> VoId {
    let body = person(key);
    let first = if lifecycle == "553" { "553" } else { "532" };
    let (vo, uid) = create(svc, PartyKind::Person, &body, first, committer).await;
    if !professional {
        if lifecycle != first {
            update(svc, vo, &uid, &body, lifecycle, committer).await;
        }
        return vo;
    }
    let (role_vo, _) = create(svc, PartyKind::Role, &role(vo), "532", committer).await;
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", role_vo)]);
    update(svc, vo, &uid, &listed, lifecycle, committer).await;
    vo
}

/// The side-table rows and the fill state as text, to show the check
/// leaves both as it found them.
async fn snapshot(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT format('%s %s %s %s %s', party_id, selector, value, lifecycle_state, \
                       retired_at IS NULL) FROM party.binding_identifier \
         UNION ALL SELECT 'state ' || digest FROM party.binding_state ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("the side table reads")
}

/// The input every test starts from: the SCIM client, the declared
/// migration and the key type of the login's issuer.
fn input() -> CheckInput {
    CheckInput {
        key_types: BTreeMap::from([(LOGIN.to_owned(), "guid".to_owned())]),
        scim_clients: vec![(MACHINES.to_owned(), CLIENT.to_owned())],
        migrations: vec![MIGRATION.to_owned()],
        provisional_report_after: ferrohealth_config::units::Duration::from_days(30),
        today: ferroehr::support::today_utc().to_string(),
        ..CheckInput::default()
    }
}

const SCIM_KEY: &str = "6a1b2c3d-4e5f-4a6b-8c7d-8e9f0a1b2c3d";
const ADMIN_KEY: &str = "7b2c3d4e-5f6a-4b7c-9d8e-9f0a1b2c3d4e";
const ADMINS_KEY: &str = "8c3d4e5f-6a7b-4c8d-ae9f-0a1b2c3d4e5f";
const PLANTED_KEY: &str = "9d4e5f6a-7b8c-4d9e-bf0a-1b2c3d4e5f6a";
const INACTIVE_KEY: &str = "0e5f6a7b-8c9d-4eaf-8a1b-2c3d4e5f6a7b";
const PROVISIONAL_KEY: &str = "1f6a7b8c-9d0e-4fb0-9b2c-3d4e5f6a7b8c";
const ABSENT_KEY: &str = "2a7b8c9d-0e1f-4ac1-ac3d-4e5f6a7b8c9d";

#[tokio::test]
async fn the_check_reports_every_category_and_writes_nothing() {
    let (db, svc) = service().await;
    let provisioned = person_with(&svc, SCIM_KEY, &scim(), true, "532").await;
    let admin = person_with(&svc, ADMIN_KEY, &named(MIGRATION), true, "532").await;
    let by_admin = person_with(&svc, ADMINS_KEY, &administrator(admin), true, "532").await;
    let planted = person_with(
        &svc,
        PLANTED_KEY,
        &named("a demographic writer"),
        false,
        "532",
    )
    .await;
    let inactive = person_with(&svc, INACTIVE_KEY, &scim(), true, "800").await;
    let provisional = person_with(&svc, PROVISIONAL_KEY, &scim(), true, "553").await;
    let before = snapshot(&db.pool()).await;

    let mut input = input();
    input.administrators = vec![Administrator::Key {
        issuer: LOGIN.to_owned(),
        key: ADMIN_KEY.to_uppercase(),
    }];
    input.provisional_report_after = ferrohealth_config::units::Duration::ZERO;
    input.basic_users = vec![
        BasicUser {
            user: "ferroehr".to_owned(),
            key_issuer: LOGIN.to_owned(),
            key: SCIM_KEY.to_owned(),
        },
        BasicUser {
            user: "nobody".to_owned(),
            key_issuer: LOGIN.to_owned(),
            key: ABSENT_KEY.to_owned(),
        },
    ];
    input.expected = Some(Expected {
        issuer: LOGIN.to_owned(),
        logins: vec![SCIM_KEY.to_uppercase(), ABSENT_KEY.to_owned()],
    });
    let report = svc.check_binding(&input).await.expect("the check runs");

    assert_eq!(
        snapshot(&db.pool()).await,
        before,
        "the check writes nothing"
    );
    assert_eq!(report.administrators.len(), 1);
    assert_eq!(report.administrators[0].party, admin.0);
    assert_eq!(report.migrations, [MIGRATION]);
    assert_eq!(report.duplicates, [], "{report:?}");
    let mut bearers = vec![
        provisioned.0,
        admin.0,
        by_admin.0,
        planted.0,
        inactive.0,
        provisional.0,
    ];
    bearers.sort();
    assert_eq!(report.issuers.len(), 1);
    assert_eq!(report.issuers[0].issuer, LOGIN);
    assert_eq!(report.issuers[0].persons, bearers);
    assert_eq!(report.inactive, [inactive.0]);
    assert_eq!(report.abandoned, Vec::<Uuid>::new());
    let table = report.side_table.as_ref().expect("no duplicate stands");
    let provisional_parties: Vec<Uuid> = table.provisional.iter().map(|p| p.party).collect();
    assert_eq!(provisional_parties, [provisional.0], "{table:?}");
    assert_eq!(table.unadopted, [planted.0], "{table:?}");
    assert_eq!(table.basic_users.len(), 2);
    assert_eq!(table.basic_users[0].party, Some(provisioned.0));
    assert_eq!(table.basic_users[1].party, None);
    let origin = |party: VoId| {
        report
            .provenance
            .iter()
            .find(|p| p.party == party.0)
            .map(|p| p.by)
            .expect("the key has a provenance")
    };
    assert_eq!(origin(provisioned), Some(Origin::Scim));
    assert_eq!(origin(admin), Some(Origin::Migration));
    assert_eq!(origin(by_admin), Some(Origin::Administrator));
    assert_eq!(origin(planted), None);
    let expected = report
        .expected
        .as_ref()
        .expect("expected logins were given");
    assert_eq!(expected.missing, [ABSENT_KEY]);

    let failures = report.failures();
    assert_eq!(failures.len(), 3, "{failures:?}");
    assert!(
        failures.iter().any(|f| f.contains("\"nobody\"")),
        "{failures:?}"
    );
    assert!(
        failures.iter().any(|f| f.contains(ABSENT_KEY)),
        "{failures:?}"
    );
    assert!(
        failures.iter().any(|f| f.contains(&planted.to_string())),
        "{failures:?}"
    );
}

#[tokio::test]
async fn a_key_two_persons_carry_is_a_duplicate_and_fails() {
    let (db, svc) = service().await;
    let first = person_with(&svc, PLANTED_KEY, &scim(), false, "532").await;
    let second = person_with(&svc, &PLANTED_KEY.to_uppercase(), &scim(), false, "532").await;
    let before = snapshot(&db.pool()).await;

    let report = svc.check_binding(&input()).await.expect("the check runs");

    assert_eq!(
        snapshot(&db.pool()).await,
        before,
        "the check writes nothing"
    );
    assert_eq!(report.duplicates.len(), 1, "{report:?}");
    let mut parties = vec![first.0, second.0];
    parties.sort();
    assert_eq!(report.duplicates[0].parties, parties);
    assert_eq!(report.duplicates[0].selector, "key");
    assert!(report.side_table.is_none(), "a fill refuses a duplicate");
    assert_eq!(report.failures().len(), 1, "{:?}", report.failures());
}

#[tokio::test]
async fn a_store_provisioned_through_scim_passes() {
    let (_db, svc) = service().await;
    let provisioned = person_with(&svc, SCIM_KEY, &scim(), true, "532").await;
    let mut input = input();
    input.basic_users = vec![BasicUser {
        user: "ferroehr".to_owned(),
        key_issuer: LOGIN.to_owned(),
        key: SCIM_KEY.to_owned(),
    }];
    input.administrators = vec![Administrator::Party(Uuid::now_v7())];

    let report = svc.check_binding(&input).await.expect("the check runs");

    assert_eq!(report.failures(), Vec::<String>::new(), "{report:?}");
    assert_eq!(report.provenance.len(), 1);
    assert_eq!(report.provenance[0].party, provisioned.0);
    assert_eq!(
        report.provenance[0].committer.identifiers,
        [format!("{MACHINES}|client|{CLIENT}")]
    );
}

#[tokio::test]
async fn an_administrator_key_no_person_carries_is_refused() {
    let (_db, svc) = service().await;
    person_with(&svc, SCIM_KEY, &scim(), true, "532").await;
    let mut input = input();
    input.administrators = vec![Administrator::Key {
        issuer: LOGIN.to_owned(),
        key: ABSENT_KEY.to_owned(),
    }];

    let refused = svc.check_binding(&input).await;

    assert!(
        matches!(refused, Err(CheckError::UnknownAdministrator { ref key, .. }) if key == ABSENT_KEY),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_service_with_no_mapping_has_nothing_to_check() {
    let (_db, svc) = crate::fixtures::service().await;
    let refused = svc.check_binding(&input()).await;
    assert!(matches!(refused, Err(CheckError::Unmapped)), "{refused:?}");
}

/// The issuer whose first logins just-in-time creation provisions.
const JIT_ISSUER: &str = "https://login.example/jit";
const JIT_KEY: &str = "3b8c9d0e-1f2a-4b3c-8d4e-5f6a7b8c9d0e";

/// A provisional `PERSON` carrying [`JIT_KEY`] and its professional `ROLE`,
/// committed in one CONTRIBUTION as just-in-time creation commits them: the
/// `249|creation|` change type, the description `jit::description` writes
/// for `issuer`, and a committer whose `external_ref` is `referenced`, or the
/// new `PERSON` itself when `None`.
async fn jit_created(svc: &FerroEhrService, issuer: &str, referenced: Option<VoId>) -> VoId {
    let (person_id, role_id) = (svc.allocate_party_id(), svc.allocate_party_id());
    let (person_vo, role_vo) = (person_id.id(), role_id.id());
    let mut body = person(JIT_KEY);
    body["roles"] = json!([party_ref("ROLE", role_vo)]);
    let committer =
        openehr_its::json::from_canonical_value(&administrator(referenced.unwrap_or(person_vo)))
            .expect("a PARTY_PROXY");
    let parties = vec![
        NewParty {
            id: person_id,
            kind: PartyKind::Person,
            body,
            lifecycle_state: Some("553".to_owned()),
        },
        NewParty {
            id: role_id,
            kind: PartyKind::Role,
            body: role(person_vo),
            lifecycle_state: None,
        },
    ];
    svc.create_parties(parties, &jit::description(issuer), Some(committer))
        .await
        .expect("the creation commits");
    person_vo
}

/// The provenance verdict of the one key the store holds under `input`.
async fn verdict(svc: &FerroEhrService, input: &CheckInput) -> Option<Origin> {
    let report = svc.check_binding(input).await.expect("the check runs");
    assert_eq!(report.provenance.len(), 1, "{report:?}");
    report.provenance[0].by
}

#[tokio::test]
async fn a_key_just_in_time_creation_introduced_passes_on_a_jit_issuer() {
    let (_db, svc) = service().await;
    let created = jit_created(&svc, JIT_ISSUER, None).await;
    let mut input = input();

    assert_eq!(
        verdict(&svc, &input).await,
        None,
        "no persons issuer is bound"
    );
    input.jit_issuers = vec![ferrohealth_secret::secret_url::SecretUrl::new(JIT_ISSUER)];
    assert_eq!(verdict(&svc, &input).await, Some(Origin::Jit));
    input.provisional_report_after = ferrohealth_config::units::Duration::ZERO;
    let report = svc.check_binding(&input).await.expect("the check runs");
    let table = report.side_table.expect("no duplicate stands");
    assert_eq!(table.provisional.len(), 1, "{table:?}");
    assert_eq!(table.provisional[0].party, created.0);
}

#[tokio::test]
async fn a_creation_naming_another_committer_is_no_jit_creation() {
    let (_db, svc) = service().await;
    jit_created(&svc, JIT_ISSUER, Some(VoId(Uuid::now_v7()))).await;
    let mut input = input();
    input.jit_issuers = vec![ferrohealth_secret::secret_url::SecretUrl::new(JIT_ISSUER)];

    assert_eq!(verdict(&svc, &input).await, None);
}
