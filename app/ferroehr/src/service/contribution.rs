// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION commit both chapters share: the EHR-scoped path and the
//! demographic one.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Contributions and
//! §Committal and Audits. Every version of the set is validated as a direct
//! commit would be, the set is written in one transaction (all versions and
//! attestations commit or none), and the cross-area hooks run around it: the
//! EHR's subject columns after an `EHR_STATUS`, the cached access settings
//! after an `EHR_ACCESS`. The parsing and classification of the version set
//! are [`crate::versioning::contribution`].

use std::collections::HashMap;

use openehr_base::prelude::HierObjectId;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use serde_json::Value;
use sqlx::Connection as _;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::error::{ServiceError, Violation};
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::demographic::validate::Container;
use crate::versioning::Kind;
use crate::versioning::attestation::PendingAttest;
use crate::versioning::audit::{AuditInput, validate_commit_audit};
use crate::versioning::change::{self, Change, CommittedContribution};
use crate::versioning::contribution::{
    Action, PlannedVersion, body_target_not_found_is_bad_request, check_kind_scope, data_kind,
    delete_change, needs_content_write_permission, parse_contribution_audit, parse_supplied_uid,
    plan_attestation, plan_version, plan_writes_content_without_reactivating, require_kind,
    typed_decode_gate,
};

/// The pool the pseudonymisation domain this CONTRIBUTION belongs to is served
/// by: the demographic one for a party-only set, the clinical one otherwise.
///
/// A CONTRIBUTION never spans the two domains — `party_only` is the scope check
/// the engine already enforces per member — so one pool serves the whole
/// transaction and the atomicity RM common master06 §Contributions requires is
/// preserved. No openEHR spec governs storage layout — our own
/// design/extension.
fn domain_pool(cx: &FerroEhrService, party_only: bool) -> &sqlx::PgPool {
    if party_only {
        &cx.demographic_pool
    } else {
        &cx.pool
    }
}

