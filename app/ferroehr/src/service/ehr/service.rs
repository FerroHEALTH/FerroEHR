// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_EHR_SERVICE` (`i_ehr_service.adoc`) + `EHR_SUMMARY` (`ehr_summary.adoc`):
//! EHR create (4 variants), `has_ehr`(`_for_subject`), `get_ehr(s)`, and the
//! folder-hierarchy reads the `EHR` wire body needs.
//!
//! Spec: arch-overview `master06-design_of_the_ehr.adoc` §The EHR (EHR root,
//! `system_id`, `EHR_ACCESS`, `EHR_STATUS`, directory, folders,
//! `time_created`) and RM ehr `master04-ehr_package.adoc` §EHR Creation /
//! §Folders. The EHR-table and folder-membership SQL is a storage seam
//! (no openEHR spec governs the schema — our own design).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::EhrId;
use crate::service::ehr::handle::EhrSummary;
use crate::service::ehr_index::types::SubjectRef;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::{CallStatusType, SmError};
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};
use openehr_base::prelude::{ArchetypeId, HierObjectId, ObjectId, ObjectRef, ObjectRefData};
use openehr_rm::prelude::{Archetyped, DvText, DvTextData, Ehr, EhrStatus, PartySelf};
use serde_json::Value;
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, commit_contribution};
use crate::versioning::object_version_id::{VersionIdError, hier_object_id};

use super::status_for_subject;

