// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The seam every subject-to-EHR resolution goes through.
//!
//! [`EhrIndexProvider`] selects who translates a subject into the clinical
//! domain: an IHE patient identity source asked over PIXm ([`super::ihe`]), or
//! nobody, for a FerroEHR run on its own (`local`), where a subject names
//! itself. Either way the EHRs are the ones whose `EHR_STATUS.subject` names
//! the clinical subject or another record subject of its `PERSON`, read from
//! the `ehr` table's promoted subject columns. A contact point's TRC patient is
//! found in the party domain alone ([`FerroEhrService::patient_ehrs`]). A
//! service that was given neither refuses every subject-scoped operation, and
//! a configured identity source that fails is reported as the failure it is.
//!
//! NOTE: no openEHR spec governs where the cross-reference lives (RM common
//! master04 §`PARTY_SELF`): our own design.

use std::sync::Arc;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr_index::IndexError;
use crate::service::ehr_index::config::{PixmConfig, SubjectIndex};
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
    /// The provider `record.subject_index` names, asking `[pixm]` when it
    /// is `"pixm"`. Validation has already refused an unset index and one
    /// that names `"pixm"` with no `[pixm]` table; `None` reaches here only
    /// from a configuration no validation ran over, and stays
    /// [`Self::Unconfigured`].
    ///
    /// # Errors
    /// [`IdentitySourceError`] when the identity source cannot be built.
    pub fn from_config(
        index: Option<SubjectIndex>,
        pixm: Option<&PixmConfig>,
    ) -> Result<Self, IdentitySourceError> {
        match (index, pixm) {
            (Some(SubjectIndex::Local), _) => Ok(Self::Local),
            (Some(SubjectIndex::Pixm), Some(pixm)) => {
                Ok(Self::Ihe(Arc::new(IheIdentitySource::new(pixm)?)))
            }
            (Some(SubjectIndex::Pixm), None) | (None, _) => Ok(Self::Unconfigured),
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
    /// `[pixm]` names, and a test opts in to [`EhrIndexProvider::Local`]
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

    /// The EHRs of the patient `subject` names, once the installed provider
    /// resolves it into the clinical domain: the EHR whose `EHR_STATUS.subject`
    /// names it and the EHR of every other record subject its `PERSON` carries
    /// ([`Self::ehr_ids_for_person`]), a merged patient's among them; empty when
    /// none.
    ///
    /// Under [`EhrIndexProvider::Ihe`] a subject outside the clinical domain is
    /// cross-referenced by the PIX Manager, which answers the survivor for a
    /// merged identifier (PIXm 3.1.0 §2:3.83.4.2.2.5; PMIR 1.6.0 §2:3.93.4.1.3):
    /// our own design (#5649).
    ///
    /// # Errors
    /// [`IndexError::Unconfigured`] with no provider installed,
    /// [`IndexError::IdentitySource`] when the source fails,
    /// [`IndexError::NotPseudonymous`] for an answered identifier the subject
    /// rule refuses, and [`IndexError::Service`] on a storage fault.
    pub async fn subject_ehrs(&self, subject: &SubjectRef) -> Result<Vec<EhrId>, IndexError> {
        let Some(clinical) = self.clinical_subject(subject).await? else {
            return Ok(Vec::new());
        };
        Ok(self
            .ehr_ids_for_person(&clinical.id, &clinical.namespace)
            .await?)
    }

    /// The EHRs of the patient a treatment relationship confirmation names by
    /// `patient`, found in the party domain without the identity source.
    ///
    /// The patient is the one `PERSON` carrying `patient` at the
    /// `[authz.party_binding.patient]` path, and its EHRs those of its reference
    /// and record subjects ([`Self::ehr_ids_for_person`]); under
    /// [`EhrIndexProvider::Local`] the identifier also names its own EHR. No or
    /// several such `PERSON`s find no EHR, and the caller refuses. eHDSI XCA
    /// Profile 8.0.1 §4.2.1 leaves finding the record to the data holder: our own
    /// design (#5648).
    ///
    /// # Errors
    /// [`IndexError::Unconfigured`] with no provider installed, and
    /// [`IndexError::Service`] when the party or the clinical store cannot
    /// answer.
    pub async fn patient_ehrs(&self, patient: &SubjectRef) -> Result<Vec<EhrId>, IndexError> {
        let mut found = match &self.ehr_index {
            EhrIndexProvider::Unconfigured => return Err(IndexError::Unconfigured),
            EhrIndexProvider::Local => {
                self.ehr_ids_for_person(&patient.id, &patient.namespace)
                    .await?
            }
            EhrIndexProvider::Ihe(_) => Vec::new(),
        };
        let persons = self
            .grants
            .patients_carrying(&patient.namespace, &patient.id)
            .await
            .map_err(crate::error::ServiceError::from)?;
        let [person] = persons.as_slice() else {
            if persons.len() > 1 {
                tracing::warn!(
                    persons = persons.len(),
                    "a patient identifier is carried by more than one PERSON, so it finds no EHR"
                );
            }
            return Ok(found);
        };
        let own = self.grants.namespace();
        for ehr_id in self.ehr_ids_for_person(&person.to_string(), &own).await? {
            if !found.contains(&ehr_id) {
                found.push(ehr_id);
            }
        }
        Ok(found)
    }
}