/// Commit a CONTRIBUTION's version set atomically under one contribution +
/// audit, returning its id together with the commit instant its audit recorded.
/// Shared by the EHR-scoped contribution path (`ehr_id = Some`, `party_only =
/// false`) and the demographic contribution path (`ehr_id = None`, `party_only
/// = true`), each of which hands over the parts of its own group's generated
/// `NewContribution` ([`decode_new_contribution`](crate::versioning::contribution::decode_new_contribution)). Each version's storage
/// action and preserved audit change-type code come from `classify` in [`crate::versioning::contribution`]; the
/// object kind from the payload `_type` (create) or the stored object
/// (modify/delete).
///
/// The target EHR must exist before committing, so a create-only
/// CONTRIBUTION to a missing EHR is a clean `NotFound`, not a storage FK error.
///
/// # Errors
/// [`ServiceError::NotFound`] for a missing target EHR; the `classify` in [`crate::versioning::contribution`]
/// rejections per version; [`ServiceError::Unprocessable`] for a malformed
/// CONTRIBUTION `uid`, an empty `versions` list, a scope-mismatched kind, or a
/// failed content/audit validation; [`ServiceError::BadRequest`] when an audit
/// is an `UPDATE_ATTESTATION`, when a `666|attestation|` member carries no
/// attestation, when a member carries an undeclared key, or when a
/// body-referenced modification target does not exist (the `400_CONTRIBUTION`
/// scope); [`ServiceError::Conflict`] for a duplicate EHR singleton/directory;
/// plus the commit-engine placement/storage/signing errors.
pub(crate) async fn commit_version_set(
    cx: &FerroEhrService,
    ehr_id: Option<EhrId>,
    uid: Option<&HierObjectId>,
    versions: Vec<UpdateVersion<Value>>,
    audit: &UpdateAudit,
    party_only: bool,
) -> Result<CommittedContribution, ServiceError> {
    // the EHR-existence check — the CONTRIBUTION's target EHR must exist.
    if let Some(ehr_id) = ehr_id {
        crate::storage::ehr_repo::require_ehr(&mut *acquire(&cx.pool).await?, ehr_id).await?;
    }

    // NOTE: `schemas/ehr/NewContribution.yaml` gives `versions` no `minItems`, so
    // an empty list is schema-valid and refused as a semantic error (422).
    if versions.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("must contain at least one version").with_path("versions"),
        ));
    }

    let supplied_uid = parse_supplied_uid(uid)?;
    let contribution_audit = parse_contribution_audit(audit, &cx.effective_system_id())?;

    // Each UPDATE_VERSION is read exactly once into a typed plan entry; the
    // modification targets are then existence/kind-checked in ONE batched
    // statement and resolved without re-reading any member.
    let mut plan: Vec<PlannedVersion> = Vec::with_capacity(versions.len());
    for (index, version) in versions.into_iter().enumerate() {
        plan.push(plan_version(version, index, &contribution_audit.system_id)?);
    }

    let target_kinds = read_target_kinds(cx, &plan, party_only).await?;

    refuse_content_into_inactive_ehr(cx, ehr_id, &plan).await?;

    let mut changes: Vec<(AuditInput, Change)> = Vec::with_capacity(plan.len());
    // 666 attestations of existing versions (committing no new version).
    let mut attests: Vec<PendingAttest> = Vec::new();
    for mut v in plan {
        // AUDIT_DETAILS.System_id_valid + committer PARTY invariants — every
        // client-supplied version commit_audit must be a valid RM instance, a
        // `666|attestation|` member's included.
        validate_commit_audit(&mut v.audit).map_err(|e| e.for_version_member(v.index))?;
        if v.action == Action::Attest {
            attests.extend(plan_attestation(&target_kinds, v, party_only)?);
            continue;
        }
        changes.push(build_change(cx, &target_kinds, v, ehr_id, party_only).await?);
    }

    // Whether the set needs the content-write gate; the gate itself runs
    // INSIDE the commit transaction (`write_contribution`), under a row lock,
    // so no pre-transaction read can race a concurrent EHR_STATUS flip.
    let needs_content_permission = needs_content_write_permission(&changes);

    // The subject-sync hook runs after the commit, so the EHR_STATUS bodies
    // are collected before `changes` moves into the commit engine.
    let status_commits: Vec<Value> = changes
        .iter()
        .filter_map(|(_, c)| match c {
            Change::Create {
                kind: Kind::EhrStatus,
                canonical,
                ..
            }
            | Change::Modify {
                kind: Kind::EhrStatus,
                canonical,
                ..
            } => Some(canonical.clone()),
            _ => None,
        })
        .collect();
    // The FOLDER bodies this set commits, for the post-insert item-reference
    // check, run after the inserts because a folder may reference a sibling of
    // its own set (RM ehr master04 §Folders).
    let folder_commits: Vec<Value> = changes
        .iter()
        .filter_map(|(_, c)| match c {
            Change::Create {
                kind: Kind::Folder,
                canonical,
                ..
            }
            | Change::Modify {
                kind: Kind::Folder,
                canonical,
                ..
            } => Some(canonical.clone()),
            _ => None,
        })
        .collect();

    let outcome = write_contribution(
        cx,
        ehr_id,
        party_only,
        ContributionWrite {
            supplied_uid,
            audit: &contribution_audit,
            changes,
            attests,
            status_commits,
            folder_commits,
            needs_content_permission,
        },
    )
    .await?;

    // Meter the COMPOSITION versions this CONTRIBUTION landed, after the commit
    // (a rolled-back contribution counts nothing).
    for c in &outcome.versions {
        change::meter_committed(c);
    }

    // An EHR_ACCESS version changes the EHR's access-control policy (the
    // settings are change-controlled — RM ehr master04 §EHR Access), and an
    // EHR_STATUS version may change the subject whose care relationships
    // decide access, so drop the copy the access gate consults per request.
    if let Some(ehr_id) = ehr_id
        && outcome
            .versions
            .iter()
            .any(|c| matches!(c.kind, Kind::EhrAccess | Kind::EhrStatus))
    {
        cx.invalidate_ehr_access(ehr_id).await;
    }

    Ok(outcome)
}

/// The resolved change set of one CONTRIBUTION, ready to write.
struct ContributionWrite<'a> {
    supplied_uid: Option<Uuid>,
    audit: &'a AuditInput,
    changes: Vec<(AuditInput, Change)>,
    attests: Vec<PendingAttest>,
    /// The `EHR_STATUS` bodies the set commits, captured before `changes` moves
    /// into the commit engine (the subject-sync hook runs after the commit).
    status_commits: Vec<Value>,
    /// The FOLDER bodies the set commits, for the post-insert item-reference
    /// check.
    folder_commits: Vec<Value>,
    /// Whether the set needs the content-write gate
    /// ([`needs_content_write_permission`]); evaluated in-transaction.
    needs_content_permission: bool,
}

