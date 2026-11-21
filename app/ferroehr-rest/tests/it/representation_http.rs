// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a bound caller reaches beyond its grants (#5086), driven through the
//! assembled router over a real `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! A representative reaches only the EHR whose `EHR_STATUS.subject` names the
//! subject it represents, and is refused with `403`
//! (`refused-representation-unrecorded`) when the party source records no live
//! representation of the subject its token asserts, or the CDR stores none
//! for the provider the deployment acts for (draft access control decision
//! procedure, step 0 Representation). A login whose `PERSON` carries the
//! representative marker alone, asserting no subject, reaches the subject a
//! stored representation names it for (#6147), and its own record as the
//! person themself (#6167). A `persons` token whose
//! only clinical grants are `patient/` is a professional confined to its
//! launch EHR, and its commit never records `PARTY_SELF`. No openEHR spec
//! governs resolving a login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::policy_register::Register;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState, Represented};
use ferrohealth_smart::profile::Representation;
use http::StatusCode;
use openehr_rm::prelude::{Composition, EhrStatus};
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{AUDIENCE, ISSUER, LOGIN, refused, service, token};

/// The namespace the party sources below name every party in.
const NAMESPACE: &str = "demographic";
/// The claim naming the subject a representative acts for.
const REPRESENTS: &str = "represents";
/// A subject no relationship of the representative names.
pub(crate) const STRANGER: &str = "8b2c3d4e-5f6a-4b7c-9d8e-0f1a2b3c4d5e";

/// The configuration trusting `ISSUER` for persons, whose representation
/// claim is `represents`, with its `patient/` grants honoured.
pub(crate) fn config() -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            representation: Some(Representation {
                claim: REPRESENTS.to_owned(),
                ground_claim: None,
            }),
            ..common::entry(ISSUER, AUDIENCE)
        }],
        party: vec![PartySection {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
            identifier_issuer: LOGIN.to_owned(),
            identifier_type: "guid".to_owned(),
            ..PartySection::default()
        }],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    };
    config.auth.smart.patient_issuers =
        vec![ferrohealth_secret::secret_url::SecretUrl::new(ISSUER)];
    config.party_binding.provider = Some(crate::portal_representatives_http::provider());
    config
}

/// A representative and a child, two `PERSON`s this CDR holds, and the EHR
/// whose `EHR_STATUS.subject` names the child.
pub(crate) struct Family {
    /// The representative's `PERSON` container.
    pub(crate) representative: String,
    /// The child's `PERSON` container.
    pub(crate) child: String,
    /// The child's EHR.
    pub(crate) ehr: String,
}

/// A fresh [`Family`] in `svc`.
pub(crate) async fn family(svc: &FerroEhrService) -> Family {
    let (representative, _) = crate::portal_representatives_http::child(svc).await;
    let (child, ehr) = crate::portal_representatives_http::child(svc).await;
    Family {
        representative: representative.to_string(),
        child: child.to_string(),
        ehr,
    }
}

impl Recorded {
    /// The source binding every login to the representative of `family`,
    /// reporting its live representation of the child.
    pub(crate) fn of(family: &Family) -> Self {
        Self {
            representative: family.representative.clone(),
            subjects: vec![family.child.clone()],
        }
    }
}

/// Stores the representation of the child of `family` by its
/// representative, for the provider the deployment acts for, as an
/// administrator.
pub(crate) async fn stored(svc: &FerroEhrService, family: &Family) {
    stored_for(svc, family, None).await;
}

/// [`stored`], for the provider `provider` when one is named.
async fn stored_for(svc: &FerroEhrService, family: &Family, provider: Option<(&str, &str)>) {
    let id = |id: &str| ferroehr::ids::VoId(id.parse().expect("a container"));
    let mut body = crate::portal_representatives_http::representation(
        id(&family.representative),
        id(&family.child),
        MOTHER,
        None,
    );
    if let Some((issuer, other)) = provider {
        body["details"]["items"][1]["value"] = json!({ "_type": "DV_IDENTIFIER",
            "issuer": issuer, "id": other, "type": "ORGANISATION" });
    }
    crate::portal_representatives_http::record(svc, &body).await;
}

