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
use ferroehr::config::agreement::{
    AccessClass, Accessor, Agreement, ConsentSource, LegalGround, Purpose,
};
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use ferrohealth_server::section::CallerRate;
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::{AuthSection, IssuerEntry, IssuerKey};
use ferrohealth_smart::federation::{FederationSection, Field, Mapping, Source};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::sender::proof::DpopSection;
use ferrohealth_smart::sender::{ClientCertificate, SenderConstraint};
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
/// The organisation statement's member carrying a chained request's
/// correlation id.
const CORRELATION_MEMBER: &str = "correlation_id";
/// The claim the issuer carries a Dutch professional's UZI role in.
const NL_ROLE_CLAIM: &str = "nl_role";
/// The claim the issuer carries a Dutch organisation type in.
const NL_TYPE_CLAIM: &str = "nl_organisation_type";

/// The `[[auth.federation]]` table of [`FEDERATION`]: IHE IUA claims, the
/// country and issuing authority beside them, the consent decision
/// reference, and the organisation statement with its correlation id.
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
                Field::ProviderIssuingAuthorityName,
                vec![Source::Constant("Registry".to_owned())],
            ),
            (
                Field::ConsentReference,
                vec![Source::Claim(CONSENT_CLAIM.to_owned())],
            ),
            (
                Field::CorrelationId,
                vec![Source::AuthorizationDetails(CORRELATION_MEMBER.to_owned())],
            ),
            (Field::NlRole, vec![Source::Claim(NL_ROLE_CLAIM.to_owned())]),
            (
                Field::NlOrganisationType,
                vec![Source::Claim(NL_TYPE_CLAIM.to_owned())],
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
    Box::pin(serve_also(agreements, None)).await
}

