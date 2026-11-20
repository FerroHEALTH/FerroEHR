// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The representative a patient-portal login is, read from the binding side
//! table on every request with no cache.
//!
//! A `patients` issuer's token names the record subject by the EHR of its
//! launch context. When the person signing in is someone else, the actor of
//! an RFC 8693 §4.1 `act` claim or a login of their own, FerroEHR binds them
//! to the `PERSON` whose binding key is that login under the token's issuer,
//! and admits them only while that `PERSON` holds a live representation of
//! the EHR's subject: a `PARTY_RELATIONSHIP` named
//! [`crate::party_binding::policy_mark::REPRESENTS`] whose `time_validity`
//! covers today, recorded for a provider the caller acts for and not written
//! by the representative itself ([`lookup::representation_of`]). A login a live `PERSON` carries is held to that `PERSON`
//! (#5622): it is the patient only when that `PERSON` is the EHR's subject,
//! a representative only through a live representation, and refused
//! otherwise, so a former representative is never the patient (#5532) and a
//! provisioned representative with no representation yet reaches nothing.
//! When the CDR knows who signs in, its facts decide, never the issuer's
//! claim of the EHR alone; a login no `PERSON` carries is the patient's. Who
//! may represent whom, and until when, is the deployment's
//! data, so the national rules on age and legal capacity reach FerroEHR as
//! the relationships and the validity the deployment records. No openEHR spec
//! governs resolving a login to a party: our own design.

use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::care_of::ProviderKey;
use crate::party_binding::key_space::KeySpaceClosed;
use crate::party_binding::lookup::{self, LiveRepresentation, LookupError};
use crate::service::FerroEhrService;
use crate::storage::access_projection::EhrSubject;

/// The `version_lifecycle_state` codes of a `PERSON` that binds: `complete`
/// and `incomplete` (RM common `master06-change_control_package.adoc`
/// §Version Lifecycle).
const ACTIVE: [&str; 2] = ["532", "553"];

/// What the side table holds for a patient-portal login and the subject of
/// the EHR it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalLogin {
    /// No `PERSON` carries the login, so it binds no one.
    Unbound,
    /// The `PERSON` carrying the login is the EHR's subject itself.
    Subject {
        /// The subject's `PERSON` container.
        person: Uuid,
    },
    /// The `PERSON` `person` carries the login and represents the subject
    /// today, on the representation `representation`.
    Represents {
        /// The representative's `PERSON` container.
        person: Uuid,
        /// The subject's `PERSON` container.
        subject: Uuid,
        /// The representation it is admitted on.
        representation: LiveRepresentation,
    },
    /// The `PERSON` `person` carries the login and holds no live
    /// representation of the subject for a provider the caller acts for: it
    /// never represented them, represents someone else, its representation
    /// ended, was deleted, is for another provider or was written by itself;
    /// or the EHR
    /// names no subject of this store. A known person is never the patient
    /// of another's EHR.
    Unrecorded {
        /// The representative's `PERSON` container.
        person: Uuid,
    },
    /// The `PERSON` carrying the login is `inactive` or `abandoned`.
    Inactive {
        /// The representative's `PERSON` container.
        person: Uuid,
    },
    /// Two `PERSON`s carry the login.
    Ambiguous,
    /// The issuer's key space is not `active`, so the side table cannot say
    /// who carries the login.
    Closed(KeySpaceClosed),
}

