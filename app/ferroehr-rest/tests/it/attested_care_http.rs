// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `attested` care path at the wire (#6078): an external person
//! admitted under a federation agreement holds a care relationship to a
//! subject when its verified credential attests a current treatment context
//! for that subject and the controller's `ACCESS_POLICY.attested_care` names
//! the agreement (draft access control decision procedure, Holding a Care
//! Relationship). The relationship is never stored, and the access-log entry
//! names the assertion and the agreement (draft decision procedure,
//! Refusals). The issuer and the claims are the deployment's data; the CDR
//! names no national credential. Driven through the assembled router over a
//! real `FerroEhrService` on a real `PostgreSQL` 18. Our own design over the
//! draft access control package.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::config::Retention;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::federation::{FederationSection, Field, Mapping, Source};
use ferrohealth_smart::kind::IssuerKind;
use http::StatusCode;
use openehr_rm::draft::access_control::terminology::{
    TERMINOLOGY_ID_ACCESS, TERMINOLOGY_ID_CONSENT_PROVISION_TYPE,
};
use openehr_rm::prelude::EhrStatus;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::agreement_parties::{Accessor, Terms};
use crate::common::{self, BASE};

/// The `federation` issuer.
const FEDERATION: &str = "https://attesting-federation.example";
/// This node's audience at that issuer.
const AUDIENCE: &str = "ferroehr-attested";
/// The read the agreement admits.
const READ: &str = "patient/composition-*.r";
/// The namespace the synthetic patient identifiers are assigned in.
const PATIENTS: &str = "urn:oid:2.999.1";
/// The patient the tests attest a treatment context for.
const PATIENT: &str = "SYNTHETIC-ATTESTED-1";
/// Another patient.
const OTHER_PATIENT: &str = "SYNTHETIC-ATTESTED-2";
/// The id of the synthetic treatment context assertion.
const ASSERTION: &str = "urn:example:assertion:trc-1";
/// The issuer of that assertion.
const ASSERTION_ISSUER: &str = "urn:example:assertion-service";

/// The issuer's `[[auth.federation]]` table: IHE IUA claims, and the
/// treatment context under `treatment`, as a deployment maps them.
fn federation_table() -> FederationSection {
    let claim = |path: &str| vec![Source::Claim(path.to_owned())];
    FederationSection {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(FEDERATION),
        contact_point: Some(false),
        mapping: Some(Mapping::Iua),
        fields: BTreeMap::from([
            (
                Field::ProfessionalCountryCode,
                claim("extensions.ihe_iua.country"),
            ),
            (
                Field::ProfessionalIssuingAuthorityName,
                claim("extensions.ihe_iua.issuing_authority"),
            ),
            (
                Field::ProviderIssuingAuthorityName,
                vec![Source::Constant("Registry".to_owned())],
            ),
            (Field::PatientId, claim("treatment.patient")),
            (Field::TrcId, claim("treatment.id")),
            (Field::TrcNotBefore, claim("treatment.not_before")),
            (Field::TrcNotOnOrAfter, claim("treatment.not_on_or_after")),
            (Field::AssertionIssuer, claim("treatment.issuer")),
        ]),
        countries: vec!["DE".to_owned()],
        issuing_authorities: vec!["Registry".to_owned()],
        provider_identifier_prefixes: vec!["urn:oid:2.999".to_owned()],
        ..FederationSection::default()
    }
}

/// The person agreement of the partner of [`FEDERATION`].
fn agreement() -> Terms {
    Terms::new(Accessor::Person, FEDERATION)
        .text("agreement_name", "Cross-border exchange")
        .text("agreement_version", "3")
        .text("gdpr_art_6_1", "e")
        .text("gdpr_art_9_2", "h")
        .text("read_scope", READ)
}

