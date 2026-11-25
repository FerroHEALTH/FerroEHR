// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Justified access outside the care relationships, end to end over the
//! assembled router, the audit sender and the local Audit Record Repository
//! (#6587).
//!
//! The controller's `ACCESS_POLICY.justifications` admit a clinician outside
//! its care relationships for a declared purpose of use (HL7 v3 `ActReason`)
//! with a reason it states in `x-access-reason`; an emergency justification
//! also lifts what EHDS Art. 11(5) lets an emergency lift. The access record
//! keeps the purpose and the reason, and the person's subject-scoped ITI-81
//! retrieval shows who looked, why, and that it was outside the care
//! relationships: FHIR R4 `AuditEvent.purposeOfEvent` and
//! `agent.purposeOfUse` carry the reason as the `CodeableConcept` text (IHE
//! BALP; EHDS Art. 9 and Annex II 3.2). A patient's restriction still
//! refuses (EHDS Art. 8, GDPR Art. 18). The header, its encoding and its
//! bound are our own design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ehds_logging::balp::{EMERGENCY_DESCRIPTION, EMERGENCY_ENTITY};
use ferroehr::config::authz::AuthzConfig;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::{
    AuditHandle, ResolvedSubject, SubjectLookup, SubjectResolver, start,
};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::{BASE, EXT, FHIR, REASON_HEADER};

/// The subject every EHR in this suite resolves to.
const SUBJECT: &str = "patient-6587";
/// The header the default configuration reads the declared purpose from.
const PURPOSE: &str = "x-purpose-of-use";
/// The clinician's reason for a second opinion, percent-encoded, and as
/// recorded.
const SECOND_OPINION: (&str, &str) = (
    "Second%20opinion%20for%20Dr.%20M%C3%BCller",
    "Second opinion for Dr. Müller",
);
/// The clinician's reason for an emergency, percent-encoded, and as recorded.
const UNRESPONSIVE: (&str, &str) = ("Unresponsive%20on%20arrival", "Unresponsive on arrival");

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// The deployment: an audited app with `TREAT` and `ETREAT` agreed, Basic
/// users for an administrator, a clinician with no care relationship to the
/// patient and the patient portal holding `read_subject_audit`, and one EHR
/// with the IPS composition committed before the policy is.
struct Deployment {
    _db: testkit::TestDb,
    pool: PgPool,
    svc: Arc<FerroEhrService>,
    app: Router,
    ehr: String,
    vo: String,
}

