// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! CONTRIBUTION classify + commit orchestration + retrieval.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Contributions,
//! §Committal and Audits + `master08-versioning.adoc` §Managing Changes. A
//! CONTRIBUTION is the change-set unit — `uid` + a `versions` list + an
//! `audit`; every change is a CONTRIBUTION, committed as a nested transaction
//! (all versions/attestations commit or none). Each version's storage action
//! **and** its preserved audit change-type code come from [`classify`]: the
//! client `commit_audit.change_type` is validated against the full openEHR
//! `audit_change_type` group and stored **verbatim** (never narrowed), while
//! the storage branch collapses to create / modify / delete / attest.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use openehr_base::prelude::{HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use serde_json::{Value, json};
use sqlx::Connection as _;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::service::error::{ServiceError, Violation};
use crate::service::status::ServiceStatus;
use crate::versioning::attestation::{AttestationInput, AttestationParts, PendingAttest};
use crate::versioning::audit::{
    AuditInput, change_type, change_type_code, decode_description, party_proxy, plain_audit,
    validate_commit_audit,
};
use crate::versioning::change::{Change, CommittedContribution};
use crate::versioning::lifecycle::{
    lifecycle_rubric, lifecycle_state_code, resolve_lifecycle, state,
};
use crate::versioning::object_version_id::{self, TreeId};
use crate::versioning::signature::signer::Signer;
use crate::versioning::wire::version_envelope;
use crate::versioning::{CommitEnv, Kind, change, read};

/// An optional `(lower, upper)` inclusive commit-time window (either side open
/// when its bound is `None`; the whole `Option` `None` = unbounded).
pub(crate) type TimeRange = Option<(Option<jiff::Timestamp>, Option<jiff::Timestamp>)>;

/// The storage branch an incoming VERSION maps to. Deliberately narrower than
/// the `audit_change_type` group: many change kinds (amendment, modification,
/// synthesis, …) are all "commit a new content version"; the audited change
/// type is carried separately, verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Create,
    Modify,
    Delete,
    /// Attach an `ATTESTATION` to an existing `ORIGINAL_VERSION`
    /// (`666|attestation|`) — no new version (master06 §Contributions).
    Attest,
}

/// Classify one VERSION of a contribution: resolve (and validate) its
/// `commit_audit.change_type` to the canonical numeric `audit_change_type`
/// code, and derive the storage [`Action`], rejecting spec-invalid combinations
/// (RM common master06 §Contributions):
///
/// - *addition of new item* → a **new** `VERSIONED_OBJECT`, `249|creation|`
///   (so `249` with a preceding is invalid, and any non-`249` needs an existing
///   object);
/// - *deletion* → a new version whose data is Void, `523|deleted|` (so data
///   alongside `523` is invalid);
/// - *modification* → `250|amendment|` / `251|modification|` / `252` / `253` /
///   `816` / `817`, content-carrying commits against an existing object;
/// - *attestation* → `666|attestation|` attaches to an existing
///   `ORIGINAL_VERSION` — **not** a new version; requires a preceding and no
///   data.
///
/// # Errors
/// [`ServiceError::BadRequest`] for a change-control mismatch the ITS-REST
/// `400_CONTRIBUTION` scope covers (`249` with a preceding; `666` without one);
/// [`ServiceError::Unprocessable`] for an out-of-group token or a data/preceding
/// combination the spec forbids (data on `523`/`666`, missing data on a
/// content commit, missing preceding on a non-creation).
fn classify(
    token: &str,
    has_preceding: bool,
    has_data: bool,
) -> Result<(Action, String), ServiceError> {
    let code = resolve_change_type(token)?;
    match code.as_str() {
        change_type::CREATION => classify_creation(code, has_preceding, has_data),
        change_type::DELETED => classify_deletion(code, has_preceding, has_data),
        change_type::ATTESTATION => classify_attestation(code, has_preceding, has_data),
        // amendment 250 / modification 251 / synthesis 252 / unknown 253 /
        // restoration 816 / format conversion 817: a content-carrying new
        // version of an existing object; the code is preserved verbatim.
        _ => classify_modification(code, has_preceding, has_data),
    }
}

/// The canonical numeric `audit_change_type` code of one version.
///
/// The token is the client's own: `change_type` is required on every audit
/// (ITS-REST `schemas/common/UpdateAudit.yaml` `required: [change_type,
/// committer]`), so the generated envelope refuses a member without one and no
/// code is ever inferred.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the supplied token is not a code of
/// the openEHR `audit_change_type` group.
fn resolve_change_type(token: &str) -> Result<String, ServiceError> {
    change_type_code(token).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "{token:?} is not a code in the openEHR audit_change_type group"
            ))
            .with_path("change_type")
            .with_invariant("AUDIT_DETAILS.Change_type_valid"),
        )
    })
}

/// Checks a `249|creation|` version: it commits a NEW `VERSIONED_OBJECT`, so it
/// carries data and no preceding version.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for a preceding version, the released
/// `400_CONTRIBUTION` trigger being directional ("first version of a
/// MODIFICATION") and not covering this mirror case, or for missing data.
fn classify_creation(
    code: String,
    has_preceding: bool,
    has_data: bool,
) -> Result<(Action, String), ServiceError> {
    if has_preceding {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "249|creation| is invalid for an existing object \
                 (preceding_version_uid present); creation commits a new \
                 VERSIONED_OBJECT",
            )
            .with_path("change_type")
            .with_invariant("RM change_control §Contributions"),
        ));
    }
    if !has_data {
        return Err(ServiceError::content_invalid(
            Violation::new("is required on a creation version").with_path("data"),
        ));
    }
    Ok((Action::Create, code))
}

/// Checks a `523|deleted|` version: a new version of an existing object whose
/// data attribute is set to Void.
///
/// # Errors
/// [`ServiceError::BadRequest`] without a preceding version — a non-creation
/// change type as a FIRST version is the released `400_CONTRIBUTION` trigger
/// family ("the modification type does not match the operation - i.e. first
/// version of a MODIFICATION"); [`ServiceError::Unprocessable`] when it
/// carries data.
fn classify_deletion(
    code: String,
    has_preceding: bool,
    has_data: bool,
) -> Result<(Action, String), ServiceError> {
    if !has_preceding {
        return Err(ServiceError::precondition(
            "deleted (523) version requires preceding_version_uid — a first \
             version's change type is 249|creation| (ITS-REST contribution \
             400: the modification type does not match the operation)"
                .to_owned(),
        ));
    }
    if has_data {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "must not be set on a deleted (523) version — its data attribute \
                 is set to Void",
            )
            .with_path("data")
            .with_invariant("RM change_control §Contributions"),
        ));
    }
    Ok((Action::Delete, code))
}

