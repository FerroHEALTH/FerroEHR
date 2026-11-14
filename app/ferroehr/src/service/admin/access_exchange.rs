// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's import and export of an EHR's access directives as FHIR
//! `Consent`, IHE BPPC and IHE APPC ([`crate::access_exchange`]).
//!
//! An import reads the document into directives, checks that every party
//! they name is a party of the demographic service, and commits them as a
//! new `EHR_ACCESS` version over the one the controller read, appended to the
//! directives in force: the person's instructions keep their history as
//! versions (BASE Architecture Overview §Security and Confidentiality,
//! "Audit trailing"). An export reads the directives of the current version.
//! Both write an access record. The routes are our own design: no openEHR
//! specification governs an exchange of access settings.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::terminology::{AccessDefault, Term};
use serde_json::Value;

use crate::access_exchange::{Format, Loss, Refused, appc, bppc};
use crate::access_log::entry::coded;
use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::response::ResourceMeta;
use crate::system_log::event::EventActionCode;
use crate::versioning::object_version_id::object_version_id;

/// The name the access record gives an export.
const EXPORT: &str = "ehr-access-export";

/// What an export reads of one EHR.
#[derive(Debug, Clone)]
struct Current {
    /// The directives of the current `EHR_ACCESS` version.
    directives: Vec<AccessDirective>,
    /// When that version was committed.
    committed: jiff::Timestamp,
    /// The subject's id and namespace, from `EHR_STATUS`.
    subject: Option<(String, String)>,
}

/// The refusal of a document, `400` naming the element.
fn refusal(refused: Refused) -> ServiceError {
    ServiceError::bad_request(refused.to_string(), refused)
}

/// Every party `directive` and its exceptions name.
fn parties(directive: &AccessDirective) -> Vec<(&'static str, &PartyRef)> {
    let mut out: Vec<(&'static str, &PartyRef)> = directive
        .applies_to
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|scope| scope.party.as_ref().map(|p| ("applies_to.party", p)))
        .collect();
    if let Some(PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(data))) =
        &directive.instructed_by
        && let Some(party) = &data.external_ref
    {
        out.push(("instructed_by", party));
    }
    for exception in directive.exceptions.as_deref().unwrap_or_default() {
        out.extend(parties(exception));
    }
    out
}

impl FerroEhrService {
    /// The `ACCESS_POLICY.catalogue` in force; empty when no policy is
    /// committed.
    #[must_use]
    pub fn access_catalogue(&self) -> Vec<AccessDirective> {
        self.access_policy()
            .in_force()
            .policy
            .as_ref()
            .and_then(|policy| policy.catalogue.as_deref().map(<[AccessDirective]>::to_vec))
            .unwrap_or_default()
    }

    /// The `default_access` of the EHR's current settings, or
    /// `server_default` when it carries none.
    ///
    /// # Errors
    /// The refusals of the `EHR_ACCESS` read.
    pub async fn ehr_default_access(
        &self,
        ehr_id: EhrId,
        server_default: AccessDefault,
    ) -> Result<AccessDefault, ServiceError> {
        let (_, _, current) = self.current_ehr_access(ehr_id).await?;
        Ok(settings_of(current.canonical.get("settings"))?
            .and_then(|s| AccessDefault::read_coded(&s.default_access).ok())
            .unwrap_or(server_default))
    }