/// Writes the resolved change set in one transaction, with the cross-area
/// commit hooks around it.
///
/// This is the single site of truth for the CONTRIBUTION path; the direct
/// create/update paths run the same hooks inline on their own write flow.
///
/// # Errors
/// The `VERSIONED_COMPOSITION` cross-version rejections (RM ehr
/// `versioned_composition.adoc`), the folder item-reference violations (judged
/// AFTER the set's own inserts so the verdict never depends on member order; a
/// violation rolls the whole set back), and the commit-engine
/// placement/storage/signing errors.
async fn write_contribution(
    cx: &FerroEhrService,
    ehr_id: Option<EhrId>,
    party_only: bool,
    write: ContributionWrite<'_>,
) -> Result<CommittedContribution, ServiceError> {
    let mut conn = acquire(domain_pool(cx, party_only)).await?;
    let mut tx = conn.begin().await?;
    // The content-write gate, under a row lock (RM ehr master04 §EHR Active
    // Status; the in-transaction evaluation is our own recorded semantics —
    // see `change::precondition::ensure_content_writable_tx`). A set that itself writes an
    // EHR_STATUS takes the exclusive lock up front (its own commit updates
    // the same row), avoiding a share-then-upgrade deadlock.
    if let Some(ehr_id) = ehr_id
        && write.needs_content_permission
    {
        change::precondition::ensure_content_writable_tx(
            &mut tx,
            ehr_id,
            !write.status_commits.is_empty(),
        )
        .await?;
    }
    // NOTE: BASE Architecture Overview, Access control of access settings: the
    // gate-keeper rule is judged against the settings current in this transaction.
    if let Some(ehr_id) = ehr_id
        && write
            .changes
            .iter()
            .any(|(_, c)| c.kind() == Kind::EhrAccess)
    {
        cx.check_ehr_access_writer(&mut tx, ehr_id).await?;
    }
    for (_, change) in &write.changes {
        if let Change::Modify {
            vo_id,
            kind: Kind::Composition,
            canonical,
            ..
        } = change
        {
            crate::validation::versioned::check_versioned_composition_invariants(
                &mut tx, *vo_id, canonical,
            )
            .await
            .map_err(body_target_not_found_is_bad_request)?;
        }
    }
    let outcome = change::commit_contribution(
        &mut tx,
        ehr_id,
        write.supplied_uid,
        write.audit,
        write.changes,
        write.attests,
        &cx.signing_ctx(),
    )
    .await
    .map_err(body_target_not_found_is_bad_request)?;
    if let Some(ehr_id) = ehr_id {
        for folder in &write.folder_commits {
            crate::validation::versioned::check_folder_item_refs(
                &mut *tx,
                ehr_id,
                &cx.effective_system_id(),
                folder,
            )
            .await?;
        }
        // Keep the EHR's promoted subject columns in sync after each committed
        // EHR_STATUS version (one EHR per subject — RM ehr master04 §EHR
        // Status).
        for status in &write.status_commits {
            cx.sync_ehr_subject(&mut tx, ehr_id, status).await?;
        }
    }
    tx.commit().await?;
    Ok(outcome)
}

/// Reads the stored kind of every modification target of the plan in ONE
/// batched statement.
///
/// # Errors
/// Storage errors from the batched read.
async fn read_target_kinds(
    cx: &FerroEhrService,
    plan: &[PlannedVersion],
    party_only: bool,
) -> Result<HashMap<VoId, Kind>, ServiceError> {
    let mut target_ids: Vec<VoId> = plan
        .iter()
        .filter_map(|v| v.target.map(|(vo_id, _)| vo_id))
        .collect();
    target_ids.sort_unstable();
    target_ids.dedup();
    Ok(
        crate::storage::version_repo::meta::object_kinds(domain_pool(cx, party_only), &target_ids)
            .await
            .map_err(ServiceError::from)?
            .into_iter()
            .filter_map(|(id, kind)| Kind::from_type(&kind).map(|k| (id, k)))
            .collect(),
    )
}

