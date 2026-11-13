// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The representative a request is confined to, and the patient-portal
//! representative FerroEHR binds itself (#5446).
//!
//! A representative of a `persons` issuer is bound by the login binding. A
//! `patients` issuer's token names its patient by the EHR of its launch
//! context; someone else signing in for that patient, the actor of its
//! RFC 8693 §4.1 `act` claim or a login of their own, is bound here to the
//! `PERSON` whose binding key is that login under the token's issuer, and
//! admitted while it holds a live representation of the EHR's subject. The
//! patient's own write of the representation that authorises a proxy is
//! committed naming their `PERSON`. No openEHR spec governs resolving a login
//! to a party: our own design.

use ferroehr::service::demographic::portal::PortalLogin;
use ferroehr::versioning::committer::bound::BoundDetails;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::resource::{Access, Caller};

use super::{AuthError, Principal};

/// The EHR a bound representative is confined to: the one whose
/// `EHR_STATUS.subject.external_ref` names the subject the token asserts and the
/// party source records a live representation of.
///
/// No openEHR spec governs resolving a login to a party: our own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentedEhr {
    /// That EHR's id, or `None` when no EHR names the subject, so the
    /// representative reaches no EHR.
    pub ehr_id: Option<String>,
    /// The representative of a `patients` issuer's token, whom FerroEHR binds
    /// itself; `None` for a representative of a `persons` issuer, whom the
    /// login binding bound.
    pub portal: Option<PortalRepresentative>,
}

/// Someone a `patients` issuer's token admits for the patient it names.
///
/// That is the actor of its RFC 8693 §4.1 `act` claim, or the person signing
/// in under their own login, bound to their `PERSON` by that login and
/// admitted while it holds a live representation of the EHR's subject.
///
/// No openEHR spec governs resolving a login to a party: our own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRepresentative {
    /// The representative's `PERSON` container.
    pub person: uuid::Uuid,
    /// The subject's `PERSON` container.
    pub subject: uuid::Uuid,
    /// The openEHR `subject_relationship` code the relationship names.
    pub relationship: Option<String>,
    /// The issuer of the representative's login.
    pub login_issuer: String,
    /// The representative's login under that issuer.
    pub login: String,
    /// The `PARTY_REF.namespace` the `PERSON`s are named under.
    pub namespace: String,
}

/// The EHR the representative of `access` is confined to, `None` for any
/// other caller.
///
/// A representative of a `persons` issuer reaches the EHR whose
/// `EHR_STATUS.subject.external_ref` names the represented subject (#5086),
/// looked up through `service`, which records the resolution on the access
/// log. Someone a `patients` issuer's token admits for its patient, the
/// actor of its `act` claim or a login of a representative's `PERSON` under
/// the token's issuer, reaches the EHR of the launch context while that
/// `PERSON` represents its subject today ([`portal`]); the `PERSON`s are
/// named under `namespace`.
///
/// # Errors
/// [`AuthError::PartyRefused`] when the bound party represents no such
/// subject, and [`AuthError::PartyUnavailable`] when the store cannot answer.
pub(super) async fn represented_ehr(
    service: &ferroehr::service::FerroEhrService,
    namespace: &str,
    access: &Access,
) -> Result<Option<RepresentedEhr>, AuthError> {
    let Some(caller) = access.caller() else {
        return Ok(None);
    };
    let represented = match caller.kind() {
        CallerKind::Representative { represented, .. } => represented,
        CallerKind::PatientRepresentative { actor } => {
            let login = (actor.issuer.as_str(), actor.sub.as_str());
            return match portal(service, namespace, caller, login).await? {
                Some(represented) => Ok(Some(represented)),
                None => Err(super::party::unrecorded()),
            };
        }
        CallerKind::Patient => {
            let login = (caller.issuer().unwrap_or_default(), caller.subject());
            return portal(service, namespace, caller, login).await;
        }
        _ => return Ok(None),
    };
    let subject = caller
        .party()
        .and_then(|outcome| super::party::represented_subject(outcome, represented))
        .ok_or_else(super::party::unrecorded)?;
    let found = service
        .ehr_id_for_subject(subject.id(), subject.namespace())
        .await
        .map_err(unavailable(
            "the EHR of the represented subject could not be read",
        ))?;
    Ok(Some(RepresentedEhr {
        ehr_id: found.map(|ehr| ehr.to_string()),
        portal: None,
    }))
}