    /// Commit `incoming` as a new `EHR_ACCESS` version over
    /// `preceding_version_uid`, appended to the directives in force.
    ///
    /// # Errors
    /// - `bad_request` (`400`): a party a directive names is no party of the
    ///   demographic service, or the settings break an invariant.
    /// - The refusals of [`FerroEhrService::commit_ehr_access_settings`].
    pub async fn import_access_directives(
        &self,
        ehr_id: EhrId,
        preceding_version_uid: &str,
        server_default: AccessDefault,
        incoming: Vec<AccessDirective>,
    ) -> Result<ResourceMeta, ServiceError> {
        for directive in &incoming {
            for (attribute, party) in parties(directive) {
                self.check_party(attribute, party).await?;
            }
        }
        let (_, _, current) = self.current_ehr_access(ehr_id).await?;
        let mut settings = settings_of(current.canonical.get("settings"))?.unwrap_or_else(|| {
            AccessDirectiveSettings {
                default_access: coded(
                    openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS_CONTROL,
                    server_default.code(),
                    server_default.code(),
                ),
                gate_keepers: None,
                directives: None,
                labels: None,
            }
        });
        let mut directives = settings
            .directives
            .take()
            .map(Vec::from)
            .unwrap_or_default();
        directives.extend(incoming);
        settings.directives = openehr_base::containers::NonEmptyVec::new(directives).ok();
        self.commit_ehr_access_settings(
            ehr_id,
            preceding_version_uid,
            openehr_its::json::to_canonical_value(&settings),
        )
        .await
    }

    /// Refuses a party the demographic service does not hold.
    async fn check_party(&self, attribute: &str, party: &PartyRef) -> Result<(), ServiceError> {
        let unresolved = || refusal(Refused::new(Format::Directive, attribute, Loss::Unresolved));
        let ObjectId::HierObjectId(id) = &party.id else {
            return Err(unresolved());
        };
        let Ok(uuid) = uuid::Uuid::parse_str(id.value()) else {
            return Err(unresolved());
        };
        match self.party_kind(VoId(uuid)).await {
            Ok(kind) if party.r#type == "PARTY" || kind.rm_type() == party.r#type => Ok(()),
            Ok(_) | Err(ServiceError::NotFound(_)) => Err(unresolved()),
            Err(error) => Err(error),
        }
    }