/// Checks a `666|attestation|` version: it attaches to an existing
/// `ORIGINAL_VERSION` identified by `preceding_version_uid` and commits no new
/// version (master06 §Contributions;
/// `VERSIONED_OBJECT.commit_attestation` pre `has_version_id`).
///
/// NOTE: the same §Contributions row spells the code's home
/// `ATTESTATION._commit_audit_._change_type_`, a path `ATTESTATION` cannot
/// have; the repaired reading `ATTESTATION.change_type` is used.
///
/// # Errors
/// [`ServiceError::BadRequest`] without a preceding version — the request
/// cannot name its target, so it is a 400 rather than a 422;
/// [`ServiceError::Unprocessable`] when it carries data.
fn classify_attestation(
    code: String,
    has_preceding: bool,
    has_data: bool,
) -> Result<(Action, String), ServiceError> {
    if !has_preceding {
        return Err(ServiceError::precondition(
            "change_type 666|attestation| requires preceding_version_uid to \
             identify the ORIGINAL_VERSION being attested (RM change_control \
             §Contributions; VERSIONED_OBJECT.commit_attestation pre \
             has_version_id)"
                .to_owned(),
        ));
    }
    if has_data {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "must not be set on a 666 attestation version — attesting an \
                 existing item adds no content",
            )
            .with_path("data")
            .with_invariant("RM change_control §Contributions"),
        ));
    }
    Ok((Action::Attest, code))
}

/// Checks a content-carrying modification version.
///
/// # Errors
/// [`ServiceError::BadRequest`] without a preceding version — THE released
/// assignment: `400_CONTRIBUTION`, "the modification type does not match the
/// operation - i.e. first version of a MODIFICATION";
/// [`ServiceError::Unprocessable`] when it carries no data.
fn classify_modification(
    code: String,
    has_preceding: bool,
    has_data: bool,
) -> Result<(Action, String), ServiceError> {
    if !has_preceding {
        return Err(ServiceError::precondition(format!(
            "change_type {code} requires preceding_version_uid — a first \
             version's change type is 249|creation| (ITS-REST contribution \
             400: the modification type does not match the operation; RM \
             change_control §Contributions)"
        )));
    }
    if !has_data {
        return Err(ServiceError::content_invalid(
            Violation::new(format!("is required on a change_type {code} version"))
                .with_path("data"),
        ));
    }
    Ok((Action::Modify, code))
}

/// One parsed `UPDATE_VERSION` of a CONTRIBUTION commit — the single-pass plan
/// entry (see the parse pass in [`commit_version_set`]).
struct PlannedVersion {
    /// The member's position in the submitted `versions` array — every
    /// member-scoped refusal names it.
    index: usize,
    action: Action,
    /// The parsed `preceding_version_uid` target (modify/delete/attest).
    target: Option<(VoId, TreeId)>,
    /// `data` (`null` ≙ absent — the deleted-version shape).
    data: Option<Value>,
    audit: AuditInput,
    /// The `lifecycle_state` code the member states (`UPDATE_VERSION.lifecycle_state`,
    /// a required `DV_CODED_TEXT`).
    lifecycle_state: String,
    incomplete: bool,
    signature: Option<String>,
    /// `UPDATE_VERSION.attestations`: committed WITH the new version on a
    /// content member, attached to the existing version on a
    /// `666|attestation|` member.
    attestations: Vec<AttestationInput>,
    /// `ORIGINAL_VERSION.other_input_version_uids`, the merge provenance the
    /// member states (empty when it merges nothing).
    other_input_version_uids: Vec<String>,
}

/// The `_type` self-tags a CONTRIBUTION version member may carry: the class the
/// released commit wire titles (`UPDATE_VERSION`) and the RM class that member
/// becomes when committed (`ORIGINAL_VERSION`).
///
/// Both names denote the same wire shape, the commit-wire partial of six
/// properties (ITS-REST `schemas/ehr/UpdateVersion.yaml`, the only member schema
/// `schemas/ehr/NewContribution.yaml` types `versions` items as). Admitting
/// `ORIGINAL_VERSION` names the class the member becomes, per the `_type` rule
/// of `specifications/docs/overview/Resources.md` §Resource representation; the
/// server still assigns the version identity, so
/// [`read_undeclared_members`] refuses a member-borne `uid` under either
/// tag.
const COMMITTABLE_MEMBER_TYPES: [&str; 2] = ["UPDATE_VERSION", "ORIGINAL_VERSION"];

