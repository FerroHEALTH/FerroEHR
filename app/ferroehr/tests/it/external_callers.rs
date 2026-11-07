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
use ferroehr::versioning::committer::{CommitterIdentity, ExternalCaller, with_committer};

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
        system_client: None,
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
    let person = external(ExternalCaller::Person {
        writes: vec![template],
    });
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
