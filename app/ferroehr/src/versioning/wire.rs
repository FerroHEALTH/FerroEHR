// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The served wire builders: `ORIGINAL_VERSION` / `VERSIONED_OBJECT` /
//! `REVISION_HISTORY` canonical-JSON value construction.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Versioned Objects /
//! §Version and its Subtypes, RM common `master04-generic_package.adoc`
//! §Revision History, BASE `base_types` `master05-identification_package.adoc`
//! §References. These builders surface the full provenance a versioned object
//! carries: its `OBJECT_VERSION_ID`, the CONTRIBUTION that produced it, the
//! mandatory commit audit, and its data. The loaded-version input comes from
//! [`super::read`]; the residual SQL (first-version time, all-version metadata)
//! is delegated to `crate::storage::version_repo`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use openehr_base::prelude::{
    HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId, TerminologyId,
};
use openehr_rm::prelude::{
    AuditDetails, CodePhrase, DvCodedText, DvText, RevisionHistory, RevisionHistoryItem,
    VersionedComposition, VersionedEhrAccess, VersionedEhrStatus, VersionedFolder, VersionedObject,
    VersionedObjectData,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::error::{ServiceError, Violation};
use crate::service::status::CallStatusType;
use crate::versioning::audit::{AuditInput, OPENEHR};
use crate::versioning::integrity;
use crate::versioning::lifecycle::lifecycle_rubric;
use crate::versioning::object_version_id::{TreeId, VersionIdError, version_id};
use crate::versioning::read::{VersionRead, WrappedOriginal};
use crate::versioning::signature::signer::Signer;

/// The `REVISION_HISTORY` of a versioned object: one item per version, each with
/// its version id and the `AUDIT_DETAILS` of the change that produced it plus
/// any attestations of that revision (RM common master04 §Revision History:
/// "there will always be at least one commit audit … there may also be further
/// attestations").
///
/// The body is constructed as the generated [`RevisionHistory`] and serialized
/// through the native codec, so it carries `_type` first and the BMM's own
/// attribute order. Stored attestations are decoded back into `AUDIT_DETAILS`
/// (the `ATTESTATION` subtype dispatches on `_type`) rather than spliced in as
/// opaque fragments.
///
/// # Errors
/// [`ServiceError::NotFound`] when the object has no stored version or is not
/// owned by `ehr_id`; the storage read errors of the metadata / attestation
/// queries; [`ServiceError::Unprocessable`] when a stored audit or attestation
/// is not a canonical `AUDIT_DETAILS`; [`ServiceError::Internal`] if the built
/// history somehow carries no commit audit to take `Last-Modified` from (every
/// item is built with one below).
pub(crate) async fn revision_history(
    pool: &sqlx::PgPool,
    ehr_id: EhrId,
    vo_id: VoId,
) -> Result<(Value, jiff::Timestamp), ServiceError> {
    let rows = crate::storage::version_repo::meta::all_version_meta(pool, vo_id).await?;
    let first = rows.first().ok_or_else(|| {
        ServiceError::sm(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("versioned object {vo_id}"),
        )
    })?;
    if first.ehr_id != Some(ehr_id) {
        return Err(ServiceError::sm(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("versioned object {vo_id} in EHR {ehr_id}"),
        ));
    }
    // A revision history is a read of the object's change record, which
    // Art. 18(2) does not admit for a restricted object any more than the
    // content itself (`docs/law/eu/gdpr/text.html`); the version-body seam
    // carries the same refusal. Our own design/extension.
    if first.restricted {
        return Err(ServiceError::restricted(&format!(
            "versioned object {vo_id}"
        )));
    }

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        // "there will always be at least one commit audit … there may also be
        // further attestations" — the commit audit first, then the version's
        // attestations in commit order (master04 §Revision History), folded
        // into the one meta statement (`VersionMeta::attestations`).
        // The commit audit makes the `1..*` bound of
        // `REVISION_HISTORY_ITEM.audits` hold by construction.
        let mut audits = openehr_base::containers::NonEmptyVec::of(
            AuditInput::from_meta(row)?.typed(&row.time_committed),
        );
        for stored in &row.attestations {
            audits.push(stored_attestation(stored)?);
        }
        items.push(RevisionHistoryItem {
            version_id: version_id(
                vo_id,
                &row.creating_system_id,
                TreeId::from_columns(row.trunk_version, row.branch_number, row.branch_version),
            )?,
            audits,
        });
    }
    // `REVISION_HISTORY.items` is `1..*`
    // (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.revision_history.adoc`
    // §Attributes): a versioned object always has at least one version, so an
    // empty history is not a representable resource.
    let items = openehr_base::containers::NonEmptyVec::new(items).map_err(|empty| {
        ServiceError::sm(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("versioned object {vo_id}: {empty}"),
        )
    })?;
    let history = RevisionHistory { items };
    // The `Last-Modified` instant is read off the built history through
    // `REVISION_HISTORY.most_recent_version_time_committed`
    // (`revision_history.adoc` §Functions) rather than re-derived from the rows:
    // two expressions of one rule can drift apart, one cannot.
    let last_modified = history
        .most_recent_version_time_committed()
        .ok_or_else(|| {
            ServiceError::exception(format!(
                "the REVISION_HISTORY built for versioned object {vo_id} has no commit audit"
            ))
        })?
        .parse::<jiff::Timestamp>()
        .map_err(|e| {
            ServiceError::exception(format!(
                "the commit instant of versioned object {vo_id}'s newest version is not a \
                 valid instant: {e}"
            ))
        })?;
    Ok((
        openehr_its::json::to_canonical_value(&history),
        last_modified,
    ))
}

