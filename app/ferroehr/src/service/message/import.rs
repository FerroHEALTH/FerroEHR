// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! EHR-Extract import (SM `I_EHR_EXTRACT_SERVICE.import_ehr` /
//! `import_ehr_extract`), the inverse of export: each received
//! `X_VERSIONED_*`'s `ORIGINAL_VERSION`s are replayed into the local store as
//! `IMPORTED_VERSION`s.
//!
//! Spec: SM `docs/specs/openehr/SM/docs/UML/classes/i_ehr_extract_service.adoc`
//! (`import_ehr` / `import_ehr_extract`); RM common
//! `master06-change_control_package.adoc` §Copying / §Distributed Versioning
//! (`IMPORTED_VERSION`, Cases 1/2/3, `creating_system_id`, local commit-time
//! rule) and its `org.openehr.rm.common.imported_version.adoc` class table; the
//! demographics-chapter routing follows
//! `docs/specs/openehr/RM/docs/ehr_extract/master09-semantics.adoc` §Creation
//! Semantics.
//!
//! This module owns the parse and dispatch: turning a received `EXTRACT` into
//! the [`ImportContainer`] sets and choosing clone against append. The
//! `IMPORTED_VERSION` replay lives in the change-control engine
//! ([`crate::versioning::import::commit_import`] /
//! [`crate::versioning::import::commit_demographic_import`]), which records the
//! local act of committal at `249|creation|` while preserving each wrapped
//! original's identity, `commit_audit`, lifecycle, data, signature and
//! attestations verbatim (master06 §Copying: "the `ORIGINAL_VERSION` instance is
//! never modified").
//!
//! `import_ehr` clones a whole EHR into an empty target (a caller-fixed id, else
//! the source EHR id reused — master06 §Copying Case 1; RM ehr §"EHR Identifier
//! Allocation"); `import_ehr_extract` lands versioned objects into an existing
//! EHR (Cases 2 and 3).
//!
//! NOTE: imported COMPOSITION content is stored verbatim without re-linking its
//! operational template (`version.template_id` stays NULL) or re-running
//! WebTemplate and RM validation, so the OPT must already be provisioned in the
//! target through the DEFINITION API (master06 §Copying, the rule the admin
//! dump/load path follows too).
//!
//! Re-validation on import is nowhere required (adjudicated on #2988):
//! master06 §The Copy Operation keeps the wrapped original "a faithful copy of
//! its original", `commit_imported_version` is the only `VERSIONED_OBJECT`
//! commit function with no `Pre` clause, SM `i_ehr_extract_service.adoc`
//! declares the import operations without preconditions, and BASE
//! `master10-archetypes.adoc` §Validation during Data Capture makes archetype
//! validation in an import service a "can", never a must.
//!
//! An imported EHR is a full local EHR: the promoted `ehr` columns are re-derived
//! from the landed `EHR_STATUS`
//! ([`crate::service::FerroEhrService::resync_promoted_columns`]), so the clone
//! is found by the subject lookup (SM `I_EHR_SERVICE.get_ehrs_for_subject`) and
//! bound by the one-EHR-per-subject rule (RM ehr master04 §EHR Status) exactly
//! like a created EHR, which makes importing a clone of an already-held subject a
//! conflict. Where the extract carries no `EHR_ACCESS`, the whole-EHR clone
//! completes the mandatory 1..1 `EHR.ehr_access` (RM ehr `ehr.adoc`
//! `Ehr_access_valid`; master04 §EHR Creation) with the local default
//! ([`crate::service::FerroEhrService::commit_default_ehr_access`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/TDD/dump-load compose over \
              verbatim stored content (RM common master06 §Copying)"
)]

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::{CallStatusType, SmError};
use crate::system_log::categories::AccessedContent;
use crate::system_log::event::EventActionCode;
use crate::versioning::Kind;
use crate::versioning::audit::{change_type, change_type_code};
use crate::versioning::import::{
    ImportContainer, ImportVersion, commit_demographic_import, commit_import,
};
use crate::versioning::lifecycle;
use crate::versioning::lifecycle::lifecycle_state_code;
use crate::versioning::object_version_id::parse_object_version_id;
use openehr_base::prelude::ObjectRef;
use openehr_rm::v1_2::common::generic::audit_details::AuditDetails;
use openehr_rm::v1_2::ehr_extract::common::extract::Extract;