/// Refuses a CONTRIBUTION version member that states an attribute the server
/// assigns or self-tags as a class this wire does not commit, and reads its
/// merge provenance.
///
/// `UPDATE_VERSION` is an open object: ITS-REST `schemas/ehr/UpdateVersion.yaml`
/// (and its demographic copy) declares six properties and no
/// `additionalProperties`, where the release closes an object it means to close
/// with `additionalProperties: false` (`ItemTag.yaml`). An undeclared member
/// with no meaning on a locally created `ORIGINAL_VERSION` (RM common
/// `org.openehr.rm.common.original_version.adoc`) is accepted and ignored, as on
/// `NewContribution` and `UpdateAudit`; `item` is one, being `IMPORTED_VERSION`'s.
///
/// Two RM attributes are the server's to assign and refused by name. `uid`: RM
/// common `master06-change_control_package.adoc` §Version Identification sets a
/// local version's `creating_system_id` to "the identifier of the local system"
/// and numbers its `version_tree_id` from 1. `contribution`: master06 §Version
/// and its Subtypes says a committed version "records the Contribution in the
/// `_contribution_` attribute", the one this commit creates. Ignoring either
/// would commit under a value other than the one the client stated.
///
/// `other_input_version_uids` is recorded ([`object_version_id::parse_merge_inputs`]).
///
/// NOTE: `_type` must name "the uppercase class name from the RM specification"
/// (ITS-REST `specifications/docs/overview/Resources.md` §Resource
/// representation), so only a class this wire commits passes.
///
/// # Errors
/// [`ServiceError::BadRequest`] naming the offending key;
/// [`ServiceError::Unprocessable`] for malformed merge provenance.
fn read_undeclared_members(
    undeclared: &std::collections::BTreeMap<String, Value>,
    index: usize,
) -> Result<Vec<String>, ServiceError> {
    for (key, assigned) in [
        (
            "uid",
            "its identity (RM common master06 §Version Identification)",
        ),
        (
            "contribution",
            "the CONTRIBUTION this commit creates (RM common master06 §Version and its Subtypes)",
        ),
    ] {
        if undeclared.contains_key(key) {
            return Err(ServiceError::precondition(format!(
                "versions[{index}]/{key} is not carried on a CONTRIBUTION version member: \
                 this repository assigns a committed version {assigned} and serves the \
                 ORIGINAL_VERSION with it, whichever of UPDATE_VERSION / ORIGINAL_VERSION \
                 the member self-tags as. A version whose identity comes from elsewhere \
                 is an IMPORTED_VERSION (RM common master06 §Copying), which the \
                 EHR-Extract import route commits"
            )));
        }
    }
    match undeclared.get("_type") {
        None => {}
        Some(Value::String(name)) if COMMITTABLE_MEMBER_TYPES.contains(&name.as_str()) => {}
        Some(other) => {
            return Err(ServiceError::precondition(format!(
                "versions[{index}]/_type {other} does not name a class the CONTRIBUTION \
                 commit wire commits: a member self-tags as UPDATE_VERSION or \
                 ORIGINAL_VERSION or not at all (ITS-REST Resources.md: the value \"MUST \
                 be the uppercase class name from the RM specification\"; \
                 UpdateVersion.yaml is the only member schema NewContribution declares). \
                 An IMPORTED_VERSION member has no released shape; the EHR-Extract import \
                 route carries foreign versions instead (RM common master06 §Copying)"
            )));
        }
    }
    undeclared
        .get("other_input_version_uids")
        .map_or(Ok(Vec::new()), object_version_id::parse_merge_inputs)
        .map_err(|e| e.for_version_member(index))
}

/// The pool the pseudonymisation domain this CONTRIBUTION belongs to is served
/// by: the demographic one for a party-only set, the clinical one otherwise.
///
/// A CONTRIBUTION never spans the two domains — `party_only` is the scope check
/// the engine already enforces per member — so one pool serves the whole
/// transaction and the atomicity RM common master06 §Contributions requires is
/// preserved. No openEHR spec governs storage layout — our own
/// design/extension.
fn domain_pool(cx: &impl CommitEnv, party_only: bool) -> &sqlx::PgPool {
    if party_only {
        cx.demographic_pool()
    } else {
        cx.pool()
    }
}

/// Decodes a CONTRIBUTION commit body through the generated `NewContribution`
/// of its API group (`T`), the one door both commit paths, EHR and
/// demographic, read the wire through.
///
/// The released operation types the body `schemas/ehr/NewContribution.yaml`
/// (and its demographic twin): `versions[]` of `schemas/ehr/UpdateVersion.yaml`
/// and an `audit` of `schemas/common/UpdateAudit.yaml`. A body the generated
/// envelope cannot read is missing a required member or carries one in a shape
/// the schema does not give it: the "malformed request syntax, syntactically
/// invalid content" ITS-REST overview `Requests_and_responses.md` answers
/// `400`, the `responses/400_CONTRIBUTION.yaml` the operation declares (it
/// declares no `422`). That covers an absent `audit` or `change_type`, a
/// `lifecycle_state` that is not a `DV_CODED_TEXT` (the `CODE_PHRASE` shape
/// `{terminology_id, code_string}` and plain strings), a `description` that is not a `DV_TEXT` object
/// (`schemas/data_types/UDvText.yaml`), and a commit audit `_type` outside the
/// generated tag set.
///
/// # Errors
/// [`ServiceError::BadRequest`] naming the JSON path the generated decoder
/// refused.
pub(crate) fn decode_new_contribution<T: serde::de::DeserializeOwned>(
    body: &Value,
) -> Result<T, ServiceError> {
    openehr_its::json::from_canonical_value::<T>(body).map_err(|e| {
        ServiceError::bad_request(
            format!(
                "the body is not a NewContribution ({e}): the released commit schema \
                 (ITS-REST schemas/ehr/NewContribution.yaml, UpdateVersion.yaml, \
                 schemas/common/UpdateAudit.yaml) refuses it, so it is syntactically \
                 invalid content (responses/400_CONTRIBUTION.yaml)"
            ),
            e,
        )
    })
}

