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

use openehr_base::prelude::{HierObjectId, ObjectId, ObjectRef, ObjectRefData};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::error::{ServiceError, Violation};
use crate::service::list::Page;
use crate::service::status::CallStatusType;
use crate::versioning::attestation::{AttestationParts, PendingAttest};
use crate::versioning::audit::{
    AuditInput, change_type, change_type_code, decode_description, dv_text, party_proxy,
    required_committer, validate_commit_audit,
};
use crate::versioning::change::{Change, CommittedContribution};
use crate::versioning::lifecycle::{
    lifecycle_rubric, lifecycle_state_code, resolve_lifecycle, state,
};
use crate::versioning::object_version_id::{self, TreeId};
use crate::versioning::signature::signer::Signer;
use crate::versioning::wire::version_envelope;
use crate::versioning::{CommitEnv, Kind, change, read};

/// An optional `(lower, upper)` inclusive commit-time window — the simple
/// realization of the SM `Interval<Iso8601_date_time>` (either side open when
/// its bound is `None`; the whole `Option` `None` = unbounded).
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
    token: Option<&str>,
    has_preceding: bool,
    has_data: bool,
) -> Result<(Action, String), ServiceError> {
    let code = resolve_change_type(token, has_preceding)?;
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
/// Without a client change type the code is inferred from the presence of
/// `preceding_version_uid`: creation for a first version, modification
/// otherwise.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the supplied token is not a code of
/// the openEHR `audit_change_type` group.
fn resolve_change_type(token: Option<&str>, has_preceding: bool) -> Result<String, ServiceError> {
    let Some(t) = token else {
        if has_preceding {
            return Ok(change_type::MODIFICATION.to_owned());
        }
        return Ok(change_type::CREATION.to_owned());
    };
    change_type_code(t).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "{t:?} is not a code in the openEHR audit_change_type group"
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
    /// The raw `commit_audit` (the 666 attestation payload).
    commit_audit: Option<Value>,
    lifecycle_state: Option<String>,
    incomplete: bool,
    signature: Option<String>,
    accompanying: Vec<Value>,
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
/// server still supplies every attribute the partial omits, `uid` included, so
/// [`reject_foreign_version_identity`] refuses a member-borne `uid` under either
/// tag.
const COMMITTABLE_MEMBER_TYPES: [&str; 2] = ["UPDATE_VERSION", "ORIGINAL_VERSION"];

/// Refuses a CONTRIBUTION version member that declares a version identity this
/// repository did not create: the three keys of an `IMPORTED_VERSION` shape.
///
/// The released commit wire declares six member properties (ITS-REST
/// `schemas/ehr/UpdateVersion.yaml`) and `schemas/ehr/NewContribution.yaml`
/// types `versions` items as `UpdateVersion` with no `oneOf` and no
/// discriminator, so no member can name its own `uid` or carry the `item` an
/// `IMPORTED_VERSION` wraps. RM common `master06-change_control_package.adoc`
/// §Copying puts the import behind `VERSIONED_OBJECT.commit_imported_version`,
/// for which the release defines no wire shape; the EHR-Extract import route
/// carries a foreign `ORIGINAL_VERSION` verbatim instead. Left accepted, such a
/// member would commit as a locally created `ORIGINAL_VERSION` under a freshly
/// minted uid, discarding the declared foreign identity without a diagnostic.
///
/// NOTE: `_type` is the one of the three keys with a legal value here, its value
/// being "the uppercase class name from the RM specification" (ITS-REST
/// `specifications/docs/overview/Resources.md` §Resource representation), but
/// only for a class this wire commits ([`COMMITTABLE_MEMBER_TYPES`]).
///
/// # Errors
/// [`ServiceError::BadRequest`] naming the offending key.
fn reject_foreign_version_identity(version: &Value, index: usize) -> Result<(), ServiceError> {
    // Anything outside the declared set is refused with the member index in
    // the path.
    // NOTE: the commit wire declares exactly these six member properties
    // (ITS-REST `schemas/ehr/UpdateVersion.yaml`) plus the `_type` self-tag
    // (overview `Resources.md` §Resource representation).
    const DECLARED: [&str; 7] = [
        "preceding_version_uid",
        "signature",
        "lifecycle_state",
        "attestations",
        "data",
        "commit_audit",
        "_type",
    ];
    const SPECIFICALLY_REFUSED: [&str; 3] = ["item", "uid", "other_input_version_uids"];
    if let Some(map) = version.as_object() {
        for key in map.keys() {
            if !DECLARED.contains(&key.as_str()) && !SPECIFICALLY_REFUSED.contains(&key.as_str()) {
                return Err(ServiceError::precondition(format!(
                    "versions[{index}]/{key} is not a member of UPDATE_VERSION — the \
                     released commit wire declares preceding_version_uid, signature, \
                     lifecycle_state, attestations, data, commit_audit (ITS-REST \
                     UpdateVersion.yaml), and a member may additionally self-tag with \
                     _type; undeclared members are refused, never silently ignored"
                )));
            }
        }
    }
    if version.get("item").is_some() {
        return Err(ServiceError::precondition(
            "item is not a member of UPDATE_VERSION — it is the sole own attribute of \
             IMPORTED_VERSION, and the released commit wire declares no import shape \
             (ITS-REST UpdateVersion.yaml / NewContribution.yaml). An imported \
             ORIGINAL_VERSION is wrapped and committed by the EHR-Extract import route \
             (RM common master06 §Copying), never by a CONTRIBUTION member"
                .to_owned(),
        ));
    }
    if version.get("uid").is_some() {
        return Err(ServiceError::precondition(
            "uid is not carried on a CONTRIBUTION version member — the member is the \
             commit-wire PARTIAL of the version it becomes (ITS-REST \
             UpdateVersion.yaml declares six properties and no uid), and the version \
             identifier of a locally committed version is allocated by this repository, \
             which then serves the completed ORIGINAL_VERSION with its mandatory uid \
             filled in. This holds whichever of UPDATE_VERSION / ORIGINAL_VERSION the \
             member self-tags as. A version whose identity comes from elsewhere is an \
             IMPORTED_VERSION, which RM common master06 §Copying commits through \
             commit_imported_version — an operation the release gives no wire shape"
                .to_owned(),
        ));
    }
    match version.get("_type") {
        None => Ok(()),
        Some(Value::String(name)) if COMMITTABLE_MEMBER_TYPES.contains(&name.as_str()) => Ok(()),
        Some(other) => Err(ServiceError::precondition(format!(
            "_type {other} does not name a class the CONTRIBUTION commit wire commits \
             — a member self-tags as UPDATE_VERSION or ORIGINAL_VERSION or not at all \
             (ITS-REST Resources.md: the value \"MUST be the uppercase class name from \
             the RM specification\"; UpdateVersion.yaml is the only member schema \
             NewContribution declares). An IMPORTED_VERSION member has no released \
             shape — the EHR-Extract import route carries foreign versions instead \
             (RM common master06 §Copying)"
        ))),
    }
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

/// Commit a CONTRIBUTION's version set atomically under one contribution +
/// audit, returning its id together with the commit instant its audit recorded.
/// Shared by the EHR-scoped contribution path (`ehr_id = Some`, `party_only =
/// false`) and the demographic contribution path (`ehr_id = None`, `party_only
/// = true`). Each version's storage action and preserved audit change-type code
/// come from [`classify`]; the object kind from the payload `_type` (create) or
/// the stored object (modify/delete).
///
/// SM `i_ehr_contribution.adoc` §`commit_contribution`
/// `Pre_has_ehr`: the target EHR must exist before committing, so a create-only
/// CONTRIBUTION to a missing EHR is a clean `NotFound`, not a storage FK error.
///
/// # Errors
/// [`ServiceError::NotFound`] for a missing target EHR; the [`classify`]
/// rejections per version; [`ServiceError::Unprocessable`] for a malformed
/// CONTRIBUTION `uid`, an empty `versions` list, a scope-mismatched kind, or a
/// failed content/audit validation; [`ServiceError::BadRequest`] when the
/// CONTRIBUTION `audit`, a member `commit_audit` or an attestation states no
/// `committer`, when a member states no `lifecycle_state`, or when a
/// body-referenced modification target does not exist (the `400_CONTRIBUTION`
/// scope); [`ServiceError::Conflict`] for a duplicate EHR singleton/directory;
/// plus the commit-engine placement/storage/signing errors.
pub(crate) async fn commit_version_set(
    cx: &impl CommitEnv,
    ehr_id: Option<EhrId>,
    body: &Value,
    party_only: bool,
) -> Result<CommittedContribution, ServiceError> {
    // `Pre_has_ehr` — the CONTRIBUTION's target EHR must exist.
    if let Some(ehr_id) = ehr_id {
        cx.ensure_ehr_exists(ehr_id).await?;
    }

    let versions = body
        .get("versions")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ServiceError::content_invalid(
                Violation::new("must contain at least one version").with_path("versions"),
            )
        })?;

    let supplied_uid = parse_supplied_uid(body)?;
    let contribution_audit = parse_contribution_audit(cx, body)?;

    // Each UPDATE_VERSION is read exactly once into a typed plan entry; the
    // modification targets are then existence/kind-checked in ONE batched
    // statement and resolved without re-reading any JSON.
    let mut plan: Vec<PlannedVersion> = Vec::with_capacity(versions.len());
    for (index, version) in versions.iter().enumerate() {
        plan.push(plan_version(version, index, &contribution_audit.system_id)?);
    }

    let target_kinds = read_target_kinds(cx, &plan, party_only).await?;

    refuse_content_into_inactive_ehr(cx, ehr_id, &plan).await?;

    let mut changes: Vec<(AuditInput, Change)> = Vec::with_capacity(plan.len());
    // 666 attestations of existing versions (committing no new version).
    let mut attests: Vec<PendingAttest> = Vec::new();
    for v in plan {
        if v.action == Action::Attest {
            attests.push(plan_attestation(&target_kinds, v, party_only)?);
            continue;
        }
        // AUDIT_DETAILS.System_id_valid + committer PARTY invariants — a
        // client-supplied version commit_audit must be a valid RM instance.
        validate_commit_audit(&v.audit).map_err(|e| e.for_version_member(v.index))?;
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
    let mut tx = domain_pool(cx, party_only).begin().await?;
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
fn parse_supplied_uid(body: &Value) -> Result<Option<Uuid>, ServiceError> {
    let Some(raw) = body
        .get("uid")
        .and_then(|u| u.get("value"))
        .and_then(Value::as_str)
    else {
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
/// Its change type and committer are the client's account of the change set
/// and both are required: RM common `audit_details.adoc` §Attributes types each
/// 1..1 on the mandatory `CONTRIBUTION.audit`, and the released commit schema
/// requires both (`schemas/common/UpdateAudit.yaml` `required: [change_type,
/// committer]`). An absent change type is refused rather than derived, master06
/// §Contributions calling the aggregate "approximate, and not expected to be
/// used as a computable value". An omitted `system_id` is the server's own
/// (`UpdateAudit.yaml`: "when omitted the server sets its own value"), and every
/// member audit that omits one takes this value (RM common master06 §Committal
/// and Audits). `time_committed` is never read from the body: the commit sets
/// it once for the whole set.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the committer is absent;
/// [`ServiceError::Unprocessable`] when the change type is absent, is not a
/// code of the openEHR `audit_change_type` group, the committer is not a valid
/// `PARTY_PROXY`, or the audit fails its RM invariants.
fn parse_contribution_audit(cx: &impl CommitEnv, body: &Value) -> Result<AuditInput, ServiceError> {
    let audit = body.get("audit");
    let token = audit
        .and_then(|a| a.get("change_type"))
        .and_then(coded_value)
        .ok_or_else(|| {
            ServiceError::content_invalid(
                Violation::new(
                    "is required on a CONTRIBUTION audit — the change set's own change \
                     type is the client's account of it and is never derived by the server",
                )
                .with_path("CONTRIBUTION.audit.change_type"),
            )
        })?;
    let code = change_type_code(&token).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "{token:?} is not a code in the openEHR audit_change_type group"
            ))
            .with_path("contribution audit change_type")
            .with_invariant("AUDIT_DETAILS.Change_type_valid"),
        )
    })?;
    let audit = parse_audit(
        audit,
        code,
        &cx.effective_system_id(),
        true,
        "CONTRIBUTION.audit",
    )?;
    validate_commit_audit(&audit)?;
    Ok(audit)
}

