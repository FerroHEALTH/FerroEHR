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
use openehr_rm::prelude::DvText;
use serde_json::Value;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::error::{ServiceError, Violation};
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::demographic::validate::Container;
use crate::service::ehr::status::SubjectColumns;
use crate::storage::version_repo::gate::{self, Ask, Gate, Shape};
use crate::system_log::categories::AccessedContent;
use crate::system_log::event::AccessDomain;
use crate::system_log::recorder::WriteFacts;
use crate::versioning::Kind;
use crate::versioning::attestation::PendingAttest;
use crate::versioning::audit::{AuditInput, validate_commit_audit};
use crate::versioning::body::{Body, decode_member, decode_member_text};
use crate::versioning::change::{self, Change, CommittedContribution};
use crate::versioning::contribution::{
    Action, PlannedVersion, body_target_not_found_is_bad_request, check_kind_scope, data_kind,
    delete_change, needs_content_write_permission, parse_contribution_audit, parse_supplied_uid,
    plan_attestation, plan_version, plan_writes_content_without_reactivating, require_kind,
};
use crate::versioning::envelope::MemberData;

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
    versions: Vec<UpdateVersion<MemberData>>,
    audit: &UpdateAudit,
    party_only: bool,
) -> Result<CommittedContribution, ServiceError> {
    // Planning reads no store, so the EHR's existence and writability, the
    // policy revisions and every target's head are read in one gate read after
    // it, and the 404 of a missing EHR still answers before a planning refusal
    // (#6256).
    let planned = plan_set(cx, uid, versions, audit);
    let gate = match ehr_id {
        Some(ehr_id) => {
            // NOTE: our own design (#6256), a set that does not plan names no targets; its
            // refusal answers once the EHR is known to exist.
            let targets = planned
                .as_ref()
                .map(|(_, _, plan)| plan_targets(plan))
                .unwrap_or_default();
            let ask = Ask {
                ehr_id,
                targets: &targets,
                shape: Shape::Members,
                persistent: None,
                addressed: None,
                items: &[],
            };
            let gate = gate::read(&mut *acquire(&cx.pool).await?, ask).await?;
            cx.ensure_gate_policy_held().await?;
            if gate.is_modifiable.is_none() {
                return Err(ServiceError::not_found(
                    crate::error::refusal::Missing::Ehr,
                    format!("EHR {ehr_id}"),
                ));
            }
            Some(gate)
        }
        None => None,
    };
    let (supplied_uid, contribution_audit, plan) = planned?;

    let target_kinds = match &gate {
        Some(gate) => kinds_of(gate),
        None => read_target_kinds(cx, &plan, party_only).await?,
    };

    let modifiable = ehr_id.zip(gate.as_ref().and_then(|gate| gate.is_modifiable));
    refuse_content_into_inactive_ehr(modifiable, &plan)?;

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
        let index = v.index;
        let change = build_change(cx, &target_kinds, v, ehr_id, party_only).await?;
        if let (
            Some(ehr_id),
            Change::Create {
                kind: Kind::Composition,
                body,
                ..
            }
            | Change::Modify {
                kind: Kind::Composition,
                body,
                ..
            },
        ) = (ehr_id, &change.1)
        {
            cx.placement_declared(ehr_id, body)
                .await
                .map_err(|e| e.for_version_member(index))?;
        }
        changes.push(change);
    }

    // Whether the set needs the content-write gate; the gate itself runs
    // INSIDE the commit transaction (`write_contribution`), under a row lock,
    // so no pre-transaction read can race a concurrent EHR_STATUS flip.
    let needs_content_permission = needs_content_write_permission(&changes);

    // The subject-sync hook and the folder item-reference check run after the
    // commit (a folder may reference a sibling of its own set, RM ehr master04
    // §Folders), so what they read is taken before `changes` moves into the
    // commit engine.
    let system_id = cx.effective_system_id();
    let mut status_commits = Vec::new();
    let mut folder_commits = Vec::new();
    for (_, change) in &changes {
        if let Change::Create { kind, body, .. } | Change::Modify { kind, body, .. } = change {
            match kind {
                Kind::EhrStatus => {
                    status_commits.extend(SubjectColumns::of_body(body));
                }
                Kind::Folder => {
                    folder_commits.push(crate::validation::versioned::local_item_refs(
                        body, &system_id,
                    ));
                }
                _ => {}
            }
        }
    }

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
            gate,
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
    /// The promoted columns of the `EHR_STATUS` bodies the set commits,
    /// captured before `changes` moves into the commit engine (the
    /// subject-sync hook runs after the commit).
    status_commits: Vec<SubjectColumns>,
    /// The local item references of each FOLDER the set commits, for the
    /// post-insert item-reference check.
    folder_commits: Vec<Vec<crate::validation::versioned::LocalItemRef>>,
    /// Whether the set needs the content-write gate
    /// ([`needs_content_write_permission`]); evaluated in-transaction.
    needs_content_permission: bool,
    /// The set's gate read, which a set of COMPOSITION members commits on
    /// first (#6256); `None` for a demographic set.
    gate: Option<Gate>,
}