impl FerroEhrService {
    /// Returns what the login `value` under the token issuer `issuer` is to
    /// the subject of the EHR `ehr_id` today, the date read in UTC, for a
    /// caller acting for `providers`.
    ///
    /// # Errors
    /// [`ServiceError`] when the party or the clinical store cannot answer.
    pub async fn portal_login(
        &self,
        (issuer, value): (&str, &str),
        ehr_id: EhrId,
        providers: &[ProviderKey],
    ) -> Result<PortalLogin, ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut found = match lookup::representative_keys(&mut conn, issuer, value).await {
            Ok(found) => found,
            Err(LookupError::Closed(closed)) => return Ok(PortalLogin::Closed(closed)),
            Err(LookupError::Storage(e)) => return Err(e.into()),
        };
        let (person, state) = match found.len() {
            0 => return Ok(PortalLogin::Unbound),
            1 => match found.pop() {
                Some(found) => found,
                None => return Ok(PortalLogin::Unbound),
            },
            _ => return Ok(PortalLogin::Ambiguous),
        };
        let Some(subject) = self.ehr_subject_person(ehr_id).await? else {
            return Ok(PortalLogin::Unrecorded { person });
        };
        if subject == person {
            return Ok(if ACTIVE.contains(&state.as_str()) {
                PortalLogin::Subject { person }
            } else {
                PortalLogin::Inactive { person }
            });
        }
        let today = crate::party_binding::source::today();
        let live = lookup::representation_of(&mut conn, (person, subject), &today, providers)
            .await?
            .flatten();
        Ok(match live {
            _ if !ACTIVE.contains(&state.as_str()) => PortalLogin::Inactive { person },
            Some(representation) => PortalLogin::Represents {
                person,
                subject,
                representation,
            },
            None => PortalLogin::Unrecorded { person },
        })
    }

    /// The representation the `PERSON` `representative` is admitted on to
    /// act for the `PERSON` `subject` today, the date read in UTC, for a
    /// caller acting for `providers`: one exact read of the binding side
    /// table on the party pool ([`lookup::representation_of`]); `None` when
    /// none admits it.
    ///
    /// # Errors
    /// [`ServiceError`] when the party store cannot answer.
    pub async fn live_representation(
        &self,
        (representative, subject): (Uuid, Uuid),
        providers: &[ProviderKey],
    ) -> Result<Option<LiveRepresentation>, ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let today = crate::party_binding::source::today();
        Ok(
            lookup::representation_of(&mut conn, (representative, subject), &today, providers)
                .await?
                .flatten(),
        )
    }

    /// Returns the `PERSON` the `EHR_STATUS` subject reference of the EHR
    /// `ehr_id` names ([`crate::party_binding::grants::live::LiveGrants::person_of`]),
    /// `None` when it names none of this store.
    ///
    /// # Errors
    /// [`ServiceError`] when the clinical store cannot answer.
    pub async fn ehr_subject_person(&self, ehr_id: EhrId) -> Result<Option<Uuid>, ServiceError> {
        let subject = crate::storage::ehr_repo::ehr_subject_columns(&self.pool, ehr_id)
            .await?
            .and_then(|(id, namespace)| EhrSubject::stored(namespace, id));
        match subject {
            Some(subject) => Ok(self.grants.person_of(&subject).await?),
            None => Ok(None),
        }
    }

    /// Returns the `PERSON` the subject reference `subject` names, as an
    /// `EHR_STATUS` subject reference resolves
    /// ([`crate::party_binding::grants::live::LiveGrants::person_of`]),
    /// `None` when it names none of this store.
    ///
    /// # Errors
    /// [`ServiceError`] when the party store cannot answer.
    pub async fn subject_person(&self, subject: &EhrSubject) -> Result<Option<Uuid>, ServiceError> {
        Ok(self.grants.person_of(subject).await?)
    }

    /// Returns the EHRs whose `EHR_STATUS` subject reference names the
    /// `PERSON` the reference `(subject_id, namespace)` names: the EHR of that
    /// reference, and the EHR of each record subject the `PERSON` declares,
    /// which after an identity merge can be more than one. Each lookup is
    /// recorded as a subject resolution on the access log
    /// ([`Self::ehr_id_for_subject`]).
    ///
    /// # Errors
    /// As [`Self::ehr_id_for_subject`].
    pub async fn ehr_ids_for_person(
        &self,
        subject_id: &str,
        namespace: &str,
    ) -> Result<Vec<EhrId>, ServiceError> {
        let named = EhrSubject::new(namespace, subject_id);
        let mut refs = vec![named.clone()];
        if let Some(person) = self.grants.person_of(&named).await? {
            refs.extend(
                self.grants
                    .subjects_of(person)
                    .await?
                    .into_iter()
                    .filter(|subject| subject.namespace != *self.grants.namespace()),
            );
        }
        let mut found = Vec::with_capacity(refs.len());
        for subject in refs {
            if let Some(ehr_id) = self
                .ehr_id_for_subject(&subject.id, &subject.namespace)
                .await?
                && !found.contains(&ehr_id)
            {
                found.push(ehr_id);
            }
        }
        Ok(found)
    }
}