/// [`serve`], also trusting `issuer` with its `[[auth.federation]]` table and
/// the `[auth.dpop]` table a sender-constrained issuer needs.
async fn serve_also(
    agreements: Vec<Agreement>,
    issuer: Option<(IssuerEntry, FederationSection, DpopSection)>,
) -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
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
    let mut entries = vec![IssuerEntry {
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
    }];
    let mut tables = vec![federation_table()];
    let mut dpop = None;
    if let Some((entry, table, proof)) = issuer {
        entries.push(entry);
        tables.push(table);
        dpop = Some(proof);
    }
    let mut auth = common::auth(AuthSection {
        issuer: IssuerKey::List(entries),
        federation: tables,
        dpop,
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        auth,
        agreements,
        ..common::api_config(true)
    };
    config.auth.smart.patient_issuers = vec![FEDERATION.to_owned()];
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

/// A consent-conditioned read refused for its missing decision reference is
/// recorded through the gated refusal path as a failed user authentication
/// (DICOM PS3.15 §A.5.1 `EventID` 110114) charged to the verified
/// professional (Wabvpz Art. 15a(1) for the condition; the record is our own
/// design, #5007).
#[tokio::test]
async fn a_consent_conditioned_read_without_a_decision_reference_is_recorded() {
    let conditioned = Agreement {
        consent: Some(ConsentSource::Local),
        ..agreement(Accessor::Person)
    };
    let (db, svc, app) = serve(vec![conditioned]).await;
    let target = ehr(&svc).await.to_string();
    let refused = professional(Some(&target), &json!({}));
    assert_eq!(
        read(&app, &target, &refused, None).await,
        StatusCode::FORBIDDEN
    );
    let pool = db.pool();
    for _ in 0..50 {
        let recorded: Option<(String, String)> = sqlx::query_as(
            "SELECT event_code, issuer FROM audit.audit_event \
             WHERE principal = 'hp-remote-1' AND summary IS NULL",
        )
        .fetch_optional(&pool)
        .await
        .expect("query");
        if let Some(recorded) = recorded {
            assert_eq!(recorded, ("110114".to_owned(), FEDERATION.to_owned()));
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the refused read was not recorded");
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

/// The correlation id the requester set.
const CORRELATION: &str = "5f0c6a1e-correlation-1";

/// A `chained` organisation agreement whose requester keeps its log 30 years.
fn chained() -> Agreement {
    Agreement {
        access_class: Some(AccessClass::Chained),
        chained_log_retention: Some(ferroehr::config::retention::RetentionPeriod::Years(30)),
        ..agreement(Accessor::Organisation)
    }
}

/// The organisation's token in the launch context of `ehr`, its statement
/// attesting `correlation`.
fn chained_organisation(ehr: &str, correlation: Option<&str>) -> String {
    let mut statement = json!({ "type": ORGANISATION_ACCESS, "locations": [AUDIENCE] });
    if let Some(id) = correlation {
        statement[CORRELATION_MEMBER] = json!(id);
    }
    token(&json!({
        "sub": "remote-system",
        "client_id": "remote-system",
        "ehrId": ehr,
        "authorization_details": [statement],
    }))
}

#[tokio::test]
async fn a_chained_organisation_request_is_served_only_with_an_attested_correlation_id() {
    let (db, svc, app) = serve(vec![chained()]).await;
    let target = ehr(&svc).await.to_string();
    let bare = chained_organisation(&target, None);
    assert_eq!(
        read(&app, &target, &bare, None).await,
        StatusCode::FORBIDDEN,
        "no correlation id"
    );
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-correlation-missing", 1).await;
    // A request header never stands in for the attested member.
    for header in [
        "x-correlation-id",
        "correlation-id",
        "x-request-id",
        "traceparent",
    ] {
        let request = common::req("GET", format!("{BASE}/ehr/{target}"))
            .auth(&bare)
            .accept("application/json")
            .header(
                header,
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            )
            .build();
        assert_eq!(
            common::send_body(&app, request).await.0,
            StatusCode::FORBIDDEN,
            "{header}"
        );
    }
    let attested = chained_organisation(&target, Some(CORRELATION));
    assert_eq!(read(&app, &target, &attested, None).await, StatusCode::OK);
    let pool = db.pool();
    stored(&pool, "organisation").await;
    let recorded: String = sqlx::query_scalar(
        "SELECT agreement::text FROM audit.audit_event \
         WHERE operation = 'ehr_get_by_id' AND binding = 'organisation'",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    let recorded: Value = serde_json::from_str(&recorded).expect("JSON");
    assert_eq!(
        recorded,
        json!({
            "name": "Regional exchange",
            "version": "1",
            "legal_ground": "GDPR Art. 6(1)(e), Art. 9(2)(h)",
            "accessor": "organisation",
            "access_class": "chained",
            "correlation_id": CORRELATION,
            "correlation_source": "authorization_details:correlation_id",
        })
    );
}

/// `agreement(accessor)` admitting each caller two requests at once and one
/// a second after.
fn limited(accessor: Accessor) -> Agreement {
    Agreement {
        caller_rate: Some(CallerRate {
            requests_per_second: std::num::NonZeroU32::MIN,
            burst: std::num::NonZeroU32::MIN.saturating_add(1),
        }),
        ..agreement(accessor)
    }
}

/// Reads the EHR `ehr` as `credential` over a connection that presented
/// `certificate`; returns the status and whether a `Retry-After` came back.
async fn read_over(
    app: &Router,
    ehr: &str,
    credential: &str,
    certificate: Option<&ClientCertificate>,
) -> (StatusCode, bool) {
    let mut request = common::req("GET", format!("{BASE}/ehr/{ehr}"))
        .auth(credential)
        .accept("application/json")
        .build();
    if let Some(certificate) = certificate {
        request.extensions_mut().insert(certificate.clone());
    }
    let (status, headers, _) = common::send(app, request).await;
    (status, headers.contains_key(http::header::RETRY_AFTER))
}

#[tokio::test]
async fn an_external_caller_above_its_caller_rate_is_refused_before_any_patient_lookup() {
    let (db, svc, app) = serve(vec![
        limited(Accessor::Person),
        agreement(Accessor::Organisation),
    ])
    .await;
    let target = ehr(&svc).await.to_string();
    let person = professional(Some(&target), &json!({}));
    for _ in 0..2 {
        assert_eq!(
            read_over(&app, &target, &person, None).await,
            (StatusCode::OK, false)
        );
    }
    // An EHR this node does not hold answers `429`, never the `404` a lookup gives.
    let unknown = uuid::Uuid::now_v7().to_string();
    let elsewhere = professional(Some(&unknown), &json!({}));
    assert_eq!(
        read_over(&app, &unknown, &elsewhere, None).await,
        (StatusCode::TOO_MANY_REQUESTS, true)
    );
    // Another accessor under its own agreement is not held to the person's rate.
    assert_eq!(
        read(&app, &target, &organisation(&target), None).await,
        StatusCode::OK
    );
    // The refusal goes into the caller's summary record, never a record of its own.
    let pool = db.pool();
    stored(&pool, "organisation").await;
    let own: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.audit_event WHERE binding_detail -> 'refusal' IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(own, 0);
}

/// NCPeH-A's authorization server, a MyHealth@EU contact point.
const CONTACT_POINT: &str = "https://ncpeh-a.example";

/// The contact point's issuer table, its `[[auth.federation]]` table with the
/// eHDSI mapping, and the `[auth.dpop]` table its sender constraint needs.
fn contact_point() -> (IssuerEntry, FederationSection, DpopSection) {
    let entry = IssuerEntry {
        issuer: CONTACT_POINT.to_owned(),
        kind: Some(IssuerKind::Federation),
        audience: vec![AUDIENCE.to_owned()],
        jwks_file: common::jwks_file().display().to_string(),
        sender_constraint: SenderConstraint::Required,
        ..IssuerEntry::default()
    };
    let table = FederationSection {
        issuer: CONTACT_POINT.to_owned(),
        contact_point: Some(true),
        mapping: Some(Mapping::Ehdsi),
        fields: BTreeMap::from([
            (
                Field::ProfessionalCountryCode,
                vec![Source::Constant("BE".to_owned())],
            ),
            (
                Field::ProfessionalIssuingAuthorityName,
                vec![Source::Constant("NCP-B Belgium".to_owned())],
            ),
        ]),
        countries: vec!["BE".to_owned()],
        ..FederationSection::default()
    };
    let dpop = DpopSection {
        external_url: "https://ehr.example".to_owned(),
        ..DpopSection::default()
    };
    (entry, table, dpop)
}

/// A contact-point token bound to `certificate`: eHDSI SAML Profile 10.0.0
/// §2.3's mandatory attributes with the assertion and TRC windows.
fn contact_point_token(certificate: &ClientCertificate) -> String {
    let now = jiff::Timestamp::now().as_second();
    common::bearer(&json!({
        "iss": CONTACT_POINT, "sub": "hp-nameid", "client_id": "ncpeh-b", "aud": AUDIENCE,
        "exp": now + 600, "scope": "patient/*.rs",
        "cnf": { "x5t#S256": certificate.thumbprint() },
        "ehdsi": {
            "professional": {
                "hp_identifier": "CN=Synthetic Doctor,O=Hospital,C=BE",
                "name": "Synthetic Doctor",
                "hp_professional_role": [{ "system": "2.16.840.1.113883.2.9.6.2.7", "code": "221" }],
                "authentication_method": ferrohealth_smart::federation::contact::AUTHENTICATION_CLASSES[4]
            },
            "provider": {
                "healthcare_provider_identifier": "urn:oid:1.2.3.4",
                "facility_type": "Hospital",
                "home_community_id": "urn:oid:1.2.3"
            },
            "point_of_care": "Synthetic Hospital",
            "purpose_of_use": "TREATMENT",
            "assertion": {
                "issuer": "https://ncp-b.example",
                "id": "urn:uuid:ida-1",
                "name_id_format": "urn:oasis:names:tc:SAML:1.1:nameid-format:X509SubjectName",
                "not_before": now - 60,
                "not_on_or_after": now + 3600
            },
            "trc": { "id": "urn:uuid:trc-1", "not_before": now - 60, "not_on_or_after": now + 3600 },
            "patient": { "id": "synthetic-patient^^^&1.2.3&ISO" }
        }
    }))
}

#[tokio::test]
async fn a_contact_point_caller_above_its_caller_rate_is_refused_too() {
    let professional_agreement = Agreement {
        issuer: CONTACT_POINT.to_owned(),
        reads: vec!["patient/*.rs".to_owned()],
        ..limited(Accessor::Person)
    };
    let (_db, svc, app) = Box::pin(serve_also(
        vec![professional_agreement],
        Some(contact_point()),
    ))
    .await;
    let target = ehr(&svc).await.to_string();
    let certificate = ClientCertificate::from_der(b"synthetic client certificate DER");
    let token = contact_point_token(&certificate);
    for _ in 0..2 {
        let (status, _) = read_over(&app, &target, &token, Some(&certificate)).await;
        assert!(
            status != StatusCode::UNAUTHORIZED && status != StatusCode::TOO_MANY_REQUESTS,
            "the contact point is admitted within its rate: {status}"
        );
    }
    assert_eq!(
        read_over(&app, &target, &token, Some(&certificate)).await,
        (StatusCode::TOO_MANY_REQUESTS, true)
    );
}

/// An external caller refused for want of an agreement, or for a scope its
/// agreement does not read, is recorded under the code naming which (#5140).
#[tokio::test]
async fn an_external_caller_outside_its_agreement_is_recorded_with_its_code() {
    let (db, svc, none) = serve(Vec::new()).await;
    let target = ehr(&svc).await.to_string();
    let credential = professional(Some(&target), &json!({}));
    assert_eq!(
        read(&none, &target, &credential, None).await,
        StatusCode::FORBIDDEN
    );
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-no-agreement", 1).await;

    let (db, svc, persons) = serve(vec![agreement(Accessor::Person)]).await;
    let target = ehr(&svc).await.to_string();
    let beyond = professional(
        Some(&target),
        &json!({ "scope": format!("{READ} patient/composition-*.c") }),
    );
    assert_eq!(
        read(&persons, &target, &beyond, None).await,
        StatusCode::FORBIDDEN
    );
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-scope-beyond-agreement", 1).await;
}

/// A verified external person's write records the professional under
/// `"<country_code>:<issuing_authority_name>"` and the vouching login, and is
/// refused with `422` unless its facility names the token's provider (#5097).
#[tokio::test]
async fn an_external_persons_write_records_its_committer_and_names_its_provider() {
    let write = "patient/composition-*.c";
    let writer = Agreement {
        reads: vec![READ.to_owned(), write.to_owned()],
        writes: vec!["International Patient Summary".to_owned()],
        ..agreement(Accessor::Person)
    };
    let (_db, svc, app) = serve(vec![writer]).await;
    svc.template_adl14_upload(&common::ips_opt_xml())
        .await
        .expect("upload the IPS template");
    let target = ehr(&svc).await.to_string();
    let credential = professional(
        Some(&target),
        &json!({ "scope": format!("{READ} {write}") }),
    );
    let post = |body: Value| {
        common::req("POST", format!("{BASE}/ehr/{target}/composition"))
            .auth(&credential)
            .json(body.to_string())
            .prefer_representation()
            .build()
    };
    let (status, body) = common::send_body(&app, post(common::ips_canonical_composition())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains("health_care_facility"), "{body}");

    let mut composition = common::ips_canonical_composition();
    composition["context"]["health_care_facility"] = json!({
        "_type": "PARTY_IDENTIFIED", "name": "Remote clinic",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "Registry",
                          "id": "urn:oid:2.999.7" }] });
    let (status, body) = common::send_body(&app, post(composition)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, version) = common::send_json(
        &app,
        common::req(
            "GET",
            format!(
                "{BASE}/ehr/{target}/versioned_composition/{}/version",
                serde_json::from_str::<Value>(&body).expect("the composition")["uid"]["value"]
                    .as_str()
                    .and_then(|uid| uid.split("::").next())
                    .expect("the composition uid")
            ),
        )
        .auth(&credential)
        .accept("application/json")
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    assert_eq!(
        version["commit_audit"]["committer"],
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Remote", "identifiers": [
            { "_type": "DV_IDENTIFIER", "issuer": "DE:Registry", "id": "hp-1",
              "type": "hp_identifier" },
            { "_type": "DV_IDENTIFIER", "issuer": FEDERATION, "id": "hp-remote-1",
              "type": "oauth2" } ] })
    );
}

/// An NL agreement of `accessor` for care, conditioned on the local consent.
fn nl(accessor: Accessor) -> Agreement {
    Agreement {
        jurisdiction: "NL".to_owned(),
        consent: Some(ConsentSource::Local),
        purpose: Some(Purpose::Care),
        ..agreement(accessor)
    }
}

/// The consent decision reference an NL read carries beside `claims`.
fn consented(claims: &Value) -> Value {
    let mut all = json!({ CONSENT_CLAIM: "https://consent.example/fhir/Consent/c-1" });
    if let (Some(all), Some(claims)) = (all.as_object_mut(), claims.as_object()) {
        all.extend(claims.clone());
    }
    all
}

/// Under an NL agreement, Wabvpz Art. 15f refuses an occupational or insurance
/// physician and a professional of an organisation asserting no care provider
/// type, each recorded under its code; a care provider's professional is
/// served (#5107).
#[tokio::test]
async fn an_nl_agreement_admits_only_a_care_providers_professional() {
    let (db, svc, app) = serve(vec![nl(Accessor::Person)]).await;
    let target = ehr(&svc).await.to_string();
    let asserting = |claims: Value| professional(Some(&target), &consented(&claims));
    for admitted in [
        json!({ NL_ROLE_CLAIM: "01.015", NL_TYPE_CLAIM: "Z3" }),
        json!({ NL_ROLE_CLAIM: ["01.015"], NL_TYPE_CLAIM: ["V4"] }),
    ] {
        assert_eq!(
            read(&app, &target, &asserting(admitted.clone()), None).await,
            StatusCode::OK,
            "{admitted}"
        );
    }
    for role in ["01.008", "01.048"] {
        let credential = asserting(json!({ NL_ROLE_CLAIM: role, NL_TYPE_CLAIM: "Z3" }));
        assert_eq!(
            read(&app, &target, &credential, None).await,
            StatusCode::FORBIDDEN,
            "{role}"
        );
    }
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-wabvpz-role", 2).await;
    for refused in [
        json!({ NL_ROLE_CLAIM: "01.015" }),
        json!({ NL_ROLE_CLAIM: "01.015", NL_TYPE_CLAIM: "ZV" }),
        json!({ NL_ROLE_CLAIM: "01.015", NL_TYPE_CLAIM: ["V4", "ZV"] }),
    ] {
        assert_eq!(
            read(&app, &target, &asserting(refused.clone()), None).await,
            StatusCode::FORBIDDEN,
            "{refused}"
        );
    }
    crate::party_binding_http::refusal_rows(&db.pool(), "refused-wabvpz-organisation-type", 3)
        .await;
}

/// An organisation under an NL agreement is served only when it asserts a care
/// provider type; an insurer, which asserts none, is refused (Wabvpz
/// Art. 15f(1), #5107).
#[tokio::test]
async fn an_nl_agreement_admits_only_a_care_provider_organisation() {
    let (_db, svc, app) = serve(vec![nl(Accessor::Organisation)]).await;
    let target = ehr(&svc).await.to_string();
    let asserting = |claims: &Value| {
        let mut all = consented(claims);
        all["sub"] = json!("remote-system");
        all["client_id"] = json!("remote-system");
        all["ehrId"] = json!(target);
        all["authorization_details"] =
            json!([{ "type": ORGANISATION_ACCESS, "locations": [AUDIENCE] }]);
        token(&all)
    };
    assert_eq!(
        read(
            &app,
            &target,
            &asserting(&json!({ NL_TYPE_CLAIM: "V4" })),
            None
        )
        .await,
        StatusCode::OK
    );
    for refused in [json!({}), json!({ NL_TYPE_CLAIM: "ZV" })] {
        assert_eq!(
            read(&app, &target, &asserting(&refused), None).await,
            StatusCode::FORBIDDEN,
            "{refused}"
        );
    }
}
