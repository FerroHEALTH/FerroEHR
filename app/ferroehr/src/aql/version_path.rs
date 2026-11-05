// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Path resolution on a `VERSION` or `VERSIONED_OBJECT` variable: the version
//! metadata fields (RM common master06 §Version and its Subtypes) and the
//! paths into `ORIGINAL_VERSION.attestations` (master06 §Attestation), plus
//! the version standard predicate of a `VERSION` class expression (QUERY
//! master03 §Standard predicate).

use openehr_query::ast::{FunctionCall, IdentifiedPath, ObjectPath, StandardPredicate, Terminal};

use super::analyze::{
    Bindings, analyze_path, analyze_rm_path, bind_from_operand, resolve_attribute,
    resolve_node_predicate,
};
use super::error::{AnalysisError, AqlError, AqlFeatureError};
use super::ir::{
    Bind, LeafPath, PathTarget, SourceId, TypeSet, TypedLit, VersionField, VersionMetaPredicate,
};

/// Resolve an identified path into `ORIGINAL_VERSION.attestations` of a
/// VERSION variable, or `None` when the path does not start there.
///
/// `ATTESTATION` is an `AUDIT_DETAILS`, not a `LOCATABLE` (RM common
/// `org.openehr.rm.common.attestation.adoc` §Inherit), so it is reached by
/// path from the version (QUERY master03 §Identified Paths), never through
/// `CONTAINS`; the residual path is analysed against `ATTESTATION` and the
/// predicate on `attestations` may test its attributes only, an archetype or
/// name predicate naming attributes the class does not have.
pub(super) fn analyze_attestation_path(
    source: SourceId,
    path: &IdentifiedPath,
    profile: crate::config::profile::SpecProfile,
) -> Result<Option<LeafPath>, AqlError> {
    let Some(op) = path.path.as_ref() else {
        return Ok(None);
    };
    let Some(head) = op.parts.first().filter(|p| p.name == "attestations") else {
        return Ok(None);
    };
    if path.predicate.is_some() {
        return Err(AqlFeatureError::UnsupportedVersionPredicate(format!(
            "{}[...]/attestations",
            path.root
        ))
        .into());
    }
    let attestation = TypeSet::single("ATTESTATION");
    let predicate = head
        .predicate
        .as_ref()
        .map(resolve_node_predicate)
        .transpose()?;
    if let Some(pred) = &predicate {
        let unmodelled = match (&pred.archetype, &pred.name) {
            (Some(_), _) => Some("archetype_node_id"),
            (None, Some(_)) => Some("name"),
            (None, None) => None,
        };
        if let Some(attribute) = unmodelled {
            return Err(AnalysisError::UnresolvableAttribute {
                attribute: attribute.to_owned(),
                on: "ATTESTATION".to_owned(),
            }
            .into());
        }
        for sp in &pred.standard {
            let mut current = attestation.clone();
            for name in &sp.path {
                current = resolve_attribute(&current, name, profile)?.0;
            }
        }
    }
    let mut rest = op.clone();
    rest.parts.remove(0);
    let rest_ref = (!rest.parts.is_empty()).then_some(&rest);
    let leaf = analyze_rm_path(source, &attestation, predicate, rest_ref, profile)?;
    if !leaf.anchor.is_empty() {
        return Err(AqlFeatureError::UnsupportedVersionPredicate(
            op.parts
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join("/"),
        )
        .into());
    }
    Ok(Some(leaf))
}

pub(super) fn resolve_version_field(
    object_path: Option<&ObjectPath>,
) -> Result<VersionField, AqlError> {
    let Some(op) = object_path else {
        // A bare VERSION variable addresses its uid.
        return Ok(VersionField::Uid);
    };
    let parts: Vec<&str> = op.parts.iter().map(|p| p.name.as_str()).collect();
    version_field_from_parts(&parts)
}

/// The field a `VERSIONED_OBJECT` path addresses: its `uid` (the bare variable
/// too) or its `owner_id`, the two attributes of RM common master06
/// §`VERSIONED_OBJECT` a stored versioned object answers from its versions.
pub(super) fn resolve_versioned_object_field(
    object_path: Option<&ObjectPath>,
) -> Result<VersionField, AqlError> {
    let Some(op) = object_path else {
        return Ok(VersionField::ObjectUid);
    };
    let parts: Vec<&str> = op.parts.iter().map(|p| p.name.as_str()).collect();
    match parts.as_slice() {
        ["uid"] | ["uid", "value"] => Ok(VersionField::ObjectUid),
        ["owner_id", "id"] | ["owner_id", "id", "value"] => Ok(VersionField::OwnerId),
        _ => Err(AqlFeatureError::UnsupportedVersionedObjectPath(parts.join("/")).into()),
    }
}