impl FerroEhrService {
    /// Create an EHR (with the given id), its initial `EHR_STATUS`, and its
    /// `EHR_ACCESS`, all committed under **one** CONTRIBUTION — RM ehr master04
    /// §EHR Creation: "the result should be a root EHR object, an EHR Status
    /// object, and an EHR Access object … created and committed in a
    /// Contribution". Shared by `POST /ehr` and `PUT /ehr/{ehr_id}`.
    ///
    /// `committal` carries the client's `openehr-version` and
    /// `openehr-audit-details` request-header metadata when the request supplied
    /// any: EHR creation commits change-controlled content, so the ITS-REST
    /// merge MUST applies exactly as it does to a COMPOSITION write (overview
    /// `Requests_and_responses.md` §"openehr-version and openehr-audit-details").
    /// `None` keeps the server-default attribution.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] when the supplied `EHR_STATUS` is
    /// structurally invalid, or a committal `change_type`/`lifecycle_state`
    /// is outside its openEHR terminology group;
    /// [`ServiceError::BadRequest`] when a committal `change_type` is a legal
    /// group code that contradicts a creation (only `249|creation|` commits a
    /// first version); [`ServiceError::Conflict`] when the EHR already
    /// exists or the subject already owns another EHR (`ehr_subject_uq`, kept
    /// in sync by [`Self::sync_ehr_subject`] → 409, ITS-REST `409_EHR.yaml`;
    /// CNF `create_ehr-two_ehrs_same_patient`); [`ServiceError::Database`] on
    /// a storage failure.
    pub(in crate::service) async fn commit_new_ehr(
        &self,
        ehr_id: EhrId,
        status: Value,
        committal: Option<&crate::service::version_update::Committal>,
    ) -> Result<ServiceResponse, ServiceError> {
        // The supplied EHR_STATUS must be a structurally valid RM instance
        // before the EHR is created (CNF master06 §Test Data Sets INVALID
        // class 2). The EHR-create wire carries no lifecycle channel, so the
        // first status version is complete — full strictness.
        self.validate_for_commit(Kind::EhrStatus, &status, false)
            .await?;

        // The creation commit's AUDIT_DETAILS, built BEFORE the transaction so
        // an illegal client `change_type` refuses without a storage round trip.
        // `from_update` constrains the code to `249|creation|`: a create commits
        // a first version (RM common master06 §Contributions).
        let audit = match committal {
            Some(c) => crate::versioning::audit::AuditInput::from_update(
                &c.audit,
                change_type::CREATION,
                "EHR creation",
                &self.effective_system_id(),
            )?,
            None => self.audit(change_type::CREATION, "EHR creation"),
        };

        let mut tx = self.pool.begin().await?;

        // EHR.system_id is recorded at creation and immutable thereafter (arch
        // master06 §System Identity — a stored value, not the live config); the
        // promoted subject / is_queryable columns ride the same INSERT, so no
        // separate `sync_ehr_subject` UPDATE runs. A subject already owned by
        // another EHR is a distinct 409 (RM ehr master04 §EHR Status).
        let (subject_id, subject_namespace, is_queryable, is_modifiable) =
            super::status::ehr_promoted_columns(&status);
        let time_created = match crate::storage::ehr_repo::insert_ehr(
            &mut tx,
            ehr_id,
            &self.effective_system_id(),
            subject_id,
            subject_namespace,
            is_queryable,
            is_modifiable,
        )
        .await
        {
            Ok(Some(t)) => t,
            Ok(None) => {
                // NOTE: SM ehr_call_status_type.adoc declares
                // ehr_create_fail_duplicate_id for this exact refusal.
                return Err(ServiceError::sm(
                    CallStatusType::EhrCreateFailDuplicateId,
                    format!("EHR {ehr_id} already exists"),
                ));
            }
            Err(crate::storage::error::StorageError::SubjectInUse(id, ns)) => {
                // NOTE: SM ehr_call_status_type.adoc declares
                // ehr_for_subject_already_exists for the one-EHR-per-subject rule.
                return Err(ServiceError::sm(
                    CallStatusType::EhrForSubjectAlreadyExists,
                    format!("an EHR already exists for subject {id}@{ns}"),
                ));
            }
            Err(e) => return Err(e.into()),
        };

        let committed = commit_contribution(
            &mut tx,
            Some(ehr_id),
            None,
            &audit,
            vec![
                (
                    audit.clone(),
                    Change::Create {
                        kind: Kind::EhrStatus,
                        canonical: status.clone(),
                        template_id: None,
                        signature: None,
                        // A client-supplied `openehr-version:
                        // lifecycle_state.code_string=…` targets the content
                        // THIS request carried — the bootstrap EHR_STATUS.
                        // NOTE: no released text governs a create committing
                        // two versions — our own design: the generated
                        // EHR_ACCESS keeps the default `532|complete|`.
                        lifecycle_state: committal.and_then(|c| c.lifecycle_state.clone()),
                        attestations: Vec::new(),
                    },
                ),
                (
                    audit.clone(),
                    Change::Create {
                        kind: Kind::EhrAccess,
                        canonical: super::access::initial_ehr_access(),
                        template_id: None,
                        signature: None,
                        lifecycle_state: None,
                        attestations: Vec::new(),
                    },
                ),
            ],
            Vec::new(),
            &self.signing_ctx(),
        )
        .await?;
        tx.commit().await?;

        // The EHR is created with the settings-less default EHR_ACCESS
        // (default-open); seed the access cache so the first EHR-scoped request
        // is a hit, not a DB miss (the access gate runs on every such request).
        self.prewarm_ehr_access_open(ehr_id).await;

        // The RM `EHR` wire body is built from the commit results rather than
        // re-read through `ehr_summary`: the version identities are in
        // `Committed`, `time_created` came back from the INSERT, and a fresh EHR
        // indexes no folder hierarchy. It is stashed so a
        // `Prefer: return=representation` response needs no re-read.
        let body = self.ehr_object_from_committed(ehr_id, time_created, &committed.versions)?;
        self.created_ehr_repr.insert(ehr_id, body.clone()).await;
        let meta = ResourceMeta::new(ehr_id.to_string(), ehr_id.to_string())
            .with_last_modified(time_created);
        Ok(ServiceResponse::new(body, meta))
    }