    /// The directives, commit time and subject of the EHR's current
    /// `EHR_ACCESS` version, recording the read.
    async fn current_directives(&self, ehr_id: EhrId) -> Result<Current, ServiceError> {
        let (_, _, current) = self.current_ehr_access(ehr_id).await?;
        let directives = settings_of(current.canonical.get("settings"))?
            .and_then(|s| s.directives.map(Vec::from))
            .unwrap_or_default();
        let subject = crate::storage::ehr_repo::ehr_subject_columns(&self.pool, ehr_id)
            .await?
            .and_then(|(id, namespace)| id.zip(namespace));
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, EXPORT)?;
        Ok(Current {
            directives,
            committed: current.time_committed,
            subject,
        })
    }

    /// The current `EHR_ACCESS` version uid of `ehr_id`, the `ETag` an
    /// import names.
    ///
    /// # Errors
    /// The refusals of the `EHR_ACCESS` read.
    pub async fn ehr_access_version_uid(&self, ehr_id: EhrId) -> Result<String, ServiceError> {
        let (vo_id, tip, current) = self.current_ehr_access(ehr_id).await?;
        Ok(object_version_id(vo_id, &current.creating_system_id, tip))
    }

    /// Import a BPPC acknowledgement held by the composition `evidence`.
    ///
    /// # Errors
    /// `bad_request` (`400`) naming the element a refused document fails
    /// at, and the refusals of [`Self::import_access_directives`].
    pub async fn import_bppc(
        &self,
        ehr_id: EhrId,
        preceding_version_uid: &str,
        server_default: AccessDefault,
        evidence: &str,
        document: &str,
    ) -> Result<(ResourceMeta, usize), ServiceError> {
        let catalogue = self.access_catalogue();
        let directives = bppc::import(
            document,
            &bppc::ImportContext {
                catalogue: &catalogue,
                evidence,
            },
        )
        .map_err(refusal)?;
        let count = directives.len();
        let meta = self
            .import_access_directives(ehr_id, preceding_version_uid, server_default, directives)
            .await?;
        Ok((meta, count))
    }

    /// Export the directives copied from the catalogue as BPPC documents.
    ///
    /// # Errors
    /// The refusals of the `EHR_ACCESS` read.
    pub async fn export_bppc(&self, ehr_id: EhrId) -> Result<bppc::Exported, ServiceError> {
        let current = self.current_directives(ehr_id).await?;
        let catalogue = self.access_catalogue();
        let controller = self.controller_uuid();
        let patient = current
            .subject
            .as_ref()
            .map(|(id, _)| id.as_str())
            .filter(|id| uuid::Uuid::parse_str(id).is_ok());
        let now = jiff::Timestamp::now().to_string();
        Ok(bppc::export(
            &current.directives,
            &bppc::ExportContext {
                catalogue: &catalogue,
                patient,
                controller: controller.as_deref(),
                now: &now,
            },
        ))
    }

    /// Import an APPC Privacy Consent Document.
    ///
    /// # Errors
    /// `bad_request` (`400`) naming the element a refused document fails
    /// at, and the refusals of [`Self::import_access_directives`].
    pub async fn import_appc(
        &self,
        ehr_id: EhrId,
        preceding_version_uid: &str,
        server_default: AccessDefault,
        document: &str,
    ) -> Result<(ResourceMeta, usize), ServiceError> {
        let catalogue = self.access_catalogue();
        let directives = appc::import(
            document,
            &appc::ImportContext {
                catalogue: &catalogue,
            },
        )
        .map_err(refusal)?;
        let count = directives.len();
        let meta = self
            .import_access_directives(ehr_id, preceding_version_uid, server_default, directives)
            .await?;
        Ok((meta, count))
    }

    /// Export the directives as one APPC Privacy Consent Document.
    ///
    /// # Errors
    /// `bad_request` (`400`) naming the attribute APPC cannot state, and the
    /// refusals of the `EHR_ACCESS` read.
    pub async fn export_appc(&self, ehr_id: EhrId) -> Result<String, ServiceError> {
        let current = self.current_directives(ehr_id).await?;
        let ehr = ehr_id.to_string();
        let (root, extension) = match &current.subject {
            Some((id, namespace)) => (namespace.as_str(), Some(id.as_str())),
            None => (ehr.as_str(), None),
        };
        let description = format!(
            "The access directives of EHR {ehr} in force since {}, exported by FerroEHR.",
            current.committed
        );
        appc::export(
            &current.directives,
            &appc::ExportContext {
                patient_root: root,
                patient_extension: extension,
                description: &description,
            },
        )
        .map_err(refusal)
    }

    /// The controller's uuid, when the policy in force names one by it.
    fn controller_uuid(&self) -> Option<String> {
        let in_force = self.access_policy().in_force();
        let policy = in_force.policy.as_ref()?;
        match &policy.controller.id {
            ObjectId::HierObjectId(id) if uuid::Uuid::parse_str(id.value()).is_ok() => {
                Some(id.value().to_owned())
            }
            _ => None,
        }
    }

    /// The controller of the policy in force.
    #[cfg(feature = "fhir")]
    fn controller(&self) -> Option<PartyRef> {
        self.access_policy()
            .in_force()
            .policy
            .as_ref()
            .map(|p| p.controller.clone())
    }
}

/// The settings `settings` holds; `None` when it holds none.
fn settings_of(settings: Option<&Value>) -> Result<Option<AccessDirectiveSettings>, ServiceError> {
    match settings {
        None | Some(Value::Null) => Ok(None),
        Some(value) => openehr_its::json::from_canonical_value::<AccessDirectiveSettings>(value)
            .map(Some)
            .map_err(|error| {
                ServiceError::conflict(format!(
                    "the EHR_ACCESS settings are not settings of the access_directives scheme: \
                     {}; repair them first",
                    error.message()
                ))
            }),
    }
}

#[cfg(feature = "fhir")]
mod fhir {
    use openehr_rm::draft::access_control::terminology::{
        AccessDefault, OperationClass, TERMINOLOGY_ID_ACCESS_CONTROL,
    };

