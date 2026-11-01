// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The seam every subject-to-EHR resolution goes through.
//!
//! [`EhrIndexProvider`] selects who translates a subject into the clinical
//! domain: an IHE patient identity source asked over PIXm ([`super::ihe`]), or
//! nobody, for a FerroEHR run on its own (`local`), where a subject names
//! itself. Either way the EHR is the one whose `EHR_STATUS.subject` names the
//! clinical subject, read from the `ehr` table's promoted subject columns. A
//! service that was given neither refuses every subject-scoped operation, and
//! a configured identity source that fails is reported as the failure it is.
//!
//! NOTE: no openEHR spec governs where the cross-reference lives (RM common
//! master04 §`PARTY_SELF`): our own design.

use std::sync::Arc;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr_index::IndexError;
use crate::service::ehr_index::config::{EhrIndexConfig, ProviderKind};
use crate::service::ehr_index::ihe::{IdentitySourceError, IheIdentitySource};
use crate::service::ehr_index::types::SubjectRef;

/// Who holds the subject-to-EHR cross-reference.
#[derive(Debug, Clone, Default)]
pub enum EhrIndexProvider {
    /// None was installed: every subject-scoped operation refuses, naming the
    /// two choices. The state of a service built by [`FerroEhrService::new`];
    /// the binary never serves in it.
    #[default]
    Unconfigured,
    /// No identity source, the explicit opt-in for a FerroEHR run on its own:
    /// a subject resolves to itself and is matched against `EHR_STATUS`
    /// content.
    Local,
    /// An IHE patient identity source asked over PIXm (ITI-83).
    Ihe(Arc<IheIdentitySource>),
}

impl EhrIndexProvider {
    /// The provider `[ehr_index]` names. Validation has already refused a
    /// section that names neither a complete identity source nor
    /// `provider = "local"`.
    ///
    /// # Errors
    /// [`IdentitySourceError`] when the identity source cannot be built.
    pub fn from_config(config: &EhrIndexConfig) -> Result<Self, IdentitySourceError> {
        match config.provider {
            ProviderKind::Local => Ok(Self::Local),
            ProviderKind::Ihe => Ok(Self::Ihe(Arc::new(IheIdentitySource::new(config)?))),
        }
    }

    /// The boot check: an identity source answers its capabilities read, so an
    /// unreachable one refuses the start rather than the first request. The
    /// local map needs no check.
    ///
    /// # Errors
    /// [`IdentitySourceError`] when the identity source does not answer as a
    /// FHIR server.
    pub async fn ready(&self) -> Result<(), IdentitySourceError> {
        match self {
            Self::Ihe(source) => source.probe().await,
            Self::Local | Self::Unconfigured => Ok(()),
        }
    }

    /// A short name for the boot line: `ihe`, `local` or `unconfigured`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Local => "local",
            Self::Ihe(_) => "ihe",
        }
    }
}

impl FerroEhrService {
    /// Install the provider that holds the subject-to-EHR cross-reference.
    ///
    /// Without it the service refuses every subject-scoped call; the binary
    /// installs the one
    /// `[ehr_index]` names, and a test opts in to [`EhrIndexProvider::Local`]
    /// explicitly.
    #[must_use]
    pub fn with_ehr_index(mut self, provider: EhrIndexProvider) -> Self {
        self.ehr_index = provider;
        self
    }

    /// The installed provider.
    #[must_use]
    pub fn ehr_index_provider(&self) -> &EhrIndexProvider {
        &self.ehr_index
    }

    /// The subject as `EHR_STATUS` names it in the clinical domain, or `None`
    /// when the identity source knows no clinical identifier for it.
    ///
    /// Under [`EhrIndexProvider::Local`] the subject names itself. Under
    /// [`EhrIndexProvider::Ihe`] a subject already in the clinical domain names
    /// itself, and any other is cross-referenced by the PIX Manager; the
    /// identifier it answers must be one the `[privacy]` subject rule admits,
    /// because it is the pseudonym `EHR_STATUS` carries.
    ///
    /// # Errors
    /// [`IndexError::Unconfigured`] with no provider installed,
    /// [`IndexError::IdentitySource`] when the source fails, and
    /// [`IndexError::NotPseudonymous`] for an answered identifier the subject
    /// rule refuses.
    pub(in crate::service) async fn clinical_subject(
        &self,
        subject: &SubjectRef,
    ) -> Result<Option<SubjectRef>, IndexError> {
        match &self.ehr_index {
            EhrIndexProvider::Unconfigured => Err(IndexError::Unconfigured),
            EhrIndexProvider::Local => Ok(Some(subject.clone())),
            EhrIndexProvider::Ihe(source) => {
                if subject.namespace == source.domain() {
                    return Ok(Some(subject.clone()));
                }
                let Some(clinical) = source
                    .clinical_identifier(&subject.namespace, &subject.id)
                    .await
                    .map_err(IndexError::IdentitySource)?
                else {
                    return Ok(None);
                };
                let id = secrecy::ExposeSecret::expose_secret(&clinical).to_owned();
                if !self.privacy.admits_subject(source.domain(), &id) {
                    return Err(IndexError::NotPseudonymous);
                }
                Ok(Some(SubjectRef {
                    id,
                    namespace: source.domain().to_owned(),
                    r#type: subject.r#type.clone(),
                }))
            }
        }
    }

    /// The EHR whose `EHR_STATUS` names `subject` once the installed provider
    /// has resolved it into the clinical domain ([`Self::clinical_subject`]),
    /// matched on the EHR's promoted subject columns; `None` when no EHR
    /// names it or the identity source knows no clinical identifier for it.
    ///
    /// # Errors
    /// As [`Self::clinical_subject`], and [`IndexError::Service`] on a storage
    /// fault.
    pub(in crate::service) async fn subject_ehr(
        &self,
        subject: &SubjectRef,
    ) -> Result<Option<EhrId>, IndexError> {
        let Some(clinical) = self.clinical_subject(subject).await? else {
            return Ok(None);
        };
        Ok(self
            .ehr_id_for_subject(&clinical.id, &clinical.namespace)
            .await?)
    }
}
