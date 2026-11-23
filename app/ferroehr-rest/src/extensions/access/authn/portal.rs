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
//! committed naming their `PERSON`. A `persons` login whose one `PERSON` is a
//! representative's alone carries no representation in its token, so it is
//! admitted here for the subject a live stored representation names it for
//! (#6147). No openEHR spec governs resolving a login to a party: our own
//! design.

use ferroehr::party_binding::grants::caller::care_of::ProviderKey;
use ferroehr::service::demographic::portal::PortalLogin;
use ferroehr::storage::access_projection::EhrSubject;
use ferroehr::versioning::committer::bound::BoundDetails;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::{Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;

use super::{AuthError, Principal};

/// The EHRs a bound representative is confined to: each one whose
/// `EHR_STATUS.subject.external_ref` names the `PERSON` the token asserts
/// and the party store holds a live representation of, recorded for the
/// provider the caller acts for and not written by the representative
/// itself, directly or through a record subject that `PERSON` declares
/// (draft access control decision procedure, step 0 Representation).
///
/// No openEHR spec governs resolving a login to a party: our own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentedEhr {
    /// Those EHRs, which after an identity merge can be more than one; empty
    /// when no EHR names the subject, so the representative reaches none.
    pub ehr_ids: Vec<ferroehr::ids::EhrId>,
    /// The `PARTY_RELATIONSHIP` container of the representation the
    /// representative is admitted on, which the access log names.
    pub representation: uuid::Uuid,
    /// The representative FerroEHR binds itself: of a `patients` issuer's
    /// token, or a `persons` login whose `PERSON` is a representative's
    /// alone; `None` for a representative whose token asserts the subject it
    /// represents, whom the login binding bound.
    pub portal: Option<PortalRepresentative>,
}

/// Someone FerroEHR admits for a subject on a representation it stores,
/// with no representation in the token.
///
/// That is the actor of a `patients` issuer's RFC 8693 §4.1 `act` claim, or
/// the person signing in under their own login, bound to their `PERSON` by
/// that login, of a `patients` or a `persons` issuer, and admitted while it
/// holds a live representation of the EHR's subject.
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

/// What a request names of the record it reaches, read before the
/// middleware awaits anything: the EHR of its path or of its query's scope,
/// and the subject reference an EHR lookup by subject names, as
/// `(subject_id, subject_namespace)` (ITS-REST EHR API `GET /ehr`).
#[derive(Debug, Clone, Default)]
pub(super) struct Reached {
    /// The EHR id the request names.
    pub(super) ehr: Option<String>,
    /// The subject reference an EHR lookup by subject names.
    pub(super) subject: Option<(String, String)>,
}

/// What the patient context of a request binds: the EHRs a representative
/// is confined to, and the `PERSON` a `patients` issuer's patient is.
#[derive(Debug, Default)]
pub(super) struct Bound {
    /// The EHRs a bound representative is confined to.
    pub(super) represented: Option<RepresentedEhr>,
    /// The `PERSON` a `patients` issuer's patient, or a representative's
    /// login reaching their own record, binds as the person themself.
    pub(super) own: Option<PartyRef>,
}

