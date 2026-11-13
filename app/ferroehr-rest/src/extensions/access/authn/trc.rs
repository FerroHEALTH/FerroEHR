// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The MyHealth@EU contact-point route at the data holder: a caller of a
//! `contact_point = true` issuer reaches patient data only for the patient of
//! its treatment relationship confirmation.
//!
//! eHDSI XCA Profile 8.0.1 §4.2.1 takes the patient identifier from the TRC
//! assertion, and §4.2.2 answers a request for another patient's documents
//! with "No Data", an audit record and a security alert. The token's
//! `patient.id` is that binding, as the contact point writes it, `system|value`
//! (the FHIR R4 search token form), read through the family's one definition,
//! `ferrohealth_smart::federation::contact::TrcPatientId`. It is resolved to
//! its EHR through the deployment's EHR index, compared on the value and the
//! assigning authority, and every patient-data request must name that EHR
//! before any lookup of its own. An unknown EHR, another patient's EHR, a TRC patient no EHR belongs to
//! and a request that names no EHR get the one `trc-patient-mismatch`
//! answer, so the answer discloses nothing. Reading the binding is our own
//! design on those sources.

use std::sync::Arc;

use axum::extract::Request;
use axum::response::{IntoResponse as _, Response};
use ferroehr::service::ehr_index::types::SubjectRef;
use ferrohealth_smart::federation::contact::{EhdsiCode, TrcPatientId};
use ferrohealth_smart::guard::Requirement;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::resource::{Access, Caller};
use openehr_its::rest::runtime::ApiError;

use super::{AuthError, AuthLayer, Principal};
use crate::overview::error::RestError;
use crate::overview::problem::Kind;
use crate::smart::routes::{Need, target_ehr};

/// The mark of a contact point's caller held to the EHR of its TRC patient,
/// which stands in for the launch context the token does not carry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrcBound;

/// Whether the caller of `access` is held to the EHR of its TRC patient.
pub(crate) fn is_bound(access: &Access) -> bool {
    access.extension::<TrcBound>().is_some()
}

/// What the TRC confinement reads of a request: whether its route serves
/// patient data, and the EHR it addresses.
#[derive(Debug, Clone)]
pub(super) struct Shape {
    patient_data: bool,
    target: Option<String>,
}

impl Shape {
    /// The shape of `req`, read before the middleware awaits anything.
    pub(super) fn of(layer: &AuthLayer, req: &Request) -> Self {
        let (matched, uri) = super::gates::route(req);
        let requirement = match layer.smart.need(req.method(), matched, uri.path()) {
            Need::Now(requirement) => requirement,
            Need::Composition(_) => Requirement::Caller,
        };
        Self {
            patient_data: layer
                .patient_data
                .as_deref()
                .is_none_or(|gate| gate.serves_patient_data(matched)),
            target: target_ehr(&requirement, matched, uri, req.headers()),
        }
    }
}

/// Holds the caller of `principal` to its agreement's `caller_rate`, then to
/// the patient it may reach on `req`: a contact point's caller to the EHR of
/// its TRC patient ([`confine`]), then a bound representative to the EHR of its
/// subject; the refusal of any, before any patient lookup of the request
/// itself.
///
/// What it reads of `req` is read before it awaits anything, so the future
/// holds no borrow of the request.
pub(super) fn patient_context<'a>(
    layer: &'a AuthLayer,
    principal: &'a mut Principal,
    req: &Request,
) -> impl Future<Output = Option<Response>> + Send + 'a {
    let shape = Shape::of(layer, req);
    let headers = req.headers().clone();
    async move {
        if let Some(refusal) = super::declared::over_rate(principal) {
            return Some(refusal);
        }
        if let Some(refusal) = confine(layer, principal, shape, &headers).await {
            return Some(refusal);
        }
        match super::party::represented_ehr(&layer.service, &principal.access).await {
            Ok(represented) => {
                principal.represented = represented;
                None
            }
            Err(e) => Some(super::refusal::refused(
                &layer.authenticator,
                &headers,
                &e,
                Some(principal.clone()),
            )),
        }
    }
}