/// Resolves one non-attestation plan entry into the [`Change`] the commit
/// engine applies, paired with the audit that member committed under.
///
/// # Errors
/// The per-member rejections of the create/modify/delete shapes — see
/// [`create_change`], [`modify_change`] and [`delete_change`].
async fn build_change(
    cx: &FerroEhrService,
    kinds: &HashMap<VoId, Kind>,
    v: PlannedVersion,
    ehr_id: Option<EhrId>,
    party_only: bool,
) -> Result<(AuditInput, Change), ServiceError> {
    let change = match v.action {
        Action::Create => create_change(cx, &v, ehr_id, party_only).await?,
        Action::Modify => modify_change(cx, kinds, &v, party_only).await?,
        Action::Delete => delete_change(kinds, &v, party_only)?,
        // The `666|attestation|` branch collects every attest member and
        // `continue`s before this call, so this arm is a typed guard against
        // that branch ever stopping to cover the action — the same shape the
        // lost-target arms use.
        Action::Attest => {
            return Err(ServiceError::exception(
                "attestation version reached the change-building match".to_owned(),
            ));
        }
    };
    Ok((v.audit, change))
}

/// Validates one content-bearing member and returns the `template_id` its
/// version row stamps.
///
/// A CONTRIBUTION commit is a full commit route: its versions are validated
/// exactly as a direct create/update, relaxed for a `553|incomplete|`
/// lifecycle (master06 §Incomplete Content). A COMPOSITION stamps
/// `version.template_id` exactly like the direct commit path — the
/// template-delete `409` guard counts that column, so a
/// contribution-committed composition must protect its template from physical
/// deletion too.
///
/// A party version is judged as part of `container`, the version container
/// the member creates or modifies (RM demographic `party.adoc`
/// `Relationships_validity`); every other kind needs none.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for a scope-mismatched kind or content that
/// fails the typed decode or the commit validation.
async fn accept_content(
    cx: &FerroEhrService,
    (kind, container): (Kind, Option<Container>),
    data: &Value,
    incomplete: bool,
    index: usize,
    party_only: bool,
) -> Result<Option<String>, ServiceError> {
    check_kind_scope(kind, party_only).map_err(|e| e.for_version_member(index))?;
    typed_decode_gate(kind, data, incomplete).map_err(|e| e.for_version_member(index))?;
    let checked = match container {
        Some(container) if kind.is_party() => {
            cx.demographic_commit_check(kind, data, incomplete, Some(container))
                .await
        }
        _ => cx.validate_for_commit(kind, data, incomplete).await,
    };
    checked.map_err(|e| e.for_version_member(index))?;
    Ok((kind == Kind::Composition)
        .then(|| crate::validation::versioned::composition_template_id(data).map(str::to_owned))
        .flatten())
}

/// Resolves a creation member. Its kind comes from the payload `_type`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for missing `data`, an unrecognized kind or
/// failed validation; [`ServiceError::Conflict`] for a duplicate EHR singleton
/// or folder hierarchy (an EHR holds exactly one `EHR_STATUS`/`EHR_ACCESS` —
/// RM ehr, EHR class; FOLDER hierarchies follow the CNF
/// master08-func_tc_ehr_contribution E.2 criterion).
async fn create_change(
    cx: &FerroEhrService,
    v: &PlannedVersion,
    ehr_id: Option<EhrId>,
    party_only: bool,
) -> Result<Change, ServiceError> {
    let Some(data) = v.data.clone() else {
        return Err(ServiceError::content_invalid(
            Violation::new("is required on a creation version").with_path("data"),
        )
        .for_version_member(v.index));
    };
    let kind = data_kind(&data).map_err(|e| e.for_version_member(v.index))?;
    // A party's id is allocated before the checks, so it is judged against the
    // container the commit gives it.
    let allocated = kind.is_party().then(|| cx.allocate_party_id());
    let container = allocated.as_ref().map(|id| Container::New(id.id()));
    let template_id = accept_content(
        cx,
        (kind, container),
        &data,
        v.incomplete,
        v.index,
        party_only,
    )
    .await?;
    if let Some(ehr_id) = ehr_id {
        reject_duplicate_singleton(cx, ehr_id, kind, &data).await?;
    }
    Ok(Change::Create {
        allocated,
        kind,
        canonical: data,
        template_id,
        signature: v.signature.clone(),
        lifecycle_state: Some(v.lifecycle_state.clone()),
        attestations: v.attestations.clone(),
        other_input_version_uids: v.other_input_version_uids.clone(),
    })
}

