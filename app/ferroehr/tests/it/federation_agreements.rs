// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The federation agreements in the CDR (#5440) over a real PostgreSQL 18: an
//! agreement is a grant of `federated_person_access` or
//! `federated_organisation_access` held by the partner's `ACTOR`, its terms
//! the coded elements of `ferroehr-operations` in the capability's
//! credentials, and every legal check the configuration once ran at boot
//! refuses the `ROLE` at its commit with `422`, a test per check. A
//! conforming agreement is listed as in force, and a change reaches the next
//! read with no restart. No openEHR spec gives a capability an authorization
//! meaning: our own design.

use std::collections::{BTreeMap, HashMap};

use assert_fs::prelude::FileWriteStr as _;
use serde_json::{Value, json};

use crate::fixtures::{if_match, object, written_vo};
use ferroehr::config::FerroEhrConfig;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::agreement::Accessor;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::grants::trust::{Correlation, FederationTrust, PartnerIssuer};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::config::Retention;

/// A `federation` issuer that maps a `chained` request's correlation id from
/// the token.
const FEDERATION: &str = "https://federation.example";
/// A MyHealth@EU contact point.
const CONTACT_POINT: &str = "https://ncpeh-a.example";
/// A `federation` issuer that maps the correlation id from a constant.
const CONSTANT: &str = "https://constant.example";
/// A `federation` issuer that maps no correlation id.
const UNMAPPED: &str = "https://unmapped.example";
/// A `persons` issuer.
const PERSONS: &str = "https://idp.example/realms/staff";

/// The configuration of `toml`.
fn config(toml: &str) -> FerroEhrConfig {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(toml).expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[]).expect("assembles")
}

/// The trust of the four `federation` issuers, the access records kept five
/// years.
fn trust() -> FederationTrust {
    let partner = |contact_point, correlation| PartnerIssuer {
        contact_point,
        correlation,
        correlation_source: None,
    };
    FederationTrust::new(
        BTreeMap::from([
            (FEDERATION.to_owned(), partner(false, Correlation::Token)),
            (CONTACT_POINT.to_owned(), partner(true, Correlation::None)),
            (CONSTANT.to_owned(), partner(false, Correlation::Constant)),
            (UNMAPPED.to_owned(), partner(false, Correlation::None)),
        ]),
        Retention::Years(5),
    )
}

/// A replica over a fresh database that may grant every operation and judges
/// the agreements against [`trust`].
async fn service() -> (testkit::TestDb, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    svc.seed_default_definitions()
        .await
        .expect("the policy definitions are seeded");
    let config = config("");
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], "ADMIN")
        .expect("the mapping compiles");
    let svc = svc
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(GrantCeiling::new(Operation::ALL))
        .with_federation_trust(trust());
    (db, svc)
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A `DV_CODED_TEXT` of the operation terminology.
fn coded(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

fn write<T: serde::de::DeserializeOwned>(data: &Value, preceding: Option<&str>) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Commits a partner `ORGANISATION` carrying the client `remote` of each of
/// `issuers`; returns its container.
async fn partner(svc: &FerroEhrService, issuers: &[&str]) -> VoId {
    let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.partner.v1";
    let items: Vec<Value> = issuers
        .iter()
        .map(|issuer| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("client"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": "remote",
                               "type": "client_id" } })
        })
        .collect();
    let body = json!({
        "_type": "ORGANISATION", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Partner"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010",
                     "name": text("details"), "items": items },
        "identities": identities("Partner")
    });
    let created = Box::pin(svc.party_create(PartyKind::Organisation, write(&body, None)))
        .await
        .expect("the partner commits");
    written_vo(&created)
}

/// The element the code `code` names, holding the text `value`.
fn term(code: &str, value: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": coded(code),
            "value": text(value) })
}

/// The element the code `code` names, holding `value`.
fn holding(code: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": coded(code),
            "value": value })
}

/// The caller rate every agreement needs.
fn rate() -> Vec<Value> {
    vec![
        holding(
            "requests_per_second",
            &json!({ "_type": "DV_COUNT", "magnitude": 5 }),
        ),
        holding("burst", &json!({ "_type": "DV_COUNT", "magnitude": 10 })),
    ]
}

/// The terms of a person agreement that keeps every rule.
fn person() -> Vec<Value> {
    let mut items = rate();
    items.push(term("read_scope", "patient/composition-*.r"));
    items
}

/// The terms of an organisation agreement that keeps every rule.
fn organisation() -> Vec<Value> {
    let mut items = person();
    items.extend([
        term("agreement_name", "Summary exchange"),
        term("agreement_version", "1.0"),
        term("gdpr_art_6_1", "e"),
        term("gdpr_art_9_2", "h"),
        term("national_provision", "a national provision"),
        term("access_class", "automated"),
    ]);
    items
}

/// The capability granting the agreement of `accessor` on `items`.
fn capability(accessor: Accessor, items: &[Value]) -> Value {
    let mut all = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
                               "name": text("operation"),
                               "value": coded(accessor.operation().code()) })];
    all.extend_from_slice(items);
    json!({ "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("agreement"),
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022",
                             "name": text("terms"), "items": all } })
}

