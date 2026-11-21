// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The treatment context an external person's verified credential attests,
//! read once at admission for the `attested` care path.
//!
//! The draft access control decision procedure (Holding a Care Relationship)
//! admits a care relationship that is never stored: the caller's verified
//! credential, issued under an agreement `ACCESS_POLICY.attested_care`
//! names, attests a treatment context for the subject that is current at the
//! access. Which claims carry it is the deployment's data: the issuer's
//! `[auth.issuer.<name>.federation]` table maps them to `patient.id` (the patient, as
//! `system|value`), `trc.id` (the assertion), `trc.not_before` and
//! `trc.not_on_or_after` (its window, in seconds since the epoch) and,
//! optionally, `assertion.issuer` and `trc.care_role` (the care role it
//! attests, one code with its code system). The patient is resolved to the EHRs whose
//! subject its identifier names, one read of the party store, and only when
//! the policy in force names an agreement at all; the decision checks the
//! agreement, the window and the subject on every access
//! (`ferroehr::access_eval::condition::RequestAccess::attested_on`). A
//! credential that carries no complete treatment context attests none. Our
//! own design (#6078).

use std::sync::Arc;

use axum::response::Response;
use ferroehr::party_binding::grants::caller::attestation::Attestation;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferrohealth_smart::federation::{Attribute, Code, External, Field};
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::resource::{Access, Caller};
use openehr_base::prelude::TerminologyId;
use openehr_rm::prelude::{CodePhrase, DvCodedText};

use super::{AuthError, AuthLayer, Principal};

/// The treatment context the caller's credential attests, carried on its
/// [`Caller`] as an extension for the evaluator's caller facts.
#[derive(Debug, Clone)]
pub(crate) struct Attested(pub(crate) Arc<Attestation>);

/// Reads the treatment context the external person of `principal` attests
/// under the agreement it was admitted by, and carries it as [`Attested`];
/// the refusal when the attested patient cannot be resolved, `None`
/// otherwise.
pub(super) async fn attest(
    layer: &AuthLayer,
    principal: &mut Principal,
    headers: &http::HeaderMap,
) -> Option<Response> {
    let agreement = super::declared::attesting(&principal.access)?;
    let caller = principal.access.caller()?;
    let CallerKind::ExternalPerson(external) = caller.kind() else {
        return None;
    };
    // NOTE: our own design (#6078), with no agreement named in attested_care the path is off,
    // so no credential's patient is resolved.
    if layer
        .service
        .access_policy()
        .in_force()
        .care()
        .attested_agreements()
        .is_empty()
    {
        return None;
    }
    let context = Context::of(external)?;
    let ehrs = match layer.service.patient_ehrs(&context.patient).await {
        Ok(ehrs) => ehrs,
        Err(error) => {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "the EHR of an attested patient could not be resolved"
            );
            let refusal = AuthError::PartyUnavailable { retry_after: None };
            return Some(super::refusal::refused(
                &layer.authenticator,
                headers,
                &refusal,
                Some(principal.clone()),
            ));
        }
    };
    let attestation = Attestation {
        agreement,
        assertion: context.assertion,
        assertion_issuer: context.assertion_issuer,
        not_before: context.not_before,
        not_on_or_after: context.not_on_or_after,
        role: layer
            .service
            .attested_care_role(context.care_role.as_ref())
            .await,
        ehrs,
    };
    principal.access = Access::Caller(Arc::new(
        Caller::clone(caller).with_extension(Attested(Arc::new(attestation))),
    ));
    None
}

/// The treatment context a credential states.
struct Context {
    patient: SubjectRef,
    assertion: String,
    assertion_issuer: Option<String>,
    not_before: Option<jiff::Timestamp>,
    not_on_or_after: jiff::Timestamp,
    care_role: Option<DvCodedText>,
}

impl Context {
    /// The context `external` states; `None` when it lacks the patient, the
    /// assertion's id or the end of its window, or a bound does not read as
    /// seconds since the epoch.
    fn of(external: &External) -> Option<Self> {
        let text = |field| external.field(field).and_then(Attribute::text);
        let instant = |field| {
            text(field).map(|seconds| {
                seconds
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
            })
        };
        let patient = external.trc_patient()?;
        let not_before = match instant(Field::TrcNotBefore) {
            Some(read) => Some(read?),
            None => None,
        };
        Some(Self {
            patient: SubjectRef::person(patient.value(), patient.system()),
            assertion: text(Field::TrcId)?.to_owned(),
            assertion_issuer: text(Field::AssertionIssuer).map(str::to_owned),
            not_before,
            not_on_or_after: instant(Field::TrcNotOnOrAfter)??,
            care_role: care_role(external),
        })
    }
}

/// The care role `external` attests (`trc.care_role`), as the coded text
/// whose `defining_code` is the code in its code system; `None` when it
/// attests none. `ferrohealth-smart` refuses a token whose `trc.care_role`
/// is not one code with its system, so a value here always names both.
fn care_role(external: &External) -> Option<DvCodedText> {
    let Attribute::Codes(codes) = external.field(Field::TrcCareRole)? else {
        return None;
    };
    let [
        Code {
            system: Some(system),
            code,
        },
    ] = codes.as_slice()
    else {
        return None;
    };
    Some(DvCodedText {
        value: code.clone(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: system.clone(),
            },
            code_string: code.clone(),
            preferred_term: None,
        },
    })
}
