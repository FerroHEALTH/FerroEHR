// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every write path checks a version's committer the same way (#4520).
//!
//! `AUDIT_DETAILS.committer` is the "identity [...] of user who committed the
//! item" (RM `UML/classes/org.openehr.rm.common.audit_details.adoc`), and
//! ITS-REST `schemas/common/UpdateAudit.yaml` takes it from the client. A
//! committer naming a login the server can verify (one issued by the caller's
//! issuer or by this server) must name the authenticated caller, a
//! `PARTY_SELF` committer must come from the person, and a committer that
//! breaks its own invariants is refused on the direct writes as on the
//! CONTRIBUTION path. A name only is kept as the caller's account. No openEHR
//! spec governs who checks the committer: our own design.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::committer::{CommitterIdentity, with_committer};
use ferroehr::service::status::{ServiceStatus, StatusError};
use ferroehr::service::version_update::Committal;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, folder, lifecycle};

/// The token issuer of the professional's login.
const ISSUER: &str = "https://idp.example";

/// The professional's login at [`ISSUER`].
const LOGIN: &str = "dr.jansen";

/// The authenticated professional, through a bearer token of [`ISSUER`].
fn professional() -> CommitterIdentity {
    CommitterIdentity {
        subject: LOGIN.to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        client: None,
        person: false,
    }
}

/// A committer naming `login` at `issuer` as its identifier.
fn identified(login: &str, issuer: &str) -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED",
        "name": login,
        "identifiers": [{ "issuer": issuer, "id": login, "type": "oauth2" }]
    })
}

/// The committer of the authenticated professional.
fn matching() -> Value {
    identified(LOGIN, ISSUER)
}

/// A committer naming another login of the same issuer.
fn mismatching() -> Value {
    identified("dr.bakker", ISSUER)
}

/// An `UPDATE_VERSION` of `data` committed by `committer` with change type
/// `code`, superseding `preceding` when given.
fn version<T: serde::de::DeserializeOwned>(
    data: &Value,
    code: &str,
    preceding: Option<&str>,
    committer: &Value,
) -> UpdateVersion<T> {
    let mut envelope = json!({
        "lifecycle_state": lifecycle("532"),
        "data": data,
        "commit_audit": {
            "change_type": change_type(code, "change"),
            "committer": committer
        }
    });
    if let Some(preceding) = preceding {
        envelope["preceding_version_uid"] = json!({ "value": preceding });
    }
    openehr_its::json::from_canonical_value(&envelope).expect("an UPDATE_VERSION")
}

/// A minimal PERSON.
fn person() -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Jane" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Jane" }
                }]
            }
        }]
    })
}

async fn setup() -> (testkit::TestDb, FerroEhrService, EhrId) {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = Box::pin(svc.create_ehr_meta(None, None))
        .await
        .map(|(id, _)| id)
        .expect("create_ehr");
    (db, svc, ehr_id)
}

/// The status a write reports, `None` when it committed.
fn outcome<T>(result: Result<T, impl Into<StatusError>>) -> Option<ServiceStatus> {
    result.err().map(|e| e.into().status)
}

/// A composition create by `identity` naming `committer`.
async fn composition_create(
    svc: &FerroEhrService,
    ehr: EhrId,
    identity: Option<CommitterIdentity>,
    committer: &Value,
) -> Option<ServiceStatus> {
    let write = svc.create_composition(ehr, version(&composition("c"), "249", None, committer));
    outcome(with_committer(identity, Box::pin(write)).await)
}