/// A `ROLE` the `performer` performs holding `capabilities`.
fn role(performer: VoId, capabilities: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.data_sharing.v1";
    json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Data sharing"),
        "identities": identities("data sharing"),
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                       "id": { "_type": "HIER_OBJECT_ID", "value": performer.to_string() } },
        "capabilities": capabilities
    })
}

/// Commits the `ROLE` of `performer` holding `capabilities`; its version uid.
async fn grant(
    svc: &FerroEhrService,
    performer: VoId,
    capabilities: &[Value],
) -> Result<String, ServiceError> {
    let created =
        Box::pin(svc.party_create(PartyKind::Role, write(&role(performer, capabilities), None)))
            .await?;
    Ok(created.meta.expect("a version").uid)
}

/// The refusal of the `ROLE` of a partner of `issuers` holding the agreement
/// of `accessor` on `items`, which must be a `422`.
async fn refused(issuers: &[&str], accessor: Accessor, items: &[Value]) -> String {
    let (_db, svc) = service().await;
    let performer = partner(&svc, issuers).await;
    let refusal = grant(&svc, performer, &[capability(accessor, items)])
        .await
        .expect_err("the agreement is refused");
    assert!(
        matches!(refusal, ServiceError::Unprocessable { .. }),
        "{refusal:?}"
    );
    format!("{refusal:?}")
}

/// `items` without the element the code `code` names.
fn without(items: Vec<Value>, code: &str) -> Vec<Value> {
    items
        .into_iter()
        .filter(|item| item["name"]["defining_code"]["code_string"] != code)
        .collect()
}

#[tokio::test]
async fn a_conforming_agreement_commits_is_listed_and_changes_with_no_restart() {
    let (_db, svc) = service().await;
    let performer = partner(&svc, &[FEDERATION]).await;
    let uid = grant(
        &svc,
        performer,
        &[
            capability(Accessor::Person, &person()),
            capability(Accessor::Organisation, &organisation()),
        ],
    )
    .await
    .expect("the agreements commit");
    svc.grants().refresh().await.expect("the grants read");
    let listed = svc.grants().agreements(jiff::Timestamp::now());
    assert_eq!(listed.len(), 2, "{listed:?}");
    let organisation_listed = listed
        .iter()
        .find(|agreement| agreement.accessor == Accessor::Organisation)
        .expect("the organisation agreement");
    assert_eq!(organisation_listed.issuer, FEDERATION);
    assert_eq!(organisation_listed.client_id, "remote");
    assert_eq!(organisation_listed.name, "Summary exchange");
    assert_eq!(
        organisation_listed.legal_ground.as_deref(),
        Some("GDPR Art. 6(1)(e), Art. 9(2)(h), a national provision")
    );
    // The next version drops the organisation agreement; the next read lists the
    // person's alone, with no restart.
    let vo = written_vo_of(&uid);
    Box::pin(svc.party_update(
        PartyKind::Role,
        object(vo.to_string()),
        if_match(&uid),
        write(
            &role(performer, &[capability(Accessor::Person, &person())]),
            Some(&uid),
        ),
    ))
    .await
    .expect("the ROLE updates");
    svc.grants().refresh().await.expect("the grants read");
    let listed = svc.grants().agreements(jiff::Timestamp::now());
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].accessor, Accessor::Person);
}

/// The container of the version uid `uid`.
fn written_vo_of(uid: &str) -> VoId {
    VoId(
        uid.split("::")
            .next()
            .and_then(|id| id.parse().ok())
            .expect("a container id"),
    )
}

#[tokio::test]
async fn an_agreement_whose_partner_is_no_federation_issuers_is_refused() {
    let said = refused(&[PERSONS], Accessor::Person, &person()).await;
    assert!(
        said.contains("no [[auth.issuer]] of kind federation"),
        "{said}"
    );
    let (_db, svc) = service().await;
    let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.partner.v1";
    let bare = json!({
        "_type": "ORGANISATION", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Partner"),
        "identities": identities("Partner")
    });
    let created = Box::pin(svc.party_create(PartyKind::Organisation, write(&bare, None)))
        .await
        .expect("the partner commits");
    let refusal = grant(
        &svc,
        written_vo(&created),
        &[capability(Accessor::Person, &person())],
    )
    .await
    .expect_err("refused");
    assert!(
        format!("{refusal:?}").contains("carries no client_id identifier"),
        "{refusal:?}"
    );
}

#[tokio::test]
async fn a_second_agreement_for_one_accessor_is_refused() {
    let (_db, svc) = service().await;
    let performer = partner(&svc, &[FEDERATION]).await;
    let twice = grant(
        &svc,
        performer,
        &[
            capability(Accessor::Person, &person()),
            capability(Accessor::Person, &person()),
        ],
    )
    .await
    .expect_err("refused");
    assert!(format!("{twice:?}").contains("twice"), "{twice:?}");
    grant(&svc, performer, &[capability(Accessor::Person, &person())])
        .await
        .expect("the first agreement commits");
    let again = grant(&svc, performer, &[capability(Accessor::Person, &person())])
        .await
        .expect_err("refused");
    assert!(
        format!("{again:?}").contains("already holds a federated_person_access agreement"),
        "{again:?}"
    );
}