/// The router over `svc`, looking every caller up in `source`.
pub(crate) async fn router(svc: Arc<FerroEhrService>, source: Arc<dyn BindingSource>) -> Router {
    let config = config();
    common::first_party(&svc, &config).await;
    let authenticator = Authenticator::new(
        &config,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
        common::standing(&svc),
    )
    .await
    .expect("the binding builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, svc, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// A party source that binds every login to one representative `PERSON`
/// whose live representation relationships name `subjects`: the local source
/// reports a relationship that has ended as none, so an empty list is the
/// ended one.
pub(crate) struct Recorded {
    pub(crate) representative: String,
    pub(crate) subjects: Vec<String>,
}

/// The openEHR `subject_relationship` code [`Recorded`] records for every
/// subject (`10|mother|`).
const MOTHER: &str = "10";

impl BindingSource for Recorded {}

impl PartySource for Recorded {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        let record = PartyRecord::new(
            PartyRef::new(NAMESPACE, "PERSON", self.representative.as_str()),
            PartyState::Active,
            Markers {
                professional: false,
                representative: true,
            },
        )
        .with_represents(
            self.subjects
                .iter()
                .map(|id| {
                    Represented::new(PartyRef::new(NAMESPACE, "PERSON", id.as_str()))
                        .with_relationship(MOTHER)
                })
                .collect(),
        );
        Box::pin(async move { Ok(Lookup::Parties(vec![record])) })
    }
}

/// Creates an EHR whose `EHR_STATUS.subject` names the `PERSON` `subject`,
/// or no subject, returning its id.
pub(crate) async fn ehr(svc: &FerroEhrService, subject: Option<&str>) -> String {
    let status = subject.map(|id| {
        let status = json!({
            "_type": "EHR_STATUS",
            "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
            "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
            "name": { "_type": "DV_TEXT", "value": "EHR Status" },
            "subject": { "_type": "PARTY_SELF", "external_ref": {
                "_type": "PARTY_REF", "namespace": NAMESPACE, "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": id } } },
            "is_queryable": true,
            "is_modifiable": true
        });
        openehr_its::json::from_canonical_value::<EhrStatus>(&status).expect("an EHR_STATUS")
    });
    let (ehr_id, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(status)))
        .await
        .expect("the EHR commits");
    ehr_id.to_string()
}

/// The status of `method` on `path` under the API base as `credential`.
async fn status(app: &Router, method: &str, path: &str, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req(method, format!("{BASE}{path}"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await
}

/// A representative's token asserting `subject`, with the full `user/` grants.
pub(crate) fn representative(subject: &str) -> String {
    token("rep-login", &json!({ REPRESENTS: subject }))
}

#[tokio::test]
async fn a_representative_reaches_only_the_represented_subjects_ehr() {
    let (_db, svc) = service().await;
    let family = family(&svc).await;
    let child = family.ehr.clone();
    stored(&svc, &family).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    let credential = representative(&family.child);

    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &credential).await;
    assert_eq!(
        code,
        StatusCode::OK,
        "the represented subject's EHR: {body}"
    );
    let (code, body) = status(&app, "GET", &format!("/ehr/{stranger}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another subject's EHR: {body}");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{}/ehr_status", child.to_ascii_uppercase()),
        &credential,
    )
    .await;
    assert_ne!(
        code,
        StatusCode::FORBIDDEN,
        "the EHR id is matched without regard to case: {body}"
    );
    // A query that names no EHR reaches every EHR, so it is refused.
    let (code, body) = status(
        &app,
        "GET",
        "/query/aql?q=SELECT%20e%2Fehr_id%2Fvalue%20FROM%20EHR%20e",
        &credential,
    )
    .await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "a query over every EHR: {body}"
    );
    // A template is no patient's data.
    let (code, body) = status(&app, "GET", "/definition/template/adl1.4", &credential).await;
    assert_eq!(code, StatusCode::OK, "the template list: {body}");
}

#[tokio::test]
async fn a_representation_the_source_does_not_record_is_refused() {
    let (_db, svc) = service().await;
    let family = family(&svc).await;
    stored(&svc, &family).await;
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    refused(
        &app,
        &representative(STRANGER),
        "refused-representation-unrecorded",
    )
    .await;
    // A relationship that has ended is no longer reported.
    let none = Recorded {
        subjects: Vec::new(),
        ..Recorded::of(&family)
    };
    let ended = router(Arc::clone(&svc), Arc::new(none)).await;
    refused(
        &ended,
        &representative(&family.child),
        "refused-representation-unrecorded",
    )
    .await;
}