/// Holds a contact point's caller of `principal` to the EHR of its TRC
/// patient on a patient-data request of `shape`, marking it [`TrcBound`];
/// the refusal, or `None` for a request it admits or any other caller.
///
/// A token without the binding is refused as
/// `ERROR_SEC_DATA_INTEGRITY_NOT_ENSURED` (MyHealth@EU Requirements Catalogue
/// 10.0.0 requirement 11.01.02) before the EHR index is asked.
async fn confine(
    layer: &AuthLayer,
    principal: &mut Principal,
    shape: Shape,
    headers: &http::HeaderMap,
) -> Option<Response> {
    let agreement = super::declared::contact_point(&principal.access)?;
    let not_found = agreement
        .ehdsi_service
        .map_or(EhdsiCode::SecGeneric.as_str(), |service| {
            service.not_found()
        });
    let caller = principal.access.caller()?;
    let CallerKind::ExternalPerson(external) = caller.kind() else {
        return None;
    };
    if !shape.patient_data {
        return None;
    }
    let auth = &layer.authenticator;
    let Some(subject) = external.trc_patient().as_ref().map(subject) else {
        let refusal = AuthError::KindRefused {
            code: "refused-trc-missing",
            message: format!(
                "{}: a contact point's patient-data request carries the patient of its \
                 treatment relationship confirmation in patient.id, as system|value",
                EhdsiCode::SecDataIntegrityNotEnsured.as_str()
            ),
            principal: None,
        };
        return Some(super::refusal::refused(
            auth,
            headers,
            &refusal,
            Some(principal.clone()),
        ));
    };
    let resolved = match layer.service.subject_ehr(&subject).await {
        Ok(resolved) => resolved,
        Err(error) => {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the EHR of the TRC patient could not be resolved"
            );
            let refusal = AuthError::PartyUnavailable { retry_after: None };
            return Some(super::refusal::refused(
                auth,
                headers,
                &refusal,
                Some(principal.clone()),
            ));
        }
    };
    let held = shape
        .target
        .as_deref()
        .zip(resolved)
        .is_some_and(|(target, ehr)| target.eq_ignore_ascii_case(&ehr.to_string()));
    if held {
        principal.access = Access::Caller(Arc::new(Caller::clone(caller).with_extension(TrcBound)));
        return None;
    }
    Some(mismatch(principal, not_found))
}

/// The EHR index subject of the TRC patient `patient`, a `PERSON` whose
/// identifier is assigned in the patient's system.
fn subject(patient: &TrcPatientId) -> SubjectRef {
    SubjectRef::person(patient.value(), patient.system())
}

/// The `trc-patient-mismatch` answer to `principal`, opening its detail with
/// the service's `not_found` code, recorded under
/// `refused-trc-patient-mismatch`, a security event.
fn mismatch(principal: &Principal, not_found: &'static str) -> Response {
    tracing::warn!(
        reason = "refused-trc-patient-mismatch",
        "a contact point's request reached beyond the patient of its TRC"
    );
    let mut resp = RestError::from(ApiError::NotFound(format!(
        "{not_found}: no data is available for the patient of the treatment relationship \
         confirmation"
    )))
    .of_kind(Kind::TrcPatientMismatch)
    .into_response();
    resp.extensions_mut()
        .insert(crate::system_log::refusals::Reason(
            "refused-trc-patient-mismatch",
        ));
    if let Some(mut record) =
        super::record::refused("refused-trc-patient-mismatch", None, None, Some(principal))
    {
        record
            .detail
            .insert("ehdsi-code".to_owned(), vec![not_found.to_owned()]);
        resp.extensions_mut().insert(record);
    }
    resp.extensions_mut().insert(principal.clone());
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binding_is_the_system_and_the_value() {
        let binding = |claim: &str| TrcPatientId::from_claim(claim).ok().as_ref().map(subject);
        assert_eq!(
            binding("urn:oid:2.999.1|SYNTHETIC-1"),
            Some(SubjectRef::person("SYNTHETIC-1", "urn:oid:2.999.1"))
        );
        for defective in ["SYNTHETIC-1", "|SYNTHETIC-1", "urn:oid:2.999.1|", " | "] {
            assert_eq!(binding(defective), None, "{defective}");
        }
    }
}