/// Resolves a modification member. Its kind comes from the stored object.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for missing `data` or failed validation;
/// [`ServiceError::precondition`] when the target does not exist.
async fn modify_change(
    cx: &FerroEhrService,
    kinds: &HashMap<VoId, Kind>,
    v: &PlannedVersion,
    party_only: bool,
) -> Result<Change, ServiceError> {
    let Some(data) = v.data.clone() else {
        return Err(ServiceError::content_invalid(
            Violation::new("is required on a modification version").with_path("data"),
        )
        .for_version_member(v.index));
    };
    let Some((vo_id, expected)) = v.target else {
        return Err(ServiceError::exception(
            "modify plan entry lost its parsed preceding target".to_owned(),
        ));
    };
    let kind = require_kind(kinds, vo_id).map_err(|e| e.for_version_member(v.index))?;
    let template_id = accept_content(
        cx,
        (kind, Some(Container::Existing(vo_id))),
        &data,
        v.incomplete,
        v.index,
        party_only,
    )
    .await?;
    Ok(Change::Modify {
        vo_id,
        kind,
        canonical: data,
        expected: Some(expected),
        template_id,
        signature: v.signature.clone(),
        lifecycle_state: Some(v.lifecycle_state.clone()),
        attestations: v.attestations.clone(),
        other_input_version_uids: v.other_input_version_uids.clone(),
    })
}

/// The EHR-state gate before any member's content is validated (#3257).
///
/// A set that writes content into a deactivated EHR is refused as the state
/// conflict it is (RM ehr master04 §EHR Active Status); a body error on a
/// record that cannot be written would send the caller fixing the wrong
/// thing. Advisory here: the locked, authoritative check still runs inside
/// the commit transaction.
///
/// # Errors
/// [`ServiceError::Conflict`] when the EHR exists and is not modifiable; the
/// storage error of the writability read.
async fn refuse_content_into_inactive_ehr(
    cx: &FerroEhrService,
    ehr_id: Option<EhrId>,
    plan: &[PlannedVersion],
) -> Result<(), ServiceError> {
    let Some(ehr_id) = ehr_id else {
        return Ok(());
    };
    if !plan_writes_content_without_reactivating(plan) {
        return Ok(());
    }
    let (exists, modifiable, _) =
        crate::storage::ehr_repo::ehr_writability(&mut *acquire(&cx.pool).await?, ehr_id).await?;
    if exists && modifiable == Some(false) {
        return Err(change::precondition::not_modifiable_error(ehr_id));
    }
    Ok(())
}

/// Reject the *creation* of a duplicate EHR structure. An EHR holds exactly
/// one `EHR_STATUS` and one `EHR_ACCESS` (RM ehr, EHR class). A FOLDER
/// creation is rejected when a LIVE hierarchy with the SAME root
/// `archetype_node_id` already exists (CNF schedule master08
/// §`commit_contribution` E.2 — creating the existing root FOLDER again is
/// negative); a distinct hierarchy joins `EHR.folders` (RM ehr master04
/// §Folders). COMPOSITIONs are unbounded.
///
/// # Errors
/// [`ServiceError::Conflict`] on a duplicate `EHR_STATUS`/`EHR_ACCESS` or an
/// already-live same-identity folder hierarchy; the lookup errors.
async fn reject_duplicate_singleton(
    cx: &FerroEhrService,
    ehr_id: EhrId,
    kind: Kind,
    data: &Value,
) -> Result<(), ServiceError> {
    match kind {
        Kind::EhrStatus | Kind::EhrAccess => {
            if cx.current_vo(ehr_id, kind).await?.is_some() {
                return Err(ServiceError::conflict(format!(
                    "EHR {ehr_id} already has a {}; only one is permitted (RM ehr, EHR class)",
                    kind.as_str()
                )));
            }
        }
        Kind::Folder => {
            let root = data
                .get("archetype_node_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = data
                .pointer("/name/value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if crate::storage::ehr_repo::live_folder_root_exists(&cx.pool, ehr_id, root, name)
                .await?
            {
                return Err(ServiceError::conflict(format!(
                    "EHR {ehr_id} already has a folder hierarchy rooted at \
                     {root:?} named {name:?}; re-creating an existing directory \
                     is invalid (CNF schedule master08 §commit_contribution \
                     E.2) — commit a modification, or a hierarchy with a \
                     distinct root (RM ehr master04 §Folders)"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}