/// Writes the resolved change set in one transaction, with the cross-area
/// commit hooks around it.
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
    mut write: ContributionWrite<'_>,
) -> Result<CommittedContribution, ServiceError> {
    // The set's large media go to the blob store before a connection is held.
    for (_, change) in &mut write.changes {
        cx.externalize_change(change).await?;
    }
    if let Some(outcome) = commit_member_set(cx, ehr_id, party_only, &mut write).await? {
        return Ok(outcome);
    }
    let mut conn = cx.recorded_conn(domain_pool(cx, party_only)).await?;
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
        cx.check_ehr_access_writer(&mut tx, ehr_id, &mut write.changes)
            .await?;
    }
    for (_, change) in &write.changes {
        if let Change::Modify {
            vo_id,
            kind: Kind::Composition,
            body,
            ..
        } = change
        {
            crate::validation::versioned::check_versioned_composition_invariants(
                &mut tx, *vo_id, body,
            )
            .await
            .map_err(body_target_not_found_is_bad_request)?;
        }
    }
    // NOTE: our own design (#6256), from here the set may land, so the request is
    // never run again, whatever its copies held.
    crate::policy_register::observed::wrote();
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
            crate::validation::versioned::check_folder_item_refs(&mut *tx, ehr_id, folder).await?;
        }
        // Keep the EHR's promoted subject columns in sync after each committed
        // EHR_STATUS version (one EHR per subject — RM ehr master04 §EHR
        // Status).
        for status in &write.status_commits {
            cx.sync_ehr_subject(&mut tx, ehr_id, status).await?;
        }
    }
    let facts = if party_only {
        WriteFacts {
            domain: Some(AccessDomain::Demographic),
            ..WriteFacts::default()
        }
    } else {
        WriteFacts {
            ehr_id: ehr_id.map(|id| id.to_string()),
            ..WriteFacts::default()
        }
    };
    // NOTE: our own design (#3621), the demographic CONTRIBUTION response names no content.
    let content = if party_only {
        Vec::new()
    } else {
        cx.written_content(Some(&mut *tx), &outcome.versions).await
    };
    let facts = WriteFacts {
        object_id: Some(outcome.id.to_string()),
        content: (!content.is_empty()).then_some(AccessedContent::Objects(content)),
        ..facts
    };
    cx.commit_recorded(tx, &facts, Vec::new()).await?;
    Ok(outcome)
}