/// Reads one submitted `UPDATE_VERSION` into a typed plan entry.
///
/// This is the only pass that touches the member's JSON: it classifies the
/// change (master06 §Change Type), parses the preceding target, reads the
/// member's own `commit_audit` (its required `committer`, and the
/// CONTRIBUTION audit's `system_id` when it states none) and captures the
/// lifecycle/signature/attestation envelope.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for a member-borne version identity, an
/// unclassifiable change type, an unparsable preceding target, an invalid
/// audit, or (on a `666|attestation|` member) a `lifecycle_state` outside the
/// openEHR `version_lifecycle_state` group; [`ServiceError::precondition`] for a
/// `commit_audit` without `committer`, a member carrying
/// `other_input_version_uids` (not a member of `UPDATE_VERSION` on this wire;
/// merge provenance is produce-only) or a member omitting the required
/// `lifecycle_state`, the `666|attestation|` member included (SM master03
/// §Version Update Semantics: "must be supplied in all cases").
fn plan_version(
    version: &Value,
    index: usize,
    contrib_system_id: &str,
) -> Result<PlannedVersion, ServiceError> {
    reject_foreign_version_identity(version, index)?;
    let token = version
        .get("commit_audit")
        .and_then(|a| a.get("change_type"))
        .and_then(coded_value);
    let data = version.get("data").cloned().filter(|d| !d.is_null());
    // NOTE: a first version legitimately carries no `preceding_version_uid`
    // (SM `update_version.adoc` types it `0..1`), and the SM glue serializes
    // a `None` preceding to JSON `null`, so a `null` counts as absent.
    let has_preceding = version
        .get("preceding_version_uid")
        .is_some_and(|v| !v.is_null());
    let (action, code) = classify(token.as_deref(), has_preceding, data.is_some())
        .map_err(|e| e.for_version_member(index))?;

    let target = if action == Action::Create {
        None
    } else {
        Some(parse_preceding(version).map_err(|e| e.for_version_member(index))?)
    };
    // A `666|attestation|` member's commit_audit is the ATTESTATION it attaches,
    // not that member's own commit audit (master06 §Contributions); the
    // attestation path owns its ATTESTATION-declared attributes.
    let audit = parse_audit(
        version.get("commit_audit"),
        code,
        contrib_system_id,
        action != Action::Attest,
        "commit_audit",
    )
    .map_err(|e| e.for_version_member(index))?;
    let lifecycle_state = lifecycle_of(version);
    if version.get("other_input_version_uids").is_some() {
        return Err(ServiceError::precondition(format!(
            "versions[{index}]: other_input_version_uids is not a member of \
             UPDATE_VERSION — the released commit wire declares no merge shape \
             (ITS-REST UpdateVersion.yaml); merge provenance is served on reads \
             only (OriginalVersion.yaml)"
        )));
    }
    if lifecycle_state.is_none() {
        return Err(ServiceError::precondition(format!(
            "versions[{index}]: lifecycle_state is required on every CONTRIBUTION \
             version, an attestation (666) member included (SM master03 §Version \
             Update Semantics: \"The lifecycle_state must be supplied in all \
             cases\"; ITS-REST UpdateVersion.yaml lists it under required)"
        )));
    }
    if action == Action::Attest {
        // An attestation commits no version, so no later commit step checks the
        // stated state against the `version_lifecycle_state` group.
        resolve_lifecycle(lifecycle_state.clone()).map_err(|e| e.for_version_member(index))?;
    }
    if action == Action::Delete {
        reject_contradictory_delete_lifecycle(lifecycle_state.as_deref())
            .map_err(|e| e.for_version_member(index))?;
    }
    // A `553|incomplete|` version gets relaxed content validation (master06
    // §Incomplete Content).
    let incomplete = lifecycle_state
        .as_deref()
        .and_then(lifecycle_state_code)
        .is_some_and(|c| c == state::INCOMPLETE);
    Ok(PlannedVersion {
        index,
        action,
        target,
        data,
        audit,
        commit_audit: version.get("commit_audit").cloned(),
        lifecycle_state,
        incomplete,
        // A client UPDATE_VERSION.signature is stored verbatim; absent, the
        // server signs (master06 §Digital Signature).
        signature: version
            .get("signature")
            .and_then(Value::as_str)
            .map(str::to_owned),
        accompanying: attestation_partials(version),
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
/// instead of committing a new one (master06 §Contributions).
///
/// # Errors
/// [`ServiceError::precondition`] for a missing target, [`ServiceError::Unprocessable`]
/// for a scope-mismatched kind, a missing `commit_audit` (the
/// `UPDATE_ATTESTATION` payload), or an undecodable attestation.
fn plan_attestation(
    kinds: &std::collections::HashMap<VoId, Kind>,
    v: PlannedVersion,
    party_only: bool,
) -> Result<PendingAttest, ServiceError> {
    let Some((vo_id, expected)) = v.target else {
        return Err(ServiceError::exception(
            "attest plan entry lost its parsed preceding target".to_owned(),
        ));
    };
    let kind = require_kind(kinds, vo_id).map_err(|e| e.for_version_member(v.index))?;
    check_kind_scope(kind, party_only).map_err(|e| e.for_version_member(v.index))?;
    let partial = v.commit_audit.ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new("is required on a 666 attestation version (the UPDATE_ATTESTATION)")
                .with_path("commit_audit"),
        )
    })?;
    Ok(PendingAttest {
        vo_id,
        kind,
        expected,
        partial: crate::versioning::attestation::AttestationInput::decode(&partial)?,
    })
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
        lifecycle_state: v.lifecycle_state.clone(),
        attestations: accompanying(&v.accompanying).map_err(|e| e.for_version_member(v.index))?,
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
        lifecycle_state: v.lifecycle_state.clone(),
        attestations: accompanying(&v.accompanying).map_err(|e| e.for_version_member(v.index))?,
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
        crate::storage::ehr_repo::ehr_writability(cx.pool(), ehr_id).await?;
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

/// The concrete `_type` values a commit audit may carry on the native
/// CONTRIBUTION wire, and whether each one commits an `ATTESTATION`.
///
/// The ITS-REST docs text points the native commit at the RM (overview
/// `Requests_and_responses.md`), and RM common master06 §Committal and Audits
/// makes `ATTESTATION` an admissible `VERSION.commit_audit` ("`AUDIT_DETAILS` …
/// or its subtype `ATTESTATION`"). The released OAS enumerates only
/// `UPDATE_AUDIT` / `AUDIT_DETAILS` / an omitted `_type`
/// (`schemas/common/UpdateAudit.yaml`), and the docs text wins on that
/// disagreement. The `UPDATE_*` spellings are the released commit DTOs of the
/// same two RM classes, accepted alike. Any other `_type` is refused.
fn commit_audit_is_attestation(audit: &Value) -> Result<bool, ServiceError> {
    match audit.get("_type").and_then(Value::as_str) {
        None | Some("UPDATE_AUDIT" | "AUDIT_DETAILS") => Ok(false),
        Some("UPDATE_ATTESTATION" | "ATTESTATION") => Ok(true),
        Some(other) => Err(ServiceError::content_invalid(
            Violation::new(format!(
                "_type {other:?} is not an AUDIT_DETAILS or its ATTESTATION subtype"
            ))
            .with_path("VERSION.commit_audit")
            .with_invariant("RM common master06 §Committal and Audits"),
        )),
    }
}

/// Decode a CONTRIBUTION version's `UPDATE_VERSION.attestations` — the
/// attestations committed WITH the new version (master06 §Attestation, "Signing
/// content at committal") — into the typed carrier the commit path completes.
///
/// # Errors
/// The [`crate::versioning::attestation::AttestationInput::decode`] rejections.
fn accompanying(
    partials: &[Value],
) -> Result<Vec<crate::versioning::attestation::AttestationInput>, ServiceError> {
    partials
        .iter()
        .map(crate::versioning::attestation::AttestationInput::decode)
        .collect()
}

/// Builds an [`AuditInput`] from an ITS-REST audit object and the
/// already-resolved numeric `audit_change_type` code.
///
/// The object is an `UpdateAudit`, an `UpdateAttestation`, or the RM class
/// either partials. It is used both for the CONTRIBUTION's own audit and for
/// each VERSION's `commit_audit`. The `committer` is the one the object states,
/// required on every audit ([`required_committer`]); an omitted `system_id` is
/// `default_system_id`, the CONTRIBUTION audit's value for a member (RM common
/// master06 §Committal and Audits). `owner` names the audit in a refusal.
///
/// `attestable` is false for a `666|attestation|` member, whose `commit_audit`
/// is the `ATTESTATION` attached to an existing version rather than that
/// member's own commit audit; the attestation path
/// ([`crate::versioning::attestation::complete_attestation`]) owns it.
///
/// # Errors
/// [`ServiceError::BadRequest`] when `committer` is absent;
/// [`ServiceError::Unprocessable`] when the committer is not a canonical
/// `PARTY_PROXY`, when the `_type` names neither `AUDIT_DETAILS` nor its
/// `ATTESTATION` subtype, when `description` is neither a string nor a
/// canonical `DV_TEXT`, or when the `ATTESTATION`-declared attributes fail
/// their RM invariants
/// ([`crate::versioning::attestation::AttestationParts::decode`]).
fn parse_audit(
    audit: Option<&Value>,
    change_type: String,
    default_system_id: &str,
    attestable: bool,
    owner: &str,
) -> Result<AuditInput, ServiceError> {
    // NOTE: RM common master06 §Committal and Audits + SM `update_version.adoc`
    // (`audit` 1..1 per version): a member's own required committer is stored as
    // stated, never replaced by the CONTRIBUTION audit's.
    let committer = required_committer(audit, owner)?;
    // Both released spellings of `description` are accepted: the ITS-REST
    // `UDvText` object (`schemas/data_types/UDvText.yaml`) and the SM
    // `String [0..1]` (`update_audit.adoc` §Attributes). The object spelling is
    // kept whole — a DV_CODED_TEXT's `defining_code` is part of the audit.
    let description = audit
        .and_then(|a| a.get("description"))
        .filter(|d| !d.is_null())
        .map(|d| match d {
            Value::String(s) => Ok(dv_text(s)),
            other => decode_description(other),
        })
        .transpose()?;
    let attestation = match audit {
        // The `_type` is checked for EVERY commit audit, attestable or not — an
        // unknown class is refused wherever it appears.
        Some(a) if commit_audit_is_attestation(a)? && attestable => {
            Some(Box::new(AttestationParts::decode(a)?))
        }
        _ => None,
    };
    let system_id = audit
        .and_then(|a| a.get("system_id"))
        .and_then(Value::as_str)
        .map_or_else(|| default_system_id.to_owned(), str::to_owned);
    Ok(AuditInput {
        system_id,
        change_type,
        description,
        committer,
        attestation,
    })
}

// NOTE: no aggregate is derived — master06 §Contributions calls a
// CONTRIBUTION-level change type "approximate, and not expected to be used as a
// computable value", so the commit path refuses an omitted one instead.

/// Enforces that a version's object kind matches the contribution's scope: a
/// demographic contribution (`party_only`) may carry only party roots and
/// `PARTY_RELATIONSHIP`, and an EHR contribution may carry neither.
///
/// NOTE: `FOLDER` is EHR-scoped in both directions, no released service surface
/// admitting a demographic folder (SM `UML/classes/i_demographic_service.adoc`
/// declares no folder operation and every ITS-REST `directory_*` operation is
/// mounted at `/ehr/{ehr_id}/directory`).
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) on a scope mismatch in either
/// direction.
fn check_kind_scope(kind: Kind, party_only: bool) -> Result<(), ServiceError> {
    if party_only && !kind.is_demographic() {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "of a demographic CONTRIBUTION may only be demographic versions, got {}",
                kind.as_str()
            ))
            .with_path("versions"),
        ));
    }
    if !party_only && kind.is_demographic() {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "of an EHR CONTRIBUTION may not be demographic versions, got {}",
                kind.as_str()
            ))
            .with_path("versions"),
        ));
    }
    Ok(())
}