impl FerroEhrService {
    /// SM `import_ehr(an_ehr_id[0..1], an_extract)` — clone a whole EHR into an
    /// empty target (master06 §Copying Case 1). The target id is the caller's
    /// fixed id (the SM's "same patient in other EHR services" case), else the
    /// source EHR id reused (master06 §Copying Case 1: the newly created EHR
    /// re-uses the source EHR identifier; RM ehr §"EHR Identifier Allocation").
    /// A completed import is audited for non-repudiation (inbound →
    /// `EventActionCode::Create`).
    ///
    /// Returns the id of the EHR the clone landed under — the caller's fixed
    /// id, or the source id the extract carried. The SM operation itself
    /// returns nothing, but the created EHR's identity is not otherwise
    /// derivable by a caller that supplied none, and a creating call that
    /// cannot name what it created is unusable over a wire.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — the extract carries no `EHR_STATUS`
    ///   versioned object, carries duplicate singleton containers, names no
    ///   source EHR id when `an_ehr_id` is absent, or any content item /
    ///   `ORIGINAL_VERSION` is malformed (see the parse errors in this module).
    /// - `ehr_create_fail_duplicate_id` — an EHR with the target id already
    ///   exists (`import_ehr` requires an empty target).
    /// - `Conflict` (`409`) — the imported `EHR_STATUS` names a subject another
    ///   EHR in this repository already holds (one EHR per subject — RM ehr
    ///   master04 §EHR Status).
    /// - `exception` — a database/replay fault mid-transaction (rolled back).
    pub async fn import_ehr(
        &self,
        an_ehr_id: Option<EhrId>,
        an_extract: Extract,
    ) -> Result<EhrId, SmError> {
        let (containers, parties) = parse_import_containers(&an_extract)?;
        // A whole-EHR clone must carry an EHR_STATUS (EHR.ehr_status 1..1, RM
        // ehr §"EHR Creation") — the target could not otherwise be a valid EHR.
        if !containers.iter().any(|c| c.kind == Kind::EhrStatus) {
            return Err(SmError::precondition(
                "import_ehr requires the extract to carry an EHR_STATUS versioned object",
            ));
        }
        reject_duplicate_singleton_containers(&containers)?;

        let ehr_id = match an_ehr_id {
            Some(id) => id,
            None => source_ehr_id(&an_extract)?,
        };

        let audit = self.audit(change_type::CREATION, "EHR Extract import");
        let signing = self.signing_ctx();
        self.import_parties(&signing, &audit, parties).await?;

        let mut tx = self.pool.begin().await.map_err(ServiceError::from)?;
        // Into an *empty* target: a duplicate EHR id is
        // `ehr_create_fail_duplicate_id`. The EHR is created locally, so its
        // immutable `system_id` is ours (master06 §Distributed Versioning — the
        // committing system is the local one).
        // NOTE: no openEHR spec governs the storage SQL — our own design: the
        // clone-target EHR row is inserted here (master06 §Copying Case-1)
        // rather than via `create_ehr`, which would mint a fresh EHR_STATUS.
        let inserted =
            sqlx::query("INSERT INTO ehr (id, system_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(ehr_id)
                .bind(self.effective_system_id())
                .execute(&mut *tx)
                .await
                .map_err(ServiceError::from)?;
        if inserted.rows_affected() == 0 {
            return Err(SmError::new(
                CallStatusType::EhrCreateFailDuplicateId,
                format!("EHR {ehr_id} already exists; import_ehr requires an empty target"),
            ));
        }
        let touches_ehr_access = containers.iter().any(|c| c.kind == Kind::EhrAccess);
        let landed: Vec<VoId> = containers.iter().map(|c| c.vo_id).collect();
        commit_import(&mut tx, &signing, &self.privacy, ehr_id, &audit, containers).await?;
        // An EHR is created as "a root EHR object, an EHR Status object, and an
        // EHR Access object" (RM ehr master04 §EHR Creation) and `EHR.ehr_access`
        // is 1..1 (`ehr.adoc` invariant `Ehr_access_valid`). An extract that
        // carried no EHR_ACCESS therefore leaves the clone incomplete — commit
        // the local default in this same transaction.
        if !touches_ehr_access {
            self.commit_default_ehr_access(&mut tx, ehr_id, "EHR Extract import: default access")
                .await?;
        }
        // The clone landed the EHR_STATUS directly, bypassing the service's
        // sync hook, so the promoted `ehr` columns — subject, `is_queryable`,
        // `is_modifiable` — are re-derived from the stored current status (RM
        // ehr master04 §EHR Status / §EHR Active Status).
        self.resync_promoted_columns(&mut tx, ehr_id).await?;
        tx.commit().await.map_err(ServiceError::from)?;
        // An imported EHR_ACCESS version changes the EHR's access policy, so the
        // cached settings are evicted (RM ehr master04 §EHR Access); a
        // bootstrapped default carries none, so that entry is seeded open.
        if touches_ehr_access {
            self.invalidate_ehr_access(ehr_id).await;
        } else {
            self.prewarm_ehr_access_open(ehr_id).await;
        }
        let content = self.landed_content(&landed).await;
        self.emit_extract_audit(ehr_id, EventActionCode::Create, Some(content))?;
        Ok(ehr_id)
    }

    /// SM `import_ehr_extract(an_ehr_id, an_extract)` — land versioned objects
    /// into an existing EHR (master06 §Copying Cases 2/3). A completed import
    /// is audited for non-repudiation (inbound → `EventActionCode::Create`).
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` — no EHR with `an_ehr_id` (`has_ehr` false).
    /// - `precondition_violation` (`400`) — duplicate singleton containers, or
    ///   any malformed content item / `ORIGINAL_VERSION` (see the parse errors
    ///   in this module).
    /// - `Conflict` (`409`) — the EHR already holds an `EHR_STATUS`/`EHR_ACCESS`
    ///   under a different object id (an EHR holds at most one of each; RM ehr,
    ///   EHR class 1..1), or an imported `EHR_STATUS` names a subject another
    ///   EHR already holds (one EHR per subject — RM ehr master04 §EHR Status).
    /// - `exception` — a database/replay fault mid-transaction (rolled back).
    pub async fn import_ehr_extract(
        &self,
        an_ehr_id: EhrId,
        an_extract: Extract,
    ) -> Result<(), SmError> {
        if !self.import_ehr_exists(an_ehr_id).await? {
            return Err(SmError::ehr_not_found(format!(
                "no EHR with id {an_ehr_id}"
            )));
        }
        let (containers, parties) = parse_import_containers(&an_extract)?;
        reject_duplicate_singleton_containers(&containers)?;

        // A *new* singleton container cannot be added when the EHR already holds
        // one of that kind under a different object id (EHR.ehr_status /
        // ehr_access 1..1 — RM ehr, EHR class). A matching object id is an
        // append (master06 §Copying Case 3), handled in `commit_import`. FOLDERs
        // are NOT singletons: each first-received hierarchy joins `EHR.folders`
        // (RM ehr master04 §Folders).
        for container in &containers {
            if matches!(container.kind, Kind::EhrStatus | Kind::EhrAccess)
                && let Some((existing_vo, _)) = self.current_vo(an_ehr_id, container.kind).await?
                && existing_vo != container.vo_id
            {
                return Err(ServiceError::conflict(format!(
                    "EHR {an_ehr_id} already has a {} ({existing_vo}); cannot import a \
                     second one ({})",
                    container.kind.as_str(),
                    container.vo_id
                ))
                .into());
            }
        }

        let audit = self.audit(change_type::CREATION, "EHR Extract import");
        let signing = self.signing_ctx();
        self.import_parties(&signing, &audit, parties).await?;

        let mut tx = self.pool.begin().await.map_err(ServiceError::from)?;
        let touches_ehr_access = containers.iter().any(|c| c.kind == Kind::EhrAccess);
        let landed: Vec<VoId> = containers.iter().map(|c| c.vo_id).collect();
        commit_import(
            &mut tx,
            &signing,
            &self.privacy,
            an_ehr_id,
            &audit,
            containers,
        )
        .await?;
        // An imported EHR_STATUS version can change the current status, subject
        // included (Copying Case 3 append), so the promoted `ehr` columns are
        // re-derived from it (RM ehr master04 §EHR Status / §EHR Active Status).
        self.resync_promoted_columns(&mut tx, an_ehr_id).await?;
        tx.commit().await.map_err(ServiceError::from)?;
        // An imported EHR_ACCESS version changes the EHR's access policy — evict
        // the cached settings the access gate consults (RM ehr master04 §EHR
        // Access; the settings are change-controlled).
        if touches_ehr_access {
            self.invalidate_ehr_access(an_ehr_id).await;
        }
        let content = self.landed_content(&landed).await;
        self.emit_extract_audit(an_ehr_id, EventActionCode::Create, Some(content))?;
        Ok(())
    }

    /// What an import landed, for its access record's EHDS priority category
    /// (Annex II 3.2(c), `docs/law/eu/ehds/text.html`): the newest content of
    /// each received versioned object.
    ///
    /// The import has committed by then, so a failed read is logged and the
    /// record is written `unclassified` rather than failing a completed import.
    async fn landed_content(&self, landed: &[VoId]) -> AccessedContent {
        match self.audit_content_of_objects(landed).await {
            Ok(content) => AccessedContent::Objects(content),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "reading the categories of an imported Extract failed; the access \
                     record is written unclassified"
                );
                AccessedContent::Objects(Vec::new())
            }
        }
    }

    /// Replay the extract's demographics chapter into the demographic domain,
    /// in a transaction of its own.
    ///
    /// It cannot ride the clinical transaction: the two domains are separate
    /// schemas reached by separate pools, which is what keeps the clinical
    /// record and the identity of its subject apart, and one transaction spans
    /// one connection. Parties therefore go FIRST and commit on their own. That
    /// ordering is the recoverable one: `commit_demographic_import` skips a
    /// container already held locally (parties are shared continuants), so a
    /// clinical failure afterwards leaves parties nothing references and a
    /// retry of the whole import completes cleanly, where the reverse order
    /// would leave an imported EHR whose parties are missing and whose retry is
    /// refused as a duplicate.
    ///
    /// # Errors
    /// Whatever [`commit_demographic_import`] rejects, plus a database fault
    /// opening or committing the demographic transaction.
    async fn import_parties(
        &self,
        signing: &crate::versioning::SigningCtx<'_>,
        audit: &crate::versioning::audit::AuditInput,
        parties: Vec<ImportContainer>,
    ) -> Result<(), ServiceError> {
        if parties.is_empty() {
            return Ok(());
        }
        let mut tx = self.demographic_pool.begin().await?;
        commit_demographic_import(&mut tx, signing, &self.privacy, audit, parties).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether an EHR with `ehr_id` exists (the `has_ehr` precondition of
    /// `import_ehr_extract`; `i_ehr_extract_service.adoc`).
    async fn import_ehr_exists(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ehr WHERE id = $1)")
                .bind(ehr_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}

/// Reverse of the export `X_VERSIONED_*` mapping: the versioned-object [`Kind`]
/// a wrapper carries, for the EHR-scoped kinds an import replays.
/// (`X_VERSIONED_PARTY` demographics-chapter content is routed to the
/// demographic repository before this dispatch; a bare `X_VERSIONED_OBJECT`
/// wrapper is not importable — master05 class tables.)
fn kind_from_x_versioned(xtype: &str) -> Option<Kind> {
    match xtype {
        "X_VERSIONED_COMPOSITION" => Some(Kind::Composition),
        "X_VERSIONED_EHR_STATUS" => Some(Kind::EhrStatus),
        "X_VERSIONED_EHR_ACCESS" => Some(Kind::EhrAccess),
        "X_VERSIONED_FOLDER" => Some(Kind::Folder),
        _ => None,
    }
}

/// The source EHR id an `import_ehr` clone reuses when no fixed id is given —
/// `EXTRACT_SPEC.manifest.entities[0].ehr_id` (a whole-EHR export always names
/// it). master06 §Copying Case 1.
fn source_ehr_id(extract: &Extract) -> Result<EhrId, SmError> {
    let raw = extract
        .specification
        .as_ref()
        .and_then(|s| s.manifest.entities.first())
        .and_then(|e| e.ehr_id.as_deref())
        .ok_or_else(|| {
            SmError::precondition(
                "import_ehr without a fixed ehr_id requires the extract's EXTRACT_SPEC to \
                 name the source EHR id (specification.manifest.entities[0].ehr_id)",
            )
        })?;
    #[expect(
        clippy::map_err_ignore,
        reason = "the mapped error already echoes the rejected token; the \
                  discarded `uuid::Error` adds only its own wording, which \
                  is not part of the wire contract"
    )]
    raw.parse()
        .map_err(|_| SmError::precondition(format!("source ehr_id {raw:?} is not a UUID")))
}

/// An EHR holds at most one of each singleton versioned object (`EHR_STATUS`,
/// `EHR_ACCESS` — RM ehr, EHR class `ehr_status`/`ehr_access` 1..1); an extract
/// carrying two distinct containers of one such kind cannot be imported. FOLDER
/// hierarchies are unbounded (`EHR.folders`, RM ehr master04 §Folders), so an
/// extract may carry several.
fn reject_duplicate_singleton_containers(containers: &[ImportContainer]) -> Result<(), SmError> {
    for singleton in [Kind::EhrStatus, Kind::EhrAccess] {
        if containers.iter().filter(|c| c.kind == singleton).count() > 1 {
            return Err(SmError::precondition(format!(
                "extract carries more than one {} versioned object; an EHR holds at most one",
                singleton.as_str()
            )));
        }
    }
    Ok(())
}

/// Parse a received `EXTRACT` into the sets of versioned objects to import,
/// grouped by cloned `vo_id` (the received `uid.object_id()` — master06
/// §Copying): the EHR-owned containers plus any demographics-chapter parties
/// (`X_VERSIONED_PARTY`, landed into the demographic repository). Each content
/// item's `X_VERSIONED_*` wrapper contributes its `ORIGINAL_VERSION`s to one
/// [`ImportContainer`]; branch / multi-system version trees are first-class
/// (master06 §Distributed Versioning).
fn parse_import_containers(
    extract: &Extract,
) -> Result<(Vec<ImportContainer>, Vec<ImportContainer>), SmError> {
    let value = openehr_its::json::to_canonical_value(extract);
    let empty: Vec<Value> = Vec::new();
    let mut by_container: BTreeMap<VoId, ImportContainer> = BTreeMap::new();
    let mut parties: BTreeMap<VoId, ImportContainer> = BTreeMap::new();

    for chapter in value
        .get("chapters")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
    {
        for item in chapter
            .get("items")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
        {
            let Some(xver) = content_item_payload(item)? else {
                continue;
            };
            let xtype = xver
                .get("_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Demographics-chapter parties land in the demographic repository
            // (master09-semantics.adoc §Creation Semantics demographics
            // chapter); each version's PARTY subtype fixes the container kind.
            if xtype == "X_VERSIONED_PARTY" {
                collect_party_versions(xver, &mut parties)?;
                continue;
            }
            let kind = kind_from_x_versioned(xtype).ok_or_else(|| {
                SmError::precondition(format!(
                    "cannot import {xtype:?} through the EHR surface (only COMPOSITION / \
                     EHR_STATUS / EHR_ACCESS / FOLDER / demographics-chapter PARTYs)"
                ))
            })?;
            collect_content_versions(xver, kind, &mut by_container)?;
        }
    }
    Ok((
        by_container.into_values().collect(),
        parties.into_values().collect(),
    ))
}

/// The `X_VERSIONED_*` wrapper one `EXTRACT_CONTENT_ITEM` carries, or `None`
/// for an entry with no versioned content (a masked item, a folder structure
/// entry).
///
/// `EXTRACT_CONTENT_ITEM.Item_validity` is `is_masked xor item /= Void`
/// (`extract_content_item.adoc`): a masked wrapper carries no item, an
/// unmasked one must.
///
/// # Errors
/// [`SmError::precondition`] for generic (ISO 13606 / CDA) content — NOTE:
/// `master06-generic_extract_package.adoc` `GENERIC_CONTENT_ITEM` is outside
/// this CDR's import scope — or for either half of an `Item_validity`
/// violation.
fn content_item_payload(item: &Value) -> Result<Option<&Value>, SmError> {
    match item.get("_type").and_then(Value::as_str) {
        Some("OPENEHR_CONTENT_ITEM") => {}
        Some("GENERIC_CONTENT_ITEM") => {
            return Err(SmError::precondition(
                "generic (ISO 13606 / CDA) content import is not supported",
            ));
        }
        // A folder structure entry carries no versioned content.
        _ => return Ok(None),
    }
    let is_masked = item
        .get("is_masked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match (item.get("item"), is_masked) {
        // Masked-out content — nothing to import.
        (None, true) => Ok(None),
        (None, false) => Err(SmError::precondition(
            "EXTRACT_CONTENT_ITEM carries no item and is not masked \
             (Item_validity: is_masked xor item present)",
        )),
        (Some(_), true) => Err(SmError::precondition(
            "EXTRACT_CONTENT_ITEM is masked but carries an item \
             (Item_validity: is_masked xor item present)",
        )),
        (Some(xver), false) => Ok(Some(xver)),
    }
}

/// Collects the versions of one `X_VERSIONED_PARTY` wrapper into the
/// demographic containers, keyed by cloned `vo_id`.
///
/// # Errors
/// [`SmError::precondition`] when a version's data is not a PARTY subtype, and
/// the [`parse_imported_version`] rejections.
fn collect_party_versions(
    xver: &Value,
    parties: &mut BTreeMap<VoId, ImportContainer>,
) -> Result<(), SmError> {
    let empty: Vec<Value> = Vec::new();
    for ov in xver
        .get("versions")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
    {
        let party_type = ov
            .pointer("/data/_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(kind) = Kind::from_type(party_type).filter(|k| k.is_demographic()) else {
            return Err(SmError::precondition(format!(
                "X_VERSIONED_PARTY version data must be a PARTY subtype, \
                 got {party_type:?}"
            )));
        };
        let (vo_id, version) = parse_imported_version(ov)?;
        push_version(parties, vo_id, kind, version);
    }
    Ok(())
}

/// Collects the versions of one EHR-owned `X_VERSIONED_*` wrapper into the
/// content containers, keyed by cloned `vo_id`.
///
/// `creating_system_id` is per VERSION: a copied tree legitimately mixes
/// source-trunk versions with branch modifications made by other systems
/// (master06 §Distributed Versioning).
///
/// # Errors
/// [`SmError::precondition`] when one `vo_id` arrives under two kinds, and the
/// [`parse_imported_version`] rejections.
fn collect_content_versions(
    xver: &Value,
    kind: Kind,
    by_container: &mut BTreeMap<VoId, ImportContainer>,
) -> Result<(), SmError> {
    let empty: Vec<Value> = Vec::new();
    for ov in xver
        .get("versions")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
    {
        let (vo_id, version) = parse_imported_version(ov)?;
        if let Some(existing) = by_container.get(&vo_id)
            && existing.kind != kind
        {
            return Err(SmError::precondition(format!(
                "versioned object {vo_id} appears as both {} and {}",
                existing.kind.as_str(),
                kind.as_str()
            )));
        }
        push_version(by_container, vo_id, kind, version);
    }
    Ok(())
}

/// Appends one version to its container, creating the container on first
/// receipt.
fn push_version(
    containers: &mut BTreeMap<VoId, ImportContainer>,
    vo_id: VoId,
    kind: Kind,
    version: ImportVersion,
) {
    match containers.get_mut(&vo_id) {
        Some(existing) => existing.versions.push(version),
        None => {
            containers.insert(
                vo_id,
                ImportContainer {
                    vo_id,
                    kind,
                    versions: vec![version],
                },
            );
        }
    }
}

/// The wrapped original's `VERSION.commit_audit` (1..1), validated and returned
/// as the canonical fragment to store verbatim — including its concrete class:
/// a version committed elsewhere with an `ATTESTATION` commit audit keeps that
/// class and its own attributes here (RM common master06 §Attestation;
/// §Copying: "the received instance is never modified"), so a re-export renders
/// what was imported.
///
/// Validity is proven by TYPING the received fragment through the canonical
/// codec — the `ATTESTATION` subtype dispatches on `_type` — after which the
/// stored fragment is that typed value's canonical encoding, byte-identical to
/// a well-formed input.
fn parse_wrapped_commit_audit(ov: &Value) -> Result<Value, SmError> {
    let audit = ov
        .get("commit_audit")
        .ok_or_else(|| SmError::precondition("imported ORIGINAL_VERSION has no commit_audit"))?;
    let typed = openehr_its::json::from_canonical_value::<AuditDetails>(audit).map_err(|e| {
        SmError::precondition(format!(
            "imported commit_audit is not a canonical AUDIT_DETAILS or its ATTESTATION \
             subtype (RM common master06 §Committal and Audits): {e}"
        ))
        .with_source(e)
    })?;
    let (system_id, change_type, time_committed) = match &typed {
        AuditDetails::AuditDetails(a) => (&a.system_id, &a.change_type, &a.time_committed),
        AuditDetails::Attestation(a) => (&a.system_id, &a.change_type, &a.time_committed),
    };
    if system_id.is_empty() {
        return Err(SmError::precondition(
            "imported commit_audit.system_id is required and non-empty",
        ));
    }
    // AUDIT_DETAILS.Change_type_valid: change_type is coded from the openEHR
    // `audit change type` group (audit_details.adoc §Invariants).
    let change_token = &change_type.defining_code.code_string;
    if change_type_code(change_token).is_none() {
        return Err(SmError::precondition(format!(
            "imported commit_audit.change_type {change_token:?} is not an audit_change_type code"
        )));
    }
    #[expect(
        clippy::map_err_ignore,
        reason = "the mapped error already echoes the rejected token; the \
                  discarded parse error adds only its own wording, which is \
                  not part of the wire contract"
    )]
    let _: jiff::Timestamp = time_committed.value.parse().map_err(|_| {
        SmError::precondition(format!(
            "imported commit_audit.time_committed {:?} is not an ISO 8601 instant",
            time_committed.value
        ))
    })?;
    Ok(openehr_its::json::to_canonical_value(&typed))
}

/// Parse one received `ORIGINAL_VERSION` into its cloned `vo_id` and the
/// [`ImportVersion`] to replay — preserving the wrapped original's full 3-part
/// identity (incl. branch `version_tree_id`s), `preceding_version_uid`,
/// `other_input_version_uids`, `contribution`, `commit_audit`, lifecycle, data,
/// signature and attestations verbatim (master06 §Copying: "the
/// `ORIGINAL_VERSION` instance is never modified"; `ehr_extract` master05
/// `X_VERSIONED_OBJECT.versions: List<ORIGINAL_VERSION>`).
fn parse_imported_version(ov: &Value) -> Result<(VoId, ImportVersion), SmError> {
    // `X_VERSIONED_OBJECT.versions` is `List<ORIGINAL_VERSION<T>>`
    // (`x_versioned_object.adoc` §Attributes) and master06 §Copying makes the
    // ORIGINAL_VERSION the unit of copying, each receiving system creating its
    // own wrapper — so wrappers never nest.
    match ov.get("_type").and_then(Value::as_str) {
        None | Some("ORIGINAL_VERSION") => {}
        Some(other) => {
            return Err(SmError::precondition(format!(
                "X_VERSIONED_OBJECT.versions members must be ORIGINAL_VERSION \
                 (RM ehr_extract master05), got _type {other:?}"
            )));
        }
    }
    let uid = ov
        .pointer("/uid/value")
        .and_then(Value::as_str)
        .ok_or_else(|| SmError::precondition("imported ORIGINAL_VERSION has no uid.value"))?;
    let (vo_id, creating_system_id, tree) = parse_object_version_id(uid)?;
    // The imported `ORIGINAL_VERSION.uid.object_id` names a versioned object.
    let vo_id = VoId(vo_id);
    // preceding_version_uid + other_input_version_uids preserved verbatim
    // (master06 §Copying / §Version Merging).
    let preceding_version_uid = ov
        .pointer("/preceding_version_uid/value")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let other_input_version_uids: Vec<String> = ov
        .get("other_input_version_uids")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|u| {
                    u.as_str()
                        .or_else(|| u.pointer("/value").and_then(Value::as_str))
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();

    // VERSION.contribution (1..1) — the SOURCE system's CONTRIBUTION reference,
    // kept verbatim: master06 §Committal and Audits puts "the knowledge of the
    // original Contribution and committal" inside the wrapped ORIGINAL_VERSION,
    // while the local import CONTRIBUTION is recorded on the IMPORTED_VERSION
    // wrapper the change-control engine builds around it.
    let contribution = ov.get("contribution").cloned().ok_or_else(|| {
        SmError::precondition(
            "imported ORIGINAL_VERSION has no contribution (VERSION.contribution 1..1)",
        )
    })?;
    if openehr_its::json::from_canonical_value::<ObjectRef>(&contribution).is_err() {
        return Err(SmError::precondition(
            "imported ORIGINAL_VERSION.contribution is not a canonical OBJECT_REF \
             (RM common version.adoc §Attributes)",
        ));
    }

    let commit_audit = parse_wrapped_commit_audit(ov)?;

    // lifecycle_state (ORIGINAL_VERSION.lifecycle_state) resolved to its code.
    let lifecycle_token = ov
        .pointer("/lifecycle_state/defining_code/code_string")
        .and_then(Value::as_str)
        .or_else(|| ov.pointer("/lifecycle_state/value").and_then(Value::as_str))
        .unwrap_or(lifecycle::state::COMPLETE);
    let lifecycle_state = lifecycle_state_code(lifecycle_token).ok_or_else(|| {
        SmError::precondition(format!(
            "imported lifecycle_state {lifecycle_token:?} is not a version_lifecycle_state code"
        ))
    })?;

    // data: Void (absent/null) exactly for a 523|deleted| version (master06
    // §Logical Deletion).
    let data = ov
        .get("data")
        .cloned()
        .filter(|d| !d.is_null())
        .unwrap_or(Value::Null);
    let deleted = lifecycle_state == lifecycle::state::DELETED;
    if deleted && !data.is_null() {
        return Err(SmError::precondition(
            "imported 523|deleted| version must not carry data (data is Void)",
        ));
    }
    if !deleted && data.is_null() {
        return Err(SmError::precondition(
            "imported non-deleted ORIGINAL_VERSION requires data",
        ));
    }

    let signature = ov
        .get("signature")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let attestations = ov
        .get("attestations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Ok((
        vo_id,
        ImportVersion {
            tree,
            creating_system_id,
            preceding_version_uid,
            other_input_version_uids,
            lifecycle_state,
            contribution,
            commit_audit,
            data,
            signature,
            attestations,
        },
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A minimal spec-shaped `ORIGINAL_VERSION` wire value for the import parser
    /// — carrying both mandatory `VERSION` attributes (`contribution` 1..1,
    /// `commit_audit` 1..1; RM common `version.adoc` §Attributes).
    fn original_version(type_field: Option<&str>) -> Value {
        let mut ov = json!({
            "uid": { "_type": "OBJECT_VERSION_ID",
                     "value": "018f4a5e-9df1-7d1e-8b6f-2b8c00000001::sysA.example.org::1" },
            "contribution": { "_type": "OBJECT_REF", "namespace": "local",
                              "type": "CONTRIBUTION",
                              "id": { "_type": "HIER_OBJECT_ID",
                                      "value": "3d2c1b0a-0000-4000-8000-000000000abc" } },
            "commit_audit": {
                "_type": "AUDIT_DETAILS",
                "system_id": "sysA.example.org",
                "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-11T10:00:00Z" },
                "change_type": { "_type": "DV_CODED_TEXT", "value": "creation",
                                 "defining_code": { "_type": "CODE_PHRASE", "code_string": "249",
                                                    "terminology_id": { "_type": "TERMINOLOGY_ID",
                                                                        "value": "openehr" } } },
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr A" }
            },
            "lifecycle_state": { "_type": "DV_CODED_TEXT", "value": "complete",
                                 "defining_code": { "_type": "CODE_PHRASE", "code_string": "532",
                                                    "terminology_id": { "_type": "TERMINOLOGY_ID",
                                                                        "value": "openehr" } } },
            "data": { "_type": "EHR_STATUS" }
        });
        if let Some(t) = type_field {
            ov.as_object_mut().unwrap().insert("_type".into(), json!(t));
        }
        ov
    }

    /// `X_VERSIONED_OBJECT.versions` members must be `ORIGINAL_VERSION` (RM
    /// `ehr_extract` master05); a foreign `_type` — e.g. an already-wrapped
    /// `IMPORTED_VERSION` — is rejected, while an explicit or absent
    /// `ORIGINAL_VERSION` tag parses. Regression for
    /// rm-common-change-control-R20.
    #[test]
    fn imported_versions_member_type_is_enforced() {
        parse_imported_version(&original_version(None)).expect("absent _type defaults");
        parse_imported_version(&original_version(Some("ORIGINAL_VERSION")))
            .expect("explicit ORIGINAL_VERSION");
        for foreign in ["IMPORTED_VERSION", "VERSION", "ORIGINAL_VERSION2"] {
            let err = parse_imported_version(&original_version(Some(foreign)))
                .expect_err("foreign _type in versions[] must be rejected");
            assert!(
                err.message.contains("ORIGINAL_VERSION") && err.message.contains(foreign),
                "error should name the expected and offending types, got: {}",
                err.message
            );
        }
    }

    /// The received original's OWN `contribution` and `commit_audit` are the
    /// FOREIGN act, kept for the wrapped `ORIGINAL_VERSION` — RM common
    /// master06 §Committal and Audits: "the knowledge of the original
    /// Contribution and committal are retained inside the wrapped
    /// `ORIGINAL_VERSION` instance". Regression for #1679, whose defect was
    /// that the received `contribution` was dropped entirely and the foreign
    /// `commit_audit` was written as the version row's own.
    #[test]
    fn wrapped_original_keeps_its_foreign_contribution_and_audit() {
        let (_, version) =
            parse_imported_version(&original_version(None)).expect("the fixture parses");
        assert_eq!(
            version
                .contribution
                .pointer("/id/value")
                .and_then(Value::as_str),
            Some("3d2c1b0a-0000-4000-8000-000000000abc"),
            "the received contribution reference must be preserved verbatim"
        );
        assert_eq!(
            version
                .commit_audit
                .pointer("/time_committed/value")
                .and_then(Value::as_str),
            Some("2026-07-11T10:00:00Z"),
            "the source commit instant must ride the wrapped original, not the row"
        );
        assert_eq!(
            version
                .commit_audit
                .get("system_id")
                .and_then(Value::as_str),
            Some("sysA.example.org"),
        );
    }

    /// `VERSION.contribution` is 1..1 (RM common `version.adoc` §Attributes);
    /// an `ORIGINAL_VERSION` arriving without one cannot be wrapped without
    /// losing the original Contribution master06 §Committal and Audits says
    /// must be retained, so it is refused.
    #[test]
    fn a_received_original_without_a_contribution_is_refused() {
        let mut ov = original_version(None);
        ov.as_object_mut().expect("object").remove("contribution");
        let err = parse_imported_version(&ov).expect_err("contribution is mandatory");
        assert!(
            err.message.contains("contribution"),
            "error should name the missing attribute, got: {}",
            err.message
        );
    }

    /// `AUDIT_DETAILS.Change_type_valid`: the commit audit's `change_type` is
    /// coded from the openEHR `audit change type` group
    /// (`UML/classes/org.openehr.rm.common.audit_details.adoc` §Invariants), so
    /// a foreign code is refused rather than stored verbatim.
    #[test]
    fn a_received_commit_audit_with_an_unknown_change_type_is_refused() {
        let mut ov = original_version(None);
        *ov.pointer_mut("/commit_audit/change_type/defining_code/code_string")
            .expect("code_string") = json!("999999");
        let err = parse_imported_version(&ov).expect_err("change_type must be an openEHR code");
        assert!(
            err.message.contains("audit_change_type"),
            "error should name the violated invariant, got: {}",
            err.message
        );
    }
}
