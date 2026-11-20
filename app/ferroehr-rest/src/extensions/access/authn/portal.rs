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
use ferrohealth_smart::party::PartyRef;
use ferrohealth_smart::resource::Caller;

use super::{AuthError, Principal};

/// The EHRs a bound representative is confined to: each one whose
/// `EHR_STATUS.subject.external_ref` names the `PERSON` the token asserts
/// and the party source records a live representation of, directly or
/// through a record subject that `PERSON` declares.
///
/// No openEHR spec governs resolving a login to a party: our own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentedEhr {
    /// Those EHRs, which after an identity merge can be more than one; empty
    /// when no EHR names the subject, so the representative reaches none.
    pub ehr_ids: Vec<ferroehr::ids::EhrId>,
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

/// What the patient context of a request binds: the EHRs a representative
/// is confined to, and the `PERSON` a `patients` issuer's patient is.
#[derive(Debug, Default)]
pub(super) struct Bound {
    /// The EHRs a bound representative is confined to.
    pub(super) represented: Option<RepresentedEhr>,
    /// The `PERSON` a `patients` issuer's patient binds as the person
    /// themself.
    pub(super) own: Option<PartyRef>,
}

/// What the patient context of `principal` binds.
///
/// A representative of a `persons` issuer reaches each EHR whose
/// `EHR_STATUS.subject.external_ref` names the represented `PERSON` (#5086,
/// #5634), looked up through `service`, which records each resolution on the
/// access log. Someone a `patients` issuer's token admits for its patient, the
/// actor of its `act` claim or a login of a representative's `PERSON` under
/// the token's issuer, reaches the EHR of the launch context while that
/// `PERSON` represents its subject today ([`portal`]); the `PERSON`s are
/// named under `namespace`. The patient of a `patients` issuer's token is
/// the person themself only when their login is a key of the `PERSON` the
/// launch EHR's subject names. A caller the login binding bound as the
/// person themself and whose token names a launch EHR is held to that EHR's
/// subject being their `PERSON` ([`own_context`]).
///
/// # Errors
/// [`AuthError::PartyRefused`] when the bound party represents no such
/// subject, when a patient's token binds no `PERSON` or names an EHR whose
/// subject is another's, and [`AuthError::PartyUnavailable`] when the store
/// cannot answer.
pub(super) async fn bound(
    service: &ferroehr::service::FerroEhrService,
    namespace: &str,
    principal: &Principal,
) -> Result<Bound, AuthError> {
    let access = &principal.access;
    let Some(caller) = access.caller() else {
        return Ok(Bound::default());
    };
    if let Some(own) = &principal.own {
        own_context(service, caller, own).await?;
        return Ok(Bound::default());
    }
    let represented = match caller.kind() {
        CallerKind::Representative { represented, .. } => represented,
        CallerKind::PatientRepresentative { actor } => {
            let login = (actor.issuer.as_str(), actor.sub.as_str());
            return match portal(service, namespace, caller, login).await? {
                Portal::Represents(represented) => Ok(Bound {
                    represented: Some(represented),
                    own: None,
                }),
                Portal::Subject(_) | Portal::Unbound => Err(super::party::unrecorded()),
            };
        }
        CallerKind::Patient => {
            let login = (caller.issuer().unwrap_or_default(), caller.subject());
            return match portal(service, namespace, caller, login).await? {
                Portal::Represents(represented) => Ok(Bound {
                    represented: Some(represented),
                    own: None,
                }),
                Portal::Subject(person) => Ok(Bound {
                    represented: None,
                    own: Some(PartyRef::new(namespace, "PERSON", person.to_string())),
                }),
                // NOTE: our own design (#6080), the party binding is the one source of own
                // access, so a patient's token whose login binds no PERSON is unbound.
                Portal::Unbound => {
                    tracing::warn!(
                        issuer = login.0,
                        outcome = "refused-unbound",
                        "the patient-portal binding names no person for a patient's own token"
                    );
                    Err(AuthError::PartyRefused {
                        code: "refused-unbound",
                        principal: None,
                        party: None,
                        named: None,
                    })
                }
            };
        }
        _ => return Ok(Bound::default()),
    };
    let subject = caller
        .party()
        .and_then(|outcome| super::party::represented_subject(outcome, represented))
        .ok_or_else(super::party::unrecorded)?;
    let ehr_ids = service
        .ehr_ids_for_person(subject.id(), subject.namespace())
        .await
        .map_err(unavailable(
            "the EHRs of the represented subject could not be read",
        ))?;
    Ok(Bound {
        represented: Some(RepresentedEhr {
            ehr_ids,
            portal: None,
        }),
        own: None,
    })
}