/// A representation the party source reports is refused,
/// `refused-representation-unrecorded`, unless the CDR stores it as a live
/// representation of that subject recorded for the provider the deployment
/// acts for (draft access control decision procedure, step 0
/// Representation): one for another provider, or none, admits nothing.
#[tokio::test]
async fn a_representation_the_cdr_does_not_store_for_the_provider_is_refused() {
    let (_db, svc) = service().await;
    let family = family(&svc).await;
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    let credential = representative(&family.child);
    refused(&app, &credential, "refused-representation-unrecorded").await;
    stored_for(&svc, &family, Some(("urn:example:providers", "clinic"))).await;
    refused(&app, &credential, "refused-representation-unrecorded").await;
    stored(&svc, &family).await;
    let (code, body) = status(&app, "GET", &format!("/ehr/{}", family.ehr), &credential).await;
    assert_eq!(code, StatusCode::OK, "stored for the provider: {body}");
}

/// #5011, #6147: a person login, one asserting no represented subject, on a
/// row that carries only the representative marker never binds a
/// professional. FerroEHR admits it as a representative for a subject a live
/// representation it stores names it for: the subject of the EHR its launch
/// context or the request names, or the one an EHR lookup by subject names.
/// Any other request is refused with `403`
/// (`refused-representation-unrecorded`). Our own design.
#[tokio::test]
async fn a_person_login_on_a_representative_row_reaches_only_a_stored_representation() {
    let (_db, svc) = service().await;
    let family = family(&svc).await;
    let child = family.ehr.clone();
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    let credential = token("rep-login", &json!({}));
    let unrecorded = |what: &str, (code, body): (StatusCode, String)| {
        assert_eq!(code, StatusCode::FORBIDDEN, "{what}: {body}");
        assert!(
            body.contains("refused-representation-unrecorded"),
            "{what}: {body}"
        );
    };
    let lookup = |subject: &str| format!("/ehr?subject_id={subject}&subject_namespace={NAMESPACE}");

    unrecorded(
        "the child's EHR before any representation is stored",
        status(&app, "GET", &format!("/ehr/{child}"), &credential).await,
    );
    stored(&svc, &family).await;
    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "the represented child's EHR: {body}");
    let (code, body) = status(&app, "GET", &lookup(&family.child), &credential).await;
    assert_eq!(code, StatusCode::OK, "the lookup of the child: {body}");
    assert!(body.contains(&child), "the child's EHR is found: {body}");
    unrecorded(
        "another subject's EHR",
        status(&app, "GET", &format!("/ehr/{stranger}"), &credential).await,
    );
    unrecorded(
        "the lookup of another subject",
        status(&app, "GET", &lookup(STRANGER), &credential).await,
    );
    unrecorded(
        "a request that names no subject",
        status(&app, "GET", "/definition/template/adl1.4", &credential).await,
    );

    let launched = token(
        "rep-login",
        &json!({ "scope": "patient/composition-*.r", "ehrId": child }),
    );
    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &launched).await;
    assert_eq!(code, StatusCode::OK, "the launch EHR: {body}");
    let (code, body) = status(&app, "GET", &format!("/ehr/{stranger}"), &launched).await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "another EHR than the launch EHR: {body}"
    );
    let elsewhere = token(
        "rep-login",
        &json!({ "scope": "patient/composition-*.r", "ehrId": stranger }),
    );
    unrecorded(
        "a launch EHR of a subject no representation names",
        status(&app, "GET", &format!("/ehr/{stranger}"), &elsewhere).await,
    );
}

/// A [`Family`] whose representative is the subject of an EHR of their own,
/// with that EHR's id.
async fn with_own_record(svc: &FerroEhrService) -> (Family, String) {
    let (representative, own) = crate::portal_representatives_http::child(svc).await;
    let (child, ehr) = crate::portal_representatives_http::child(svc).await;
    let family = Family {
        representative: representative.to_string(),
        child: child.to_string(),
        ehr,
    };
    (family, own)
}

/// Turns the EHDS Art. 3(3) delay on for patient summaries and commits one
/// into `own` outside any request, so it is held from the person; its
/// versioned object id.
async fn held_summary(svc: &FerroEhrService, own: &str) -> String {
    svc.put_policy_register(
        Register::AccessDelay,
        0,
        r#"{"enabled": true, "categories": ["Patient-Summaries"], "max_delay": "72h"}"#,
    )
    .await
    .expect("turn the access delay on");
    svc.mirror_category_map()
        .await
        .expect("mirror the category map");
    svc.template_adl14_upload(&common::ips_opt_xml())
        .await
        .expect("the template uploads");
    let summary = openehr_its::json::from_canonical_value::<Composition>(
        &common::ips_canonical_composition(),
    )
    .expect("a COMPOSITION");
    svc.create_composition(
        ferroehr::ids::EhrId(own.parse().expect("an EHR id")),
        WriteRequest::new(summary),
    )
    .await
    .expect("the summary commits")
    .vo_id
    .to_string()
}