/// What the patient context of `principal` binds.
///
/// Each caller is held to its subject: a `patients` issuer's actor or
/// representative login to the launch EHR while it represents that subject
/// ([`portal`]), the person themself to the EHRs of their own `PERSON`
/// ([`own_context`]), and a representative the login binding bound to the
/// subject the request names ([`by_store`]).
///
/// # Errors
/// [`AuthError::PartyRefused`] when the bound party represents no such
/// subject, when a patient's token binds no `PERSON` or names an EHR whose
/// subject is another's, and [`AuthError::PartyUnavailable`] when the store
/// cannot answer.
pub(super) async fn bound(
    service: &ferroehr::service::FerroEhrService,
    (namespace, providers): (&str, &[ProviderKey]),
    principal: &Principal,
    reached: &Reached,
) -> Result<Bound, AuthError> {
    let access = &principal.access;
    let Some(caller) = access.caller() else {
        return Ok(Bound::default());
    };
    if let Some(own) = &principal.own {
        own_context(service, caller, own).await?;
        return Ok(Bound::default());
    }
    if let Some(person) = super::party::own::representative_person(caller) {
        return by_store(service, (namespace, providers), caller, person, reached).await;
    }
    let represented = match caller.kind() {
        CallerKind::Representative { represented, .. } => represented,
        CallerKind::PatientRepresentative { actor } => {
            let login = (actor.issuer.as_str(), actor.sub.as_str());
            return match portal(service, (namespace, providers), caller, login).await? {
                Portal::Represents(represented) => Ok(Bound {
                    represented: Some(represented),
                    own: None,
                }),
                Portal::Subject(_) | Portal::Unbound => Err(super::party::unrecorded()),
            };
        }
        CallerKind::Patient => {
            let login = (caller.issuer().unwrap_or_default(), caller.subject());
            return match portal(service, (namespace, providers), caller, login).await? {
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
    let outcome = caller.party();
    let subject = outcome
        .and_then(|outcome| super::party::represented_subject(outcome, represented))
        .ok_or_else(super::party::unrecorded)?;
    // NOTE: the local party source names each party by its container's UUID, so a party
    // another source names is no PERSON of this store and holds no stored representation.
    let pair = outcome
        .and_then(Outcome::bound)
        .and_then(|bound| bound.party.id().parse::<uuid::Uuid>().ok())
        .zip(subject.id().parse::<uuid::Uuid>().ok())
        .ok_or_else(super::party::unrecorded)?;
    let representation = service
        .live_representation(pair, providers)
        .await
        .map_err(unavailable("the representation could not be read"))?
        .ok_or_else(|| {
            tracing::warn!(
                issuer = caller.issuer().unwrap_or("-"),
                outcome = "refused-representation-unrecorded",
                "no stored representation admits the representative for the provider it acts for"
            );
            super::party::unrecorded()
        })?;
    let ehr_ids = service
        .ehr_ids_for_person(subject.id(), subject.namespace())
        .await
        .map_err(unavailable(
            "the EHRs of the represented subject could not be read",
        ))?;
    Ok(Bound {
        represented: Some(RepresentedEhr {
            ehr_ids,
            representation: representation.container,
            portal: None,
        }),
        own: None,
    })
}

/// What `caller`, bound by the login binding to the representative's
/// `PERSON` `person`, is admitted as on a request that `reached` names: the
/// person themself when the subject it reaches is `person`, else a
/// representative confined to the subject's EHRs, with the representation it
/// is admitted on.
///
/// The subject is that of the EHR the token's launch context names (`ehrId`,
/// master07 §Context Selection), which then holds the caller to that EHR,
/// else that of the EHR the request names, else the `PERSON` an EHR lookup by
/// subject names. The caller is admitted only on a live representation of
/// that subject for a provider it acts for, not written by itself, read in
/// one exact indexed read (draft access control decision procedure, step 0
/// Representation). A person who represents others is still the subject of
/// their own record, which they reach with everything own access carries
/// (EHDS Art. 3). No openEHR spec governs resolving a login to a party: our
/// own design (#6147, #6167).
///
/// # Errors
/// [`AuthError::PartyRefused`] (`refused-representation-unrecorded`) when
/// the request names no subject of this store or no such representation
/// names the caller for it, and [`AuthError::PartyUnavailable`] when a store
/// cannot answer.
async fn by_store(
    service: &ferroehr::service::FerroEhrService,
    (namespace, providers): (&str, &[ProviderKey]),
    caller: &Caller,
    person: &PartyRef,
    reached: &Reached,
) -> Result<Bound, AuthError> {
    let unrecorded = |why: &'static str| {
        tracing::warn!(
            issuer = caller.issuer().unwrap_or("-"),
            outcome = "refused-representation-unrecorded",
            "{why}"
        );
        super::party::unrecorded()
    };
    // NOTE: the local party source names each party by its container's UUID, so a party
    // another source names is no PERSON of this store and holds no stored representation.
    let Ok(representative) = person.id().parse::<uuid::Uuid>() else {
        return Err(unrecorded(
            "the representative's PERSON is no party of this store",
        ));
    };
    // NOTE: our own design (#6147), a launch context that names no EHR id names no subject,
    // so the representative reaches none.
    let launch = caller
        .ehr_id()
        .map(|ehr| ehr.parse::<uuid::Uuid>().ok().map(ferroehr::ids::EhrId));
    let subject = match (launch, reached) {
        (Some(Some(ehr)), _) => service.ehr_subject_person(ehr).await,
        (None, Reached { ehr: Some(ehr), .. }) => match ehr.parse::<uuid::Uuid>() {
            Ok(ehr) => service.ehr_subject_person(ferroehr::ids::EhrId(ehr)).await,
            Err(_not_an_ehr_id) => Ok(None),
        },
        (
            None,
            Reached {
                subject: Some((id, subject_namespace)),
                ..
            },
        ) => {
            service
                .subject_person(&EhrSubject::new(subject_namespace.as_str(), id.as_str()))
                .await
        }
        (Some(None) | None, Reached { .. }) => Ok(None),
    }
    .map_err(unavailable(
        "the subject the representative reaches could not be read",
    ))?;
    let Some(subject) = subject else {
        return Err(unrecorded(
            "the request of a representative names no subject of this store",
        ));
    };
    if let Some(own) = as_the_person(person, representative, subject) {
        return Ok(own);
    }
    let Some(representation) = service
        .live_representation((representative, subject), providers)
        .await
        .map_err(unavailable("the representation could not be read"))?
    else {
        return Err(unrecorded(
            "no stored representation admits the representative for the subject it reaches",
        ));
    };
    let ehr_ids = match launch {
        Some(Some(ehr)) => vec![ehr],
        _ => service
            .ehr_ids_for_person(&subject.to_string(), namespace)
            .await
            .map_err(unavailable(
                "the EHRs of the represented subject could not be read",
            ))?,
    };
    Ok(Bound {
        represented: Some(RepresentedEhr {
            ehr_ids,
            representation: representation.container,
            portal: Some(PortalRepresentative {
                person: representative,
                subject,
                relationship: representation.relationship,
                login_issuer: caller.issuer().unwrap_or_default().to_owned(),
                login: caller.subject().to_owned(),
                namespace: namespace.to_owned(),
            }),
        }),
        own: None,
    })
}

/// What a representative's login bound to `person`, the container
/// `representative`, binds as the person themself on a request reaching
/// `subject`: their own `PERSON`, when `subject` is that `PERSON`.
///
/// No openEHR spec governs resolving a login to a party: our own design
/// (#6167).
fn as_the_person(
    person: &PartyRef,
    representative: uuid::Uuid,
    subject: uuid::Uuid,
) -> Option<Bound> {
    // NOTE: EHDS Art. 3, the orchestrator's ruling on #6167: the subject's own PERSON reaches
    // its record as the person themself, whatever representative marker it also carries.
    (subject == representative).then(|| Bound {
        represented: None,
        own: Some(person.clone()),
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
    (namespace, providers): (&str, &[ProviderKey]),
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
        .portal_login((issuer, value), ferroehr::ids::EhrId(ehr), providers)
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
            representation,
        } => {
            return Ok(Portal::Represents(RepresentedEhr {
                ehr_ids: vec![ferroehr::ids::EhrId(ehr)],
                representation: representation.container,
                portal: Some(PortalRepresentative {
                    person,
                    subject,
                    relationship: representation.relationship,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_representative_reaching_their_own_subject_is_the_person_themself() {
        let own = uuid::Uuid::from_u128(1);
        let child = uuid::Uuid::from_u128(2);
        let person = PartyRef::new("demographic", "PERSON", own.to_string());
        let bound = as_the_person(&person, own, own).expect("their own subject");
        assert_eq!(bound.own, Some(person.clone()));
        assert!(bound.represented.is_none());
        assert!(as_the_person(&person, own, child).is_none());
    }
}
