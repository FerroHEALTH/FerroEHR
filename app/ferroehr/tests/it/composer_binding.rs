// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The composer of a bound professional's COMPOSITION (#6130), over a real
//! PostgreSQL 19.
//!
//! RM `COMPOSITION.composer` is "the person primarily responsible for the
//! content of the Composition" (`org.openehr.rm.composition.composition.adoc`)
//! and `AUDIT_DETAILS.committer` the "user who committed the item"
//! (`org.openehr.rm.common.audit_details.adoc`). The server fills the
//! placeholder composer with the professional's `PARTY_IDENTIFIED`, keeps a
//! composer stating nothing it does not hold, admits another `PERSON` of the
//! same provider under a `compose_for` grant, and refuses every other
//! composer: our own design, as no openEHR spec governs it.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::committer_binding::{ISSUER, KEY, PROVIDER, bound, placeholder, professional, service};
use crate::fixtures::{composition, object};
use crate::policy_fixtures::{capability, create};
use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::grants::capabilities::{Actor, Capabilities, with_capabilities};
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The key of the colleague's `PERSON`.
const COLLEAGUE_KEY: &str = "3d4e5f6a-7b8c-4d9e-8f0a-1b2c3d4e5f6a";

/// `composition("…")` composed by `composer`.
fn composed_by(name: &str, composer: &Value) -> Value {
    let mut body = composition(name);
    body["composer"] = composer.clone();
    body
}

/// A `PARTY_REF` to the `PERSON` `vo`.
fn person_ref(vo: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": vo.to_string() } })
}

/// The provider identifier of the deployment the tests bind under.
fn provider_identifier() -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": PROVIDER.0, "id": PROVIDER.1,
            "type": "healthcare-provider" })
}

/// The composer naming the colleague `vo` of the provider.
fn colleague(vo: VoId) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "external_ref": person_ref(vo), "name": "Dr Colleague",
            "identifiers": [provider_identifier()] })
}

/// Commits `body` into `ehr` as `identity` under `capabilities`: a creation,
/// or with `preceding` a modify of that version; the stored version.
async fn commit(
    svc: &FerroEhrService,
    ehr: EhrId,
    identity: CommitterIdentity,
    capabilities: Arc<Capabilities>,
    body: &Value,
    preceding: Option<&str>,
) -> Result<Value, ServiceError> {
    let mut write: WriteRequest<openehr_rm::prelude::Composition> =
        WriteRequest::new(crate::typed_body::typed(body));
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    let written = with_capabilities(
        capabilities,
        with_committer(Some(identity), async {
            match preceding {
                None => Box::pin(svc.create_composition(ehr, write)).await,
                Some(uid) => {
                    let vo = object(uid.split("::").next().expect("a versioned object id"));
                    Box::pin(svc.update_composition(ehr, vo, write)).await
                }
            }
        }),
    )
    .await
    .map_err(ServiceError::flattened)?;
    let uid = format!(
        "{}::{}::{}",
        written.vo_id, written.creating_system_id, written.tree
    );
    let mut stored = svc
        .composition_version_envelope(ehr, uid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the stored version")
        .body;
    stored["uid_written"] = json!(uid);
    Ok(stored)
}

/// The refusal naming `COMPOSITION.composer`.
fn refused(result: Result<Value, ServiceError>) {
    match result {
        Err(ServiceError::Forbidden(refusal)) => {
            let message = format!("{refusal:?}");
            assert!(message.contains("COMPOSITION.composer"), "{message}");
        }
        other => panic!("expected 403 naming COMPOSITION.composer, got {other:?}"),
    }
}

/// The capabilities of the party `vo` after the grants are read again.
async fn party_grants(svc: &FerroEhrService, vo: VoId) -> Arc<Capabilities> {
    svc.grants().refresh().await.expect("the grants read");
    Arc::new(
        svc.grants()
            .capabilities(Actor::Party(vo.0), jiff::Timestamp::now()),
    )
}

/// Grants the `PERSON` `performer` `compose_for` through a `ROLE` of the
/// deployment's own archetype.
async fn grant_compose_for(svc: &FerroEhrService, performer: VoId) {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.scribe.v1";
    let role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype }, "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Scribe" },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "details" },
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": "scribe" } }] } }],
        "performer": person_ref(performer),
        "capabilities": [capability("compose_for", &[])]
    });
    create(svc, PartyKind::Role, &role)
        .await
        .expect("the compose_for ROLE commits");
}