fn version_field_from_parts(parts: &[&str]) -> Result<VersionField, AqlError> {
    match parts {
        ["uid", ..] => Ok(VersionField::Uid),
        // The two CODED version fields are sub-path-sensitive: the stored
        // representation is the numeric group code, the rubric renders from the
        // openEHR terminology group, and the terminology id is the constant
        // `openehr` — a flat mapping would silently never match.
        ["lifecycle_state", rest @ ..] => coded_version_field(
            rest,
            parts,
            VersionField::LifecycleState,
            VersionField::LifecycleStateRubric,
            VersionField::LifecycleStateTerminology,
        ),
        ["contribution", ..] => Ok(VersionField::ContributionId),
        ["commit_audit", rest @ ..] => match rest {
            ["time_committed", ..] => Ok(VersionField::TimeCommitted),
            ["system_id", ..] => Ok(VersionField::SystemId),
            ["change_type", rest2 @ ..] => coded_version_field(
                rest2,
                parts,
                VersionField::ChangeType,
                VersionField::ChangeTypeRubric,
                VersionField::ChangeTypeTerminology,
            ),
            ["committer", ..] => Ok(VersionField::Committer),
            // The two mandatory ATTESTATION attributes of a commit audit that is
            // an ATTESTATION (RM common master06 §Attestation, "Marking content
            // for review and signing").
            ["is_pending"] => Ok(VersionField::CommitAuditIsPending),
            ["reason"] => Ok(VersionField::CommitAuditReason),
            ["reason", "value"] => Ok(VersionField::CommitAuditReasonValue),
            // AUDIT_DETAILS.description is a DV_TEXT whose DV_CODED_TEXT
            // subtype carries a defining_code (`audit_details.adoc`
            // §Attributes), so each addressed representation resolves to its own
            // extraction.
            ["description", rest @ ..] => match rest {
                [] => Ok(VersionField::Description),
                ["value"] => Ok(VersionField::DescriptionValue),
                ["defining_code", "code_string"] => Ok(VersionField::DescriptionCode),
                ["defining_code", "terminology_id", "value"] => {
                    Ok(VersionField::DescriptionTerminology)
                }
                _ => Err(AqlFeatureError::UnsupportedVersionPredicate(parts.join("/")).into()),
            },
            _ => Err(AqlFeatureError::UnsupportedVersionPredicate(parts.join("/")).into()),
        },
        _ => Err(AqlFeatureError::UnsupportedVersionPredicate(parts.join("/")).into()),
    }
}

/// Resolve the sub-path of a coded (`DV_CODED_TEXT`) version field to the
/// representation it addresses; any other suffix — including the bare coded
/// object, which has no defined scalar comparison form — is a typed reject.
fn coded_version_field(
    suffix: &[&str],
    full: &[&str],
    code: VersionField,
    rubric: VersionField,
    terminology: VersionField,
) -> Result<VersionField, AqlError> {
    match suffix {
        ["defining_code", "code_string"] => Ok(code),
        ["value"] => Ok(rubric),
        ["defining_code", "terminology_id", "value"] => Ok(terminology),
        _ => Err(AqlFeatureError::UnsupportedVersionPredicate(full.join("/")).into()),
    }
}

/// Resolve a version standard predicate (`VERSION v[commit_audit/... op val]`).
/// Rejects branch (non-trunk) version ids explicitly.
pub(crate) fn resolve_version_predicate(
    sp: &StandardPredicate,
) -> Result<VersionMetaPredicate, AqlError> {
    let parts: Vec<&str> = sp.path.parts.iter().map(|p| p.name.as_str()).collect();
    let field = version_field_from_parts(&parts)?;
    let value = bind_from_operand(&sp.operand)?;
    if field == VersionField::Uid
        && let Bind::Literal(TypedLit::String(s)) = &value
        && is_branch_version_id(s)
    {
        return Err(AqlFeatureError::BranchVersionAddressing.into());
    }
    Ok(VersionMetaPredicate {
        field,
        op: sp.op,
        value,
    })
}

/// Whether an `OBJECT_VERSION_ID` string names a branch version. The format is
/// `object_id::creating_system_id::version_tree_id`; a trunk `version_tree_id` is
/// a plain integer, while a branch id contains dots (`1.1.1`).
fn is_branch_version_id(s: &str) -> bool {
    s.split("::")
        .nth(2)
        .is_some_and(|tree_id| tree_id.contains('.'))
}

/// The FerroEHR extension function `LAST(<path below v/attestations>)`: the
/// path read off the most recent attestation of the version, or `None` for
/// any other call.
///
/// RM common master04 §Attestation completes a pending attestation by adding
/// another, and a pending one may follow a completed one, so only the latest
/// says whether a version waits for signature; AQL 1.1.0 cannot select the
/// last element of a list.
///
/// # Errors
/// [`AqlFeatureError::LastArgument`] when the call is not exactly one path
/// below `attestations`, and the path analysis errors of that path.
pub(super) fn last_attestation(
    call: &FunctionCall,
    bindings: &Bindings,
) -> Result<Option<PathTarget>, AqlError> {
    let FunctionCall::Other { name, args } = call else {
        return Ok(None);
    };
    // NOTE: QUERY master03 §Functions ("Various other functions may exist however
    // in various AQL implementations"); `LAST` is FerroEHR's own extension.
    if !name.eq_ignore_ascii_case("LAST") {
        return Ok(None);
    }
    let [Terminal::Path(path)] = args.as_slice() else {
        return Err(AqlFeatureError::LastArgument(format!("{} argument(s)", args.len())).into());
    };
    match analyze_path(path, bindings)? {
        PathTarget::Attestations { leaf, .. } => {
            Ok(Some(PathTarget::Attestations { leaf, last: true }))
        }
        _ => Err(AqlFeatureError::LastArgument(format!("`{}`", path.column_path_text())).into()),
    }
}