/// Commit a CONTRIBUTION's version set atomically under one contribution +
/// audit, returning its id together with the commit instant its audit recorded.
/// Shared by the EHR-scoped contribution path (`ehr_id = Some`, `party_only =
/// false`) and the demographic contribution path (`ehr_id = None`, `party_only
/// = true`), each of which hands over the parts of its own group's generated
/// `NewContribution` ([`decode_new_contribution`]). Each version's storage
/// action and preserved audit change-type code come from [`classify`]; the
/// object kind from the payload `_type` (create) or the stored object
/// (modify/delete).
///
/// The target EHR must exist before committing, so a create-only
/// CONTRIBUTION to a missing EHR is a clean `NotFound`, not a storage FK error.
///
/// # Errors
/// [`ServiceError::NotFound`] for a missing target EHR; the [`classify`]
/// rejections per version; [`ServiceError::Unprocessable`] for a malformed
/// CONTRIBUTION `uid`, an empty `versions` list, a scope-mismatched kind, or a
/// failed content/audit validation; [`ServiceError::BadRequest`] when an audit
/// is an `UPDATE_ATTESTATION`, when a `666|attestation|` member carries no
/// attestation, when a member carries an undeclared key, or when a
/// body-referenced modification target does not exist (the `400_CONTRIBUTION`
/// scope); [`ServiceError::Conflict`] for a duplicate EHR singleton/directory;
/// plus the commit-engine placement/storage/signing errors.
pub(crate) async fn commit_version_set(
    cx: &impl CommitEnv,
    ehr_id: Option<EhrId>,
    uid: Option<&HierObjectId>,
    versions: Vec<UpdateVersion<Value>>,
    audit: &UpdateAudit,
    party_only: bool,
) -> Result<CommittedContribution, ServiceError> {
    // the EHR-existence check — the CONTRIBUTION's target EHR must exist.
    if let Some(ehr_id) = ehr_id {
        cx.ensure_ehr_exists(ehr_id).await?;
    }

    // NOTE: `schemas/ehr/NewContribution.yaml` gives `versions` no `minItems`, so
    // an empty list is schema-valid and refused as a semantic error (422).
    if versions.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("must contain at least one version").with_path("versions"),
        ));
    }

    let supplied_uid = parse_supplied_uid(uid)?;
    let contribution_audit = parse_contribution_audit(cx, audit)?;

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
    for v in plan {
        // AUDIT_DETAILS.System_id_valid + committer PARTY invariants — every
        // client-supplied version commit_audit must be a valid RM instance, a
        // `666|attestation|` member's included.
        validate_commit_audit(&v.audit).map_err(|e| e.for_version_member(v.index))?;
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
    // check (the hook doc on `CommitEnv::check_folder_item_refs` carries the
    // ordering rationale — a folder may reference a sibling of its own set).
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
    // settings are change-controlled — RM ehr master04 §EHR Access), so drop the
    // cached settings the access gate consults per request.
    if let Some(ehr_id) = ehr_id
        && outcome.versions.iter().any(|c| c.kind == Kind::EhrAccess)
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
    cx: &impl CommitEnv,
    ehr_id: Option<EhrId>,
    party_only: bool,
    write: ContributionWrite<'_>,
) -> Result<CommittedContribution, ServiceError> {
    let mut conn = acquire(domain_pool(cx, party_only)).await?;
    let mut tx = conn.begin().await?;
    // The content-write gate, under a row lock (RM ehr master04 §EHR Active
    // Status; the in-transaction evaluation is our own recorded semantics —
    // see `change::ensure_content_writable_tx`). A set that itself writes an
    // EHR_STATUS takes the exclusive lock up front (its own commit updates
    // the same row), avoiding a share-then-upgrade deadlock.
    if let Some(ehr_id) = ehr_id
        && write.needs_content_permission
    {
        change::ensure_content_writable_tx(&mut tx, ehr_id, !write.status_commits.is_empty())
            .await?;
    }
    for (_, change) in &write.changes {
        if let Change::Modify {
            vo_id,
            kind: Kind::Composition,
            canonical,
            ..
        } = change
        {
            cx.pre_composition_modify(&mut tx, *vo_id, canonical)
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
            cx.check_folder_item_refs(&mut tx, ehr_id, folder).await?;
        }
        // Keep the EHR's promoted subject columns in sync after each committed
        // EHR_STATUS version (one EHR per subject — RM ehr master04 §EHR
        // Status).
        for status in &write.status_commits {
            cx.post_status_commit(&mut tx, ehr_id, status).await?;
        }
    }
    tx.commit().await?;
    Ok(outcome)
}

/// The client-supplied CONTRIBUTION `uid`, honoured when unused and rejected
/// when malformed (ITS-REST `contribution_create`; master06 §CONTRIBUTION
/// `uid`).
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the supplied value is not a
/// `HIER_OBJECT_ID` UUID.
fn parse_supplied_uid(uid: Option<&HierObjectId>) -> Result<Option<Uuid>, ServiceError> {
    let Some(raw) = uid.map(HierObjectId::value) else {
        return Ok(None);
    };
    #[expect(
        clippy::map_err_ignore,
        reason = "the mapped error already echoes the rejected token; the \
                  discarded `uuid::Error` adds only its own wording, which \
                  is not part of the wire contract"
    )]
    let uid = raw.parse::<Uuid>().map_err(|_| {
        ServiceError::content_invalid(
            Violation::new(format!("{raw:?} is not a valid HIER_OBJECT_ID UUID"))
                .with_path("CONTRIBUTION.uid"),
        )
    })?;
    Ok(Some(uid))
}

/// Parses the CONTRIBUTION's own audit and validates it as an RM instance.
///
/// The generated envelope already required the audit, its change type and its
/// committer (`schemas/ehr/NewContribution.yaml` `required: [versions, audit]`,
/// `schemas/common/UpdateAudit.yaml` `required: [change_type, committer]`), so
/// a body missing any of them never reaches here. An omitted `system_id` is the
/// server's own (`UpdateAudit.yaml`: "when omitted the server sets its own
/// value"), and every member audit that omits one takes this value (RM common
/// master06 §Committal and Audits). `time_committed` is never read from the
/// body: the commit sets it once for the whole set.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the audit is an `UPDATE_ATTESTATION`
/// ([`plain_audit`]); [`ServiceError::Unprocessable`] when the change type is
/// not a code of the openEHR `audit_change_type` group or the audit fails its
/// RM invariants.
fn parse_contribution_audit(
    cx: &impl CommitEnv,
    audit: &UpdateAudit,
) -> Result<AuditInput, ServiceError> {
    let audit = plain_audit(audit, "CONTRIBUTION.audit")?;
    let token = &audit.change_type.defining_code.code_string;
    let code = change_type_code(token).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "{token:?} is not a code in the openEHR audit_change_type group"
            ))
            .with_path("contribution audit change_type")
            .with_invariant("AUDIT_DETAILS.Change_type_valid"),
        )
    })?;
    let audit = AuditInput::from_commit_audit(audit, code, &cx.effective_system_id());
    validate_commit_audit(&audit)?;
    Ok(audit)
}