/// #6167: a login whose `PERSON` carries the representative marker and is
/// the subject of its own EHR reaches that EHR as the person themself, with
/// no stored representation of itself (EHDS Art. 3): it finds and reads it,
/// a new result stays held from it under the Art. 3(3) delay, it alters no
/// data a professional inserted (Art. 5), and its own entry names no party
/// (`PARTY_SELF`). Another subject still needs the stored representation.
/// Our own design.
#[tokio::test]
async fn a_representative_reaches_their_own_record_as_the_person_themself() {
    let (_db, svc) = service().await;
    let (family, own) = with_own_record(&svc).await;
    let child = family.ehr.clone();
    let held = held_summary(&svc, &own).await;
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    let credential = token("rep-login", &json!({}));
    let lookup = |subject: &str| format!("/ehr?subject_id={subject}&subject_namespace={NAMESPACE}");

    let (code, body) = status(&app, "GET", &format!("/ehr/{own}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "their own EHR: {body}");
    let (code, body) = status(&app, "GET", &lookup(&family.representative), &credential).await;
    assert_eq!(code, StatusCode::OK, "the lookup of their own EHR: {body}");
    assert!(body.contains(&own), "their own EHR is found: {body}");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{own}/composition/{held}"),
        &credential,
    )
    .await;
    assert_eq!(
        code,
        StatusCode::NOT_FOUND,
        "a held result reads as absent to the person: {body}"
    );
    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "the unrecorded child: {body}");
    assert!(body.contains("refused-representation-unrecorded"), "{body}");

    let (code, headers, body) = common::send(
        &app,
        common::req("GET", format!("{BASE}/ehr/{own}/ehr_status"))
            .auth(&credential)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "their own EHR's status: {body}");
    let current = common::etag_uid(&headers);
    let (code, _, body) = common::send(
        &app,
        common::req("PUT", format!("{BASE}/ehr/{own}/ehr_status"))
            .auth(&credential)
            .accept("application/json")
            .if_match(format!("\"{current}\""))
            .json(body.clone())
            .build(),
    )
    .await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "the person alters no data a professional inserted: {body}"
    );
    assert!(body.contains("EHDS Art. 5"), "{body}");
    let mut entry = common::ips_canonical_composition();
    if let Some(object) = entry.as_object_mut() {
        object.remove("uid");
        object.insert("composer".to_owned(), json!({ "_type": "PARTY_SELF" }));
    }
    let (code, headers, body) = common::send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{own}/composition"))
            .auth(&credential)
            .accept("application/json")
            .json(entry.to_string())
            .build(),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "their own entry: {body}");
    let uid = common::etag_uid(&headers);
    let vo = uid.split("::").next().expect("a versioned object id");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{own}/versioned_composition/{vo}/version/{uid}"),
        &credential,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "their own entry's version: {body}");
    let version: Value = serde_json::from_str(&body).expect("a version");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["_type"], "PARTY_SELF", "{committer}");
}

/// #6167: the representative who is also a patient reaches a represented
/// subject only on the stored representation, and a launch context holds it
/// to the launch EHR, its own or the represented subject's. Our own design.
#[tokio::test]
async fn a_representative_with_a_record_of_their_own_is_held_to_the_launch_ehr() {
    let (_db, svc) = service().await;
    let (family, own) = with_own_record(&svc).await;
    let child = family.ehr.clone();
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    let credential = token("rep-login", &json!({}));
    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "the unrecorded child: {body}");
    let (code, body) = status(&app, "GET", &format!("/ehr/{own}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "their own EHR: {body}");
    stored(&svc, &family).await;
    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "the represented child: {body}");
    let for_child = token(
        "rep-login",
        &json!({ "scope": "patient/composition-*.r", "ehrId": child }),
    );
    let (code, body) = status(&app, "GET", &format!("/ehr/{own}"), &for_child).await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "their own EHR under the child's launch: {body}"
    );
    let for_self = token(
        "rep-login",
        &json!({ "scope": "patient/composition-*.r", "ehrId": own }),
    );
    let (code, body) = status(&app, "GET", &format!("/ehr/{own}"), &for_self).await;
    assert_eq!(code, StatusCode::OK, "their own launch EHR: {body}");
    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &for_self).await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "the child's EHR under their own launch: {body}"
    );
}

