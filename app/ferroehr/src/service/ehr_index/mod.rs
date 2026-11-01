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

/// The precise EHR-index failure kind.
///
/// [`From<IndexError> for ServiceError`] maps each variant; a generic
/// [`ServiceError`] (a DB/codec fault) rides through unchanged.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// A generic service/DB fault (mapped by the shared table).
    #[error(transparent)]
    Service(#[from] ServiceError),
    /// No provider translates a subject: the service was built without one
    /// ([`provider::EhrIndexProvider::Unconfigured`]).
    #[error("{UNCONFIGURED}")]
    Unconfigured,
    /// The identity source failed; nothing falls back to another answer.
    #[error("{SOURCE_FAILED}")]
    IdentitySource(#[source] ihe::IdentitySourceError),
    /// The identity source answered a clinical-domain identifier the
    /// `[privacy]` subject rule refuses, so it is no pseudonym.
    #[error("{NOT_PSEUDONYMOUS}")]
    NotPseudonymous,
}

/// What a service without an EHR index provider answers.
const UNCONFIGURED: &str = "no EHR index provider is installed: configure an IHE patient \
    identity source ([ehr_index] pixm_url and assigning_authority) or opt in to provider = \
    \"local\"";

/// What a failed identity source answers.
const SOURCE_FAILED: &str = "the IHE patient identity source failed";

/// What a clinical identifier that is no pseudonym answers.
const NOT_PSEUDONYMOUS: &str =
    "the IHE patient identity source answered a clinical identifier that is no pseudonym";

impl From<IndexError> for ServiceError {
    /// A service fault passes through; every other EHR-index failure is a
    /// `500` whose text names what the deployment has to fix.
    fn from(e: IndexError) -> Self {
        match e {
            IndexError::Service(e) => e,
            IndexError::Unconfigured => ServiceError::fault(UNCONFIGURED),
            IndexError::NotPseudonymous => ServiceError::fault(NOT_PSEUDONYMOUS),
            IndexError::IdentitySource(source) => ServiceError::fault_caused(SOURCE_FAILED, source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::refusal::{Missing, Refusal};

    /// A generic service miss passes through with its kind.
    #[test]
    fn a_service_fault_keeps_its_kind() {
        let svc = ServiceError::from(IndexError::Service(ServiceError::not_found(
            Missing::VersionedObject,
            "x",
        )));
        assert!(matches!(
            svc,
            ServiceError::NotFound(Refusal {
                kind: Missing::VersionedObject,
                ..
            })
        ));
    }
}
