// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR index (`service/ehr_index/`), our own design: who translates a
//! subject identifier into the clinical domain, and the subject reference it
//! translates.
//!
//! Which EHR belongs to which subject has one source, the EHR's own
//! `EHR_STATUS.subject.external_ref` (RM ehr master04 §EHR Status), promoted
//! to the `ehr` table's subject columns on every EHR create and `EHR_STATUS`
//! update. The [`provider`] decides how a subject from another identifier
//! domain becomes the clinical subject those columns hold: itself under the
//! local provider, the PIXm answer of an IHE patient identity source
//! ([`ihe`]) under the other. No wire is mounted, EHR Index having no
//! ITS-REST contract.
//!
//! NOTE: no openEHR spec governs the index: our own design.

pub mod config;
pub mod ihe;
pub mod provider;
pub mod types;

use crate::service::error::ServiceError;
use crate::service::status::StatusError;

/// The precise EHR-index failure kind.
///
/// [`From<IndexError> for StatusError`] maps each variant onto its
/// `ServiceStatus`; a generic [`ServiceError`] (a DB/codec fault) rides
/// through unchanged.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// A generic service/DB fault (mapped by the shared table).
    #[error(transparent)]
    Service(#[from] ServiceError),
    /// No provider translates a subject: the service was built without one
    /// ([`provider::EhrIndexProvider::Unconfigured`]).
    #[error(
        "no EHR index provider is installed: configure an IHE patient identity source \
         ([ehr_index] pixm_url and assigning_authority) or opt in to provider = \"local\""
    )]
    Unconfigured,
    /// The identity source failed; nothing falls back to another answer.
    #[error("the IHE patient identity source failed")]
    IdentitySource(#[source] ihe::IdentitySourceError),
    /// The identity source answered a clinical-domain identifier the
    /// `[privacy]` subject rule refuses, so it is no pseudonym.
    #[error("the IHE patient identity source answered a clinical identifier that is no pseudonym")]
    NotPseudonymous,
}

impl From<IndexError> for StatusError {
    /// Map the EHR-index errors onto their statuses; a service fault keeps the
    /// status the shared [`From<ServiceError> for StatusError`] table gives it.
    fn from(e: IndexError) -> Self {
        match e {
            IndexError::Service(e) => e.into(),
            refused @ (IndexError::Unconfigured | IndexError::NotPseudonymous) => {
                StatusError::exception(refused.to_string())
            }
            IndexError::IdentitySource(source) => {
                StatusError::exception("the IHE patient identity source failed").with_source(source)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::status::ServiceStatus;

    /// A generic service miss routes through the shared table, carrying the
    /// status it was constructed with.
    #[test]
    fn a_service_fault_keeps_its_status() {
        let svc: StatusError = IndexError::Service(ServiceError::new(
            ServiceStatus::VersionedObjectDoesNotExist,
            "x",
        ))
        .into();
        assert_eq!(svc.status, ServiceStatus::VersionedObjectDoesNotExist);
    }
}
