// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Own access has one source, the party binding (#6080). `ferrohealth-smart`
//! reads a `persons` token as the patient's own (`Caller::is_the_person`)
//! when the issuer's profile sets `fhir_user_patient`, its `fhirUser` is
//! `Patient/<x>`, its `patient` claim is `x` and its scopes are `patient/`
//! alone, as FerroSMART's preset does. FerroEHR checks that reading against
//! the binding and never grants on it: the login has to bind the `PERSON`
//! the EHR's subject names. Each refusal reuses a binding code the access
//! record already carries:
//!
//! - a login that binds no `PERSON`: `refused-unbound`;
//! - a login bound to a professional's `PERSON`: `refused-wrong-marker`;
//! - a launch EHR whose subject is another `PERSON`: `refused-representation-unrecorded`.
//!
//! No openEHR spec governs resolving a login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::party_binding::source::BindingSource;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{service, token};
use crate::representation_http::{STRANGER, config, ehr};

/// The `PERSON` every login of [`Source`] binds.
const PERSON: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

/// The scopes of a patient's own launch: `patient/` alone.
const PATIENT_SCOPES: &str = "patient/composition-*.rs";

/// A party source answering every login with one `PERSON` of `markers`, or
/// with no party at all.
struct Source(Option<Markers>);

impl BindingSource for Source {}

impl PartySource for Source {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        let parties = self
            .0
            .clone()
            .map(|markers| {
                PartyRecord::new(
                    PartyRef::new("demographic", "PERSON", PERSON),
                    PartyState::Active,
                    markers,
                )
            })
            .into_iter()
            .collect();
        Box::pin(async move { Ok(Lookup::Parties(parties)) })
    }
}

/// The configuration of [`config`] whose issuer reads a patient's own token
/// as FerroSMART's preset does.
fn patient_reading() -> AppConfig {
    let mut config = config();
    for entry in &mut config.auth.shared.issuer {
        entry.fhir_user_patient = Some(true);
    }
    config
}

/// The router over `svc`, looking every login up in `source`.
async fn router(svc: Arc<ferroehr::service::FerroEhrService>, source: Source) -> Router {
    let config = patient_reading();
    common::first_party(&svc, &config).await;
    let authenticator = Authenticator::new(
        &config,
        Some(Arc::new(source)),
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

/// A token of the login `eva-login` whose `fhirUser` is `Patient/<fhir_user>`,
/// whose `patient` claim is `patient`, granting `scope`, launched for `ehr`
/// when given.
fn patient_token(fhir_user: &str, patient: &str, scope: &str, ehr: Option<&str>) -> String {
    let mut claims = json!({
        "fhirUser": format!("Patient/{fhir_user}"),
        "patient": patient,
        "scope": scope,
    });
    if let Some(ehr) = ehr {
        claims["ehrId"] = json!(ehr);
    }
    token("eva-login", &claims)
}

/// The status and body of `GET` on `path` under the API base as `credential`.
async fn get(app: &Router, path: &str, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req("GET", format!("{BASE}{path}"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await
}

/// The body of a refusal names the binding code it was refused with.
fn refused_with(body: &str, code: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .map(|value| value.to_string().contains(code))
        .unwrap_or_else(|_| body.contains(code))
}

#[tokio::test]
async fn a_patients_own_token_bound_to_their_person_reads_their_own_ehr() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Source(Some(Markers::default()))).await;
    let credential = patient_token("p-1", "p-1", PATIENT_SCOPES, Some(&own));

    let (code, body) = get(&app, &format!("/ehr/{own}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "her own EHR: {body}");
    let (code, body) = get(&app, &format!("/ehr/{stranger}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR: {body}");
}

#[tokio::test]
async fn a_patients_own_token_whose_login_binds_no_person_is_refused_unbound() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let app = router(Arc::clone(&svc), Source(None)).await;
    let credential = patient_token("p-1", "p-1", PATIENT_SCOPES, Some(&own));

    let (code, body) = get(&app, &format!("/ehr/{own}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(refused_with(&body, "refused-unbound"), "{body}");
}

#[tokio::test]
async fn a_patients_own_token_bound_to_a_professional_is_refused_wrong_marker() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let professional = Markers {
        professional: true,
        representative: false,
    };
    let app = router(Arc::clone(&svc), Source(Some(professional))).await;
    let credential = patient_token("p-1", "p-1", PATIENT_SCOPES, Some(&own));

    let (code, body) = get(&app, &format!("/ehr/{own}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(refused_with(&body, "refused-wrong-marker"), "{body}");
}

#[tokio::test]
async fn a_patients_own_token_launched_for_another_persons_ehr_is_refused() {
    let (_db, svc) = service().await;
    ehr(&svc, Some(PERSON)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Source(Some(Markers::default()))).await;
    let credential = patient_token("p-1", "p-1", PATIENT_SCOPES, Some(&stranger));

    let (code, body) = get(&app, &format!("/ehr/{stranger}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(
        refused_with(&body, "refused-representation-unrecorded"),
        "{body}"
    );
}

/// A professional's token with `patient/` scopes alone, bound to the
/// professional's `PERSON`, is no patient's own: it is a professional
/// confined to its launch context, and its own EHR reading never applies.
#[tokio::test]
async fn a_professional_token_with_patient_scopes_is_not_the_persons_own() {
    let (_db, svc) = service().await;
    let launch = ehr(&svc, Some(STRANGER)).await;
    let professional = Markers {
        professional: true,
        representative: false,
    };
    let app = router(Arc::clone(&svc), Source(Some(professional))).await;
    let credential = token(
        "eva-login",
        &json!({ "scope": PATIENT_SCOPES, "ehrId": launch }),
    );

    let (code, body) = get(&app, &format!("/ehr/{launch}"), &credential).await;
    assert_ne!(code, StatusCode::UNAUTHORIZED, "{body}");
    assert!(!refused_with(&body, "refused-wrong-marker"), "{body}");
    assert!(
        !refused_with(&body, "refused-representation-unrecorded"),
        "{body}"
    );
}

/// A token whose `patient` claim is not the `Patient` its `fhirUser` names
/// is no patient's own token: a login bound to a professional's `PERSON`
/// binds as that professional, where a patient's own token would be
/// refused `refused-wrong-marker`.
#[tokio::test]
async fn a_token_whose_patient_claim_differs_from_its_fhir_user_is_not_the_persons_own() {
    let (_db, svc) = service().await;
    let launch = ehr(&svc, Some(STRANGER)).await;
    let professional = Markers {
        professional: true,
        representative: false,
    };
    let app = router(Arc::clone(&svc), Source(Some(professional))).await;
    let credential = patient_token("p-1", "p-2", PATIENT_SCOPES, Some(&launch));

    let (code, body) = get(&app, &format!("/ehr/{launch}"), &credential).await;
    assert!(!refused_with(&body, "refused-wrong-marker"), "{code}: {body}");
}