    /// Assembles the RM `EHR` wire body for a just-created EHR from the
    /// CONTRIBUTION commit results, with no storage reads.
    ///
    /// The status and access version identities come from the
    /// [`Committed`](crate::versioning::change::Committed) rows (RM ehr master04
    /// §EHR Creation), the status ref carries its `OBJECT_VERSION_ID` (master06
    /// §Distributed Versioning), and a fresh EHR has no `directory` or `folders`
    /// (RM ehr master04 §Folders, 0..1). Both this and [`Self::ehr_summary`] go
    /// through [`ehr_object`], so they cannot drift apart.
    ///
    /// # Errors
    /// [`ServiceError::Internal`] when the commit results carry no `EHR_STATUS`
    /// or no `EHR_ACCESS` version — both are 1..1 on `EHR` (see [`ehr_object`]),
    /// and EHR creation commits both, so this cannot happen on the create path.
    fn ehr_object_from_committed(
        &self,
        ehr_id: EhrId,
        time_created: jiff::Timestamp,
        committed: &[crate::versioning::change::Committed],
    ) -> Result<Value, ServiceError> {
        let vo_of = |kind: Kind| {
            committed
                .iter()
                .find(|c| c.kind == kind)
                .map(|c| c.vo_id.to_string())
        };
        ehr_object(
            &self.effective_system_id(),
            ehr_id,
            vo_of(Kind::EhrStatus).as_deref(),
            vo_of(Kind::EhrAccess).as_deref(),
            time_created,
            &[],
        )
    }

    /// Find an EHR by the subject its current `EHR_STATUS` names (external ref
    /// `id.value` + `namespace`), recording the resolution as an access.
    ///
    /// Served from the promoted `ehr.subject_*` columns (unique per subject —
    /// `ehr_subject_uq`), on the CLINICAL pool: the value matched is the
    /// opaque subject pseudonym [`crate::privacy::PrivacyPolicy::check_subject`]
    /// admits, so this resolves a pseudonym and consults no linkage map. The
    /// access event is emitted all the same, and in the `linkage` domain,
    /// because "who resolved a subject to an EHR" is the question that domain
    /// exists to answer and it is worth an answer wherever it is asked.
    ///
    /// NOTE: the DB constraint narrows the SM `List<EHR_SUMMARY>` of
    /// `i_ehr_service.adoc` §`get_ehrs_for_subject` to at most one, per the
    /// one-EHR-per-subject rule (RM ehr master04 §EHR Status; CNF
    /// `create_ehr-two_ehrs_same_patient` expects `409` on a second EHR).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when no EHR names the subject;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn ehr_by_subject(
        &self,
        subject_id: &str,
        namespace: &str,
    ) -> Result<ServiceResponse, ServiceError> {
        let found =
            crate::storage::ehr_repo::ehr_id_by_subject(&self.pool, subject_id, namespace).await?;
        self.emit_subject_resolution(namespace, found)?;
        let ehr_id = found.ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR for subject {subject_id}@{namespace}"),
            )
        })?;
        self.ehr_summary(ehr_id).await
    }

    /// Record one subject-to-EHR resolution on the access log.
    ///
    /// The record names the subject NAMESPACE and never the subject id: the id
    /// is a pseudonym by policy, but a deployment that declares no
    /// `[privacy] subject_namespaces` can still carry a real identifier there,
    /// and an access log is the last place that should be the copy that
    /// outlives it. A miss is recorded like a hit — it says someone asked
    /// whether this deployment holds a record for that subject.
    fn emit_subject_resolution(
        &self,
        namespace: &str,
        resolved: Option<EhrId>,
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        event.domain = AccessDomain::Linkage;
        event.object_id = Some(format!("subject-namespace:{namespace}"));
        event.ehr_id = resolved.map(|id| id.to_string());
        event.result_count = Some(u64::from(resolved.is_some()));
        event.purpose = crate::system_log::access_context::current_purpose();
        if let Some(committer) = crate::service::committer::current_committer() {
            event.user_id = committer.subject;
        }
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event).map_err(ServiceError::Unrecorded)
    }

    /// Build the canonical RM `EHR` object for an existing EHR, with its
    /// `ehr_id` metadata (the `ETag`/`Location` for `POST /ehr`). ITS-REST
    /// extension: the wire `GET /ehr/{id}` returns the RM `EHR`, not the SM
    /// `EHR_SUMMARY`.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR or its current `EHR_STATUS` does
    /// not exist; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn ehr_summary(
        &self,
        ehr_id: EhrId,
    ) -> Result<ServiceResponse, ServiceError> {
        // ONE statement for the whole representation — header, EHR_STATUS
        // identity, EHR_ACCESS ref and folder hierarchies. EHR.system_id is
        // IMMUTABLE after creation (arch master06 §System Identity): the stored
        // per-EHR value, never the live config.
        let read = crate::storage::ehr_repo::ehr_summary_read(&self.pool, ehr_id)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(CallStatusType::EhrIdDoesNotExist, format!("EHR {ehr_id}"))
            })?;
        let time_created = read.time_created;
        let status = read.status.ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR_STATUS for EHR {ehr_id}"),
            )
        })?;
        let folders: Vec<String> = read.folders.iter().map(ToString::to_string).collect();
        let body = ehr_object(
            &read.system_id,
            ehr_id,
            Some(&status.vo_id.to_string()),
            read.access_vo.map(|vo| vo.to_string()).as_deref(),
            time_created,
            &folders,
        )?;
        let meta = ResourceMeta::new(ehr_id.to_string(), ehr_id.to_string())
            .with_last_modified(time_created);
        Ok(ServiceResponse::new(body, meta))
    }

    /// SM `EHR_SUMMARY` (`ehr_summary.adoc`) — all six mandatory attributes.
    /// `system_id` is the stored `EHR.system_id`; `ehr_status` is the current
    /// bare `EHR_STATUS`; `composition_count` is the number of "(versioned)
    /// Compositions" — distinct versioned objects (`vo_id`), not versions.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR or its current `EHR_STATUS` does
    /// not exist; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn summarize_ehr(
        &self,
        ehr_id: EhrId,
    ) -> Result<EhrSummary, ServiceError> {
        let (stored_system_id, time_created) =
            crate::storage::ehr_repo::ehr_header(&self.pool, ehr_id)
                .await?
                .ok_or_else(|| {
                    ServiceError::sm(CallStatusType::EhrIdDoesNotExist, format!("EHR {ehr_id}"))
                })?;

        // Copy of EHR.ehr_status: the current EHR_STATUS (bare, with its uid).
        let ehr_status = self.status_at(ehr_id, None).await?.body;

        let contribution_count =
            crate::storage::version_repo::contribution::ehr_contribution_count(&self.pool, ehr_id)
                .await?;
        let composition_count =
            crate::storage::version_repo::meta::composition_count(&self.pool, ehr_id).await?;

        Ok(EhrSummary {
            ehr_id: ehr_id.to_string(),
            system_id: stored_system_id,
            ehr_status,
            time_created: time_created.to_string(),
            contribution_count,
            composition_count,
        })
    }
}