/// The change-type code of a `DV_CODED_TEXT`: its `defining_code.code_string` if
/// present, else its `value`.
fn coded_value(dv: &Value) -> Option<String> {
    dv.get("defining_code")
        .and_then(|c| c.get("code_string"))
        .and_then(Value::as_str)
        // TODO(#3595): adjudicate whether the SM spelling stays accepted on the
        // CONTRIBUTION wire now that the direct routes are typed.
        // NOTE: the two released sources spell this attribute differently —
        // ITS-REST `UpdateAudit.yaml` `$ref`s `DvCodedText`, SM
        // `update_audit.adoc` types it `Terminology_code`; this lane reads both.
        .or_else(|| dv.get("code_string").and_then(Value::as_str))
        .or_else(|| dv.get("value").and_then(Value::as_str))
        .map(str::to_owned)
}

/// Extract a VERSION item's `lifecycle_state` token (master06 §Version
/// Lifecycle), if present. `UPDATE_VERSION.lifecycle_state` is a
/// `DV_CODED_TEXT` on the released wire (ITS-REST
/// `schemas/common/UpdateVersion.yaml`); this raw-body lane also accepts the
/// SM `Terminology_code` spelling (`{terminology_id, code_string}`,
/// `UML/classes/update_version.adoc`), a bare `{value}`, or a plain string.
/// A member whose `lifecycle_state` is absent, the `666|attestation|` member
/// included, is refused ([`commit_version_set`]).
fn lifecycle_of(version: &Value) -> Option<String> {
    let ls = version.get("lifecycle_state")?;
    if let Some(s) = ls.as_str() {
        return Some(s.to_owned());
    }
    ls.get("code_string")
        .and_then(Value::as_str)
        .or_else(|| {
            ls.get("defining_code")
                .and_then(|c| c.get("code_string"))
                .and_then(Value::as_str)
        })
        .or_else(|| ls.get("value").and_then(Value::as_str))
        .map(str::to_owned)
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
/// A CONTRIBUTION member arrives raw (the envelope is not decoded through the
/// generated `NewContribution`), so the payload runs the same typed door
/// every direct commit route passes through: content that cannot be converted to
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
        Some(e) => Err(ServiceError::precondition(format!(
            "invalid canonical JSON body: {e}"
        ))),
        None => Ok(()),
    }
}

