// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The representative a request is confined to.
//!
//! A representative, a `RelatedPerson` the login binding bound to their
//! `PERSON` or a user whose one `PERSON` is a representative's alone, is
//! admitted for the subject a live stored representation names it for, read
//! from the store, never from the token (EHDS Art. 4(2)). The patient's own
//! write of the representation that authorises a proxy is committed naming
//! their `PERSON`. No openEHR spec governs resolving a login to a party: our
//! own design.

use ferroehr::party_binding::grants::caller::care_of::ProviderKey;
use ferroehr::storage::access_projection::EhrSubject;
use ferroehr::versioning::committer::bound::BoundDetails;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::PartyRef;
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
    /// The representative and the subject, as the store names them.
    pub portal: Option<PortalRepresentative>,
}

/// Someone FerroEHR admits for a subject on a representation it stores,
/// with no representation in the token.
///
/// That is the person signing in under their own login, bound to their
/// `PERSON` by that login, and admitted while it holds a live representation
/// of the EHR's subject.
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
/// is confined to, and the `PERSON` a representative reaching their own
/// record is.
#[derive(Debug, Default)]
pub(super) struct Bound {
    /// The EHRs a bound representative is confined to.
    pub(super) represented: Option<RepresentedEhr>,
    /// The `PERSON` a representative's login reaching their own record binds
    /// as the person themself.
    pub(super) own: Option<PartyRef>,
}

/// What the patient context of `principal` binds.
///
/// Each caller is held to its subject: the person themself to the EHRs of
/// their own `PERSON` ([`own_context`]), and a representative the login
/// binding bound to the subject the request names ([`by_store`]).
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
    // NOTE: our own design (#6080), the party binding is the one source of own access, so a
    // patient's token whose login binds no PERSON is unbound.
    if matches!(caller.kind(), CallerKind::Patient) {
        tracing::warn!(
            issuer = caller.issuer().unwrap_or("-"),
            outcome = "refused-unbound",
            "the login binding names no person for a patient's own token"
        );
        return Err(AuthError::PartyRefused {
            code: "refused-unbound",
            principal: None,
            party: None,
            named: None,
        });
    }
    Ok(Bound::default())
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
/// A known person is never the patient of another's EHR. FerroEHR cannot map a FHIR
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