#[tokio::test]
async fn the_placeholder_composer_is_stored_as_the_professional() {
    let (_db, svc, ehr) = service().await;
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let identity = bound(&svc, "sub-a", KEY).await;
    let none = Arc::new(Capabilities::default());
    let stored = commit(
        &svc,
        ehr,
        identity.clone(),
        Arc::clone(&none),
        &composed_by("by placeholder", &placeholder(ISSUER, "sub-a")),
        None,
    )
    .await
    .expect("the placeholder is filled");
    let committer = &stored["commit_audit"]["committer"];
    assert_eq!(&stored["data"]["composer"], committer, "{stored}");
    assert_eq!(committer["external_ref"], person_ref(vo));
    assert_eq!(committer["identifiers"][1], provider_identifier());
    // A composer that states nothing the server does not hold is kept as sent.
    let partial = json!({ "_type": "PARTY_IDENTIFIED", "external_ref": person_ref(vo),
                          "name": "Dr Jane Roe", "identifiers": [provider_identifier()] });
    let kept = commit(
        &svc,
        ehr,
        identity.clone(),
        Arc::clone(&none),
        &composed_by("partial", &partial),
        None,
    )
    .await
    .expect("a composer stating nothing more");
    assert_eq!(kept["data"]["composer"], partial);
    // The modify of the professional's own entry takes the placeholder too.
    let uid = stored["uid_written"].as_str().expect("a uid").to_owned();
    let modified = commit(
        &svc,
        ehr,
        identity,
        none,
        &composed_by("amended", &placeholder(ISSUER, "sub-a")),
        Some(&uid),
    )
    .await
    .expect("the modify is filled");
    assert_eq!(modified["data"]["composer"], *committer);
}

#[tokio::test]
async fn a_composer_naming_anyone_else_is_refused_on_create_and_modify() {
    let (_db, svc, ehr) = service().await;
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let other = professional(&svc, "Dr Colleague", COLLEAGUE_KEY).await;
    let identity = bound(&svc, "sub-a", KEY).await;
    let none = Arc::new(Capabilities::default());
    for composer in [
        colleague(other),
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Fake" }),
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe",
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "urn:x", "id": "9" }] }),
        json!({ "_type": "PARTY_IDENTIFIED", "external_ref": person_ref(vo), "name": "Dr Other" }),
        placeholder(ISSUER, "sub-other"),
        json!({ "_type": "PARTY_SELF" }),
    ] {
        refused(
            commit(
                &svc,
                ehr,
                identity.clone(),
                Arc::clone(&none),
                &composed_by("refused", &composer),
                None,
            )
            .await,
        );
    }
    let own = commit(
        &svc,
        ehr,
        identity.clone(),
        Arc::clone(&none),
        &composed_by("own", &placeholder(ISSUER, "sub-a")),
        None,
    )
    .await
    .expect("the professional's own entry");
    let uid = own["uid_written"].as_str().expect("a uid").to_owned();
    refused(
        commit(
            &svc,
            ehr,
            identity,
            none,
            &composed_by("as another", &colleague(other)),
            Some(&uid),
        )
        .await,
    );
}

/// A modify that keeps the composer of the version it replaces claims
/// nothing new and needs no grant; the committer records who corrected it.
/// Changing the composer to another `PERSON` still needs the grant.
#[tokio::test]
async fn a_correction_keeping_the_author_needs_no_grant() {
    let (_db, svc, ehr) = service().await;
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let other = professional(&svc, "Dr Colleague", COLLEAGUE_KEY).await;
    let none = Arc::new(Capabilities::default());
    // The colleague's entry, composed by the colleague through the placeholder.
    let author = bound(&svc, "sub-c", COLLEAGUE_KEY).await;
    let written = commit(
        &svc,
        ehr,
        author,
        Arc::clone(&none),
        &composed_by("by the colleague", &placeholder(ISSUER, "sub-c")),
        None,
    )
    .await
    .expect("the colleague's own entry");
    let composer = written["data"]["composer"].clone();
    assert_eq!(composer["external_ref"], person_ref(other));
    let uid = written["uid_written"].as_str().expect("a uid").to_owned();
    let corrector = bound(&svc, "sub-a", KEY).await;
    let corrected = commit(
        &svc,
        ehr,
        corrector.clone(),
        Arc::clone(&none),
        &composed_by("corrected", &composer),
        Some(&uid),
    )
    .await
    .expect("keeping the author needs no grant");
    assert_eq!(corrected["data"]["composer"], composer);
    assert_eq!(
        corrected["commit_audit"]["committer"]["external_ref"],
        person_ref(vo),
        "the committer is who corrected it"
    );
    // Naming yet another PERSON on the next modify is refused without the grant.
    let next = corrected["uid_written"].as_str().expect("a uid").to_owned();
    let third = professional(&svc, "Dr Third", "4e5f6a7b-8c9d-4e0f-9a1b-2c3d4e5f6a7b").await;
    refused(
        commit(
            &svc,
            ehr,
            corrector,
            none,
            &composed_by("reassigned", &colleague(third)),
            Some(&next),
        )
        .await,
    );
}