/// Parse a VERSION's `preceding_version_uid` (`OBJECT_VERSION_ID`, string or
/// `{value}`) into the object id and the version it must currently be at —
/// through the strict BASE three-part parse.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when absent/`null` on a modify/delete/attest
/// item, or when the value is not a valid `OBJECT_VERSION_ID`.
fn parse_preceding(version: &Value) -> Result<(VoId, TreeId), ServiceError> {
    let raw = version
        .get("preceding_version_uid")
        // Treat a JSON `null` as absent (the SM glue serializes `None` to
        // `null`) — consistent with the create-vs-modify classification.
        .filter(|p| !p.is_null())
        .and_then(|p| {
            p.as_str()
                .or_else(|| p.get("value").and_then(Value::as_str))
        })
        .ok_or_else(|| {
            ServiceError::content_invalid(
                Violation::new("required for modify/delete").with_path("preceding_version_uid"),
            )
        })?;
    Ok(object_version_id::parse_version_uid(raw)?)
}

/// The raw wire `UPDATE_VERSION.attestations` array of a version item (partial
/// `UPDATE_ATTESTATION`s), empty when absent.
fn attestation_partials(version: &Value) -> Vec<Value> {
    version
        .get("attestations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
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
        ServiceError::sm(
            CallStatusType::ContributionDoesNotExist,
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
                ServiceError::sm(
                    CallStatusType::ObjectVersionDoesNotExist,
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

/// SM `I_EHR_CONTRIBUTION.list_contributions` — the ids of the EHR's
/// CONTRIBUTIONs, oldest-first, within the optional commit-time window, paged
/// (SM `i_ehr_contribution.adoc`).
///
/// # Errors
/// [`ServiceError::NotFound`] when the EHR does not exist; the storage read
/// error of the list query.
pub(crate) async fn list_contributions(
    pool: &sqlx::PgPool,
    ehr_id: EhrId,
    time_range: TimeRange,
    page: Page,
) -> Result<Vec<Uuid>, ServiceError> {
    ensure_ehr_exists(pool, ehr_id).await?;
    let (lower, upper) = time_range.unwrap_or((None, None));
    let offset = i64::try_from(page.offset()).unwrap_or(i64::MAX);
    let limit = page.limit().map(|l| i64::try_from(l).unwrap_or(i64::MAX));
    Ok(
        crate::storage::version_repo::contribution::list_contributions(
            pool, ehr_id, lower, upper, offset, limit,
        )
        .await?,
    )
}

/// SM `I_EHR_CONTRIBUTION.contribution_count` — the number of CONTRIBUTIONs in
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

/// The EHR-existence precheck for the read paths above (SM `ehr_does_not_exist`
/// → `NotFound`).
///
/// Storage exposes the read (`version_repo::meta::ehr_exists`) so versioning stays
/// self-contained.
async fn ensure_ehr_exists(pool: &sqlx::PgPool, ehr_id: EhrId) -> Result<(), ServiceError> {
    if crate::storage::version_repo::meta::ehr_exists(pool, ehr_id).await? {
        Ok(())
    } else {
        Err(ServiceError::sm(
            CallStatusType::EhrIdDoesNotExist,
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
mod tests {
    use super::*;

    /// A commit audit that IS an `ATTESTATION` round-trips whole: the wire
    /// `_type`, the coded `reason`, `is_pending` and `proof` survive the
    /// parse → store → rebuild path and come back on an `ATTESTATION`-typed
    /// `AUDIT_DETAILS`.
    ///
    /// Spec: RM common `master04-generic_package.adoc` §Attestation ("a
    /// Composition Version will be committed with a `commit_audit` of type
    /// `ATTESTATION`, rather than just `AUDIT_DETAILS`; the `is_pending` flag
    /// will be set to True") + `master06-change_control_package.adoc`
    /// §Attestation / §Committal and Audits ("`AUDIT_DETAILS` … or its subtype
    /// `ATTESTATION`").
    #[test]
    fn attestation_commit_audit_round_trips() {
        let committer = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" });
        let wire = json!({
            "_type": "ATTESTATION",
            "system_id": "sysA.example.org",
            "committer": committer,
            "change_type": { "_type": "DV_CODED_TEXT", "value": "modification",
                "defining_code": { "_type": "CODE_PHRASE", "code_string": "251",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" } } },
            // 648|witnessed| is an `attestation reason` group member
            // (ATTESTATION.Reason_valid).
            "reason": { "_type": "DV_CODED_TEXT", "value": "witnessed",
                "defining_code": { "_type": "CODE_PHRASE", "code_string": "648",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" } } },
            "is_pending": true,
            "proof": "-----BEGIN PGP SIGNATURE-----",
            "items": [{ "_type": "DV_EHR_URI", "value": "ehr://x/y" }]
        });
        let audit = parse_audit(
            Some(&wire),
            change_type::MODIFICATION.to_owned(),
            "fallback.system",
            true,
            "commit_audit",
        )
        .expect("an ATTESTATION commit audit is a valid commit audit");
        let now: jiff::Timestamp = "2026-08-02T10:11:12Z"
            .parse()
            .expect("the literal is a valid RFC 3339 instant");
        let rebuilt = audit.canonical(&now);
        assert_eq!(
            rebuilt.get("_type").and_then(Value::as_str),
            Some("ATTESTATION")
        );
        assert_eq!(rebuilt.get("is_pending"), Some(&Value::Bool(true)));
        assert_eq!(
            rebuilt.get("proof").and_then(Value::as_str),
            Some("-----BEGIN PGP SIGNATURE-----")
        );
        assert_eq!(
            rebuilt
                .pointer("/reason/defining_code/code_string")
                .and_then(Value::as_str),
            Some("648")
        );
        assert_eq!(
            rebuilt.pointer("/items/0/value").and_then(Value::as_str),
            Some("ehr://x/y")
        );
        assert_eq!(
            rebuilt.get("system_id").and_then(Value::as_str),
            Some("sysA.example.org")
        );
    }

    /// A `DV_CODED_TEXT` `AUDIT_DETAILS.description` round-trips with its
    /// `defining_code` intact — the attribute is a `DV_TEXT` (RM common
    /// `UML/classes/org.openehr.rm.common.audit_details.adoc` §Attributes),
    /// whose coded subtype carries a code a bare string would discard.
    #[test]
    fn coded_description_round_trips() {
        let committer = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" });
        let wire = json!({
            "_type": "UPDATE_AUDIT",
            "committer": committer,
            "description": { "_type": "DV_CODED_TEXT", "value": "amended after review",
                "defining_code": { "_type": "CODE_PHRASE", "code_string": "at0007",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" } } }
        });
        let audit = parse_audit(
            Some(&wire),
            change_type::MODIFICATION.to_owned(),
            "fallback.system",
            true,
            "commit_audit",
        )
        .expect("a coded description is a valid DV_TEXT");
        let now: jiff::Timestamp = "2026-08-02T10:11:12Z"
            .parse()
            .expect("the literal is a valid RFC 3339 instant");
        let rebuilt = audit.canonical(&now);
        assert_eq!(
            rebuilt
                .pointer("/description/_type")
                .and_then(Value::as_str),
            Some("DV_CODED_TEXT")
        );
        assert_eq!(
            rebuilt
                .pointer("/description/defining_code/code_string")
                .and_then(Value::as_str),
            Some("at0007")
        );
        assert_eq!(
            rebuilt
                .pointer("/description/defining_code/terminology_id/value")
                .and_then(Value::as_str),
            Some("local")
        );
        // The plain-string spelling of the wire `UDvText` still becomes a
        // DV_TEXT.
        let plain = parse_audit(
            Some(&json!({ "committer": committer, "description": "free text" })),
            change_type::MODIFICATION.to_owned(),
            "fallback.system",
            true,
            "commit_audit",
        )
        .expect("a plain-string description is accepted")
        .canonical(&now);
        assert_eq!(
            plain.pointer("/description/_type").and_then(Value::as_str),
            Some("DV_TEXT")
        );
        assert_eq!(
            plain.pointer("/description/value").and_then(Value::as_str),
            Some("free text")
        );
    }

    /// A `commit_audit` `_type` naming neither `AUDIT_DETAILS` nor its
    /// `ATTESTATION` subtype is refused (422), never stored as if it were an
    /// audit (RM common master06 §Committal and Audits).
    #[test]
    fn unknown_commit_audit_type_is_refused() {
        let committer = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" });
        for bad in ["COMPOSITION", "REVISION_HISTORY_ITEM", "UPDATE_VERSION"] {
            let err = parse_audit(
                Some(&json!({ "_type": bad, "committer": committer })),
                change_type::MODIFICATION.to_owned(),
                "fallback.system",
                true,
                "commit_audit",
            )
            .expect_err("an unknown commit_audit class must be refused");
            match err {
                // Asserted as DATA: the refused attribute path, and the
                // offending `_type` in the violation's own detail.
                ServiceError::Unprocessable { violation: v, .. } => {
                    assert_eq!(v.path(), Some("VERSION.commit_audit"), "{bad}");
                    assert_eq!(
                        v.invariant(),
                        Some("RM common master06 §Committal and Audits"),
                        "{bad}"
                    );
                    assert!(v.detail().contains(bad), "{bad}: {v}");
                }
                other => panic!("{bad}: expected Unprocessable, got {other:?}"),
            }
        }
        // An ATTESTATION missing its mandatory attributes is refused too — the
        // subtype's own invariants apply wherever it appears
        // (ATTESTATION.reason 1..1, is_pending 1..1).
        let err = parse_audit(
            Some(&json!({ "_type": "ATTESTATION", "committer": committer })),
            change_type::MODIFICATION.to_owned(),
            "fallback.system",
            true,
            "commit_audit",
        )
        .expect_err("an ATTESTATION without reason must be refused");
        match err {
            ServiceError::Unprocessable { violation: v, .. } => {
                assert_eq!(v.path(), Some("ATTESTATION.reason"));
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    /// A commit audit without `committer` is refused with the 400 naming the
    /// audit, never completed with a committer the client did not state:
    /// ITS-REST `schemas/common/UpdateAudit.yaml` `required: [change_type,
    /// committer]`, SM `UML/classes/update_audit.adoc` (`committer` 1..1). An
    /// absent audit object and a `null` committer are the same absence, and the
    /// rule holds for a `666|attestation|` member's audit too.
    #[test]
    fn commit_audit_without_committer_is_refused() {
        let no_committer = json!({ "_type": "UPDATE_AUDIT" });
        let null_committer = json!({ "_type": "UPDATE_AUDIT", "committer": null });
        for (audit, attestable) in [
            (Some(&no_committer), true),
            (Some(&null_committer), true),
            (None, true),
            (Some(&no_committer), false),
        ] {
            match parse_audit(
                audit,
                change_type::MODIFICATION.to_owned(),
                "contribution.system",
                attestable,
                "commit_audit",
            ) {
                Err(ServiceError::BadRequest(e)) => {
                    assert!(
                        e.message.starts_with("commit_audit.committer is required"),
                        "{e:?}"
                    );
                }
                other => panic!("{audit:?}: expected the 400 refusal, got {other:?}"),
            }
        }
    }

    /// A commit audit that states its own `committer` keeps it, and one that
    /// omits `system_id` takes the CONTRIBUTION audit's (RM common master06
    /// §Committal and Audits).
    #[test]
    fn commit_audit_keeps_its_committer_and_defaults_its_system_id() {
        let wire = json!({
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "member author" }
        });
        let audit = parse_audit(
            Some(&wire),
            change_type::MODIFICATION.to_owned(),
            "contribution.system",
            true,
            "commit_audit",
        )
        .expect("a commit audit stating its committer is accepted");
        let now: jiff::Timestamp = "2026-08-02T10:11:12Z"
            .parse()
            .expect("the literal is a valid RFC 3339 instant");
        let rebuilt = audit.canonical(&now);
        assert_eq!(
            rebuilt.pointer("/committer/name").and_then(Value::as_str),
            Some("member author")
        );
        assert_eq!(
            rebuilt.get("system_id").and_then(Value::as_str),
            Some("contribution.system")
        );
    }

    fn classify_err(token: Option<&str>, has_preceding: bool, has_data: bool) -> Violation {
        match classify(token, has_preceding, has_data) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => v,
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    fn classify_bad_request(token: Option<&str>, has_preceding: bool, has_data: bool) -> String {
        match classify(token, has_preceding, has_data) {
            Err(ServiceError::BadRequest(e)) => e.message,
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn classify_preserves_the_full_change_type_set() {
        for code in ["250", "251", "252", "253", "816", "817"] {
            let (action, kept) = classify(Some(code), true, true).expect(code);
            assert_eq!(action, Action::Modify);
            assert_eq!(kept, code);
        }
        let (action, kept) = classify(Some("amendment"), true, true).unwrap();
        assert_eq!((action, kept.as_str()), (Action::Modify, "250"));
        let (action, kept) = classify(Some("creation"), false, true).unwrap();
        assert_eq!((action, kept.as_str()), (Action::Create, "249"));
        let (action, kept) = classify(Some("523"), true, false).unwrap();
        assert_eq!((action, kept.as_str()), (Action::Delete, "523"));
    }

    #[test]
    fn classify_rejects_spec_invalid_combinations() {
        // A non-creation change type as the FIRST version is THE released
        // `400_CONTRIBUTION` trigger ("the modification type does not match
        // the operation"); creation WITH a preceding is its unassigned mirror,
        // adjudicated 422.
        let creation_with_preceding = classify_err(Some("249"), true, true);
        assert_eq!(creation_with_preceding.path(), Some("change_type"));
        assert_eq!(
            creation_with_preceding.invariant(),
            Some("RM change_control §Contributions")
        );
        assert!(classify_bad_request(Some("250"), false, true).contains("preceding_version_uid"));
        let deleted_with_data = classify_err(Some("523"), true, true);
        assert_eq!(deleted_with_data.path(), Some("data"));
        assert_eq!(
            deleted_with_data.invariant(),
            Some("RM change_control §Contributions")
        );
        assert!(classify_bad_request(Some("523"), false, false).contains("preceding_version_uid"));
        let out_of_group = classify_err(Some("999"), true, true);
        assert_eq!(out_of_group.path(), Some("change_type"));
        assert_eq!(
            out_of_group.invariant(),
            Some("AUDIT_DETAILS.Change_type_valid")
        );
        assert_eq!(classify_err(Some("251"), true, false).path(), Some("data"));
    }

    #[test]
    fn classify_attestation_of_existing_version() {
        let (action, kept) = classify(Some("666"), true, false).unwrap();
        assert_eq!((action, kept.as_str()), (Action::Attest, "666"));
        let (action, kept) = classify(Some("attestation"), true, false).unwrap();
        assert_eq!((action, kept.as_str()), (Action::Attest, "666"));
        assert!(classify_bad_request(Some("666"), false, false).contains("preceding_version_uid"));
        let attest_with_data = classify_err(Some("666"), true, true);
        assert_eq!(attest_with_data.path(), Some("data"));
        assert!(
            attest_with_data.detail().contains("adds no content"),
            "{attest_with_data}"
        );
    }

    #[test]
    fn classify_defaults_without_a_change_type() {
        let (action, code) = classify(None, false, true).unwrap();
        assert_eq!((action, code.as_str()), (Action::Create, "249"));
        let (action, code) = classify(None, true, true).unwrap();
        assert_eq!((action, code.as_str()), (Action::Modify, "251"));
    }

    /// A delete member states the `523|deleted|` its change type commits, in
    /// any accepted spelling; every other state of the group contradicts the
    /// deletion procedure and is the 400, and a token outside the group keeps
    /// the shared out-of-group 422.
    #[test]
    fn a_delete_member_states_the_deleted_lifecycle_or_nothing() {
        // The accepting twins: the code, its rubric, and the absent state the
        // required-lifecycle check owns instead.
        for accepted in ["523", "deleted", "Deleted"] {
            assert!(
                reject_contradictory_delete_lifecycle(Some(accepted)).is_ok(),
                "{accepted} is the deleted state"
            );
        }
        assert!(reject_contradictory_delete_lifecycle(None).is_ok());

        // Every other member of the group contradicts the change type.
        for contradicting in [
            state::COMPLETE,
            state::INCOMPLETE,
            state::INACTIVE,
            state::ABANDONED,
            "complete",
        ] {
            match reject_contradictory_delete_lifecycle(Some(contradicting)) {
                Err(ServiceError::BadRequest(e)) => {
                    assert!(e.message.contains("Logical Deletion"), "{e:?}");
                    assert!(e.message.contains("contradicts change_type"), "{e:?}");
                }
                other => panic!("{contradicting} must be a 400, got {other:?}"),
            }
        }

        // An out-of-group token is the same 422 every other member's state
        // gets (ORIGINAL_VERSION.Lifecycle_state_valid).
        match reject_contradictory_delete_lifecycle(Some("999")) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert_eq!(v.path(), Some("lifecycle_state"));
                assert_eq!(
                    v.invariant(),
                    Some("ORIGINAL_VERSION.Lifecycle_state_valid")
                );
            }
            other => panic!("an out-of-group delete state must be 422, got {other:?}"),
        }
    }
}