/// The router trusting [`FEDERATION`], whose partner holds [`agreement`],
/// over the local EHR index, an EHR with no settings `restricted` to the
/// callers a care relationship, a permit or a gate-keeper names when asked;
/// returns it with its service and pool.
async fn serve(restricted: bool) -> (testkit::TestDb, PgPool, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) = start(common::local_audit(), None, pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_ehr_index(EhrIndexProvider::Local)
            .with_audit(sender)
            .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool.clone())),
    );
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(FEDERATION),
            kind: Some(IssuerKind::Federation),
            audience: vec![AUDIENCE.to_owned()],
            jwks_file: common::jwks_file().display().to_string(),
            assurance: Some(AssuranceSection {
                levels: BTreeMap::from([("high".to_owned(), AssuranceLevel::High)]),
                ..AssuranceSection::default()
            }),
            ..IssuerEntry::default()
        }],
        federation: vec![federation_table()],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    svc.grants()
        .set_ceiling(crate::agreement_parties::ceiling());
    svc.grants()
        .set_federation(ferroehr::config::federation::trust(
            &auth.shared,
            Retention::Years(10),
        ));
    crate::agreement_parties::commit(&svc, FEDERATION, &["remote-portal"], &[agreement()])
        .await
        .expect("the agreement commits");
    let mut config = AppConfig {
        auth,
        ..common::api_config(true)
    };
    config.auth.smart.patient_issuers =
        vec![ferrohealth_secret::secret_url::SecretUrl::new(FEDERATION)];
    config.public_root = Some(
        ferroehr::config::server::PublicRoot::new("https://ehr.example", "")
            .expect("a public root"),
    );
    let authz = restricted.then(|| {
        let mut authz = ferroehr::config::authz::AuthzConfig::default();
        authz.rbac.enabled = false;
        authz.rbac.ehr_access_default = ferroehr::config::authz::EhrAccessDefault::Restricted;
        ferroehr_rest::extensions::access::authz::AuthzHandle::build(&authz, &config.api.base_path)
            .map(Arc::new)
    });
    let app = common::build_full(
        config,
        Arc::clone(&svc),
        authz.flatten(),
        Observability::default(),
    )
    .await;
    (db, pool, svc, app)
}

/// The container of the agreement `ROLE` the partner's grant is held in.
async fn agreement_role(pool: &PgPool) -> String {
    let role: uuid::Uuid = sqlx::query_scalar(
        "SELECT role_id FROM party.binding_capability WHERE operation = 'federated_person_access'",
    )
    .fetch_one(pool)
    .await
    .expect("the agreement ROLE");
    role.to_string()
}

/// Creates the EHR whose `EHR_STATUS` names the patient `id` under
/// [`PATIENTS`]; returns its id.
async fn patient_ehr(svc: &FerroEhrService, id: &str) -> String {
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": {
            "_type": "PARTY_REF", "namespace": PATIENTS, "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } } },
        "is_queryable": true,
        "is_modifiable": true
    });
    let status =
        openehr_its::json::from_canonical_value::<EhrStatus>(&status).expect("an EHR_STATUS");
    let (ehr_id, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(status))))
        .await
        .expect("the EHR commits");
    ehr_id.to_string()
}

/// A coded term of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// One controller rule of `effect` with `extra` members.
fn rule(effect: &str, extra: &Value) -> Value {
    let mut rule = json!({ "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Care rule" },
        "archetype_node_id": "id15",
        "kind": coded(TERMINOLOGY_ID_ACCESS, "controller_rule"),
        "effect": coded(TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, effect),
        "emergency_override": false });
    if let (Some(rule), Some(extra)) = (rule.as_object_mut(), extra.as_object()) {
        rule.extend(extra.clone());
    }
    rule
}

/// The controller's policy: an access service reads only in a care
/// relationship (a deny with a permit exception naming `care`), with
/// `attested_care` holding `attested` when given.
fn policy(attested: Option<Value>) -> Value {
    policy_in(attested, &json!({}))
}

/// [`policy`], its care scope also holding `care` (such as `care_path`).
fn policy_in(attested: Option<Value>, care: &Value) -> Value {
    let archetype = "openEHR-EHR-ACCESS_POLICY.controller.v1";
    let mut scope = json!({ "_type": "ACCESS_ACTOR_SCOPE",
        "relationship": coded(TERMINOLOGY_ID_ACCESS, "care") });
    if let (Some(scope), Some(care)) = (scope.as_object_mut(), care.as_object()) {
        scope.extend(care.clone());
    }
    let care_rule = rule(
        "deny",
        &json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "caller_kind": coded(TERMINOLOGY_ID_ACCESS, "access_service") }],
            "exceptions": [rule("permit", &json!({ "applies_to": [scope] }))] }),
    );
    let mut body = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Attested care policy" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": false,
        "purpose_required": false,
        "rules": [care_rule]
    });
    if let Some(attested) = attested {
        body["attested_care"] = json!([attested]);
    }
    body
}