/// Decode a stored `vo_attestation.data` fragment back into its RM type. The
/// slot is `AUDIT_DETAILS` (`REVISION_HISTORY_ITEM.audits`), whose `ATTESTATION`
/// subtype dispatches on `_type` — so a stored attestation returns as
/// [`AuditDetails::Attestation`].
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the stored fragment is not a canonical
/// `AUDIT_DETAILS` / `ATTESTATION`.
fn stored_attestation(stored: &Value) -> Result<AuditDetails, ServiceError> {
    openehr_its::json::from_canonical_value::<AuditDetails>(stored).map_err(|e| {
        ServiceError::content_invalid(
            Violation::new("a stored attestation is not a canonical ATTESTATION")
                .with_decode_failure(&e),
        )
    })
}

/// A versioned-object wire body for `vo_id` owned by `ehr_id`, carrying the
/// concrete RM `_type` (`VERSIONED_COMPOSITION`, `VERSIONED_EHR_STATUS` or
/// `VERSIONED_FOLDER`; RM ehr master04 defines the concrete bindings of
/// `VERSIONED_OBJECT<T>` and the ITS-REST `200_VERSIONED_COMPOSITION` schema
/// pins the discriminator to the concrete class) and the mandatory
/// `time_created`, the commit time of the object's first version (RM common
/// master06 §Versioned Objects).
///
/// The body is constructed as the generated [`VersionedObject`] subtype and
/// serialized through the native codec, so it carries `_type` first and the
/// BMM's own attribute order.
///
/// NOTE: the earliest HELD version supplies `time_created` rather than a
/// hardcoded `sys_version = 1`, a latest-only clone (master06 §Copying)
/// legitimately holding a partial trunk history whose lowest version is above
/// one.
///
/// Returns the wire body plus the newest held version's commit instant, the
/// container resource's `Last-Modified` value "derived from
/// `VERSION.commit_audit.time_committed.value`" (ITS-REST overview
/// `Requests_and_responses.md` §"`ETag` and Last-Modified"). `None` when the
/// object has no stored version or is not owned by `ehr_id`, ownership riding
/// the same statement as the bounds, and each caller maps the miss to its own
/// resource's 404.
///
/// # Errors
/// The storage read error of `version_repo::meta::commit_bounds`;
/// [`ServiceError::Internal`] when the built body fails to serialize.
pub(crate) async fn versioned_object(
    pool: &sqlx::PgPool,
    vo_id: VoId,
    ehr_id: EhrId,
    rm_type: &str,
) -> Result<Option<(Value, jiff::Timestamp)>, ServiceError> {
    let Some((_, time_created, last_modified)) =
        crate::storage::version_repo::meta::commit_bounds(pool, vo_id)
            .await?
            .filter(|(owner, _, _)| *owner == Some(ehr_id))
    else {
        return Ok(None);
    };
    // Both keys are UUIDs by type, so the conversions are total (BASE
    // `master05-identification_package.adoc` §Syntaxes:
    // `uid = iso_oid | uuid | internet_id`).
    let uid = HierObjectId::from(vo_id.0);
    let owner_id = ObjectRef::ObjectRef(ObjectRefData {
        namespace: "local".to_owned(),
        r#type: "EHR".to_owned(),
        id: ObjectId::HierObjectId(HierObjectId::from(ehr_id.0)),
    });
    let time_created = crate::versioning::audit::dv_date_time(&time_created);
    let container = match rm_type {
        "VERSIONED_COMPOSITION" => VersionedObject::VersionedComposition(VersionedComposition {
            uid,
            owner_id,
            time_created,
        }),
        "VERSIONED_EHR_STATUS" => VersionedObject::VersionedEhrStatus(VersionedEhrStatus {
            uid,
            owner_id,
            time_created,
        }),
        "VERSIONED_EHR_ACCESS" => VersionedObject::VersionedEhrAccess(VersionedEhrAccess {
            uid,
            owner_id,
            time_created,
        }),
        "VERSIONED_FOLDER" => VersionedObject::VersionedFolder(VersionedFolder {
            uid,
            owner_id,
            time_created,
        }),
        // The generic container of RM common master06 §Versioned Objects, for a
        // kind with no dedicated `VERSIONED_*` binding (the EHR-extract export's
        // own fallback).
        // NOTE: there is deliberately NO `VERSIONED_PARTY` arm — this builder is
        // EHR-scoped by signature and stamps `owner_id` as the containing EHR
        // (RM common `versioned_object.adoc` §Attributes); a party has none.
        _ => VersionedObject::VersionedObject(VersionedObjectData {
            uid,
            owner_id,
            time_created,
        }),
    };
    Ok(Some((
        openehr_its::json::to_canonical_value(&container),
        last_modified,
    )))
}

