// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A verified external person or organisation of a `federation` issuer is
//! admitted only within the `[[authz.federation.agreement]]` of its issuer
//! and accessor (#5101), driven through the assembled router over a real
//! `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! Its reads are `patient/` scopes, confined to the token's launch-context EHR
//! (SMART on openEHR `master07` §Context Selection and `master08-scopes.adoc`
//! §Resource Scopes); it is an access-service caller by its kind, so an
//! opted-out EHR reads as absent to it (EHDS Art. 10(1)), and only a person
//! agreement with `opt_out_override` reaches one under an emergency purpose
//! (Art. 10(2)). What an external caller may do is our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use ferroehr::config::agreement::{AccessClass, Accessor, Agreement, ConsentSource, LegalGround};
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::{AuthSection, IssuerEntry, IssuerKey};
use ferrohealth_smart::federation::{FederationSection, Field, Mapping, Source};
use ferrohealth_smart::kind::IssuerKind;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};

/// The `federation` issuer.
const FEDERATION: &str = "https://federation.example";
/// This node's audience at that issuer.
const AUDIENCE: &str = "ferroehr-federation";
/// The read an agreement admits.
const READ: &str = "patient/composition-*.r";
/// The header the default audit configuration reads the declared purpose from.
const PURPOSE: &str = "x-purpose-of-use";
/// The organisation statement's `authorization_details` type.
const ORGANISATION_ACCESS: &str = "organisation-access";
/// The claim the issuer carries its consent decision reference in.
const CONSENT_CLAIM: &str = "consent_ref";

/// The `[[auth.federation]]` table of [`FEDERATION`]: IHE IUA claims, the
/// country and issuing authority beside them, the consent decision
/// reference and the organisation statement.
fn federation_table() -> FederationSection {
    FederationSection {
        issuer: FEDERATION.to_owned(),
        contact_point: Some(false),
        mapping: Some(Mapping::Iua),
        fields: BTreeMap::from([
            (
                Field::ProfessionalCountryCode,
                vec![Source::Claim("extensions.ihe_iua.country".to_owned())],
            ),
            (
                Field::ProfessionalIssuingAuthorityName,
                vec![Source::Claim(
                    "extensions.ihe_iua.issuing_authority".to_owned(),
                )],
            ),
            (
                Field::ConsentReference,
                vec![Source::Claim(CONSENT_CLAIM.to_owned())],
            ),
        ]),
        countries: vec!["DE".to_owned()],
        issuing_authorities: vec!["Registry".to_owned()],
        provider_identifier_prefixes: vec!["urn:oid:2.999".to_owned()],
        attests_authorization_details: true,
        authorization_details_type: Some(ORGANISATION_ACCESS.to_owned()),
        organisation_types: vec![ORGANISATION_ACCESS.to_owned()],
        ..FederationSection::default()
    }
}

/// An agreement of [`FEDERATION`] for `accessor`, reading [`READ`].
fn agreement(accessor: Accessor) -> Agreement {
    Agreement {
        issuer: FEDERATION.to_owned(),
        accessor: Some(accessor),
        name: "Regional exchange".to_owned(),
        version: "1".to_owned(),
        legal_ground: Some(LegalGround {
            gdpr_6_1: "e".to_owned(),
            gdpr_9_2: "h".to_owned(),
            national: String::new(),
        }),
        reads: vec![READ.to_owned()],
        access_class: (accessor == Accessor::Organisation).then_some(AccessClass::Automated),
        ..Agreement::default()
    }
}

/// The router trusting [`FEDERATION`] under `agreements`, with `ETREAT`
/// agreed as an emergency purpose; returns it with its service.
async fn serve(agreements: Vec<Agreement>) -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        emergency_purpose_codes: vec!["ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool.clone())),
    );
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(vec![IssuerEntry {
            issuer: FEDERATION.to_owned(),
            kind: Some(IssuerKind::Federation),
            audience: vec![AUDIENCE.to_owned()],
            jwks_file: common::jwks_file().display().to_string(),
            // NOTE: the 2026/2099 Annex levels a domestic federation requires of
            // a professional's authentication.
            assurance: Some(AssuranceSection {
                levels: BTreeMap::from([("high".to_owned(), AssuranceLevel::High)]),
                ..AssuranceSection::default()
            }),
            ..IssuerEntry::default()
        }]),
        federation: vec![federation_table()],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        auth,
        agreements,
        ..common::api_config(true)
    };
    config.smart.patient_issuers = vec![FEDERATION.to_owned()];
    let app = ferroehr_rest::build_full(config, Arc::clone(&svc), None, Observability::default())
        .await
        .expect("build app");
    (db, svc, app)
}

/// A token of [`FEDERATION`] carrying `claims`.
fn token(claims: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut all = json!({ "iss": FEDERATION, "aud": AUDIENCE, "exp": exp, "scope": READ });
    if let (Some(all), Some(claims)) = (all.as_object_mut(), claims.as_object()) {
        all.extend(claims.clone());
    }
    common::bearer(&all)
}