/// An `OBJECT_REF` to a version CONTAINER held in this system.
///
/// The referenced object IS the container, so its `id` is the container's
/// `HIER_OBJECT_ID` (BASE `base_types` `master05-identification_package.adoc`
/// §References: `OBJECT_REF.id` is the "Globally unique id of an object"), and
/// `namespace` is `"local"` (same section: `"local"` is the namespace of the
/// containing system).
///
/// # Errors
/// [`VersionIdError`] when the stored container key is not a well-formed
/// `HIER_OBJECT_ID` (BASE `master05-identification_package.adoc` §Syntaxes).
fn container_ref(rm_type: &str, vo_id: &str) -> Result<ObjectRef, VersionIdError> {
    Ok(ObjectRef::ObjectRef(ObjectRefData {
        namespace: "local".to_owned(),
        r#type: rm_type.to_owned(),
        id: ObjectId::HierObjectId(hier_object_id(vo_id)?),
    }))
}

/// Assemble the canonical RM `EHR` wire body from the identities that make it
/// up — the single builder behind both the create path
/// ([`FerroEhrService::ehr_object_from_committed`]) and the read path
/// ([`FerroEhrService::ehr_summary`]), so the two cannot drift apart.
///
/// Built as the generated [`Ehr`] and serialized through the native canonical
/// codec, so `_type` comes first and the attribute order is the BMM's own. The
/// mandatory `EHR_STATUS` / `EHR_ACCESS` references are 1..1 on the RM class
/// (RM ehr `ehr.adoc` invariants `Ehr_status_valid` / `Ehr_access_valid`), and
/// the generated type makes that structural: an EHR missing either cannot be
/// constructed. `contributions` and `compositions` stay empty — the codec omits
/// an empty list, so the served body carries neither master list (they are
/// unbounded and the wire has dedicated collection endpoints for both).
///
/// `directory` is `folders.item(1)` (RM ehr `ehr.adoc` invariant
/// `Directory_in_folders`), derived here rather than passed in, so the two can
/// never disagree.
///
/// # Errors
/// [`ServiceError::Internal`] when `status_vo` or `access_vo` is absent — an
/// EHR without either is RM-invalid and cannot be represented; a
/// [`VersionIdError`]-derived error when a stored container key or the
/// configured `system_id` is not a well-formed BASE identifier.
fn ehr_object(
    system_id: &str,
    ehr_id: EhrId,
    status_vo: Option<&str>,
    access_vo: Option<&str>,
    time_created: jiff::Timestamp,
    folder_vos: &[String],
) -> Result<Value, ServiceError> {
    // NOTE: the `ehr_status` ref names the version CONTAINER, per the normative
    // `Ehr_status_valid: ehr_status.type.is_equal("VERSIONED_EHR_STATUS")` (RM
    // ehr `ehr.adoc`), which wins over the non-normative ITS-REST example.
    let ehr_status = status_vo
        .map(|vo| container_ref("VERSIONED_EHR_STATUS", vo))
        .transpose()?;
    let ehr_access = access_vo
        .map(|vo| container_ref("VERSIONED_EHR_ACCESS", vo))
        .transpose()?;
    let (Some(ehr_status), Some(ehr_access)) = (ehr_status, ehr_access) else {
        return Err(ServiceError::exception(format!(
            "EHR {ehr_id} has no EHR_STATUS or no EHR_ACCESS version; both are \
             mandatory on the RM EHR class"
        )));
    };
    // EHR.folders (0..1): the LIVE hierarchies in rank order, each an
    // OBJECT_REF to a VERSIONED_FOLDER (RM ehr `ehr.adoc` invariant
    // `Folders_valid`; RM ehr master04 §Folders).
    let folders: Vec<ObjectRef> = folder_vos
        .iter()
        .map(|vo| container_ref("VERSIONED_FOLDER", vo))
        .collect::<Result<Vec<_>, _>>()?;
    let ehr = Ehr {
        system_id: hier_object_id(system_id)?,
        // The EHR id is a UUID by type, so the conversion is total.
        ehr_id: HierObjectId::from(ehr_id.0),
        contributions: openehr_base::containers::present(Vec::new()),
        ehr_status,
        ehr_access,
        compositions: openehr_base::containers::present(Vec::new()),
        directory: folders.first().cloned(),
        time_created: crate::versioning::audit::dv_date_time(&time_created),
        folders: openehr_base::containers::present(folders),
        tags: openehr_base::containers::present(Vec::new()),
    };
    Ok(openehr_its::json::to_canonical_value(&ehr))
}