/// The CONTRIBUTION's uid and audit, and each `UPDATE_VERSION` read exactly
/// once into a typed plan entry, which reads no store.
///
/// # Errors
/// The `422` of an empty `versions` list, and the uid, audit and member
/// refusals of planning.
fn plan_set(
    cx: &FerroEhrService,
    uid: Option<&HierObjectId>,
    versions: Vec<UpdateVersion<MemberData>>,
    audit: &UpdateAudit,
) -> Result<(Option<Uuid>, AuditInput, Vec<PlannedVersion>), ServiceError> {
    // NOTE: `schemas/ehr/NewContribution.yaml` gives `versions` no `minItems`, so
    // an empty list is schema-valid and refused as a semantic error (422).
    if versions.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("must contain at least one version").with_path("versions"),
        ));
    }
    let supplied_uid = parse_supplied_uid(uid)?;
    let contribution_audit = parse_contribution_audit(audit, &cx.effective_system_id())?;
    let mut plan: Vec<PlannedVersion> = Vec::with_capacity(versions.len());
    for (index, version) in versions.into_iter().enumerate() {
        plan.push(plan_version(version, index, &contribution_audit.system_id)?);
    }
    Ok((supplied_uid, contribution_audit, plan))
}

/// The objects the plan's members modify, delete or attest, each once.
fn plan_targets(plan: &[PlannedVersion]) -> Vec<VoId> {
    let mut targets: Vec<VoId> = plan
        .iter()
        .filter_map(|v| v.target.map(|(vo_id, _)| vo_id))
        .collect();
    targets.sort_unstable();
    targets.dedup();
    targets
}

/// The stored kind of every target the gate read found.
fn kinds_of(gate: &Gate) -> HashMap<VoId, Kind> {
    gate.targets
        .iter()
        .filter_map(|target| {
            let tip = target.placement.tip.as_ref()?;
            Kind::from_type(&tip.kind).map(|kind| (target.vo_id, kind))
        })
        .collect()
}