/// Holds `caller`, bound as the person themself to `own`, to the EHR its
/// token's launch context names (`ehrId`, master07 §Context Selection): that
/// EHR's subject has to be `own`, read with one indexed read of the subject
/// reference.
///
/// A known person is never the patient of another's EHR, as a portal login
/// is not ([`PortalLogin::Unrecorded`]). FerroEHR cannot map a FHIR
/// `Patient` id to a `PERSON` without a pairing, so a token's `patient`
/// claim is not compared. No openEHR spec governs this: our own design
/// (#6080).
///
/// # Errors
/// [`AuthError::PartyRefused`] (`refused-representation-unrecorded`) when
/// the EHR's subject is another `PERSON` or none, or the context is no EHR
/// id, and [`AuthError::PartyUnavailable`] when the store cannot answer.
async fn own_context(
    service: &ferroehr::service::FerroEhrService,
    caller: &Caller,
    own: &PartyRef,
) -> Result<(), AuthError> {
    let Some(ehr) = caller.ehr_id() else {
        return Ok(());
    };
    let subject = match ehr.parse::<uuid::Uuid>() {
        Ok(ehr) => service
            .ehr_subject_person(ferroehr::ids::EhrId(ehr))
            .await
            .map_err(unavailable(
                "the subject of the launch EHR could not be read",
            ))?,
        Err(_not_an_ehr_id) => None,
    };
    if subject.is_some_and(|subject| subject.to_string().eq_ignore_ascii_case(own.id())) {
        return Ok(());
    }
    tracing::warn!(
        issuer = caller.issuer().unwrap_or("-"),
        outcome = "refused-representation-unrecorded",
        "the launch EHR of a person's own token names another subject"
    );
    Err(super::party::unrecorded())
}

/// What a `patients` issuer's login is to the subject of its launch EHR.
enum Portal {
    /// The login is a representative's, admitted for the subject.
    Represents(RepresentedEhr),
    /// The login is a key of the subject's own `PERSON`.
    Subject(uuid::Uuid),
    /// No `PERSON` carries the login, or the token names no launch EHR.
    Unbound,
}

/// What the login `(issuer, value)` of `caller`, a `patients` issuer's
/// token, is to the subject of its launch EHR, named under `namespace`.
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
) -> Result<Portal, AuthError> {
    let ehr = caller.ehr_id().map(str::parse::<uuid::Uuid>);
    let Some(Ok(ehr)) = ehr else {
        // NOTE: master07 §Context Selection, a token naming no EHR reaches none, so its
        // login binds no subject and no representation.
        return Ok(Portal::Unbound);
    };
    let found = service
        .portal_login(issuer, value, ferroehr::ids::EhrId(ehr))
        .await
        .map_err(unavailable(
            "the party store could not bind the portal login",
        ))?;
    let (code, person) = match found {
        PortalLogin::Unbound => return Ok(Portal::Unbound),
        PortalLogin::Subject { person } => return Ok(Portal::Subject(person)),
        PortalLogin::Represents {
            person,
            subject,
            relationship,
        } => {
            return Ok(Portal::Represents(RepresentedEhr {
                ehr_ids: vec![ferroehr::ids::EhrId(ehr)],
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
        PortalLogin::Closed(closed) => return Err(super::party::closed::refusal(&closed)),
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
/// that authorises a proxy, as [`super::gates::patient_person`] read it
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
    let mut committer = super::party::committer_identity(principal, granted, &auth.local_issuer);
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