/// The representative the login `(issuer, value)` of `caller`, a `patients`
/// issuer's token, is for the subject of its launch EHR, named under
/// `namespace`: `None` when no representative's `PERSON` carries the login or
/// that `PERSON` is the subject itself, so the caller is the patient.
///
/// # Errors
/// [`AuthError::PartyRefused`] when the `PERSON` holds no live representation
/// of the subject (`refused-representation-unrecorded`), is inactive or
/// abandoned, or two `PERSON`s carry the login; [`AuthError::PartyUnavailable`]
/// when the store cannot answer.
async fn portal(
    service: &ferroehr::service::FerroEhrService,
    namespace: &str,
    caller: &Caller,
    (issuer, value): (&str, &str),
) -> Result<Option<RepresentedEhr>, AuthError> {
    let ehr = caller.ehr_id().map(str::parse::<uuid::Uuid>);
    let Some(Ok(ehr)) = ehr else {
        // NOTE: master07 §Context Selection, a token naming no EHR reaches none, so its
        // patient has no subject a representation could hold.
        return Ok(None);
    };
    let found = service
        .portal_login(issuer, value, ferroehr::ids::EhrId(ehr))
        .await
        .map_err(unavailable(
            "the party store could not bind the portal login",
        ))?;
    let (code, person) = match found {
        PortalLogin::Unbound | PortalLogin::Subject => return Ok(None),
        PortalLogin::Represents {
            person,
            subject,
            relationship,
        } => {
            return Ok(Some(RepresentedEhr {
                ehr_id: Some(ehr.to_string()),
                portal: Some(PortalRepresentative {
                    person,
                    subject,
                    relationship,
                    login_issuer: issuer.to_owned(),
                    login: value.to_owned(),
                    namespace: namespace.to_owned(),
                }),
            }));
        }
        PortalLogin::Unrecorded { person } => ("refused-representation-unrecorded", Some(person)),
        PortalLogin::Inactive { person } => ("refused-inactive", Some(person)),
        PortalLogin::Ambiguous => ("refused-ambiguous", None),
    };
    tracing::warn!(
        issuer,
        outcome = code,
        "the patient-portal binding refused the request"
    );
    Err(AuthError::PartyRefused {
        code,
        principal: None,
        party: person.map(|person| {
            Box::new(ferroehr::system_log::binding::PartyTriple {
                namespace: namespace.to_owned(),
                class: "PERSON".to_owned(),
                id: person.to_string(),
            })
        }),
        named: None,
    })
}

/// The `503` of a store that could not answer, logged as `what`.
fn unavailable<E: std::error::Error + 'static>(what: &'static str) -> impl FnOnce(E) -> AuthError {
    move |error| {
        tracing::warn!(error = %ferroehr::error::ErrorChain::new(&error), "{what}");
        AuthError::PartyUnavailable { retry_after: None }
    }
}

/// What the committer of the patient of `principal`, whose `PERSON` is
/// `person`, named under `namespace`, carries when they write the
/// representation that authorises a proxy: their `PERSON` and their login,
/// since a demographic version has no record subject a `PARTY_SELF` could
/// name (RM common master04 §Audit Details).
pub(super) fn patient_details(
    principal: &Principal,
    person: uuid::Uuid,
    namespace: &str,
) -> BoundDetails {
    let caller = principal.access.caller();
    BoundDetails {
        namespace: namespace.to_owned(),
        portal: Some(ferroehr::versioning::committer::bound::PortalParty {
            person,
            relationship: None,
            login_issuer: caller
                .and_then(Caller::issuer)
                .unwrap_or_default()
                .to_owned(),
            login: principal.subject.clone(),
        }),
        ..BoundDetails::default()
    }
}

/// The committer identity of `principal`, holding `granted`, with what
/// the server knows of the person beyond the binding: the bound party's
/// details, or the `PERSON` of the patient writing the representation
/// that authorises a proxy, as [`gates::patient_person`] read it
/// (`patient`).
///
/// Returns it with that patient's `PERSON`.
///
/// # Errors
/// The refusal `patient` carries, or the one the binding's details meet.
pub(super) async fn committer(
    auth: &super::Authenticator,
    principal: &Principal,
    granted: &ferroehr::party_binding::grants::capabilities::Capabilities,
    patient: Result<Option<uuid::Uuid>, AuthError>,
) -> Result<
    (
        ferroehr::versioning::committer::CommitterIdentity,
        Option<uuid::Uuid>,
    ),
    AuthError,
> {
    let mut committer = super::party::committer_identity(principal, granted);
    let patient = patient?;
    if let Some(person) = patient {
        committer.bound = Some(patient_details(principal, person, &auth.namespace));
    }
    if let Some(binding) = auth.party.as_deref()
        && let Some(details) = binding.details(principal).await?
    {
        committer.bound = Some(details);
    }
    Ok((committer, patient))
}

/// Runs `fut` as a request of the patient whose `PERSON` is `patient`, when
/// one is, so the party protection admits the representation of that
/// `PERSON` it writes.
pub(super) async fn as_patient<F: Future>(patient: Option<uuid::Uuid>, fut: F) -> F::Output {
    match patient {
        Some(person) => ferroehr::party_binding::protect::patient(person, fut).await,
        None => fut.await,
    }
}