/// Reads one submitted `UPDATE_VERSION` into a typed plan entry.
///
/// This is the only pass over the member: it classifies the change (master06
/// §Change Type), parses the preceding target, reads the member's own
/// `commit_audit` (the CONTRIBUTION audit's `system_id` when it states none)
/// and captures the lifecycle/signature/attestation envelope. The generated
/// `UpdateVersion` already required `lifecycle_state` as a `DV_CODED_TEXT` and
/// a `commit_audit` with its `change_type` and `committer`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for an unclassifiable change type, an
/// unparsable preceding target, an attestation failing its RM invariants, or
/// (on a `666|attestation|` member) a `lifecycle_state` outside the openEHR
/// `version_lifecycle_state` group; [`ServiceError::precondition`] for an
/// member-borne `uid`, `contribution` or foreign `_type` ([`read_undeclared_members`]) or a
/// `commit_audit` that is an `UPDATE_ATTESTATION` ([`plain_audit`]).
fn plan_version(
    version: UpdateVersion<Value>,
    index: usize,
    contrib_system_id: &str,
) -> Result<PlannedVersion, ServiceError> {
    let other_input_version_uids = read_undeclared_members(&version.additional_properties, index)?;
    let UpdateVersion {
        preceding_version_uid,
        signature,
        lifecycle_state,
        attestations,
        data,
        commit_audit,
        additional_properties: _,
    } = version;
    let member_audit =
        plain_audit(&commit_audit, "commit_audit").map_err(|e| e.for_version_member(index))?;
    let (action, code) = classify(
        &member_audit.change_type.defining_code.code_string,
        preceding_version_uid.is_some(),
        data.is_some(),
    )
    .map_err(|e| e.for_version_member(index))?;

    let target = if action == Action::Create {
        None
    } else {
        Some(
            parse_preceding(preceding_version_uid.as_ref())
                .map_err(|e| e.for_version_member(index))?,
        )
    };
    let audit = AuditInput::from_commit_audit(member_audit, code, contrib_system_id);
    let lifecycle_state = lifecycle_state.defining_code.code_string;
    if action == Action::Attest && !other_input_version_uids.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("names merge inputs, but a 666|attestation| member commits no version")
                .with_path("other_input_version_uids"),
        )
        .for_version_member(index));
    }
    if action == Action::Attest {
        // NOTE: no openEHR spec governs the meaning of lifecycle_state on a member
        // that creates no version, so a 666 member's state is checked against its
        // group here and never stored.
        resolve_lifecycle(Some(lifecycle_state.clone()))
            .map_err(|e| e.for_version_member(index))?;
    }
    if action == Action::Delete {
        reject_contradictory_delete_lifecycle(Some(&lifecycle_state))
            .map_err(|e| e.for_version_member(index))?;
    }
    // A `553|incomplete|` version gets relaxed content validation (master06
    // §Incomplete Content).
    let incomplete = lifecycle_state_code(&lifecycle_state).is_some_and(|c| c == state::INCOMPLETE);
    // Every supplied attestation is converted — never a `filter_map` that
    // would drop one silently.
    let attestations = attestations
        .iter()
        .flatten()
        .map(AttestationInput::from_update)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.for_version_member(index))?;
    Ok(PlannedVersion {
        index,
        action,
        target,
        data,
        audit,
        lifecycle_state,
        incomplete,
        // A client UPDATE_VERSION.signature is stored verbatim; absent, the
        // server signs (master06 §Digital Signature).
        signature,
        attestations,
        other_input_version_uids,
    })
}