/// Commit a set of COMPOSITION members in one guarded statement (#6256),
/// when `write` is one the statement covers; `None` leaves `write` to the
/// per-member path.
///
/// # Errors
/// The [`FerroEhrService::commit_member_set`] refusals.
async fn commit_member_set(
    cx: &FerroEhrService,
    ehr_id: Option<EhrId>,
    party_only: bool,
    write: &mut ContributionWrite<'_>,
) -> Result<Option<CommittedContribution>, ServiceError> {
    let Some(ehr_id) = ehr_id.filter(|_| {
        !party_only
            && write.attests.is_empty()
            && crate::service::contribution_set::covers(&write.changes)
    }) else {
        return Ok(None);
    };
    let changes = std::mem::take(&mut write.changes);
    let first = write.gate.take();
    match cx
        .commit_member_set(ehr_id, (write.supplied_uid, write.audit), changes, first)
        .await?
    {
        crate::service::contribution_set::Outcome::Committed(outcome) => Ok(Some(outcome)),
        crate::service::contribution_set::Outcome::PerMember(changes) => {
            write.changes = changes;
            Ok(None)
        }
    }
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
    mut v: PlannedVersion,
    ehr_id: Option<EhrId>,
    party_only: bool,
) -> Result<(AuditInput, Change), ServiceError> {
    let change = match v.action {
        Action::Create => create_change(cx, &mut v, ehr_id, party_only).await?,
        Action::Modify => modify_change(cx, kinds, &mut v, party_only).await?,
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
    data: MemberData,
    (incomplete, signed): (bool, bool),
    index: usize,
    party_only: bool,
) -> Result<(Body, Option<String>), ServiceError> {
    check_kind_scope(kind, party_only).map_err(|e| e.for_version_member(index))?;
    // A complete, unsigned member is read from its text. Any other, and one
    // the text reader refuses, is read from its JSON value, which words the
    // refusal and is what a signed or incomplete member is stored from.
    let read = match data.text() {
        Some(text) if !incomplete && !signed => decode_member_text(kind, text),
        _ => None,
    };
    let body = if let Some(typed) = read {
        check_typed(cx, kind, container, &typed)
            .await
            .map_err(|e| e.for_version_member(index))?;
        typed
    } else {
        let data = data.into_value().map_err(|e| e.for_version_member(index))?;
        let typed =
            decode_member(kind, &data, incomplete).map_err(|e| e.for_version_member(index))?;
        let checked = match (container, &typed) {
            (container, Some(typed)) => check_typed(cx, kind, container, typed).await,
            (Some(container), None) if kind.is_party() => {
                cx.demographic_commit_check(kind, &data, incomplete, Some(container))
                    .await
            }
            (_, None) => cx.validate_for_commit(kind, &data, incomplete).await,
        };
        checked.map_err(|e| e.for_version_member(index))?;
        // NOTE: a member is stored as its typed canonical rendering, as every route
        // stores; one its client signed keeps the bytes that signature covers (#6003).
        match typed {
            Some(typed) if !signed => typed,
            _ => Body::Canonical(data),
        }
    };
    let template_id = (kind == Kind::Composition)
        .then(|| body.template_id().map(str::to_owned))
        .flatten();
    Ok((body, template_id))
}

/// The commit validation of a member read as typed RM: a demographic kind
/// as part of its `container`, every other kind by itself.
///
/// # Errors
/// The refusals of the two validations.
async fn check_typed(
    cx: &FerroEhrService,
    kind: Kind,
    container: Option<Container>,
    typed: &Body,
) -> Result<(), ServiceError> {
    if kind.is_demographic() {
        cx.demographic_typed_check(kind, typed, container).await
    } else {
        cx.validate_body(kind, typed, false).await
    }
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
    v: &mut PlannedVersion,
    ehr_id: Option<EhrId>,
    party_only: bool,
) -> Result<Change, ServiceError> {
    let Some(data) = v.data.take() else {
        return Err(ServiceError::content_invalid(
            Violation::new("is required on a creation version").with_path("data"),
        )
        .for_version_member(v.index));
    };
    let kind = data_kind(&data.rm_type).map_err(|e| e.for_version_member(v.index))?;
    // A party's id is allocated before the checks, so it is judged against the
    // container the commit gives it.
    let allocated = kind.is_party().then(|| cx.allocate_party_id());
    let container = allocated.as_ref().map(|id| Container::New(id.id()));
    let (body, template_id) = accept_content(
        cx,
        (kind, container),
        data.data,
        (v.incomplete, v.signature.is_some()),
        v.index,
        party_only,
    )
    .await?;
    if let Some(ehr_id) = ehr_id {
        reject_duplicate_singleton(cx, ehr_id, kind, &body).await?;
    }
    Ok(Change::Create {
        allocated,
        kind,
        body,
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
    v: &mut PlannedVersion,
    party_only: bool,
) -> Result<Change, ServiceError> {
    let Some(data) = v.data.take() else {
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
    let (body, template_id) = accept_content(
        cx,
        (kind, Some(Container::Existing(vo_id))),
        data.data,
        (v.incomplete, v.signature.is_some()),
        v.index,
        party_only,
    )
    .await?;
    Ok(Change::Modify {
        vo_id,
        kind,
        body,
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
/// `modifiable` is the EHR and its `is_modifiable` flag as the set's first
/// read found them; `None` for a demographic set.
///
/// # Errors
/// [`ServiceError::Conflict`] when the EHR is not modifiable.
fn refuse_content_into_inactive_ehr(
    modifiable: Option<(EhrId, bool)>,
    plan: &[PlannedVersion],
) -> Result<(), ServiceError> {
    match modifiable {
        Some((ehr_id, false)) if plan_writes_content_without_reactivating(plan) => {
            Err(change::precondition::not_modifiable_error(ehr_id))
        }
        _ => Ok(()),
    }
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
    body: &Body,
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
            let root = body.archetype_node_id().unwrap_or_default();
            let name = match body {
                Body::Folder(folder) => match &folder.name {
                    DvText::DvCodedText(text) => text.value.as_str(),
                    DvText::DvText(text) => text.value.as_str(),
                },
                other => other
                    .canonical()
                    .and_then(|data| data.pointer("/name/value"))
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            };
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