/// The VERSION resource's wire form for a loaded version: an `ORIGINAL_VERSION`
/// for a locally created version and an `IMPORTED_VERSION` wrapping the received
/// original for an imported one (RM common master06 §Version and its Subtypes;
/// the ITS-REST 1.1.0 `UVersionOf*` schemas declare the version resource as that
/// `_type`-discriminated union).
///
/// Read-time signature verification (master06 §Digital Signature) applies to the
/// served envelope, which for an imported version is the wrapper's own
/// signature, the wrapped original's foreign signature being served verbatim and
/// never re-verified. A `warn` mismatch logs and meters; a `strict` mismatch is a
/// 5xx integrity failure.
///
/// # Errors
/// [`ServiceError::Signing`] when `verify_on_read = strict` and the stored
/// signature fails verification, or (in any non-`off` mode) when the served
/// version's canonical form cannot be recomputed; the
/// [`build_original_version`] rejection of an uninterpretable stored commit
/// audit.
pub(crate) fn version_envelope(read: &VersionRead, signer: &Signer) -> Result<Value, ServiceError> {
    let Some(wrapped) = &read.wrapped else {
        return original_version(read, signer);
    };
    let item = build_wrapped_original(read, wrapped)?;
    let mut iv = build_imported_version(
        &contribution_ref(read.contribution_id),
        &read.audit.canonical(&read.time_committed),
        &item,
        read.signature.as_deref(),
    );
    integrity::verify_on_read(
        signer,
        &iv,
        read.signature.as_deref(),
        read.signature_client_supplied,
    )?;
    // The attestations the wrapped original carried AT importing are already
    // inside `item`, which master06 §Digital Signature includes in the wrapper's
    // signed serialisation. Anything attested AFTERWARDS post-dates that
    // signature and is appended here, after verification.
    if let Some(item) = iv.get_mut("item") {
        append_after_committal_attestations(item, &read.attestations_after_committal);
    }
    Ok(iv)
}