impl Deployment {
    async fn new() -> Self {
        let (db, pool) = common::migrated_pool().await;
        let audit = AuditConfig {
            purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
            ..common::local_audit()
        };
        let resolver: SubjectResolver = Arc::new(|_ehr_id| {
            Box::pin(async {
                SubjectLookup::Found(ResolvedSubject {
                    id: SUBJECT.to_owned(),
                    namespace: Some(common::TEST_SUBJECTS.to_owned()),
                })
            })
        });
        let (sender, _handle): (_, AuditHandle) = start(audit, Some(resolver), pool.clone())
            .await
            .expect("the audit sender");
        let svc = Arc::new(
            common::binding_service(&pool)
                .with_audit(sender)
                .with_audit_store(AuditStore::new(pool.clone()))
                .with_binding_mapping(crate::party_binding_http::mapping())
                .with_grant_ceiling(GrantCeiling::new([Operation::ReadSubjectAudit])),
        );
        let mut config = common::api_config(true);
        common::with_basic_users(
            &mut config,
            &svc,
            &pool,
            &[
                ("root", &["ADMIN"]),
                ("clinician", &["USER"]),
                ("portal", &["PORTAL"]),
            ],
        )
        .await;
        common::grant_to(&svc, &pool, "portal", Operation::ReadSubjectAudit).await;
        let mut authz = AuthzConfig::default();
        authz.rbac.enabled = true;
        let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
        let app =
            common::build_full(config, Arc::clone(&svc), authz, Observability::default()).await;
        let (status, body) = common::send_body(
            &app,
            Request::post(format!("{BASE}/definition/template/adl1.4"))
                .header(header::AUTHORIZATION, basic("root"))
                .header(header::CONTENT_TYPE, "application/xml")
                .body(Body::from(common::ips_opt_xml()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
        let (status, body) = common::send_body(
            &app,
            Request::post(format!("{BASE}/ehr"))
                .header(header::AUTHORIZATION, basic("clinician"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("Prefer", "return=representation")
                .body(Body::from(common::subject_status(SUBJECT)))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
        let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
            .as_str()
            .expect("ehr_id")
            .to_owned();
        let (status, headers, body) = common::send(
            &app,
            Request::post(format!("{BASE}/ehr/{ehr}/composition"))
                .header(header::AUTHORIZATION, basic("clinician"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(common::ips_canonical_composition().to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "commit: {body}");
        let vo = headers
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .expect("ETag")
            .trim_start_matches("W/")
            .trim_matches('"')
            .split("::")
            .next()
            .expect("object id")
            .to_owned();
        Self {
            _db: db,
            pool,
            svc,
            app,
            ehr,
            vo,
        }
    }

    /// Commits the controller's policy: a care relationship required of
    /// every professional, a justification of `TREAT` for a consultation or
    /// a second opinion, and an emergency justification of `ETREAT`.
    async fn justified(self) -> Self {
        common::commit_justified_policy(
            &self.svc,
            &[
                common::justification(&["TREAT"], false, &["professional"]),
                common::justification(&["ETREAT"], true, &["professional"]),
            ],
            &json!({ "care_relationship_required": true }),
        )
        .await;
        self
    }

    /// Reads the composition as the clinician, declaring `purpose` with the
    /// percent-encoded `reason` when given.
    async fn read(&self, purpose: Option<&str>, reason: Option<&str>) -> (StatusCode, String) {
        let mut req = Request::get(format!("{BASE}/ehr/{}/composition/{}", self.ehr, self.vo))
            .header(header::AUTHORIZATION, basic("clinician"))
            .header(header::ACCEPT, "application/json");
        if let Some(purpose) = purpose {
            req = req.header(PURPOSE, purpose);
        }
        if let Some(reason) = reason {
            req = req.header(REASON_HEADER, reason);
        }
        common::send_body(&self.app, req.body(Body::empty()).unwrap()).await
    }

    /// The `(purpose, outcome, justification)` of every stored
    /// `composition_get` record, polled until `expected` have landed.
    async fn reads(&self, expected: usize) -> Vec<(Option<String>, i16, Option<Value>)> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let rows: Vec<(Option<String>, i16, Option<Value>)> = sqlx::query_as(
                "SELECT purpose, outcome, justification FROM audit.access_trail \
                 WHERE operation = 'composition_get' ORDER BY chain_seq",
            )
            .fetch_all(&self.pool)
            .await
            .expect("read the trail");
            if rows.len() >= expected {
                return rows;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {} of {expected} composition_get records landed",
                rows.len()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The `composition_get` `AuditEvent`s of the patient portal's
    /// subject-scoped ITI-81 retrieval.
    async fn subject_log(&self) -> Vec<Value> {
        let (status, body) = common::send_body(
            &self.app,
            Request::get(format!(
                "{FHIR}/AuditEvent?date=ge2026-01-01&patient={SUBJECT}"
            ))
            .header(header::AUTHORIZATION, basic("portal"))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "subject-scoped retrieval: {body}");
        let bundle: Value = serde_json::from_str(&body).expect("bundle json");
        bundle["entry"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["resource"].clone())
            .filter(|resource| {
                resource["subtype"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|c| c["code"] == "composition_get"))
            })
            .collect()
    }
}

/// The refusal code of a problem body.
fn refusal(body: &str) -> Value {
    serde_json::from_str::<Value>(body).expect("a problem body")["refusal"].clone()
}

/// The purpose and its text of `resource`, as `purposeOfEvent` carries them.
fn purpose_of_event(resource: &Value) -> (Value, Value) {
    let purpose = &resource["purposeOfEvent"][0];
    (
        purpose["coding"][0]["code"].clone(),
        purpose["text"].clone(),
    )
}

/// Outside its care relationships, the clinician is refused; with the
/// policy's `TREAT` justification and a reason it is admitted, and the
/// access record keeps the purpose and the reason as the clinician stated it.
#[tokio::test]
async fn a_justified_access_is_admitted_with_its_reason_and_refused_without() {
    let deployment = Box::pin(Deployment::new()).await.justified().await;

    let (status, body) = deployment.read(None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body), json!("refused-no-care-relationship"));
    let (status, body) = deployment.read(Some("TREAT"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no reason: {body}");
    assert_eq!(refusal(&body), json!("refused-reason-missing"));
    let (status, body) = deployment.read(Some("TREAT"), Some(SECOND_OPINION.0)).await;
    assert_eq!(status, StatusCode::OK, "justified: {body}");

    assert_eq!(
        deployment.reads(3).await,
        vec![
            (None, 4, None),
            (Some("TREAT".to_owned()), 4, None),
            (
                Some("TREAT".to_owned()),
                0,
                Some(json!({ "kind": "justification", "reason": SECOND_OPINION.1 }))
            ),
        ]
    );
}

/// The patient's subject-scoped ITI-81 retrieval shows the justified access:
/// who looked, why in the clinician's own words, and that it rested on a
/// justification rather than a care relationship, with no emergency mark.
#[tokio::test]
async fn the_subjects_own_log_shows_the_reason() {
    let deployment = Box::pin(Deployment::new()).await.justified().await;
    let (status, body) = deployment.read(Some("TREAT"), Some(SECOND_OPINION.0)).await;
    assert_eq!(status, StatusCode::OK, "justified: {body}");
    deployment.reads(1).await;

    let log = deployment.subject_log().await;
    let justified = log
        .iter()
        .find(|resource| resource["outcome"] == "0")
        .unwrap_or_else(|| panic!("the justified read is in the subject's log: {log:?}"));
    assert_eq!(
        purpose_of_event(justified),
        (json!("TREAT"), json!(SECOND_OPINION.1)),
        "{justified}"
    );
    let requestor = justified["agent"]
        .as_array()
        .expect("agents")
        .iter()
        .find(|agent| agent["requestor"] == true)
        .expect("the requesting agent");
    assert_eq!(
        requestor["purposeOfUse"][0]["text"],
        json!(SECOND_OPINION.1)
    );
    assert!(
        justified["entity"]
            .as_array()
            .expect("entities")
            .iter()
            .all(|entity| entity["name"] != EMERGENCY_ENTITY),
        "a justification is no emergency: {justified}"
    );
}

/// An emergency is a justification that lifts the patient's restriction
/// where its override holds: refused without a reason, before any directive
/// is read; admitted with one, marked as an emergency (EHDS Art. 11(5)) and
/// showing the reason in the patient's log.
#[tokio::test]
async fn an_emergency_lifts_an_overridable_restriction_and_is_marked() {
    let deployment = Box::pin(Deployment::new()).await.justified().await;
    let ehr = deployment.ehr.parse().expect("an EHR id");
    let restriction = common::access_directive("restriction", &[&deployment.vo]);
    common::commit_access_settings(
        &deployment.svc,
        ehr,
        common::directive_settings(&[restriction]),
    )
    .await;

    let (status, body) = deployment.read(Some("TREAT"), Some(SECOND_OPINION.0)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a justification lifts no restriction: {body}"
    );
    let (status, body) = deployment.read(Some("ETREAT"), None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no reason, the restriction holds: {body}"
    );
    let (status, body) = deployment.read(Some("ETREAT"), Some(UNRESPONSIVE.0)).await;
    assert_eq!(status, StatusCode::OK, "the emergency: {body}");

    let reads = deployment.reads(3).await;
    assert_eq!(
        reads[2],
        (
            Some("ETREAT".to_owned()),
            0,
            Some(json!({ "kind": "emergency", "reason": UNRESPONSIVE.1 }))
        )
    );
    let log = deployment.subject_log().await;
    let emergency = log
        .iter()
        .find(|resource| resource["outcome"] == "0")
        .unwrap_or_else(|| panic!("the emergency read is in the subject's log: {log:?}"));
    assert_eq!(
        purpose_of_event(emergency),
        (json!("ETREAT"), json!(UNRESPONSIVE.1))
    );
    let mark = emergency["entity"]
        .as_array()
        .expect("entities")
        .iter()
        .find(|entity| entity["name"] == EMERGENCY_ENTITY)
        .cloned()
        .unwrap_or_else(|| panic!("the emergency mark: {emergency}"));
    assert_eq!(mark["description"], EMERGENCY_DESCRIPTION);
}

/// Without a restriction, an emergency without a reason is refused before
/// any directive is read, naming the missing reason.
#[tokio::test]
async fn an_emergency_without_a_reason_is_refused() {
    let deployment = Box::pin(Deployment::new()).await.justified().await;
    let (status, body) = deployment.read(Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body), json!("refused-reason-missing"));
}

/// A GDPR Art. 18 restriction still refuses a justified access and an
/// emergency alike: Art. 18(2) admits no processing for the clinician's
/// reasons, and the refused attempts are logged with their purposes.
#[tokio::test]
async fn a_gdpr_restriction_refuses_a_justified_access() {
    let deployment = Box::pin(Deployment::new()).await.justified().await;
    let (status, body) = common::send_body(
        &deployment.app,
        Request::post(format!("{EXT}/admin/restriction"))
            .header(header::AUTHORIZATION, basic("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(
                r#"{{"ehr_id":"{}","vo_id":"{}","ground":"gdpr-18-1-a"}}"#,
                deployment.ehr, deployment.vo
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "restrict: {body}");

    let (status, body) = deployment.read(Some("TREAT"), Some(SECOND_OPINION.0)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "justified: {body}");
    let (status, body) = deployment.read(Some("ETREAT"), Some(UNRESPONSIVE.0)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "emergency: {body}");
    let reads = deployment.reads(2).await;
    assert!(
        reads.iter().all(|(_, outcome, _)| *outcome == 4),
        "{reads:?}"
    );
}

/// A reason over the bound, or one that is no percent-encoded UTF-8, is a
/// `400` before the access is judged, so no reason is recorded in part.
#[tokio::test]
async fn a_reason_over_the_bound_or_malformed_is_refused() {
    let deployment = Box::pin(Deployment::new()).await.justified().await;
    let over = "a".repeat(ferroehr::access_eval::justification::REASON_MAX_CHARS + 1);
    for (name, reason) in [("over the bound", over.as_str()), ("malformed", "%C3%28")] {
        let (status, body) = deployment.read(Some("TREAT"), Some(reason)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert!(body.contains(REASON_HEADER), "{name}: {body}");
    }
    let at = "a".repeat(ferroehr::access_eval::justification::REASON_MAX_CHARS);
    let (status, body) = deployment.read(Some("TREAT"), Some(&at)).await;
    assert_eq!(status, StatusCode::OK, "at the bound: {body}");
}

/// A deployment that writes no justification behaves as one without the
/// feature: the purpose and a reason admit nothing outside the care
/// relationships, and no reason is recorded.
#[tokio::test]
async fn without_justifications_a_reason_admits_nothing() {
    let deployment = Box::pin(Deployment::new()).await;
    common::commit_justified_policy(
        &deployment.svc,
        &[],
        &json!({ "care_relationship_required": true }),
    )
    .await;
    let (status, body) = deployment.read(Some("ETREAT"), Some(UNRESPONSIVE.0)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body), json!("refused-no-care-relationship"));
    assert_eq!(
        deployment.reads(1).await,
        vec![(Some("ETREAT".to_owned()), 4, None)]
    );
}