#[tokio::test]
async fn a_persons_token_with_patient_grants_reaches_its_launch_ehr_and_never_commits_as_party_self()
 {
    let (_db, svc) = service().await;
    let key = "0c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f";
    crate::party_binding_http::professional(&svc, key, "532").await;
    let launched = ehr(&svc, None).await;
    let other = ehr(&svc, None).await;
    let source = Arc::new(svc.party_source(NAMESPACE));
    let app = router(Arc::clone(&svc), source).await;
    let credential = token(
        key,
        &json!({ "scope": "patient/composition-*.cru", "ehrId": launched }),
    );

    let (code, body) = status(&app, "GET", &format!("/ehr/{launched}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "the launch EHR: {body}");
    let (code, body) = status(&app, "GET", &format!("/ehr/{other}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR: {body}");

    let (code, headers, body) = common::send(
        &app,
        common::req("GET", format!("{BASE}/ehr/{launched}/ehr_status"))
            .auth(&credential)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "the launch EHR's status: {body}");
    let current = common::etag_uid(&headers);
    let (code, _, body) = common::send(
        &app,
        common::req("PUT", format!("{BASE}/ehr/{launched}/ehr_status"))
            .auth(&credential)
            .accept("application/json")
            .header("prefer", "return=minimal")
            .if_match(format!("\"{current}\""))
            .json(body.clone())
            .build(),
    )
    .await;
    assert!(code.is_success(), "the status update: {code} {body}");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{launched}/versioned_ehr_status/version"),
        &credential,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "the latest status version: {body}");
    let version: Value = serde_json::from_str(&body).expect("a version");
    let committer = &version["commit_audit"]["committer"];
    assert_ne!(committer["_type"], "PARTY_SELF", "{committer}");
    assert_eq!(committer["_type"], "PARTY_IDENTIFIED", "{committer}");
}

/// #5002: a representative commits under their own bound `PERSON` as a
/// `PARTY_RELATED` whose `relationship` is the `subject_relationship` code
/// the party source records for the subject, never as `PARTY_SELF` (RM
/// common `master04-generic_package.adoc`: `PARTY_RELATED` "is used whenever
/// the relationship of the party to the record subject is required"; EHDS
/// Art. 5). The committer's shape is our own design.
#[tokio::test]
async fn a_representatives_commit_names_its_person_and_the_recorded_relationship() {
    let (_db, svc) = service().await;
    svc.template_adl14_upload(&common::ips_opt_xml())
        .await
        .expect("the template uploads");
    let family = family(&svc).await;
    let child = family.ehr.clone();
    stored(&svc, &family).await;
    let app = router(Arc::clone(&svc), Arc::new(Recorded::of(&family))).await;
    let credential = representative(&family.child);
    let mut composition = common::ips_canonical_composition();
    if let Some(object) = composition.as_object_mut() {
        object.remove("uid");
        object.insert(
            "composer".to_owned(),
            json!({ "_type": "PARTY_RELATED", "name": "Bea",
                "relationship": { "_type": "DV_CODED_TEXT", "value": "mother",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": MOTHER } } }),
        );
    }
    let (code, headers, body) = common::send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{child}/composition"))
            .auth(&credential)
            .accept("application/json")
            .json(composition.to_string())
            .build(),
    )
    .await;
    assert_eq!(
        code,
        StatusCode::CREATED,
        "the representative's entry: {body}"
    );
    let uid = common::etag_uid(&headers);
    let vo = uid.split("::").next().expect("a versioned object id");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{child}/versioned_composition/{vo}/version/{uid}"),
        &credential,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "the stored version: {body}");
    let version: Value = serde_json::from_str(&body).expect("a version");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["_type"], "PARTY_RELATED", "{committer}");
    assert_eq!(
        (
            &committer["external_ref"]["type"],
            &committer["external_ref"]["id"]["value"]
        ),
        (&json!("PERSON"), &json!(family.representative)),
        "the representative's own PERSON: {committer}"
    );
    assert_eq!(
        (
            &committer["relationship"]["defining_code"]["terminology_id"]["value"],
            &committer["relationship"]["defining_code"]["code_string"]
        ),
        (&json!("openehr"), &json!(MOTHER)),
        "the recorded relationship: {committer}"
    );
}
