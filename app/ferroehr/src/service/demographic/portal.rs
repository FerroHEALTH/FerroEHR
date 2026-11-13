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
//! the EHR's subject: a `PARTY_RELATIONSHIP` carrying the representation code
//! ([`crate::party_binding::policy_mark::REPRESENTS`]) whose `time_validity`
//! covers today. Who may represent whom, and until when, is the deployment's
//! data, so the national rules on age and legal capacity reach FerroEHR as
//! the relationships and the validity the deployment records. No openEHR spec
//! governs resolving a login to a party: our own design.

use uuid::Uuid;

use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::lookup;
use crate::service::FerroEhrService;

/// The `version_lifecycle_state` codes of a `PERSON` that binds: `complete`
/// and `incomplete` (RM common `master06-change_control_package.adoc`
/// §Version Lifecycle).
const ACTIVE: [&str; 2] = ["532", "553"];

/// What the side table holds for a patient-portal login and the subject of
/// the EHR it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalLogin {
    /// No representative's `PERSON` carries the login.
    Unbound,
    /// The `PERSON` carrying the login is the EHR's subject itself.
    Subject,
    /// The `PERSON` `person` carries the login and represents the subject
    /// today, under the openEHR `subject_relationship` code `relationship`
    /// when the relationship names one.
    Represents {
        /// The representative's `PERSON` container.
        person: Uuid,
        /// The subject's `PERSON` container.
        subject: Uuid,
        /// The openEHR `subject_relationship` code.
        relationship: Option<String>,
    },
    /// The `PERSON` `person` carries the login and holds no live
    /// representation of the subject, or the EHR names no subject of this
    /// store.
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
}

impl FerroEhrService {
    /// Returns what the login `value` under the token issuer `issuer` is to
    /// the subject of the EHR `ehr_id` today, the date read in UTC.
    ///
    /// # Errors
    /// [`ServiceError`] when the party or the clinical store cannot answer.
    pub async fn portal_login(
        &self,
        issuer: &str,
        value: &str,
        ehr_id: EhrId,
    ) -> Result<PortalLogin, ServiceError> {
        let mut conn = self.demographic_pool.acquire().await?;
        let mut found = lookup::representative_keys(&mut conn, issuer, value).await?;
        let (person, state) = match found.len() {
            0 => return Ok(PortalLogin::Unbound),
            1 => match found.pop() {
                Some(found) => found,
                None => return Ok(PortalLogin::Unbound),
            },
            _ => return Ok(PortalLogin::Ambiguous),
        };
        if !ACTIVE.contains(&state.as_str()) {
            return Ok(PortalLogin::Inactive { person });
        }
        let Some(subject) = self.ehr_subject_person(ehr_id).await? else {
            return Ok(PortalLogin::Unrecorded { person });
        };
        if subject == person {
            return Ok(PortalLogin::Subject);
        }
        let today = crate::party_binding::source::today();
        let rows = lookup::representations(&mut conn, &[person], &today).await?;
        Ok(rows
            .into_iter()
            .find(|row| row.subject_id == subject)
            .map_or(PortalLogin::Unrecorded { person }, |row| {
                PortalLogin::Represents {
                    person,
                    subject,
                    relationship: row.relationship,
                }
            }))
    }

    /// Returns the `PERSON` container the `EHR_STATUS` of the EHR `ehr_id`
    /// names as its subject, `None` when the EHR names none of this store.
    ///
    /// # Errors
    /// [`ServiceError`] when the clinical store cannot answer.
    pub async fn ehr_subject_person(&self, ehr_id: EhrId) -> Result<Option<Uuid>, ServiceError> {
        // NOTE: the local source names each party by its container's UUID, so a subject
        // id that is no UUID names no PERSON of this store.
        Ok(
            crate::storage::ehr_repo::ehr_subject_columns(&self.pool, ehr_id)
                .await?
                .and_then(|(subject, _)| subject)
                .and_then(|subject| subject.parse::<Uuid>().ok()),
        )
    }
}
