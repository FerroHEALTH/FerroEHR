// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The seam every subject-to-EHR resolution goes through.
//!
//! [`EhrIndexProvider`] selects who holds the cross-reference: an IHE patient
//! identity source asked over PIXm ([`super::ihe`]), or the in-instance
//! `linkage` map a FerroEHR run on its own opts in to. A service that was
//! given neither refuses every subject-scoped operation, and a configured
//! identity source that fails is reported as the failure it is, never answered
//! from the local map.
//!
//! NOTE: no openEHR spec governs where the cross-reference lives (RM common
//! master04 §`PARTY_SELF`): our own design.

use std::sync::Arc;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr_index::IndexError;
use crate::service::ehr_index::config::{EhrIndexConfig, ProviderKind};
use crate::service::ehr_index::ihe::{IdentitySourceError, IheIdentitySource};
use crate::service::ehr_index::types::{EhrIndexEntry, ResourceStatus, SubjectRef};
use crate::service::error::ServiceError;
use crate::storage::ehr_repo;

/// Who holds the subject-to-EHR cross-reference.
#[derive(Debug, Clone, Default)]
pub enum EhrIndexProvider {
    /// None was installed: every subject-scoped operation refuses, naming the
    /// two choices. The state of a service built by [`FerroEhrService::new`];
    /// the binary never serves in it.
    #[default]
    Unconfigured,
    /// The in-instance `linkage` map, the explicit opt-in for a FerroEHR run
    /// on its own: a subject resolves to itself and is matched against
    /// `EHR_STATUS` content.
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
    /// Without it the service refuses every subject-scoped `I_EHR_SERVICE`
    /// call and every `I_EHR_INDEX` call; the binary installs the one
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

    /// The subjects associated with an EHR, as the installed provider holds
    /// them.
    ///
    /// Under an identity source, the one subject is the clinical identifier
    /// the EHR's `EHR_STATUS` carries, with a `Primary` status and no
    /// location: the source holds no `RESOURCE_STATUS` or `LOCATION_DESC`,
    /// and FerroEHR invents none beyond the SM's own default instance type
    /// (`i_ehr_index.adoc`). The source is never asked which identifiers other
    /// domains hold for that pseudonym: resolving it back would hand the CDR
    /// the additional information GDPR Art. 4(5) keeps separately. An EHR
    /// whose subject is in no clinical-domain namespace has none to answer.
    ///
    /// # Errors
    /// [`IndexError::Unconfigured`] with no provider installed, and
    /// [`IndexError::Service`] on a storage fault.
    pub(in crate::service) async fn provider_ehr_subjects(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<EhrIndexEntry>, IndexError> {
        let source = match &self.ehr_index {
            EhrIndexProvider::Unconfigured => return Err(IndexError::Unconfigured),
            EhrIndexProvider::Local => return self.index_ehr_subjects(ehr_id).await,
            EhrIndexProvider::Ihe(source) => source,
        };
        let columns = ehr_repo::ehr_subject_columns(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::from)?;
        let Some((Some(clinical), Some(namespace))) = columns else {
            return Ok(Vec::new());
        };
        if namespace != source.domain() {
            return Ok(Vec::new());
        }
        Ok(vec![EhrIndexEntry {
            ehr_id: ehr_id.to_string(),
            subject: SubjectRef::person(clinical, namespace),
            status: ResourceStatus::default(),
            location: None,
        }])
    }

    /// The EHRs associated with a subject, as the installed provider holds
    /// them: under an identity source, the one EHR whose `EHR_STATUS` names
    /// the subject's clinical identifier, with a `Primary` status and no
    /// location.
    ///
    /// # Errors
    /// As [`Self::clinical_subject`], and [`IndexError::Service`] on a storage
    /// fault.
    pub(in crate::service) async fn provider_subject_ehrs(
        &self,
        subject: &SubjectRef,
    ) -> Result<Vec<EhrIndexEntry>, IndexError> {
        match &self.ehr_index {
            EhrIndexProvider::Unconfigured => Err(IndexError::Unconfigured),
            EhrIndexProvider::Local => self.index_subject_ehrs(subject).await,
            EhrIndexProvider::Ihe(_) => {
                let Some(clinical) = self.clinical_subject(subject).await? else {
                    return Ok(Vec::new());
                };
                let found = self
                    .ehr_id_for_subject(&clinical.id, &clinical.namespace)
                    .await?;
                Ok(found
                    .map(|ehr_id| EhrIndexEntry {
                        ehr_id: ehr_id.to_string(),
                        subject: subject.clone(),
                        status: ResourceStatus::default(),
                        location: None,
                    })
                    .into_iter()
                    .collect())
            }
        }
    }

    /// Refuse an `I_EHR_INDEX` call the installed provider does not serve
    /// from the local map.
    ///
    /// # Errors
    /// [`IndexError::Unconfigured`] with no provider installed, and
    /// [`IndexError::HeldBySource`] under an identity source, which holds the
    /// cross-reference itself.
    pub(in crate::service) fn local_index(
        &self,
        operation: &'static str,
    ) -> Result<(), IndexError> {
        match &self.ehr_index {
            EhrIndexProvider::Local => Ok(()),
            EhrIndexProvider::Unconfigured => Err(IndexError::Unconfigured),
            EhrIndexProvider::Ihe(_) => Err(IndexError::HeldBySource { operation }),
        }
    }
}
