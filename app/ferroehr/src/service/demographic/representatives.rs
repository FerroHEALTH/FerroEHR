// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads a representative's admission takes: the live representation a
//! `PERSON` holds of a subject, the `PERSON` an EHR's subject reference names,
//! and the EHRs of a `PERSON`, each read from the binding side table on every
//! request with no cache.
//!
//! A representative is admitted only while its `PERSON` holds a live
//! representation of the subject: a `PARTY_RELATIONSHIP` named
//! [`crate::party_binding::policy_mark::REPRESENTS`] whose `time_validity`
//! covers today, recorded for a provider the caller acts for and not written
//! by the representative itself ([`lookup::representation_of`]). Who may
//! represent whom, and until when, is the deployment's data, so the national
//! rules on age and legal capacity reach FerroEHR as the relationships and
//! the validity the deployment records. No openEHR spec governs resolving a
//! login to a party: our own design.

use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::care_of::ProviderKey;
use crate::party_binding::lookup::{self, LiveRepresentation};
use crate::service::FerroEhrService;
use crate::storage::access_projection::EhrSubject;

impl FerroEhrService {
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
