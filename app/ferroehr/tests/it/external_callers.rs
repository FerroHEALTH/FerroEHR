// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a verified external caller of a `federation` issuer commits (#5101),
//! over a real PostgreSQL 18: an external person only a COMPOSITION of a
//! template its agreement's `writes` lists, and an external organisation
//! nothing (EHDS Art. 13(4)). What an external caller may commit is our own
//! design.

use serde_json::{Value, json};

use crate::committer_binding::service;
use crate::fixtures::{attestation_by, ckm_example, deleting};
use ferroehr::error::ServiceError;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::external::ExternalProfessional;
use ferroehr::versioning::committer::{CommitterIdentity, ExternalCaller, with_committer};

/// The provider the professional's token asserts.
const PROVIDER: &str = "urn:oid:2.999.7";
/// The issuing authority of the provider and the professional.
const AUTHORITY: &str = "Registry";

/// What a domestic federation token asserts about the professional and the
/// provider.
fn asserted() -> ExternalProfessional {
    ExternalProfessional {
        name: Some("Dr Remote".to_owned()),
        hp_identifier: Some("hp-1".to_owned()),
        country_code: Some("DE".to_owned()),
        issuing_authority: Some(AUTHORITY.to_owned()),
        provider_identifier: Some(PROVIDER.to_owned()),
        provider_authority: Some(AUTHORITY.to_owned()),
        contact_point: false,
        permissions: Vec::new(),
        name_id: None,
        assertion_issuer: None,
    }
}

/// An external person writing `template`, asserting `professional`.
fn person(template: &str, professional: ExternalProfessional) -> CommitterIdentity {
    external(ExternalCaller::Person {
        writes: vec![template.to_owned()],
        professional: Box::new(professional),
        discard_window_hours: None,
    })
}

/// `composition` naming `facility` as its `EVENT_CONTEXT.health_care_facility`.
fn at_facility(mut composition: Value, facility: &Value) -> Value {
    if let Some(context) = composition
        .get_mut("context")
        .and_then(Value::as_object_mut)
    {
        context.insert("health_care_facility".to_owned(), facility.clone());
    }
    composition
}

/// The template `composition` is of.
fn template_of(composition: &Value) -> String {
    composition
        .pointer("/archetype_details/template_id/value")
        .and_then(Value::as_str)
        .expect("the example names its template")
        .to_owned()
}

/// The provider as a `PARTY_IDENTIFIED` facility.
fn provider_facility() -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": "Remote clinic",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": AUTHORITY, "id": PROVIDER }] })
}