#[tokio::test]
async fn every_agreement_needs_a_caller_rate_patient_reads_and_a_coded_legal_ground() {
    let mut items = without(without(person(), "requests_per_second"), "burst");
    items.push(term("read_scope", "user/composition-*.r"));
    items.extend([term("gdpr_art_6_1", "g"), term("gdpr_art_9_2", "h")]);
    let said = refused(&[FEDERATION], Accessor::Person, &items).await;
    for expected in [
        "requests_per_second and burst",
        "patient/ scopes only",
        "gdpr_art_6_1 is not a point a to f",
    ] {
        assert!(said.contains(expected), "{expected}: {said}");
    }
}

#[tokio::test]
async fn an_element_holding_no_value_of_its_kind_is_refused() {
    let mut items = person();
    items.push(term("access_class", "sometimes"));
    items.push(holding(
        "discard_window",
        &json!({ "_type": "DV_DURATION", "value": "PT0H" }),
    ));
    let said = refused(&[FEDERATION], Accessor::Person, &items).await;
    assert!(
        said.contains("access_class holds \\\"sometimes\\\""),
        "{said}"
    );
    assert!(said.contains("discard_window holds"), "{said}");
}

#[tokio::test]
async fn a_contact_points_person_agreement_names_its_ehdsi_service_and_no_other_does() {
    let said = refused(&[CONTACT_POINT], Accessor::Person, &person()).await;
    assert!(said.contains("needs ehdsi_service"), "{said}");
    let mut served = person();
    served.push(term("ehdsi_service", "eprescription"));
    let (_db, svc) = service().await;
    let contact = partner(&svc, &[CONTACT_POINT]).await;
    grant(&svc, contact, &[capability(Accessor::Person, &served)])
        .await
        .expect("a contact point's agreement naming its service commits");
    let said = refused(&[FEDERATION], Accessor::Person, &served).await;
    assert!(
        said.contains("only a contact point's agreement may"),
        "{said}"
    );
}

#[tokio::test]
async fn every_organisation_agreement_refusal_is_named() {
    for (code, expected) in [
        ("agreement_name", "needs agreement_name"),
        ("agreement_version", "needs agreement_version"),
        ("access_class", "needs access_class"),
    ] {
        let said = refused(
            &[FEDERATION],
            Accessor::Organisation,
            &without(organisation(), code),
        )
        .await;
        assert!(said.contains(expected), "{expected}: {said}");
    }
    let grounded = without(
        without(without(organisation(), "gdpr_art_6_1"), "gdpr_art_9_2"),
        "national_provision",
    );
    let said = refused(&[FEDERATION], Accessor::Organisation, &grounded).await;
    assert!(
        said.contains("needs gdpr_art_6_1 and gdpr_art_9_2"),
        "{said}"
    );
    for (extra, expected) in [
        (
            term("template", "Vital signs.v1"),
            "an organisation access is read-only",
        ),
        (
            holding(
                "opt_out_override",
                &json!({ "_type": "DV_BOOLEAN", "value": true }),
            ),
            "only a person agreement may",
        ),
        (term("consent", "registry"), "takes consent from a registry"),
    ] {
        let mut items = organisation();
        items.push(extra);
        let said = refused(&[FEDERATION], Accessor::Organisation, &items).await;
        assert!(said.contains(expected), "{expected}: {said}");
    }
    let said = refused(&[CONTACT_POINT], Accessor::Organisation, &organisation()).await;
    assert!(
        said.contains("contact point, whose requests name a professional"),
        "{said}"
    );
}

#[tokio::test]
async fn a_chained_agreement_needs_a_long_enough_requester_log_and_a_token_correlation() {
    let chained = |retention: Option<&str>| {
        let mut items = without(organisation(), "access_class");
        items.push(term("access_class", "chained"));
        if let Some(period) = retention {
            items.push(term("requester_log_retention", period));
        }
        items
    };
    let said = refused(&[FEDERATION], Accessor::Organisation, &chained(None)).await;
    assert!(said.contains("needs requester_log_retention"), "{said}");
    let said = refused(&[FEDERATION], Accessor::Organisation, &chained(Some("P2Y"))).await;
    assert!(
        said.contains("keeps the requester's log for \\\"P2Y\\\", shorter than"),
        "{said}"
    );
    let said = refused(&[UNMAPPED], Accessor::Organisation, &chained(Some("P30Y"))).await;
    assert!(said.contains("maps no correlation_id"), "{said}");
    let said = refused(&[CONSTANT], Accessor::Organisation, &chained(Some("P30Y"))).await;
    assert!(said.contains("from a constant"), "{said}");
    let (_db, svc) = service().await;
    let performer = partner(&svc, &[FEDERATION]).await;
    grant(
        &svc,
        performer,
        &[capability(Accessor::Organisation, &chained(Some("P30Y")))],
    )
    .await
    .expect("a chained agreement whose issuer reads the id from the token commits");
}