/// The committer stored on the version `committed` names.
async fn stored_committer(
    svc: &FerroEhrService,
    ehr: EhrId,
    committed: &ferroehr::versioning::change::Committed,
) -> Value {
    let ovid = format!(
        "{}::{}::{}",
        committed.vo_id, committed.creating_system_id, committed.tree
    );
    svc.composition_version_envelope(ehr, ovid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the stored version")["commit_audit"]["committer"]
        .clone()
}

#[tokio::test]
async fn composition_create_checks_the_committer() {
    let (_db, svc, ehr) = setup().await;
    assert_eq!(
        composition_create(&svc, ehr, Some(professional()), &matching()).await,
        None,
        "the professional's own login commits"
    );
    assert_eq!(
        composition_create(&svc, ehr, Some(professional()), &mismatching()).await,
        Some(ServiceStatus::AuthFailure),
        "another login of the caller's issuer is refused"
    );
    assert_eq!(
        composition_create(
            &svc,
            ehr,
            Some(professional()),
            &identified("dr.bakker", "ferroehr")
        )
        .await,
        Some(ServiceStatus::AuthFailure),
        "another login this server holds is refused"
    );
    assert_eq!(
        composition_create(
            &svc,
            ehr,
            Some(professional()),
            &identified("BIG-123", "https://register.example")
        )
        .await,
        None,
        "an identifier of another authority cannot be compared and is kept"
    );
    assert_eq!(
        composition_create(&svc, ehr, None, &mismatching()).await,
        None,
        "with no authenticated caller there is nothing to compare with"
    );
}

#[tokio::test]
async fn a_basic_login_is_checked_against_this_server() {
    let (_db, svc, ehr) = setup().await;
    let basic = CommitterIdentity {
        subject: "alice".to_owned(),
        id_type: "basic",
        issuer: None,
        client: Some("alice".to_owned()),
        person: false,
    };
    assert_eq!(
        composition_create(
            &svc,
            ehr,
            Some(basic.clone()),
            &identified("alice", "ferroehr")
        )
        .await,
        None
    );
    assert_eq!(
        composition_create(&svc, ehr, Some(basic), &identified("bob", "ferroehr")).await,
        Some(ServiceStatus::AuthFailure)
    );
}

#[tokio::test]
async fn a_name_only_committer_is_kept_as_stated() {
    let (_db, svc, ehr) = setup().await;
    let named = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" });
    let write = svc.create_composition(ehr, version(&composition("c"), "249", None, &named));
    let committed = with_committer(Some(professional()), Box::pin(write))
        .await
        .expect("a name cannot be compared with a login");
    assert_eq!(
        stored_committer(&svc, ehr, &committed).await["name"],
        "Dr Jones"
    );
}

#[tokio::test]
async fn party_self_is_the_persons_committer_only() {
    let (_db, svc, ehr) = setup().await;
    let party_self = json!({ "_type": "PARTY_SELF" });
    assert_eq!(
        composition_create(&svc, ehr, Some(professional()), &party_self).await,
        Some(ServiceStatus::AuthFailure),
        "a professional may not state that the record subject committed"
    );

    // The person's committer is the server's PARTY_SELF, whatever the
    // request names.
    let person = CommitterIdentity {
        subject: "jane".to_owned(),
        person: true,
        ..professional()
    };
    let mut body = composition("mine");
    body["composer"] = json!({ "_type": "PARTY_SELF" });
    let write = svc.create_composition(ehr, version(&body, "249", None, &mismatching()));
    let committed = with_committer(Some(person), Box::pin(write))
        .await
        .expect("the person's own write commits");
    let stored = stored_committer(&svc, ehr, &committed).await;
    assert_eq!(stored["_type"], "PARTY_SELF");
    assert_eq!(stored["external_ref"]["id"]["value"], "jane");
}

#[tokio::test]
async fn a_direct_write_runs_the_committer_invariants() {
    let (_db, svc, ehr) = setup().await;
    let empty_name = json!({ "_type": "PARTY_IDENTIFIED", "name": "" });
    assert_eq!(
        composition_create(&svc, ehr, None, &empty_name).await,
        Some(ServiceStatus::ContentInvalid),
        "PARTY_IDENTIFIED.Name_valid, as on the CONTRIBUTION path"
    );
}

#[tokio::test]
async fn ehr_create_checks_the_committer() {
    let (_db, svc, _) = setup().await;
    let create = |committer: Value| {
        let svc = svc.clone();
        async move {
            let committal = Committal {
                audit: openehr_its::json::from_canonical_value::<UpdateAudit>(&json!({
                    "change_type": change_type("249", "creation"),
                    "committer": committer
                }))
                .expect("an UPDATE_AUDIT"),
                lifecycle_state: None,
            };
            let write = svc.create_ehr_meta(None, Some(&committal));
            outcome(with_committer(Some(professional()), Box::pin(write)).await)
        }
    };
    assert_eq!(create(matching()).await, None);
    assert_eq!(
        create(mismatching()).await,
        Some(ServiceStatus::AuthFailure)
    );
}

#[tokio::test]
async fn ehr_status_update_checks_the_committer() {
    let (_db, svc, ehr) = setup().await;
    let replace = |committer: Value| {
        let svc = svc.clone();
        async move {
            let mut body = svc
                .ehr_status_at_time_response(ehr, None)
                .await
                .map(|r| r.body)
                .expect("the current EHR_STATUS");
            let preceding = body["uid"]["value"].as_str().expect("uid").to_owned();
            body.as_object_mut().expect("an object").remove("uid");
            let write = svc
                .replace_ehr_status_meta(ehr, version(&body, "251", Some(&preceding), &committer));
            outcome(with_committer(Some(professional()), Box::pin(write)).await)
        }
    };
    assert_eq!(
        replace(mismatching()).await,
        Some(ServiceStatus::AuthFailure)
    );
    assert_eq!(replace(matching()).await, None);
}

#[tokio::test]
async fn directory_create_checks_the_committer() {
    let (_db, svc, ehr) = setup().await;
    let create = |committer: Value| {
        let write = svc.create_directory(ehr, version(&folder("root"), "249", None, &committer));
        async move { outcome(with_committer(Some(professional()), Box::pin(write)).await) }
    };
    assert_eq!(
        create(mismatching()).await,
        Some(ServiceStatus::AuthFailure)
    );
    assert_eq!(create(matching()).await, None);
}

#[tokio::test]
async fn demographic_create_checks_the_committer() {
    let (_db, svc, _) = setup().await;
    let create = |committer: Value| {
        let write = Box::pin(crate::fixtures::create_party(
            &svc,
            version(&person(), "249", None, &committer),
        ));
        async move { outcome(with_committer(Some(professional()), write).await) }
    };
    assert_eq!(create(matching()).await, None);
    assert_eq!(
        create(mismatching()).await,
        Some(ServiceStatus::AuthFailure)
    );
}

#[tokio::test]
async fn contribution_checks_every_committer() {
    let (_db, svc, ehr) = setup().await;
    let commit = |audit: Value, member: Value| {
        let body = json!({
            "audit": { "change_type": change_type("249", "creation"), "committer": audit },
            "versions": [{
                "lifecycle_state": lifecycle("532"),
                "commit_audit": { "change_type": change_type("249", "creation"), "committer": member },
                "data": composition("in a contribution")
            }]
        });
        let write = svc.create_ehr_contribution(ehr, body);
        async move { outcome(with_committer(Some(professional()), Box::pin(write)).await) }
    };
    assert_eq!(commit(matching(), matching()).await, None);
    assert_eq!(
        commit(mismatching(), matching()).await,
        Some(ServiceStatus::AuthFailure),
        "the CONTRIBUTION audit's committer"
    );
    assert_eq!(
        commit(matching(), mismatching()).await,
        Some(ServiceStatus::AuthFailure),
        "a member's commit_audit committer"
    );
}