/// Builds the `EHR_STATUS` a new EHR starts with (queryable, modifiable,
/// `PARTY_SELF`) — RM ehr master04 §EHR Creation.
///
/// `archetype_details` is mandatory here: `EHR_STATUS` is unconditionally an
/// archetype root (RM ehr `ehr_status.adoc` invariant `Is_archetype_root`),
/// and RM common `locatable.adoc` `Archetyped_valid: is_archetype_root xor
/// archetype_details = Void` makes a root without `archetype_details`
/// RM-invalid; at a root, `archetype_node_id` "is always the stringified
/// form of the `archetype_id` found in the `archetype_details` object"
/// (`locatable.adoc` §`archetype_node_id`).
pub(in crate::service) fn initial_ehr_status() -> Value {
    let status = EhrStatus {
        name: DvText::DvText(DvTextData {
            value: "EHR Status".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        }),
        archetype_node_id: DEFAULT_EHR_STATUS_ARCHETYPE.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(archetyped(DEFAULT_EHR_STATUS_ARCHETYPE)),
        feeder_audit: None,
        subject: PartySelf { external_ref: None },
        is_queryable: true,
        is_modifiable: true,
        other_details: None,
    };
    openehr_its::json::to_canonical_value(&status)
}

/// The archetype a server-minted default `EHR_STATUS` declares.
const DEFAULT_EHR_STATUS_ARCHETYPE: &str = "openEHR-EHR-EHR_STATUS.generic.v1";

