// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The root rules of the always-root EHR kinds, `EHR_STATUS`, `EHR_ACCESS`
//! and FOLDER, over their canonical value, beside the whole-instance RM pass
//! each runs ([`super::validation::validate_rm_invariants_for_commit`]).
//!
//! Spec: RM ehr `ehr_status.adoc` / `ehr_access.adoc`, RM common `folder.adoc`
//! and the inherited `locatable.adoc`; RM common master06 §Incomplete Content.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical value a `553|incomplete|` \
              commit and a server-built write are validated over"
)]

use serde_json::Value;

use super::validation::validate_rm_invariants_for_commit;
use crate::error::{ServiceError, Violation};

/// The ROOT-ONLY half of `LOCATABLE.Archetyped_valid` for the always-root EHR
/// kinds (`EHR_STATUS`, `EHR_ACCESS` — RM ehr `ehr_status.adoc` /
/// `ehr_access.adoc`, each with an unconditional `Is_archetype_root`):
/// `is_archetype_root xor archetype_details = Void` (RM common
/// `locatable.adoc` §Invariants) means such a root MUST carry the `ARCHETYPED`
/// block, with its mandatory `archetype_id`.
pub(in crate::service) fn validate_root_locatable<'a, T: crate::versioning::tree::Tree<'a>>(
    obj: T,
    kind: &str,
) -> Result<(), ServiceError> {
    let details = obj
        .get("archetype_details")
        .filter(|v| v.shape() == crate::versioning::tree::Shape::Object);
    let archetype_id = details
        .and_then(|details| details.pointer_text("/archetype_id/value"))
        .is_some_and(|id| !id.is_empty());
    root_archetyped(kind, details.is_some(), archetype_id)
}

/// The root half of `LOCATABLE.Archetyped_valid` over what a root of `kind`
/// carries: whether it has `archetype_details`, and whether that names a
/// non-empty `archetype_id`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the missing attribute.
pub(in crate::service) fn root_archetyped(
    kind: &str,
    details: bool,
    archetype_id: bool,
) -> Result<(), ServiceError> {
    if !details {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "is mandatory: {kind} is an archetype root (Is_archetype_root), \
                 and a root without ARCHETYPED is invalid; supply \
                 archetype_details with its archetype_id (for a plain \
                 status, openEHR-EHR-EHR_STATUS.generic.v1)"
            ))
            .with_path(format!("{kind}.archetype_details"))
            .with_invariant("LOCATABLE.Archetyped_valid"),
        ));
    }
    if !archetype_id {
        return Err(ServiceError::content_invalid(
            Violation::new("is mandatory")
                .with_path(format!("{kind}.archetype_details.archetype_id.value"))
                .with_invariant("ARCHETYPED.archetype_id 1..1"),
        ));
    }
    Ok(())
}

/// Validate an `EHR_STATUS` before it is committed (on EHR create,
/// `EHR_STATUS` update, or a CONTRIBUTION). Rejects every malformed data set
/// the CNF `master06 §Test Data Sets` (INVALID class 2) enumerates with a
/// `422`.
///
/// Only the two rules that are properties of the SLOT rather than of the
/// instance live here:
///
/// - the container holds an `EHR_STATUS`, so a foreign or absent `_type` is
///   invalid (nothing inside the instance can know which resource it was
///   posted to);
/// - the ROOT half of `LOCATABLE.Archetyped_valid`
///   ([`validate_root_locatable`]).
///
/// Everything else RM ehr `ehr_status.adoc` and the inherited `LOCATABLE` demand
/// is enforced by the whole-instance pass
/// ([`validate_rm_invariants_for_commit`]) from the generated model rather than
/// restated here: `name`, `is_queryable`, `is_modifiable` and `subject`
/// mandatoriness and typing, the `ITEM_STRUCTURE` typing of `other_details`, the
/// `OBJECT_REF` rules on a present `subject.external_ref`, and every invariant
/// below the root.
///
/// `incomplete` (a `553|incomplete|` commit) relaxes that pass's existence and
/// cardinality lower bounds, as for every other committable kind (RM common
/// master06 §Incomplete Content, which carries no content-type exclusion). The
/// two slot rules stay unconditional, the incomplete state lifting lower bounds
/// and never typing ("All other validity requirements must be satisfied").
///
/// # Errors
/// [`ServiceError::Unprocessable`] for the two slot rules above, or
/// [`ServiceError::ValidationFailed`] carrying the RM-invariant violations
/// (both → 422).
pub(in crate::service) fn validate_ehr_status(
    status: &Value,
    incomplete: bool,
) -> Result<(), ServiceError> {
    let unproc = |m: String| ServiceError::content_invalid(Violation::new(m));
    let obj = status
        .as_object()
        .ok_or_else(|| unproc("EHR_STATUS must be a JSON object".to_owned()))?;

    match obj.get("_type").and_then(Value::as_str) {
        Some("EHR_STATUS") => {}
        Some(other) => {
            return Err(unproc(format!(
                "EHR_STATUS _type must be \"EHR_STATUS\", got {other:?}"
            )));
        }
        None => {
            return Err(unproc(
                "EHR_STATUS is missing its _type discriminator".to_owned(),
            ));
        }
    }

    validate_root_locatable(status, "EHR_STATUS")?;
    validate_rm_invariants_for_commit(status, "EHR_STATUS", incomplete)
}