/// An `attested_care` entry naming the agreement `ROLE` container `role`,
/// under `issuer` when given.
fn naming(role: &str, issuer: Option<&str>) -> Value {
    let mut named = json!({ "_type": "DV_IDENTIFIER", "id": role });
    if let Some(issuer) = issuer {
        named["issuer"] = json!(issuer);
    }
    named
}

/// Commits `body` as the access policy through `svc`, then reads it in.
async fn commit(svc: &FerroEhrService, body: &Value) {
    svc.put_access_policy(&PolicyPrecondition::Create, body)
        .await
        .expect("the policy commits");
    svc.read_access_policy().await.expect("the policy reads");
}

/// A professional's token in the launch context of `ehr`, attesting a
/// treatment context for `patient` whose window ends `ends_in` seconds from
/// now, when a patient is given.
fn token(ehr: &str, patient: Option<&str>, ends_in: i64) -> String {
    let now = jiff::Timestamp::now().as_second();
    let mut claims = json!({
        "iss": FEDERATION, "aud": AUDIENCE, "exp": now + 3600, "scope": READ,
        "sub": "hp-remote-1", "acr": "high", "client_id": "remote-portal", "ehrId": ehr,
        "extensions": { "ihe_iua": {
            "subject_name": "Dr Remote",
            "subject_role": "doctor",
            "national_provider_identifier": "hp-1",
            "subject_organization_id": "urn:oid:2.999.7",
            "subject_organization": "Remote clinic",
            "country": "DE",
            "issuing_authority": "Registry",
        } },
    });
    if let Some(patient) = patient {
        claims["treatment"] = json!({
            "patient": format!("{PATIENTS}|{patient}"),
            "id": ASSERTION,
            "issuer": ASSERTION_ISSUER,
            "not_before": now - 3600,
            "not_on_or_after": now + ends_in,
        });
    }
    common::bearer(&claims)
}

/// The status of a read of the EHR `ehr` with `credential`.
async fn read(app: &Router, ehr: &str, credential: &str) -> StatusCode {
    let request = common::req("GET", format!("{BASE}/ehr/{ehr}"))
        .auth(credential)
        .accept("application/json");
    common::send_body(app, request.build()).await.0
}

/// With `attested_care` void the path is off: a caller whose credential
/// attests a current treatment context for the subject matches no care
/// rule and is refused.
#[tokio::test]
async fn with_attested_care_off_an_attested_caller_matches_no_care_rule() {
    let (_db, _pool, svc, app) = Box::pin(serve(false)).await;
    let target = patient_ehr(&svc, PATIENT).await;
    commit(&svc, &policy(None)).await;
    assert_eq!(
        read(&app, &target, &token(&target, Some(PATIENT), 600)).await,
        StatusCode::FORBIDDEN
    );
}

/// With the agreement named, a current attestation for the subject admits
/// the caller, and an attestation for another subject, an expired one, one
/// under an agreement the policy does not name and a credential that attests
/// nothing admit nothing.
#[tokio::test]
async fn an_attestation_admits_only_for_its_subject_while_current_under_a_named_agreement() {
    let (_db, pool, svc, app) = Box::pin(serve(false)).await;
    let target = patient_ehr(&svc, PATIENT).await;
    patient_ehr(&svc, OTHER_PATIENT).await;
    let role = agreement_role(&pool).await;
    commit(&svc, &policy(Some(naming(&role, Some(FEDERATION))))).await;

    let attested = token(&target, Some(PATIENT), 600);
    assert_eq!(read(&app, &target, &attested).await, StatusCode::OK);
    for (case, credential) in [
        ("another subject", token(&target, Some(OTHER_PATIENT), 600)),
        ("expired", token(&target, Some(PATIENT), -1)),
        ("no treatment context", token(&target, None, 0)),
    ] {
        assert_eq!(
            read(&app, &target, &credential).await,
            StatusCode::FORBIDDEN,
            "{case}"
        );
    }
}

/// An attestation under an agreement the policy does not name admits
/// nothing.
#[tokio::test]
async fn an_attestation_under_an_agreement_the_policy_does_not_name_admits_nothing() {
    let (_db, _pool, svc, app) = Box::pin(serve(false)).await;
    let target = patient_ehr(&svc, PATIENT).await;
    commit(
        &svc,
        &policy(Some(naming(&uuid::Uuid::now_v7().to_string(), None))),
    )
    .await;
    assert_eq!(
        read(&app, &target, &token(&target, Some(PATIENT), 600)).await,
        StatusCode::FORBIDDEN
    );
}