#[tokio::test]
async fn composing_for_a_colleague_takes_the_compose_for_grant() {
    let (_db, svc, ehr) = service().await;
    let svc = svc.with_grant_ceiling(GrantCeiling::new(Operation::ALL));
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let other = professional(&svc, "Dr Colleague", COLLEAGUE_KEY).await;
    let identity = bound(&svc, "sub-a", KEY).await;
    let body = composed_by("dictated", &colleague(other));
    refused(
        commit(
            &svc,
            ehr,
            identity.clone(),
            party_grants(&svc, vo).await,
            &body,
            None,
        )
        .await,
    );
    grant_compose_for(&svc, vo).await;
    let granted = party_grants(&svc, vo).await;
    assert!(granted.holds(Operation::ComposeFor));
    let stored = commit(
        &svc,
        ehr,
        identity.clone(),
        Arc::clone(&granted),
        &body,
        None,
    )
    .await
    .expect("the grant admits the colleague");
    assert_eq!(stored["data"]["composer"], colleague(other));
    assert_eq!(
        stored["commit_audit"]["committer"]["external_ref"],
        person_ref(vo),
        "the committer is the caller"
    );
    // A colleague of another provider, or named with no provider, is refused
    // under the grant too.
    for composer in [
        json!({ "_type": "PARTY_IDENTIFIED", "external_ref": person_ref(other),
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": PROVIDER.0, "id": "99",
                                  "type": "healthcare-provider" }] }),
        json!({ "_type": "PARTY_IDENTIFIED", "external_ref": person_ref(other) }),
    ] {
        refused(
            commit(
                &svc,
                ehr,
                identity.clone(),
                Arc::clone(&granted),
                &composed_by("elsewhere", &composer),
                None,
            )
            .await,
        );
    }
    // The grant admits it on a modify, with the caller still the committer.
    let uid = stored["uid_written"].as_str().expect("a uid").to_owned();
    let modified = commit(
        &svc,
        ehr,
        identity,
        granted,
        &composed_by("corrected", &colleague(other)),
        Some(&uid),
    )
    .await
    .expect("the grant admits the modify");
    assert_eq!(
        modified["commit_audit"]["committer"]["external_ref"],
        person_ref(vo)
    );
}

#[tokio::test]
async fn a_basic_or_unbound_caller_keeps_its_composer() {
    let (_db, svc, ehr) = service().await;
    professional(&svc, "Dr Jane Roe", KEY).await;
    let anyone = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Anyone" });
    let unbound = CommitterIdentity {
        subject: "sub-u".to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        local_issuer: ferroehr::service::DEFAULT_SYSTEM_ID.into(),
        client: Some("app".to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: None,
    };
    let basic = CommitterIdentity {
        subject: "clinician".to_owned(),
        id_type: "basic",
        issuer: None,
        client: None,
        ..unbound.clone()
    };
    let mut basic_bound = bound(&svc, "clinician", KEY).await;
    basic_bound.id_type = "basic";
    basic_bound.issuer = None;
    for identity in [unbound, basic, basic_bound] {
        let stored = commit(
            &svc,
            ehr,
            identity,
            Arc::new(Capabilities::default()),
            &composed_by("as sent", &anyone),
            None,
        )
        .await
        .expect("the composer is kept");
        assert_eq!(stored["data"]["composer"], anyone);
    }
}