/// The `ORIGINAL_VERSION` view of a loaded version: for an imported version the
/// wrapped original with its own foreign contribution, commit audit and
/// signature, otherwise the version itself. This is the form an EHR Extract
/// carries (`X_VERSIONED_OBJECT.versions: List<ORIGINAL_VERSION<T>>`, RM
/// `ehr_extract` `x_versioned_object.adoc` §Attributes), so a re-export
/// reproduces what was received (master06 §Copying).
///
/// Read-time signature verification runs only for a locally created version:
/// the wrapped original's signature is foreign and is never re-verified
/// (master06 §Digital Signature).
///
/// # Errors
/// [`ServiceError::Signing`] when `verify_on_read = strict` and the stored
/// signature fails verification, or (in any non-`off` mode) when the served
/// version's canonical form cannot be recomputed; the
/// [`build_original_version`] rejection of an uninterpretable stored commit
/// audit.
pub(crate) fn original_version(read: &VersionRead, signer: &Signer) -> Result<Value, ServiceError> {
    if let Some(wrapped) = &read.wrapped {
        let mut ov = build_wrapped_original(read, wrapped)?;
        append_after_committal_attestations(&mut ov, &read.attestations_after_committal);
        return Ok(ov);
    }
    let mut ov = build_original_version(&OriginalVersionParts {
        creating_system_id: &read.creating_system_id,
        vo_id: read.vo_id,
        tree: read.tree,
        preceding_version_uid: read.preceding_version_uid.as_deref(),
        other_input_version_uids: &read.other_input_version_uids,
        contribution: &contribution_ref(read.contribution_id),
        commit_audit: &read.audit.canonical(&read.time_committed),
        lifecycle_state: &read.lifecycle_state,
        data: &read.canonical,
        attestations: &read.attestations_at_committal,
        signature: read.signature.as_deref(),
    })?;
    integrity::verify_on_read(
        signer,
        &ov,
        read.signature.as_deref(),
        read.signature_client_supplied,
    )?;
    // §Attestation equally says an attestation "can be added at any time after
    // committal of the content being attested", and §Contributions makes that a
    // change to "an EXISTING `ORIGINAL_VERSION`"; such an attestation necessarily
    // post-dates the signature, so it is appended HERE, after verification, and
    // never enters the canonical form.
    // NOTE: RM common master06 §Digital Signature signs "the entire Version
    // object", `signature` alone excluded, so at-committal attestations are
    // inside the signed form and were built into `ov` above.
    append_after_committal_attestations(&mut ov, &read.attestations_after_committal);
    Ok(ov)
}

/// Append the after-committal `ORIGINAL_VERSION.attestations` to an
/// already-built (and already-verified) version value, extending the
/// at-committal list the builder put there when both are present (RM common
/// master06 §Attestation). A no-op when there are none.
pub(crate) fn append_after_committal_attestations(ov: &mut Value, after_committal: &[Value]) {
    if after_committal.is_empty() {
        return;
    }
    let Value::Object(map) = ov else { return };
    match map.get_mut("attestations") {
        Some(Value::Array(existing)) => existing.extend_from_slice(after_committal),
        _ => {
            map.insert(
                "attestations".to_owned(),
                Value::Array(after_committal.to_vec()),
            );
        }
    }
}

/// Rebuilds the `ORIGINAL_VERSION` an `IMPORTED_VERSION` wraps: the row's own
/// identity, lifecycle and content, which the import stored unchanged, plus the
/// source system's `contribution`, `commit_audit` and `signature` verbatim
/// (master06 §Copying) and the attestations the received original carried at the
/// act of importing, which are part of the wrapper's signed form (master06
/// §Digital Signature). Attestations added after the import are appended by the
/// caller.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the stored foreign `commit_audit` is not
/// a canonical `AUDIT_DETAILS` object, which would make the served
/// `ORIGINAL_VERSION` misreport the provenance it exists to preserve.
fn build_wrapped_original(
    read: &VersionRead,
    wrapped: &WrappedOriginal,
) -> Result<Value, ServiceError> {
    if !wrapped.commit_audit.is_object() {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "of the wrapped ORIGINAL_VERSION stored for versioned object {} is not \
                 an object",
                read.vo_id
            ))
            .with_path("ORIGINAL_VERSION.commit_audit"),
        ));
    }
    build_original_version(&OriginalVersionParts {
        creating_system_id: &read.creating_system_id,
        vo_id: read.vo_id,
        tree: read.tree,
        preceding_version_uid: read.preceding_version_uid.as_deref(),
        other_input_version_uids: &read.other_input_version_uids,
        contribution: &wrapped.contribution,
        commit_audit: &wrapped.commit_audit,
        lifecycle_state: &read.lifecycle_state,
        data: &read.canonical,
        attestations: &read.attestations_at_committal,
        signature: wrapped.signature.as_deref(),
    })
    .map_err(Into::into)
}