/// The access-log entry of an access the attested path admitted names the
/// treatment relationship by the assertion it rests on and the agreement it
/// was attested under. The EHR is restricted, so the care relationship is
/// what admits the access, and with no attestation the same read is
/// refused.
#[tokio::test]
async fn the_access_log_names_the_assertion_and_the_agreement() {
    let (_db, pool, svc, app) = Box::pin(serve(true)).await;
    let target = patient_ehr(&svc, PATIENT).await;
    let role = agreement_role(&pool).await;
    let mut body = policy(Some(naming(&role, None)));
    if let Some(members) = body.as_object_mut() {
        members.remove("rules");
    }
    commit(&svc, &body).await;
    assert_eq!(
        read(&app, &target, &token(&target, None, 0)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        read(&app, &target, &token(&target, Some(PATIENT), 600)).await,
        StatusCode::OK
    );
    let kept = crate::access_log_http::entries(&pool, 1, |stored| {
        stored.ehr_id.as_deref() == Some(target.as_str())
            && stored.result.as_deref() == Some("granted")
    })
    .await;
    let entry: Value = serde_json::from_str(&kept[0].content).expect("a canonical entry");
    let grounds = entry["authorisations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let care = grounds
        .iter()
        .find(|ground| ground["kind"]["value"] == "treatment relationship")
        .unwrap_or_else(|| panic!("a treatment relationship ground: {entry}"));
    assert_eq!(care["reference"]["id"], ASSERTION, "{entry}");
    assert_eq!(care["reference"]["issuer"], ASSERTION_ISSUER, "{entry}");
    assert_eq!(care["name"], "Cross-border exchange", "{entry}");
    assert_eq!(care["version"], "3", "{entry}");
}

/// A policy whose `attested_care` names the agreement under another issuer
/// than its partner's, or names under an issuer an id that is no agreement
/// `ROLE`, is refused `422` naming the element; the partner's own issuer is
/// accepted.
#[tokio::test]
async fn an_attested_agreement_under_another_issuer_is_refused_naming_the_element() {
    let (_db, pool, svc, _app) = Box::pin(serve(false)).await;
    let role = agreement_role(&pool).await;
    for named in [
        naming(&role, Some("https://other-federation.example")),
        naming(&uuid::Uuid::now_v7().to_string(), Some(FEDERATION)),
        naming("urn:example:agreement", Some(FEDERATION)),
    ] {
        let refused = svc
            .put_access_policy(&PolicyPrecondition::Create, &policy(Some(named.clone())))
            .await
            .expect_err("the policy is refused");
        assert!(
            matches!(&refused, ServiceError::ValidationFailed(wrong)
                if wrong.iter().any(|v| v.path == "/attested_care/0/issuer")),
            "a 422 naming the element, {named}: {refused:?}"
        );
    }
    commit(&svc, &policy(Some(naming(&role, Some(FEDERATION))))).await;
}

/// A rule's `care_path` and `care_roles` read the attested relationship: one
/// naming `attested`, or the role `PART` the credential holds it in, admits
/// the attested caller, and one naming `direct` or another role does not,
/// since no path implies another and no role another (draft decision
/// procedure, Holding a Care Relationship).
#[tokio::test]
async fn a_rule_names_the_attested_path_and_its_role() {
    let participation = "HL7_ParticipationType";
    for (care, expected) in [
        (
            json!({ "care_path": coded(TERMINOLOGY_ID_ACCESS, "attested") }),
            StatusCode::OK,
        ),
        (
            json!({ "care_roles": [coded(participation, "PART")] }),
            StatusCode::OK,
        ),
        (
            json!({ "care_path": coded(TERMINOLOGY_ID_ACCESS, "direct") }),
            StatusCode::FORBIDDEN,
        ),
        (
            json!({ "care_roles": [coded(participation, "RESP")] }),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let (_db, pool, svc, app) = Box::pin(serve(false)).await;
        let target = patient_ehr(&svc, PATIENT).await;
        let role = agreement_role(&pool).await;
        commit(&svc, &policy_in(Some(naming(&role, None)), &care)).await;
        assert_eq!(
            read(&app, &target, &token(&target, Some(PATIENT), 600)).await,
            expected,
            "{care}"
        );
    }
}
