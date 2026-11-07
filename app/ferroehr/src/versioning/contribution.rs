// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION version set: its decode, parsing and classification, and
//! its retrieval.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Contributions,
//! §Committal and Audits + `master08-versioning.adoc` §Managing Changes. A
//! CONTRIBUTION is the change-set unit — `uid` + a `versions` list + an
//! `audit`; every change is a CONTRIBUTION, committed as a nested transaction
//! (all versions/attestations commit or none). Each version's storage action
//! **and** its preserved audit change-type code come from [`classify`]: the
//! client `commit_audit.change_type` is validated against the full openEHR
//! `audit_change_type` group and stored **verbatim** (never narrowed), while
//! the storage branch collapses to create / modify / delete / attest. The
//! commit that validates and writes the planned set is
//! [`crate::service::contribution`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use openehr_base::prelude::{HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::refusal::Missing;
use crate::error::{ServiceError, Violation};
use crate::ids::{EhrId, VoId};
use crate::versioning::attestation::{AttestationInput, AttestationParts, PendingAttest};
use crate::versioning::audit::{
    AuditInput, change_type, change_type_code, decode_description, party_proxy, plain_audit,
    validate_commit_audit,
};
use crate::versioning::change::Change;
use crate::versioning::lifecycle::{
    lifecycle_rubric, lifecycle_state_code, resolve_lifecycle, state,
};
use crate::versioning::object_version_id::{self, TreeId};
use crate::versioning::signature::signer::Signer;
use crate::versioning::wire::version_envelope;
use crate::versioning::{Kind, read};

/// The storage branch an incoming VERSION maps to. Deliberately narrower than
/// the `audit_change_type` group: many change kinds (amendment, modification,
/// synthesis, …) are all "commit a new content version"; the audited change
/// type is carried separately, verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
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
/// entry (see the parse pass in [`commit_version_set`](crate::service::contribution::commit_version_set)).
pub(crate) struct PlannedVersion {
    /// The member's position in the submitted `versions` array — every
    /// member-scoped refusal names it.
    pub(crate) index: usize,
    pub(crate) action: Action,
    /// The parsed `preceding_version_uid` target (modify/delete/attest).
    pub(crate) target: Option<(VoId, TreeId)>,
    /// `data` (`null` ≙ absent — the deleted-version shape).
    pub(crate) data: Option<Value>,
    pub(crate) audit: AuditInput,
    /// The `lifecycle_state` code the member states (`UPDATE_VERSION.lifecycle_state`,
    /// a required `DV_CODED_TEXT`).
    pub(crate) lifecycle_state: String,
    pub(crate) incomplete: bool,
    pub(crate) signature: Option<String>,
    /// `UPDATE_VERSION.attestations`: committed WITH the new version on a
    /// content member, attached to the existing version on a
    /// `666|attestation|` member.
    pub(crate) attestations: Vec<AttestationInput>,
    /// `ORIGINAL_VERSION.other_input_version_uids`, the merge provenance the
    /// member states (empty when it merges nothing).
    pub(crate) other_input_version_uids: Vec<String>,
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

/// The client-supplied CONTRIBUTION `uid`, honoured when unused and rejected
/// when malformed (ITS-REST `contribution_create`; master06 §CONTRIBUTION
/// `uid`).
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the supplied value is not a
/// `HIER_OBJECT_ID` UUID.
pub(crate) fn parse_supplied_uid(uid: Option<&HierObjectId>) -> Result<Option<Uuid>, ServiceError> {
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
pub(crate) fn parse_contribution_audit(
    audit: &UpdateAudit,
    system_id: &str,
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
    let mut audit = AuditInput::from_commit_audit(audit, code, system_id);
    validate_commit_audit(&mut audit)?;
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
pub(crate) fn plan_version(
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
pub(crate) fn require_kind(
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
pub(crate) fn plan_attestation(
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

/// Resolves a deletion member.
///
/// # Errors
/// [`ServiceError::precondition`] when the target does not exist;
/// [`ServiceError::Unprocessable`] for a scope-mismatched kind;
/// [`ServiceError::Conflict`] when the member targets the `EHR_STATUS`, which
/// is mandatory (RM ehr, EHR class: `ehr_status` 1..1) — deleting the only one
/// would leave the EHR violating its own invariant.
pub(crate) fn delete_change(
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

/// The planned set's answer to [`needs_content_write_permission`] before any
/// member is decoded: a member whose data names a non-`EHR_STATUS` root (or a
/// deletion, which targets content) is content, and an `EHR_STATUS` member
/// with `is_modifiable = true` reactivates. A member whose type is not a
/// versioned root is left for content validation to name.
pub(crate) fn plan_writes_content_without_reactivating(plan: &[PlannedVersion]) -> bool {
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
pub(crate) fn needs_content_write_permission(changes: &[(AuditInput, Change)]) -> bool {
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
pub(crate) fn check_kind_scope(kind: Kind, party_only: bool) -> Result<(), ServiceError> {
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
/// [`commit_version_set`](crate::service::contribution::commit_version_set)'s business, not this function's.
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
pub(crate) fn data_kind(data: &Value) -> Result<Kind, ServiceError> {
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
pub(crate) fn typed_decode_gate(
    kind: Kind,
    data: &Value,
    incomplete: bool,
) -> Result<(), ServiceError> {
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
        ServiceError::not_found(
            Missing::Contribution,
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
                ServiceError::not_found(Missing::ObjectVersion, format!("VERSION {vo_id}::{tree}"))
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

/// Inside a CONTRIBUTION commit the modification/deletion target is
/// **body-referenced** (`preceding_version_uid`), not the request URI: the
/// ITS-REST `contribution_create` operation declares `404` only for an
/// unknown `ehr_id` (the URI resource), so a missing target versioned object
/// is a change-control mismatch in the committed content — the
/// `400_CONTRIBUTION` scope — never a 404.
pub(crate) fn body_target_not_found_is_bad_request(e: ServiceError) -> ServiceError {
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