/// Build the `IMPORTED_VERSION` wrapper JSON from the LOCAL act of committal
/// plus the wrapped original — the single shared builder used by both the
/// import path ([`super::integrity::sign_imported_version`]) and the read path
/// ([`version_envelope`]), so the bytes signed at import and served at read are
/// identical.
///
/// Spec: `IMPORTED_VERSION` "inherits `commit_audit` and `contribution` from
/// `VERSION<T>`, providing imported versions with their own audit trail and
/// Contribution, distinct from those of the imported `ORIGINAL_VERSION`"
/// (`UML/classes/org.openehr.rm.common.imported_version.adoc`). Its `uid`,
/// `preceding_version_uid`, `lifecycle_state` and `data` are effected
/// FUNCTIONS over `item` and are therefore not serialized attributes.
pub(crate) fn build_imported_version(
    contribution: &Value,
    commit_audit: &Value,
    item: &Value,
    signature: Option<&str>,
) -> Value {
    // NOTE: a JSON-literal envelope over VERBATIM fragments — `contribution`,
    // `commit_audit` and the wrapped foreign `item` are already-canonical stored
    // bytes, and this serialization is what gets signed.
    let mut iv = json!({
        "_type": "IMPORTED_VERSION",
        "contribution": contribution.clone(),
        "commit_audit": commit_audit.clone(),
        "item": item.clone()
    });
    if let Some(sig) = signature
        && let Value::Object(map) = &mut iv
    {
        map.insert("signature".to_owned(), Value::String(sig.to_owned()));
    }
    iv
}

/// One `OBJECT_VERSION_ID` reference as canonical JSON — the `uid`,
/// `preceding_version_uid` and `other_input_version_uids` slots of a
/// `VERSION` (RM common master06 §Version and its Subtypes).
///
/// # Errors
/// [`VersionIdError`] when `value` is not a well-formed `OBJECT_VERSION_ID` —
/// the reference goes through the BASE construction door, so a malformed one
/// never reaches the wire.
fn object_version_ref(value: &str) -> Result<Value, VersionIdError> {
    let uid = ObjectVersionId::new(value).map_err(|source| VersionIdError::Malformed {
        raw: value.to_owned(),
        source,
    })?;
    Ok(openehr_its::json::to_canonical_value(&uid))
}

/// The `VERSION.contribution` `OBJECT_REF` naming a CONTRIBUTION held by THIS
/// repository (BASE `base_types` `master05-identification_package.adoc`
/// §References). A foreign contribution reference is never rebuilt this way —
/// it is stored and served verbatim.
pub(crate) fn contribution_ref(contribution_id: Uuid) -> Value {
    openehr_its::json::to_canonical_value(&ObjectRef::ObjectRef(ObjectRefData {
        namespace: "local".to_owned(),
        r#type: "CONTRIBUTION".to_owned(),
        // A contribution id is a UUID by type, so the conversion is total.
        id: ObjectId::HierObjectId(HierObjectId::from(contribution_id)),
    }))
}

/// The attributes of one `ORIGINAL_VERSION` to render, with `commit_audit`
/// already in its canonical `AUDIT_DETAILS` form.
///
/// Spec: `VERSION.contribution` / `VERSION.commit_audit` 1..1;
/// `VERSION.Preceding_version_uid_validity`;
/// `ORIGINAL_VERSION.lifecycle_state` coded from `version_lifecycle_state`
/// (RM common master06 §Version and its Subtypes).
#[derive(Debug)]
pub(crate) struct OriginalVersionParts<'a> {
    /// The `OBJECT_VERSION_ID` middle segment (master06 §Distributed Versioning).
    pub(crate) creating_system_id: &'a str,
    /// The owning version container's id — the `OBJECT_VERSION_ID` object id.
    pub(crate) vo_id: VoId,
    /// The `VERSION_TREE_ID` this version sits at.
    pub(crate) tree: TreeId,
    /// The STORED prior `OBJECT_VERSION_ID`; `None` for a first version.
    pub(crate) preceding_version_uid: Option<&'a str>,
    /// The merge provenance (master06 §Version Merging); empty when not a merge.
    pub(crate) other_input_version_uids: &'a [String],
    /// The `VERSION.contribution` `OBJECT_REF` — local for a locally created
    /// version, the source system's for a wrapped imported original.
    pub(crate) contribution: &'a Value,
    /// The canonical `AUDIT_DETAILS` (or `ATTESTATION`) of `commit_audit`.
    pub(crate) commit_audit: &'a Value,
    /// The numeric `version_lifecycle_state` code.
    pub(crate) lifecycle_state: &'a str,
    /// The canonical content, or [`Value::Null`] for a deleted version.
    pub(crate) data: &'a Value,
    /// `ORIGINAL_VERSION.attestations` (0..1) as they stood at the act of
    /// committal — the ones inside the signed canonical form (master06
    /// §Digital Signature / §Attestation). Empty ≙ absent
    /// (`ORIGINAL_VERSION.Attestations_valid`: a present list is non-empty).
    pub(crate) attestations: &'a [Value],
    /// `VERSION.signature` (0..1), when known.
    pub(crate) signature: Option<&'a str>,
}