/// The identity of an external caller that may commit as `external`.
fn external(external: ExternalCaller) -> CommitterIdentity {
    CommitterIdentity {
        subject: "hp-remote-1".to_owned(),
        id_type: "oauth2",
        issuer: Some("https://federation.example".to_owned()),
        client: Some("remote-portal".to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: Some(external),
    }
}

fn forbidden<T: std::fmt::Debug>(result: Result<T, ServiceError>) {
    match result.map_err(ServiceError::flattened) {
        Err(ServiceError::Forbidden(_)) => {}
        other => panic!("expected 403, got {other:?}"),
    }
}

#[tokio::test]
async fn an_external_person_commits_only_a_listed_template_and_an_organisation_nothing() {
    let (_db, svc, ehr) = service().await;
    let example = ckm_example(&svc, "vital-signs").await;
    let template = example
        .pointer("/archetype_details/template_id/value")
        .and_then(Value::as_str)
        .expect("the example names its template")
        .to_owned();
    let example = at_facility(example, &provider_facility());
    let person = person(&template, asserted());
    let commit = |identity: CommitterIdentity, body: Value| {
        let svc = &svc;
        async move {
            let request = WriteRequest::new(crate::typed_body::typed(&body));
            with_committer(
                Some(identity),
                Box::pin(svc.create_composition(ehr, request)),
            )
            .await
        }
    };
    let committed = commit(person.clone(), example.clone())
        .await
        .expect("a listed template commits");
    // A template outside the writes, an EHR, a demographic party, a deletion
    // and an attestation are refused.
    forbidden(commit(person.clone(), crate::fixtures::composition("c")).await);
    forbidden(
        with_committer(
            Some(person.clone()),
            Box::pin(svc.create_ehr_meta(WriteRequest::new(None))),
        )
        .await,
    );
    let party = json!({ "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "p" },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "s" }, "items": [] } }] });
    forbidden(
        with_committer(
            Some(person.clone()),
            Box::pin(svc.party_create(
                PartyKind::Person,
                WriteRequest::new(crate::typed_body::typed(&party)),
            )),
        )
        .await,
    );
    let uid: openehr_base::prelude::ObjectVersionId = format!(
        "{}::{}::{}",
        committed.vo_id, committed.creating_system_id, committed.tree
    )
    .parse()
    .expect("an OBJECT_VERSION_ID");
    forbidden(
        with_committer(
            Some(person.clone()),
            Box::pin(svc.delete_composition(ehr, &uid, deleting(Some(uid.clone())))),
        )
        .await,
    );
    let mut attested = WriteRequest::new(crate::typed_body::typed(&example));
    attested.attestations = vec![
        openehr_its::json::from_canonical_value(&attestation_by("x", "signed", false))
            .expect("an UPDATE_ATTESTATION"),
    ];
    forbidden(
        with_committer(
            Some(person),
            Box::pin(svc.create_composition(ehr, attested)),
        )
        .await,
    );
    forbidden(commit(external(ExternalCaller::Organisation), example).await);
}

fn unprocessable<T: std::fmt::Debug>(result: Result<T, ServiceError>) {
    match result.map_err(ServiceError::flattened) {
        Err(ServiceError::Unprocessable { .. }) => {}
        other => panic!("expected 422, got {other:?}"),
    }
}

/// The external person's committer names the professional under
/// `"<country_code>:<issuing_authority_name>"` and the login the federation
/// issuer vouched for, with no `external_ref` (#5097).
#[tokio::test]
async fn an_external_persons_commit_records_the_professional_and_the_vouching_login() {
    let (_db, svc, ehr) = service().await;
    let example = ckm_example(&svc, "vital-signs").await;
    let template = template_of(&example);
    let asserted = ExternalProfessional {
        issuing_authority: Some(" Bundes\u{0061}\u{0308}rztekammer \t Registry ".to_owned()),
        ..asserted()
    };
    let written = with_committer(
        Some(person(&template, asserted)),
        Box::pin(svc.create_composition(
            ehr,
            WriteRequest::new(crate::typed_body::typed(&at_facility(
                example,
                &provider_facility(),
            ))),
        )),
    )
    .await
    .expect("the external write commits");
    let ovid = format!(
        "{}::{}::{}",
        written.vo_id, written.creating_system_id, written.tree
    );
    let committer = svc
        .composition_version_envelope(ehr, ovid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the stored version")
        .body["commit_audit"]["committer"]
        .clone();
    assert_eq!(
        committer,
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Remote", "identifiers": [
            { "_type": "DV_IDENTIFIER", "issuer": "DE:Bundes\u{00E4}rztekammer Registry",
              "id": "hp-1", "type": "hp_identifier" },
            { "_type": "DV_IDENTIFIER", "issuer": "https://federation.example",
              "id": "hp-remote-1", "type": "oauth2" } ] })
    );
}

/// A country code that is no upper-case ISO 3166-1 alpha-2 code, and an
/// issuing authority with a `:`, are refused with `403`; a composition
/// whose facility does not name the token's provider is refused with `422`.
#[tokio::test]
async fn an_external_write_with_a_bad_field_or_without_its_provider_is_refused() {
    let (_db, svc, ehr) = service().await;
    let example = ckm_example(&svc, "vital-signs").await;
    let template = template_of(&example);
    let commit = |professional: ExternalProfessional, body: Value| {
        let svc = &svc;
        let identity = person(&template, professional);
        async move {
            with_committer(
                Some(identity),
                Box::pin(
                    svc.create_composition(ehr, WriteRequest::new(crate::typed_body::typed(&body))),
                ),
            )
            .await
        }
    };
    let listed = at_facility(example.clone(), &provider_facility());
    for country in ["de", "DEU"] {
        forbidden(
            commit(
                ExternalProfessional {
                    country_code: Some(country.to_owned()),
                    ..asserted()
                },
                listed.clone(),
            )
            .await,
        );
    }
    forbidden(
        commit(
            ExternalProfessional {
                issuing_authority: Some("DE:Registry".to_owned()),
                ..asserted()
            },
            listed,
        )
        .await,
    );
    let elsewhere = json!({ "_type": "PARTY_IDENTIFIED", "name": "Elsewhere",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": AUTHORITY, "id": "urn:oid:2.999.8" }] });
    unprocessable(commit(asserted(), at_facility(example, &elsewhere)).await);
}