/// Reads the stored kind of every modification target of the plan in ONE
/// batched statement.
///
/// # Errors
/// Storage errors from the batched read.
async fn read_target_kinds(
    cx: &impl CommitEnv,
    plan: &[PlannedVersion],
    party_only: bool,
) -> Result<std::collections::HashMap<VoId, Kind>, ServiceError> {
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

/// The stored kind of a body-referenced modification target.
///
/// Every target reached here is body-referenced (the `preceding_version_uid`
/// of a modification/deletion/attestation member), so a missing object is the
/// `400_CONTRIBUTION` scope — the ITS-REST `contribution_create` operation
/// declares `404` only for an unknown `ehr_id` (the URI resource), never for
/// content the committed CONTRIBUTION refers to.
///
/// # Errors
/// [`ServiceError::precondition`] when the target does not exist.
fn require_kind(
    kinds: &std::collections::HashMap<VoId, Kind>,
    vo_id: VoId,
) -> Result<Kind, ServiceError> {
    kinds.get(&vo_id).copied().ok_or_else(|| {
        ServiceError::precondition(format!(
            "modification target does not exist: versioned object {vo_id} \
             (ITS-REST contribution 400 — the modification does not match \
             a stored object)"
        ))
    })
}

/// Resolves a `666|attestation|` member, which attests an existing version
/// instead of committing a new one (master06 §Contributions), into one pending
/// attestation per `UPDATE_VERSION.attestations` entry.
///
/// The attestations travel in the member's `attestations` (ITS-REST
/// `schemas/ehr/UpdateVersion.yaml`, items `schemas/common/UpdateAttestation.yaml`);
/// its `commit_audit` is the plain `UPDATE_AUDIT` carrying the `666` change
/// type. An attestation that states no `system_id` takes the member audit's,
/// which takes the CONTRIBUTION audit's (RM common master06 §Committal and
/// Audits: the contribution's `system_id` "should be copied" into each member).
///
/// # Errors
/// [`ServiceError::precondition`] for a missing target or a member with no
/// attestation to attach; [`ServiceError::Unprocessable`] for a
/// scope-mismatched kind; [`ServiceError::Internal`] when the plan entry lost
/// its parsed target, which [`plan_version`] already required.
fn plan_attestation(
    kinds: &std::collections::HashMap<VoId, Kind>,
    v: PlannedVersion,
    party_only: bool,
) -> Result<Vec<PendingAttest>, ServiceError> {
    let Some((vo_id, expected)) = v.target else {
        return Err(ServiceError::exception(
            "attest plan entry lost its parsed preceding target".to_owned(),
        ));
    };
    let kind = require_kind(kinds, vo_id).map_err(|e| e.for_version_member(v.index))?;
    check_kind_scope(kind, party_only).map_err(|e| e.for_version_member(v.index))?;
    if v.attestations.is_empty() {
        return Err(ServiceError::precondition(format!(
            "versions[{}]: a 666|attestation| member adds an ATTESTATION to the \
             attestations list of an existing ORIGINAL_VERSION (RM common master06 \
             §Contributions), so it carries at least one UPDATE_ATTESTATION in its \
             attestations (ITS-REST schemas/ehr/UpdateVersion.yaml); this one carries \
             none (ITS-REST contribution 400: the modification type does not match the \
             operation)",
            v.index
        )));
    }
    let system_id = v.audit.system_id;
    Ok(v.attestations
        .into_iter()
        .map(|partial| PendingAttest {
            vo_id,
            kind,
            expected,
            system_id: system_id.clone(),
            partial,
        })
        .collect())
}

/// Resolves one non-attestation plan entry into the [`Change`] the commit
/// engine applies, paired with the audit that member committed under.
///
/// # Errors
/// The per-member rejections of the create/modify/delete shapes — see
/// [`create_change`], [`modify_change`] and [`delete_change`].
async fn build_change(
    cx: &impl CommitEnv,
    kinds: &std::collections::HashMap<VoId, Kind>,
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
/// # Errors
/// [`ServiceError::Unprocessable`] for a scope-mismatched kind or content that
/// fails the typed decode or the commit validation.
async fn accept_content(
    cx: &impl CommitEnv,
    kind: Kind,
    data: &Value,
    incomplete: bool,
    index: usize,
    party_only: bool,
) -> Result<Option<String>, ServiceError> {
    check_kind_scope(kind, party_only).map_err(|e| e.for_version_member(index))?;
    typed_decode_gate(kind, data, incomplete).map_err(|e| e.for_version_member(index))?;
    cx.validate_for_commit(kind, data, incomplete)
        .await
        .map_err(|e| e.for_version_member(index))?;
    Ok((kind == Kind::Composition)
        .then(|| crate::service::ehr::validation::composition_template_id(data).map(str::to_owned))
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
    cx: &impl CommitEnv,
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
    let template_id = accept_content(cx, kind, &data, v.incomplete, v.index, party_only).await?;
    if let Some(ehr_id) = ehr_id {
        reject_duplicate_singleton(cx, ehr_id, kind, &data).await?;
    }
    Ok(Change::Create {
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
    cx: &impl CommitEnv,
    kinds: &std::collections::HashMap<VoId, Kind>,
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
    let template_id = accept_content(cx, kind, &data, v.incomplete, v.index, party_only).await?;
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

/// Resolves a deletion member.
///
/// # Errors
/// [`ServiceError::precondition`] when the target does not exist;
/// [`ServiceError::Unprocessable`] for a scope-mismatched kind;
/// [`ServiceError::Conflict`] when the member targets the `EHR_STATUS`, which
/// is mandatory (RM ehr, EHR class: `ehr_status` 1..1) — deleting the only one
/// would leave the EHR violating its own invariant.
fn delete_change(
    kinds: &std::collections::HashMap<VoId, Kind>,
    v: &PlannedVersion,
    party_only: bool,
) -> Result<Change, ServiceError> {
    let Some((vo_id, expected)) = v.target else {
        return Err(ServiceError::exception(
            "delete plan entry lost its parsed preceding target".to_owned(),
        ));
    };
    let kind = require_kind(kinds, vo_id).map_err(|e| e.for_version_member(v.index))?;
    check_kind_scope(kind, party_only).map_err(|e| e.for_version_member(v.index))?;
    if kind == Kind::EhrStatus {
        return Err(ServiceError::conflict(
            "the EHR_STATUS cannot be deleted — EHR.ehr_status is mandatory \
             (RM ehr, EHR class, ehr_status: 1..1)"
                .to_owned(),
        ));
    }
    Ok(Change::Delete {
        vo_id,
        kind,
        expected: Some(expected),
        signature: v.signature.clone(),
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
    cx: &impl CommitEnv,
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
        crate::storage::ehr_repo::ehr_writability(&mut *acquire(cx.pool()).await?, ehr_id).await?;
    if exists && modifiable == Some(false) {
        return Err(change::not_modifiable_error(ehr_id));
    }
    Ok(())
}

/// The planned set's answer to [`needs_content_write_permission`] before any
/// member is decoded: a member whose data names a non-`EHR_STATUS` root (or a
/// deletion, which targets content) is content, and an `EHR_STATUS` member
/// with `is_modifiable = true` reactivates. A member whose type is not a
/// versioned root is left for content validation to name.
fn plan_writes_content_without_reactivating(plan: &[PlannedVersion]) -> bool {
    let mut carries_content = false;
    let mut reactivates = false;
    for v in plan {
        if v.action == Action::Attest {
            continue;
        }
        match v.data.as_ref() {
            None => carries_content = true,
            Some(data) => match data_kind(data) {
                Ok(Kind::EhrStatus) => {
                    reactivates |=
                        data.pointer("/is_modifiable").and_then(Value::as_bool) == Some(true);
                }
                Ok(_) => carries_content = true,
                Err(_) => {}
            },
        }
    }
    carries_content && !reactivates
}

/// Whether this change set needs the EHR to be content-writable.
///
/// `EHR_STATUS.is_modifiable = False` forbids content writes (RM ehr master04
/// §EHR Active Status), so a CONTRIBUTION that creates/modifies/deletes any
/// EHR content is refused on a deactivated EHR. An EHR_STATUS-only
/// CONTRIBUTION stays allowed (the `EHR_STATUS` "can always be written to" — the
/// `ehr_status` class docs), and so does a REACTIVATING mixed CONTRIBUTION:
/// content is refused exactly when the set carries no `EHR_STATUS` member with
/// `is_modifiable = true`, order-independently over the atomic set.
///
/// NOTE: no openEHR spec governs per-member gate timing (the versions list has
/// no spec-assigned order semantics) — our own design.
fn needs_content_write_permission(changes: &[(AuditInput, Change)]) -> bool {
    let carries_content = changes.iter().any(|(_, c)| c.kind() != Kind::EhrStatus);
    let reactivates = changes.iter().any(|(_, c)| match c {
        Change::Create {
            kind: Kind::EhrStatus,
            canonical,
            ..
        }
        | Change::Modify {
            kind: Kind::EhrStatus,
            canonical,
            ..
        } => canonical.pointer("/is_modifiable").and_then(Value::as_bool) == Some(true),
        _ => false,
    });
    carries_content && !reactivates
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
/// already-live same-identity folder hierarchy; the `CommitEnv` lookup errors.
async fn reject_duplicate_singleton(
    cx: &impl CommitEnv,
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
            if cx.folder_root_exists(ehr_id, root, name).await? {
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

// NOTE: no aggregate is derived — master06 §Contributions calls a
// CONTRIBUTION-level change type "approximate, and not expected to be used as a
// computable value", so the commit path refuses an omitted one instead.

/// Enforces that a version's object kind matches the contribution's scope: a
/// demographic contribution (`party_only`) may carry only party roots and
/// `PARTY_RELATIONSHIP`, and an EHR contribution may carry neither.
///
/// NOTE: ITS-REST `schemas/ehr/UVersionable.yaml` and `schemas/demographic/UVersionable.yaml`
/// close each member set with a `oneOf`, so a member outside it fails the request schema (400).
///
/// `FOLDER` is EHR-scoped in both directions: no released service surface
/// admits a demographic folder (every ITS-REST `directory_*` operation is
/// mounted at `/ehr/{ehr_id}/directory`).
///
/// # Errors
/// [`ServiceError::BadRequest`] (`400`, ITS-REST `responses/400_CONTRIBUTION.yaml`)
/// on a scope mismatch in either direction.
fn check_kind_scope(kind: Kind, party_only: bool) -> Result<(), ServiceError> {
    if party_only && !kind.is_demographic() {
        return Err(ServiceError::precondition(format!(
            "a member of a demographic CONTRIBUTION may only be a demographic version, \
             got {}",
            kind.as_str()
        )));
    }
    if !party_only && kind.is_demographic() {
        return Err(ServiceError::precondition(format!(
            "a member of an EHR CONTRIBUTION may not be a demographic version, got {}",
            kind.as_str()
        )));
    }
    Ok(())
}

/// Refuses a delete member whose declared `lifecycle_state` is not the
/// `523|deleted|` its own change type commits.
///
/// RM common `master06-change_control_package.adoc` §Logical Deletion states the
/// deletion as one procedure whose third step sets `_lifecycle_state_` to the
/// code for `deleted`, so a member pairing change type `523|deleted|` with any
/// other state asks for two contradictory things. The code is the released
/// `400_CONTRIBUTION` change-control trigger, "the modification type does not
/// match the operation" (`responses/400_CONTRIBUTION.yaml`). An absent state is
/// [`commit_version_set`]'s business, not this function's.
///
/// # Errors
/// [`ServiceError::BadRequest`] when `declared` resolves to a
/// `version_lifecycle_state` other than `523|deleted|`;
/// [`ServiceError::Unprocessable`] when it names no member of that group at
/// all (`ORIGINAL_VERSION.Lifecycle_state_valid`).
fn reject_contradictory_delete_lifecycle(declared: Option<&str>) -> Result<(), ServiceError> {
    let Some(declared) = declared else {
        return Ok(());
    };
    let code = resolve_lifecycle(Some(declared.to_owned()))?;
    if code == state::DELETED {
        return Ok(());
    }
    Err(ServiceError::precondition(format!(
        "lifecycle_state {code}|{}| contradicts change_type 523|deleted| — logical \
         deletion deletes the version's data and sets the state to the code for \
         deleted in one act, so a delete member commits a 523|deleted| version (RM \
         common master06 §Logical Deletion; ITS-REST contribution 400: the \
         modification type does not match the operation)",
        lifecycle_rubric(&code)
    )))
}

/// The versioned-object kind of a VERSION's `data`, from its `_type`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the `_type` is not a versioned root.
fn data_kind(data: &Value) -> Result<Kind, ServiceError> {
    let rm_type = data
        .get("_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Kind::from_type(rm_type).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "names {rm_type:?}, which is not a versioned root type"
            ))
            .with_path("data._type"),
        )
    })
}

/// The strict canonical-JSON door over a raw commit member's `data`.
///
/// The generated envelope leaves a CONTRIBUTION member's `data` raw (the
/// `NewContribution<Value>` instantiation keeps the bytes the signature
/// covers), so the payload runs the same typed door every direct commit route
/// passes through: content that cannot be converted to
/// its RM resource is the 400 row (ITS-REST overview `Requests_and_responses.md`
/// §HTTP status codes). The demographic kinds keep their typed door in
/// `service::demographic::validate`; this gate covers the EHR kinds. A
/// `553|incomplete|` commit skips it, the generated types making mandatory
/// attributes structural where RM common master06 §Incomplete Content lifts
/// those bounds.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the strict reader refuses the payload.
fn typed_decode_gate(kind: Kind, data: &Value, incomplete: bool) -> Result<(), ServiceError> {
    if incomplete {
        return Ok(());
    }
    let refused = match kind {
        Kind::Composition => {
            openehr_its::json::from_canonical_value::<openehr_rm::prelude::Composition>(data)
                .map(drop)
                .err()
        }
        Kind::EhrStatus => {
            openehr_its::json::from_canonical_value::<openehr_rm::prelude::EhrStatus>(data)
                .map(drop)
                .err()
        }
        Kind::EhrAccess => {
            openehr_its::json::from_canonical_value::<openehr_rm::prelude::EhrAccess>(data)
                .map(drop)
                .err()
        }
        Kind::Folder => {
            openehr_its::json::from_canonical_value::<openehr_rm::prelude::Folder>(data)
                .map(drop)
                .err()
        }
        Kind::Agent
        | Kind::Group
        | Kind::Organisation
        | Kind::Person
        | Kind::Role
        | Kind::PartyRelationship => None,
    };
    match refused {
        Some(e) => Err(ServiceError::canonical_decode(data, e)),
        None => Ok(()),
    }
}

/// Parse a VERSION's `preceding_version_uid` into the object id and the
/// version it must currently be at, through the strict BASE three-part parse.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when absent on a modify/delete/attest item,
/// or when the value is not a valid `OBJECT_VERSION_ID`.
fn parse_preceding(preceding: Option<&ObjectVersionId>) -> Result<(VoId, TreeId), ServiceError> {
    let raw = preceding.map(ObjectVersionId::value).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new("required for modify/delete").with_path("preceding_version_uid"),
        )
    })?;
    Ok(object_version_id::parse_version_uid(raw)?)
}

/// Retrieve a CONTRIBUTION by id (scoped to the EHR), with its audit and the
/// `OBJECT_REF`s of the versions it committed. With `resolve_refs` the
/// `versions` list carries the resolved `ORIGINAL_VERSION` objects instead of
/// `OBJECT_REF`s (ITS-REST `Prefer: resolve_refs`).
///
/// # Errors
/// [`ServiceError::NotFound`] when the CONTRIBUTION does not exist in `ehr_id`,
/// or (under `resolve_refs`) a referenced version row is gone; the storage read
/// errors; the [`version_envelope`] verification error under `resolve_refs`.
pub(crate) async fn get_contribution(
    pool: &sqlx::PgPool,
    signer: &Signer,
    profile: crate::config::profile::SpecProfile,
    ehr_id: EhrId,
    contribution_id: Uuid,
    resolve_refs: bool,
) -> Result<Value, ServiceError> {
    let audit = crate::storage::version_repo::contribution::contribution_audit(
        pool,
        contribution_id,
        Some(ehr_id),
    )
    .await?
    .ok_or_else(|| {
        ServiceError::new(
            ServiceStatus::ContributionDoesNotExist,
            format!("CONTRIBUTION {contribution_id}"),
        )
    })?;
    let time_committed = audit.time_committed;

    // CONTRIBUTION.versions lists the affected VERSION objects (master06
    // §Contributions); a 666 attestation commits no new version but still
    // affects an existing one, so the storage query unions the versions
    // referenced by this contribution's `vo_attestation` rows (dedup).
    let referenced = crate::storage::version_repo::contribution::contribution_version_refs(
        pool,
        contribution_id,
    )
    .await?;

    // Resolved members load in ONE batched statement (never one point read
    // per member — a K-member CONTRIBUTION resolves with K+2 statements
    // otherwise).
    let mut resolved = if resolve_refs {
        let refs: Vec<(VoId, TreeId)> = referenced
            .iter()
            .map(|(vo_id, (t, b, v), _, _)| (*vo_id, TreeId::from_columns(*t, *b, *v)))
            .collect();
        read::read_versions(pool, profile, &refs).await?
    } else {
        std::collections::HashMap::new()
    };

    let mut versions = Vec::with_capacity(referenced.len());
    for (vo_id, (t, b, v), creating_system_id, kind) in referenced {
        let tree = TreeId::from_columns(t, b, v);
        if resolve_refs {
            let loaded = resolved.remove(&(vo_id, tree)).ok_or_else(|| {
                ServiceError::new(
                    ServiceStatus::ObjectVersionDoesNotExist,
                    format!("VERSION {vo_id}::{tree}"),
                )
            })?;
            // `CONTRIBUTION.versions` lists the affected VERSION objects
            // (master06 §Contributions), so an imported member resolves to its
            // IMPORTED_VERSION, not the ORIGINAL_VERSION it wraps.
            versions.push(version_envelope(&loaded, signer)?);
        } else {
            versions.push(openehr_its::json::to_canonical_value(
                &ObjectRef::ObjectRef(ObjectRefData {
                    namespace: "local".to_owned(),
                    r#type: kind.clone(),
                    id: ObjectId::ObjectVersionId(object_version_id::version_id(
                        vo_id,
                        &creating_system_id,
                        tree,
                    )?),
                }),
            ));
        }
    }

    let audit_details = AuditInput {
        system_id: audit.system_id,
        change_type: audit.change_type,
        description: audit
            .description
            .as_ref()
            .map(decode_description)
            .transpose()?,
        committer: party_proxy(&audit.committer)?,
        attestation: audit
            .attestation
            .as_ref()
            .map(AttestationParts::decode)
            .transpose()?
            .map(Box::new),
    }
    .canonical(&time_committed);
    // NOTE: a JSON-literal envelope over already-canonical parts — `versions`
    // holds EITHER `OBJECT_REF`s OR whole resolved VERSION envelopes, which the
    // generated `Contribution.versions: Vec<ObjectRef>` cannot express.
    Ok(json!({
        "_type": "CONTRIBUTION",
        "uid": openehr_its::json::to_canonical_value(&HierObjectId::from(contribution_id)),
        "audit": audit_details,
        "versions": versions
    }))
}

/// The number of CONTRIBUTIONs in
/// the EHR within the optional commit-time window.
///
/// # Errors
/// [`ServiceError::NotFound`] when the EHR does not exist; the storage read
/// error of the count query.
pub(crate) async fn count_contributions(
    pool: &sqlx::PgPool,
    ehr_id: EhrId,
    time_range: TimeRange,
) -> Result<i64, ServiceError> {
    ensure_ehr_exists(pool, ehr_id).await?;
    let (lower, upper) = time_range.unwrap_or((None, None));
    Ok(
        crate::storage::version_repo::contribution::count_contributions(pool, ehr_id, lower, upper)
            .await?,
    )
}

/// The EHR-existence precheck for the read paths above (`ehr_id_does_not_exist`
/// → `NotFound`).
///
/// Storage exposes the read (`version_repo::meta::ehr_exists`) so versioning stays
/// self-contained.
async fn ensure_ehr_exists(pool: &sqlx::PgPool, ehr_id: EhrId) -> Result<(), ServiceError> {
    if crate::storage::version_repo::meta::ehr_exists(pool, ehr_id).await? {
        Ok(())
    } else {
        Err(ServiceError::new(
            ServiceStatus::EhrIdDoesNotExist,
            format!("EHR {ehr_id}"),
        ))
    }
}

/// Inside a CONTRIBUTION commit the modification/deletion target is
/// **body-referenced** (`preceding_version_uid`), not the request URI: the
/// ITS-REST `contribution_create` operation declares `404` only for an
/// unknown `ehr_id` (the URI resource), so a missing target versioned object
/// is a change-control mismatch in the committed content — the
/// `400_CONTRIBUTION` scope — never a 404.
fn body_target_not_found_is_bad_request(e: ServiceError) -> ServiceError {
    match e {
        ServiceError::NotFound(m) => ServiceError::precondition(format!(
            "modification target does not exist: {m} (ITS-REST contribution 400 — \
             the modification does not match a stored object)"
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests;