/// Build the `ORIGINAL_VERSION` JSON from its parts — the single shared builder
/// used by the read path ([`original_version`]), the commit path
/// ([`super::integrity::sign_version`]) and the import path
/// ([`super::import`]), so the bytes signed at commit/import and served at read
/// are identical.
///
/// # Errors
/// [`VersionIdError`] when any of the version identifiers this envelope carries
/// (`uid`, `preceding_version_uid`, `other_input_version_uids`) is not a
/// well-formed `OBJECT_VERSION_ID`: they go through the BASE construction door
/// rather than a struct literal, so a malformed identifier is refused before it
/// is signed and served.
pub(crate) fn build_original_version(
    parts: &OriginalVersionParts<'_>,
) -> Result<Value, VersionIdError> {
    // NOTE: the ENVELOPE stays a JSON literal — its fragments arrive as
    // already-canonical stored bytes and `ORIGINAL_VERSION` is exactly the
    // serialization master06 §Digital Signature signs; re-encoding could drift.
    let mut ov = json!({
        "_type": "ORIGINAL_VERSION",
        "uid": openehr_its::json::to_canonical_value(&version_id(
            parts.vo_id,
            parts.creating_system_id,
            parts.tree,
        )?),
        "contribution": parts.contribution.clone(),
        "commit_audit": parts.commit_audit.clone(),
        "lifecycle_state": openehr_its::json::to_canonical_value(&DvText::DvCodedText(
            DvCodedText {
                value: lifecycle_rubric(parts.lifecycle_state).clone(),
                hyperlink: None,
                formatting: None,
                mappings: None,
                language: None,
                encoding: None,
                defining_code: CodePhrase {
                    terminology_id: TerminologyId {
                        value: OPENEHR.to_owned(),
                    },
                    code_string: parts.lifecycle_state.to_owned(),
                    preferred_term: None,
                },
            },
        )),
    });
    if let Value::Object(map) = &mut ov {
        // preceding_version_uid: the STORED prior OBJECT_VERSION_ID, never
        // synthesized, because under branching and import it may carry a
        // different creating_system_id (RM common master06 §Distributed
        // Versioning).
        //
        // NOTE: `VERSION.Preceding_version_uid_validity` is enforced in its
        // TRUNK-ONLY sense — BASE's `is_first` is "trunk_version is 1" alone,
        // making the literal invariant unsatisfiable for branches off trunk 1.
        if let Some(preceding) = parts.preceding_version_uid {
            map.insert(
                "preceding_version_uid".to_owned(),
                object_version_ref(preceding)?,
            );
        }
        // other_input_version_uids: merge provenance (master06 §Version
        // Merging); `is_merged` is its derived boolean
        // (`VERSION.Is_merged_validity`: is_merged = not …is_empty).
        if !parts.other_input_version_uids.is_empty() {
            map.insert(
                "other_input_version_uids".to_owned(),
                Value::Array(
                    parts
                        .other_input_version_uids
                        .iter()
                        .map(|uid| object_version_ref(uid))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            );
        }
        // A deleted version carries no data (canonical is Null).
        if !parts.data.is_null() {
            map.insert("data".to_owned(), parts.data.clone());
        }
        // attestations: the ones present AT committal, which master06 §Digital
        // Signature includes in the signed serialisation of "the entire Version
        // object". Omitted when empty — `Attestations_valid` forbids a
        // present-but-empty list.
        if !parts.attestations.is_empty() {
            map.insert(
                "attestations".to_owned(),
                Value::Array(parts.attestations.to_vec()),
            );
        }
        if let Some(sig) = parts.signature {
            map.insert("signature".to_owned(), Value::String(sig.to_owned()));
        }
    }
    Ok(ov)
}