/// A professional's token in the launch context of `ehr`, with `extra` claims.
fn professional(ehr: Option<&str>, extra: &Value) -> String {
    let mut claims = json!({
        "sub": "hp-remote-1",
        "acr": "high",
        "client_id": "remote-portal",
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
    if let Some(ehr) = ehr {
        claims["ehrId"] = json!(ehr);
    }
    if let (Some(claims), Some(extra)) = (claims.as_object_mut(), extra.as_object()) {
        claims.extend(extra.clone());
    }
    token(&claims)
}

/// The organisation's token in the launch context of `ehr`.
fn organisation(ehr: &str) -> String {
    token(&json!({
        "sub": "remote-system",
        "client_id": "remote-system",
        "ehrId": ehr,
        "authorization_details": [{ "type": ORGANISATION_ACCESS, "locations": [AUDIENCE] }],
    }))
}

/// Creates an EHR; returns its id.
async fn ehr(svc: &FerroEhrService) -> EhrId {
    Box::pin(svc.create_ehr_meta(WriteRequest::new(None)))
        .await
        .map(|(id, _)| id)
        .expect("an EHR")
}

/// Reads the EHR `ehr` as `credential`, declaring `purpose`.
async fn read(app: &Router, ehr: &str, credential: &str, purpose: Option<&str>) -> StatusCode {
    let mut request = common::req("GET", format!("{BASE}/ehr/{ehr}"))
        .auth(credential)
        .accept("application/json");
    if let Some(purpose) = purpose {
        request = request.header(PURPOSE, purpose);
    }
    common::send_body(app, request.build()).await.0
}

#[tokio::test]
async fn an_external_person_reads_its_token_bound_patient_within_its_agreement() {
    let (_db, svc, app) = serve(vec![agreement(Accessor::Person)]).await;
    let mine = ehr(&svc).await.to_string();
    let other = ehr(&svc).await.to_string();
    assert_eq!(
        read(&app, &mine, &professional(Some(&mine), &json!({})), None).await,
        StatusCode::OK
    );
    assert_eq!(
        read(&app, &other, &professional(Some(&mine), &json!({})), None).await,
        StatusCode::FORBIDDEN,
        "another EHR than the token's"
    );
    assert_eq!(
        read(&app, &mine, &professional(None, &json!({})), None).await,
        StatusCode::FORBIDDEN,
        "a token without a bound patient"
    );
    // A scope beyond the agreement's reads refuses the token.
    for scope in [
        format!("{READ} patient/composition-*.c"),
        format!("{READ} user/composition-*.r"),
        format!("{READ} __ferroehr.platform"),
    ] {
        let credential = professional(Some(&mine), &json!({ "scope": scope }));
        assert_eq!(
            read(&app, &mine, &credential, None).await,
            StatusCode::FORBIDDEN,
            "{scope}"
        );
    }
    // A federation token's roles grant nothing, the administrator's least of all.
    let admin = professional(Some(&mine), &json!({ "roles": ["ADMIN"] }));
    let (status, body) = common::send_body(
        &app,
        common::req("GET", common::full("/admin/access-opt-out"))
            .auth(&admin)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn an_external_caller_is_admitted_only_under_its_accessors_agreement() {
    let (_db, svc, persons) = serve(vec![agreement(Accessor::Person)]).await;
    let target = ehr(&svc).await.to_string();
    assert_eq!(
        read(&persons, &target, &organisation(&target), None).await,
        StatusCode::FORBIDDEN,
        "an organisation under a person agreement"
    );
    let (_db, svc, organisations) = serve(vec![agreement(Accessor::Organisation)]).await;
    let target = ehr(&svc).await.to_string();
    assert_eq!(
        read(
            &organisations,
            &target,
            &professional(Some(&target), &json!({})),
            None
        )
        .await,
        StatusCode::FORBIDDEN,
        "a person under an organisation agreement"
    );
    assert_eq!(
        read(&organisations, &target, &organisation(&target), None).await,
        StatusCode::OK,
        "the organisation reads under its own agreement"
    );
    let (_db, svc, none) = serve(Vec::new()).await;
    let target = ehr(&svc).await.to_string();
    assert_eq!(
        read(
            &none,
            &target,
            &professional(Some(&target), &json!({})),
            None
        )
        .await,
        StatusCode::FORBIDDEN,
        "no agreement"
    );
}

#[tokio::test]
async fn a_consent_conditioned_read_without_a_decision_reference_is_refused() {
    let conditioned = Agreement {
        consent: Some(ConsentSource::Local),
        ..agreement(Accessor::Person)
    };
    let (_db, svc, app) = serve(vec![conditioned]).await;
    let target = ehr(&svc).await.to_string();
    assert_eq!(
        read(
            &app,
            &target,
            &professional(Some(&target), &json!({})),
            None
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_consent_conditioned_read_carrying_its_decision_reference_is_served_and_recorded() {
    let conditioned = Agreement {
        consent: Some(ConsentSource::Local),
        ..agreement(Accessor::Person)
    };
    let (db, svc, app) = serve(vec![conditioned]).await;
    let target = ehr(&svc).await.to_string();
    let reference = "https://consent.example/fhir/Consent/c-1";
    let carrying = professional(Some(&target), &json!({ CONSENT_CLAIM: reference }));
    assert_eq!(read(&app, &target, &carrying, None).await, StatusCode::OK);
    // The reference confines nothing by itself: the token's patient grant
    // still holds it to the launch-context EHR.
    let other = ehr(&svc).await.to_string();
    assert_eq!(
        read(&app, &other, &carrying, None).await,
        StatusCode::FORBIDDEN,
        "another EHR than the token's"
    );
    let coded = professional(Some(&target), &json!({ CONSENT_CLAIM: [reference] }));
    assert_eq!(
        read(&app, &target, &coded, None).await,
        StatusCode::UNAUTHORIZED,
        "a reference is one string: a coded one refuses the token"
    );
    let pool = db.pool();
    for _ in 0..50 {
        let recorded: Option<String> = sqlx::query_scalar(
            "SELECT external_fields -> 'consent.reference' ->> 0 FROM audit.audit_event \
             WHERE operation = 'ehr_get_by_id' AND binding = 'external'",
        )
        .fetch_optional(&pool)
        .await
        .expect("query")
        .flatten();
        if let Some(recorded) = recorded {
            assert_eq!(recorded, reference);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no external record was stored");
}

#[tokio::test]
async fn an_opted_out_ehr_reads_as_absent_to_every_external_caller() {
    let overriding = Agreement {
        opt_out_override: true,
        ..agreement(Accessor::Person)
    };
    let (_db, svc, app) = serve(vec![overriding, agreement(Accessor::Organisation)]).await;
    let target = ehr(&svc).await;
    svc.record_access_opt_out(target, None)
        .await
        .expect("the opt-out records");
    let target = target.to_string();
    let person = professional(Some(&target), &json!({}));
    assert_eq!(
        read(&app, &target, &person, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        read(&app, &target, &organisation(&target), None).await,
        StatusCode::NOT_FOUND
    );
    // The emergency purpose overrides the opt-out for the person alone.
    assert_eq!(
        read(&app, &target, &person, Some("ETREAT")).await,
        StatusCode::OK
    );
    assert_eq!(
        read(&app, &target, &organisation(&target), Some("ETREAT")).await,
        StatusCode::NOT_FOUND
    );
    // Without opt_out_override the person stays refused.
    let (_db, svc, strict) = serve(vec![agreement(Accessor::Person)]).await;
    let target = ehr(&svc).await;
    svc.record_access_opt_out(target, None)
        .await
        .expect("the opt-out records");
    let target = target.to_string();
    let person = professional(Some(&target), &json!({}));
    assert_eq!(
        read(&strict, &target, &person, Some("ETREAT")).await,
        StatusCode::NOT_FOUND
    );
}

/// The `(binding, acting_mode, agreement name, provider id)` of the stored
/// `ehr_get_by_id` record whose binding is `binding`, waiting for the drain.
async fn stored(pool: &sqlx::PgPool, binding: &str) -> (String, String, String, Option<String>) {
    for _ in 0..50 {
        let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT binding, acting_mode, agreement ->> 'name', provider ->> 'id' \
             FROM audit.audit_event WHERE operation = 'ehr_get_by_id' AND binding = $1",
        )
        .bind(binding)
        .fetch_optional(pool)
        .await
        .expect("query");
        if let Some(row) = row {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no {binding} record was stored");
}

#[tokio::test]
async fn every_external_record_carries_its_binding_agreement_and_provider() {
    let (db, svc, app) = serve(vec![
        agreement(Accessor::Person),
        agreement(Accessor::Organisation),
    ])
    .await;
    let target = ehr(&svc).await.to_string();
    let person = professional(Some(&target), &json!({}));
    assert_eq!(read(&app, &target, &person, None).await, StatusCode::OK);
    assert_eq!(
        read(&app, &target, &organisation(&target), None).await,
        StatusCode::OK
    );
    let pool = db.pool();
    assert_eq!(
        stored(&pool, "external").await,
        (
            "external".to_owned(),
            "external".to_owned(),
            "Regional exchange".to_owned(),
            Some("urn:oid:2.999.7".to_owned())
        )
    );
    let (_, mode, name, _) = stored(&pool, "organisation").await;
    assert_eq!(
        (mode.as_str(), name.as_str()),
        ("external", "Regional exchange")
    );
}