/// Validate a client-supplied `EHR_ACCESS` before it is committed (via a
/// CONTRIBUTION — there is no direct ITS-REST `EHR_ACCESS` write). RM ehr
/// `ehr_access.adoc`:
///
/// - a foreign `_type` in this slot is invalid (the container holds
///   `EHR_ACCESS` only) — a slot rule, not an instance one;
/// - `settings` (0..1) is a subtype of the ABSTRACT `ACCESS_CONTROL_SETTINGS`
///   — the RM defines no concrete scheme, so a present `settings` must carry a
///   non-empty concrete `_type`, which `scheme()` names (`Scheme_valid`).
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the first violated rule, or
/// [`ServiceError::ValidationFailed`] carrying the RM-invariant violations
/// found below the root (both → 422), and the `400` of
/// [`check_written_settings`](crate::service::ehr::access_settings::check_written_settings).
pub(in crate::service) fn validate_ehr_access(
    access: &Value,
    incomplete: bool,
) -> Result<(), ServiceError> {
    let unproc = |m: String| ServiceError::content_invalid(Violation::new(m));
    let obj = access
        .as_object()
        .ok_or_else(|| unproc("EHR_ACCESS must be a JSON object".to_owned()))?;
    match obj.get("_type").and_then(Value::as_str) {
        None | Some("EHR_ACCESS") => {}
        Some(other) => {
            return Err(unproc(format!(
                "expected an EHR_ACCESS, got _type {other:?}"
            )));
        }
    }
    // EHR_ACCESS carries the same unconditional Is_archetype_root as
    // EHR_STATUS (RM ehr `ehr_access.adoc`), so the root-LOCATABLE
    // invariants apply identically.
    validate_root_locatable(access, "EHR_ACCESS")?;
    if let Some(settings) = obj.get("settings").filter(|v| !v.is_null()) {
        crate::service::ehr::access_settings::check_written_settings(settings)?;
    }
    // `EHR_ACCESS.settings` is the RM's one implementation-defined slot
    // (`org.openehr.rm.ehr.ehr_access.adoc` §Attributes) and its type is
    // abstract with no attributes, invariants or RM-defined descendant, so a
    // pass that walked in would refuse every legal instance; `Scheme_valid`
    // above is the whole of the RM's demand on it.
    // NOTE: `settings` is therefore excluded from the whole-instance RM pass,
    // and only from it — the one RM-mandated OPEN slot.
    let mut without_settings = access.clone();
    if let Some(map) = without_settings.as_object_mut() {
        map.remove("settings");
    }
    validate_rm_invariants_for_commit(&without_settings, "EHR_ACCESS", incomplete)
}

/// Validate a client-supplied FOLDER tree before it is committed (directory
/// create/update and the CONTRIBUTION FOLDER path): the whole-instance RM
/// class-invariant + terminology pass ([`validate_rm_invariants_for_commit`]),
/// which covers:
///
/// - the declared-slot-type conformance rule (root and every member) from the
///   generated RM model: `FOLDER.items` takes `OBJECT_REF` and `FOLDER.folders`
///   takes `FOLDER`, so a COMPOSITION committed by value into `items` is refused
///   whatever the lifecycle state ("Folder structures do not contain
///   Compositions, only references to them", RM ehr master04 §Folders);
/// - the presence layer (`OBJECT_REF`'s mandatory `id`/`namespace`/`type`,
///   `LOCATABLE.name`, `Archetype_node_id_valid`, `Links_valid`, the
///   archetype-root identity rule), which relaxes on a `553|incomplete|` commit,
///   so no lifecycle special-casing is needed.
///
/// The root FOLDER whose `archetype_node_id` is an archetype id is an archetype
/// root, so it must carry `archetype_details` (RM common master03 §The LOCATABLE
/// Class), a presence rule a `553|incomplete|` commit relaxes. A nested FOLDER
/// keeps `archetype_details` optional.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for an archetype root without
/// `archetype_details`; [`ServiceError::ValidationFailed`] carrying every
/// violation found in the tree, each at its own path (→ 422).
pub(in crate::service) fn validate_folder(
    folder: &Value,
    incomplete: bool,
) -> Result<(), ServiceError> {
    // NOTE: RM master03 §The LOCATABLE Class: `archetype_details` "is non-Void for archetype
    // root points in data", where `archetype_node_id` "carries the archetype identifier".
    let archetype_root = folder
        .get("archetype_node_id")
        .and_then(Value::as_str)
        .is_some_and(openehr_adl::hrid::is_archetype_id);
    if !incomplete
        && archetype_root
        && !folder
            .get("archetype_details")
            .is_some_and(Value::is_object)
    {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "is mandatory: the directory root names an archetype in archetype_node_id, \
                 so it is an archetype root and must carry ARCHETYPED",
            )
            .with_path("FOLDER.archetype_details")
            .with_invariant("LOCATABLE.Archetyped_valid"),
        ));
    }
    validate_rm_invariants_for_commit(folder, "FOLDER", incomplete)
}