    use super::refusal;
    use crate::access_exchange::fhir::export::{ExportContext, export};
    use crate::access_exchange::fhir::import::{ImportContext, foreign_actions, import};
    use crate::access_exchange::fhir::model::{Code, Ident, Ref};
    use crate::access_exchange::fhir::{Release, bundle, read};
    use crate::error::ServiceError;
    use crate::ids::EhrId;
    use crate::service::FerroEhrService;
    use crate::service::response::ResourceMeta;

    impl FerroEhrService {
        /// Import a FHIR `Consent` of `release`.
        ///
        /// A foreign action coding is translated to an operation class by the
        /// terminology server; with none configured it stays untranslated and
        /// the `Consent` is refused naming it.
        ///
        /// # Errors
        /// `bad_request` (`400`) naming the element a refused `Consent` fails
        /// at, `exception` when the terminology server fails, and the
        /// refusals of [`Self::import_access_directives`].
        pub async fn import_fhir_consent(
            &self,
            ehr_id: EhrId,
            preceding_version_uid: &str,
            server_default: AccessDefault,
            release: Release,
            body: &[u8],
        ) -> Result<(ResourceMeta, usize), ServiceError> {
            let document = read(release, body).map_err(refusal)?;
            let mut translated: Vec<(Code, OperationClass)> = Vec::new();
            if let Some(server) = self.terminology_server() {
                for coding in foreign_actions(&document) {
                    let (Some(system), Some(code)) = (&coding.system, &coding.code) else {
                        continue;
                    };
                    let target = server
                        .translate_to(system, code, TERMINOLOGY_ID_ACCESS_CONTROL)
                        .await?;
                    if let Some(class) = target.and_then(|t| {
                        // NOTE: our own design, a translation into no operation class is no
                        // translation, which the import refuses naming the coding.
                        <OperationClass as openehr_rm::draft::access_control::terminology::Term>::read(
                            TERMINOLOGY_ID_ACCESS_CONTROL,
                            &t,
                        )
                        .ok()
                    }) {
                        translated.push((coding.clone(), class));
                    }
                }
            }
            let operations = |coding: &Code| {
                translated
                    .iter()
                    .find(|(known, _)| known == coding)
                    .map(|(_, class)| *class)
            };
            let catalogue = self.access_catalogue();
            let default_access = self.ehr_default_access(ehr_id, server_default).await?;
            let directives = import(
                release,
                &document,
                &ImportContext {
                    catalogue: &catalogue,
                    default_access,
                    operations: &operations,
                },
            )
            .map_err(refusal)?;
            let count = directives.len();
            let meta = self
                .import_access_directives(ehr_id, preceding_version_uid, server_default, directives)
                .await?;
            Ok((meta, count))
        }

        /// Export the directives as a FHIR `Bundle` of `Consent`s of
        /// `release`.
        ///
        /// # Errors
        /// `bad_request` (`400`) naming the attribute a `Consent` cannot
        /// state, and the refusals of the `EHR_ACCESS` read.
        pub async fn export_fhir_consent(
            &self,
            ehr_id: EhrId,
            release: Release,
        ) -> Result<Vec<u8>, ServiceError> {
            let current = self.current_directives(ehr_id).await?;
            let subject = match &current.subject {
                Some((id, namespace)) => Ref {
                    kind: Some("Patient".to_owned()),
                    identifier: Some(Ident {
                        system: Some(namespace.clone()),
                        value: Some(id.clone()),
                        ..Ident::default()
                    }),
                    ..Ref::default()
                },
                None => Ref {
                    display: Some(format!("the subject of EHR {ehr_id}")),
                    ..Ref::default()
                },
            };
            let context = ExportContext {
                subject,
                controller: self.controller(),
                stated: current.committed.to_string(),
            };
            let documents = export(release, &current.directives, &context).map_err(refusal)?;
            bundle(release, &documents).map_err(refusal)
        }
    }
}