/// A root `ARCHETYPED` block for a server-minted LOCATABLE (RM common
/// `archetyped.adoc`: `archetype_id` 1..1, `rm_version` 1..1 — the RM
/// release this server implements — the current RM `Generation`'s pin).
pub(in crate::service) fn archetyped(archetype_id: &str) -> Archetyped {
    Archetyped {
        archetype_id: ArchetypeId {
            value: archetype_id.to_owned(),
        },
        template_id: None,
        rm_version: openehr_rm::Generation::default().spec_version().to_owned(),
    }
}

// ── The SM I_EHR_SERVICE call surface ─────────────────────────────────────────

impl FerroEhrService {
    /// SM `I_EHR_SERVICE.has_ehr` — whether the EHR exists.
    ///
    /// # Errors
    /// [`SmError`] if the existence read fails (a missing EHR is `Ok(false)`).
    pub async fn has_ehr(&self, ehr_id: EhrId) -> Result<bool, SmError> {
        match self.ensure_ehr_exists(ehr_id).await {
            Ok(()) => Ok(true),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// SM `I_EHR_SERVICE.has_ehr_for_subject` — whether an EHR exists whose
    /// current `EHR_STATUS` names the subject.
    ///
    /// # Errors
    /// [`SmError`] if the subject lookup fails (no matching EHR is
    /// `Ok(false)`).
    pub async fn has_ehr_for_subject(&self, a_subject_id: SubjectRef) -> Result<bool, SmError> {
        match self
            .ehr_by_subject(&a_subject_id.id, &a_subject_id.namespace)
            .await
        {
            Ok(_) => Ok(true),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// SM `I_EHR_SERVICE.create_ehr` — create an EHR with a server-assigned id
    /// and the given (or default) `EHR_STATUS`, returning the new `ehr_id`.
    ///
    /// # Errors
    /// [`SmError`] when the status is structurally invalid (422-equivalent),
    /// the subject already owns an EHR (409-equivalent), or storage fails.
    pub async fn create_ehr(&self, an_ehr_status: Option<EhrStatus>) -> Result<EhrId, SmError> {
        // Boxed: the typed EHR_STATUS argument makes the bootstrap-commit
        // future wide (clippy `large_futures`).
        // NOTE: the SM precondition `an_ehr_status.subject = Void`
        // (`i_ehr_service.adoc` §create_ehr `Pre_no_subject`) is NOT enforced —
        // `POST /ehr` accepts a subject-bearing status, an accepted divergence.
        Ok(Box::pin(self.create_ehr_meta(an_ehr_status, None)).await?.0)
    }

    /// SM `I_EHR_SERVICE.create_ehr_with_id` — create an EHR under the
    /// caller-supplied id (`PUT /ehr/{ehr_id}`).
    ///
    /// # Errors
    /// [`SmError`] when the EHR already exists (409-equivalent), the status is
    /// invalid, the subject already owns an EHR, or storage fails.
    pub async fn create_ehr_with_id(
        &self,
        an_ehr_id: EhrId,
        an_ehr_status: Option<EhrStatus>,
    ) -> Result<EhrId, SmError> {
        // Boxed, as in `create_ehr` (clippy `large_futures`).
        Box::pin(self.create_ehr_with_id_meta(an_ehr_id, an_ehr_status, None)).await?;
        Ok(an_ehr_id)
    }

    /// SM `I_EHR_SERVICE.create_ehr_for_subject` — create an EHR whose
    /// `EHR_STATUS.subject` names the given subject.
    ///
    /// NOTE: no committal argument. ITS-REST 1.1.0 binds EHR creation to
    /// exactly two operations — `ehr_create` (`POST /ehr`) and
    /// `ehr_create_with_id` (`PUT /ehr/{ehr_id}`), both of which route through
    /// [`Self::create_ehr_meta`] / [`Self::create_ehr_with_id_meta`]; the
    /// subject-scoped SM creates have no wire binding and therefore no
    /// request headers to merge (the in-process caller is the FHIR ingest
    /// path, which commits under the server's own attribution).
    ///
    /// # Errors
    /// [`SmError`] when the subject already owns an EHR (409-equivalent), the
    /// status is invalid, or storage fails.
    pub async fn create_ehr_for_subject(
        &self,
        a_subject_id: SubjectRef,
        an_ehr_status: Option<Value>,
    ) -> Result<EhrId, SmError> {
        let ehr_id = EhrId::minted(self.stamp());
        let status = status_for_subject(
            an_ehr_status.unwrap_or_else(initial_ehr_status),
            &a_subject_id,
        );
        self.commit_new_ehr(ehr_id, status, None).await?;
        Ok(ehr_id)
    }

    /// SM `I_EHR_SERVICE.create_ehr_for_subject_with_id` — subject-scoped
    /// creation under a caller-supplied EHR id.
    ///
    /// No committal argument, for the reason given on
    /// [`Self::create_ehr_for_subject`].
    ///
    /// # Errors
    /// [`SmError`] when the EHR already exists, the subject already owns an
    /// EHR, the status is invalid, or storage fails.
    pub async fn create_ehr_for_subject_with_id(
        &self,
        an_ehr_id: EhrId,
        a_subject_id: SubjectRef,
        an_ehr_status: Option<Value>,
    ) -> Result<EhrId, SmError> {
        let status = status_for_subject(
            an_ehr_status.unwrap_or_else(initial_ehr_status),
            &a_subject_id,
        );
        self.commit_new_ehr(an_ehr_id, status, None).await?;
        Ok(an_ehr_id)
    }

    /// SM `I_EHR_SERVICE.get_ehr` — the `EHR_SUMMARY` of an EHR.
    ///
    /// # Errors
    /// [`SmError`] when the EHR does not exist (404-equivalent) or a read
    /// fails.
    pub async fn get_ehr(&self, an_ehr_id: EhrId) -> Result<EhrSummary, SmError> {
        Ok(self.summarize_ehr(an_ehr_id).await?)
    }

    /// SM `I_EHR_SERVICE.get_ehrs_for_subject` — the `EHR_SUMMARY` list for a
    /// subject (≤1 under the one-EHR-per-subject rule; see the note on
    /// `Self::ehr_by_subject`).
    ///
    /// # Errors
    /// [`SmError`] if a read fails, or when a found EHR body carries no
    /// `ehr_id` (an internal invariant violation).
    pub async fn get_ehrs_for_subject(
        &self,
        a_subject_id: SubjectRef,
    ) -> Result<Vec<EhrSummary>, SmError> {
        // one EHR per subject narrows the List to ≤1 (see `ehr_by_subject`).
        match self
            .ehr_by_subject(&a_subject_id.id, &a_subject_id.namespace)
            .await
        {
            Ok(resp) => {
                let ehr_id = resp
                    .body
                    .pointer("/ehr_id/value")
                    .and_then(Value::as_str)
                    .and_then(|v| Uuid::parse_str(v).ok())
                    .map(EhrId)
                    .ok_or_else(|| SmError::exception("EHR body carries no ehr_id"))?;
                Ok(vec![self.summarize_ehr(ehr_id).await?])
            }
            Err(ServiceError::NotFound(_)) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// The canonical RM `EHR` wire object (`GET /ehr/{ehr_id}` — an ITS-REST
    /// shape, not the SM `EHR_SUMMARY`).
    ///
    /// # Errors
    /// [`SmError`] when the EHR or its current `EHR_STATUS` does not exist, or
    /// a read fails.
    pub async fn ehr_object(&self, an_ehr_id: EhrId) -> Result<Value, SmError> {
        Ok(self.ehr_summary(an_ehr_id).await?.body)
    }

    /// The RM `EHR` wire object for a just-created EHR — the
    /// `Prefer: return=representation` body of `POST /ehr` / `PUT /ehr/{id}`.
    ///
    /// # Errors
    /// [`SmError`] when the fallback full read finds no such EHR, or a read
    /// fails.
    pub async fn ehr_created_object(&self, an_ehr_id: EhrId) -> Result<Value, SmError> {
        // Serve the create-time representation from the stash the commit path
        // populated (built from `Committed`, no re-read); a popped entry cannot
        // be reused. Fall back to a full read when the entry has been evicted
        // (short TTL) or the EHR was created off this path (import/clone).
        if let Some(body) = self.created_ehr_repr.remove(&an_ehr_id).await {
            return Ok(body);
        }
        self.ehr_object(an_ehr_id).await
    }

    /// The canonical RM `EHR` wire object located by subject
    /// (`GET /ehr?subject_id=…&subject_namespace=…`).
    ///
    /// # Errors
    /// [`SmError`] when no EHR names the subject (404-equivalent) or a read
    /// fails.
    pub async fn ehr_object_for_subject(
        &self,
        subject_id: &str,
        subject_namespace: &str,
    ) -> Result<Value, SmError> {
        Ok(self
            .ehr_by_subject(subject_id, subject_namespace)
            .await?
            .body)
    }
}

// ── ITS-REST create-response adapter (adapter-support extension) ──────────────
//
// The SM creates return only the new `ehr_id`, but ITS-REST
// `Requests_and_responses.md` §"`ETag` and Last-Modified" mandates both headers,
// the `ETag` being `EHR.ehr_id.value` and `Last-Modified` the creating
// CONTRIBUTION's commit time, so these siblings also return a [`ResourceMeta`]
// (no openEHR spec governs the envelope — our own design).

impl FerroEhrService {
    /// [`Self::create_ehr`] returning the new `ehr_id` together with the
    /// created EHR's [`ResourceMeta`] (the `ETag`/`Location` id + the creation
    /// instant).
    ///
    /// `committal` is the `POST /ehr` request's `openehr-version` /
    /// `openehr-audit-details` metadata (see `Self::commit_new_ehr`).
    ///
    /// # Errors
    /// [`SmError`] when the status is structurally invalid (422-equivalent),
    /// a committal `change_type`/`lifecycle_state` is illegal for a creation
    /// (400/422-equivalent), the subject already owns an EHR
    /// (409-equivalent), or storage fails.
    pub async fn create_ehr_meta(
        &self,
        an_ehr_status: Option<EhrStatus>,
        committal: Option<&crate::service::version_update::Committal>,
    ) -> Result<(EhrId, ResourceMeta), SmError> {
        let ehr_id = EhrId::minted(self.stamp());
        let meta = self
            .create_ehr_with_id_meta(ehr_id, an_ehr_status, committal)
            .await?;
        Ok((ehr_id, meta))
    }

    /// [`Self::create_ehr_with_id`] returning the created EHR's
    /// [`ResourceMeta`].
    ///
    /// `committal` is the `PUT /ehr/{ehr_id}` request's `openehr-version` /
    /// `openehr-audit-details` metadata (see `Self::commit_new_ehr`).
    ///
    /// # Errors
    /// [`SmError`] when the EHR already exists (409-equivalent), the status is
    /// invalid, a committal `change_type`/`lifecycle_state` is illegal for a
    /// creation (400/422-equivalent), the subject already owns an EHR, or
    /// storage fails.
    pub async fn create_ehr_with_id_meta(
        &self,
        an_ehr_id: EhrId,
        an_ehr_status: Option<EhrStatus>,
        committal: Option<&crate::service::version_update::Committal>,
    ) -> Result<ResourceMeta, SmError> {
        // see `create_ehr` — `Pre_no_subject` deliberately not enforced.
        let status = an_ehr_status.map_or_else(initial_ehr_status, |s| {
            openehr_its::json::to_canonical_value(&s)
        });
        let created = self.commit_new_ehr(an_ehr_id, status, committal).await?;
        created.meta.ok_or_else(|| {
            SmError::exception(format!(
                "EHR {an_ehr_id} was created without resource metadata"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::initial_ehr_status;

    /// The default `EHR_STATUS` must be a valid structure root for the storage
    /// codec (one root node — the decomposition granularity of
    /// `crate::storage::codec::decompose`).
    #[test]
    fn default_status_decomposes() {
        let rows = crate::storage::codec::decompose(initial_ehr_status()).expect("decompose");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rm_type, "EHR_STATUS");
    }
}
