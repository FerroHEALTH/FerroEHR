// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Attestation: attaching an `ATTESTATION` to an `ORIGINAL_VERSION` at or after
//! committal.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Attestation + RM
//! common `master04-generic_package.adoc` §Attestation. An `ATTESTATION` is an
//! `AUDIT_DETAILS` subtype (`items?`, `reason`, `proof`, `is_pending`); it "can
//! be added at any time after committal" and a `666|attestation|` member of a
//! CONTRIBUTION adds **no** new version. Attestations of an old version are not
//! valid for a new version (they are keyed to `(vo_id, sys_version)` and never
//! copied forward).
//!
//! The two arrival routes stand on opposite sides of the `VERSION.signature`
//! (master06 §Digital Signature and §Attestation):
//!
//! * Committed with the version (`UPDATE_VERSION.attestations`): attributes of
//!   the Version at the moment it is serialised, and §Digital Signature signs
//!   "the entire Version object (note that the signature attribute will be Void
//!   at this point)". They are completed before the signature is computed
//!   ([`complete_accompanying`]) and stored with `at_committal = true`, so the
//!   bytes signed at commit are the bytes served at read.
//! * Added afterwards (the `666|attestation|` CONTRIBUTION member, [`attest`]):
//!   these post-date the signature, so they are stored with
//!   `at_committal = false` and appended to the served version after
//!   verification, never entering its canonical form.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use openehr_rm::prelude::{Attestation, DvEhrUri, DvMultimedia, DvText, PartyProxy};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::error::{ServiceError, Violation};
use crate::service::status::ServiceStatus;
use crate::versioning::Kind;
use crate::versioning::audit::{
    change_type, change_type_rubric, dv_date_time, openehr_coded_text, validate_committer,
};
use crate::versioning::change::Committed;
use crate::versioning::object_version_id::TreeId;

/// A `666|attestation|` of an **existing** `ORIGINAL_VERSION` committed within a
/// CONTRIBUTION (master06 §Contributions — adds no new version). Carried
/// alongside the change set so it commits in the same transaction.
pub(crate) struct PendingAttest {
    pub(crate) vo_id: VoId,
    pub(crate) kind: Kind,
    /// The target version to attest (from `preceding_version_uid` — trunk or
    /// branch).
    pub(crate) expected: TreeId,
    /// The `system_id` the attestation takes when it states none: the
    /// `666|attestation|` member audit's.
    pub(crate) system_id: String,
    /// The client-supplied half of the attestation, completed into a full RM
    /// `ATTESTATION` at commit time.
    pub(crate) partial: AttestationInput,
}

/// Attach an `ATTESTATION` to an **existing** `ORIGINAL_VERSION` (a
/// `666|attestation|` version item; master06 §Contributions — no new version,
/// and the object's head row is untouched). Realizes `VERSIONED_OBJECT.commit_attestation`
/// precondition `has_version_id` (master06 §Versioned Objects). `attestation`
/// is the already-completed full RM `ATTESTATION`.
///
/// # Errors
/// [`ServiceError::NotFound`] when the target `(vo_id, tree, kind)` does not
/// exist or does not belong to `ehr_id`; the storage errors of the target
/// lookup / attestation insert.
#[expect(
    clippy::too_many_arguments,
    reason = "the parts of an attestation act plus its commit instant; a \
              parameter struct would not read clearer at the one call site"
)]
pub(crate) async fn attest(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: VoId,
    kind: Kind,
    expected: TreeId,
    attestation: &Attestation,
    contribution_id: Uuid,
    time_committed: jiff::Timestamp,
) -> Result<Committed, ServiceError> {
    // The target's owning EHR is compared against the caller's, its ordinal
    // keys the attestation, and its `creating_system_id` rides the outbox.
    let target = crate::storage::version_repo::attestation::attestation_target(
        tx,
        vo_id,
        expected.columns(),
        kind.as_str(),
    )
    .await?;
    let Some(target) = target.filter(|t| t.ehr_id == ehr_id) else {
        return Err(ServiceError::new(
            ServiceStatus::ObjectVersionDoesNotExist,
            format!("{} version {vo_id}::{expected}", kind.as_str()),
        ));
    };
    // The is_original_version(a_ver_id) half of the commit_attestation
    // precondition (`…common.versioned_object.adoc` §Functions: "Attestations
    // can only be added to Original versions"): an IMPORTED_VERSION wraps a
    // foreign ORIGINAL_VERSION that must stay byte-verbatim (master06
    // §Copying — "the ORIGINAL_VERSION instance is never modified"), so it is
    // not an attestable target.
    if target.imported {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "names an IMPORTED_VERSION — attestations can only be added to \
                 Original versions",
            )
            .with_path("preceding_version_uid")
            .with_invariant("VERSIONED_OBJECT.commit_attestation"),
        ));
    }
    crate::storage::version_repo::attestation::insert_attestation(
        tx,
        vo_id,
        target.sys_version,
        contribution_id,
        // AFTER committal by construction: this route attaches to a version
        // that already exists (master06 §Contributions, "an existing
        // `ORIGINAL_VERSION`"), so the attestation post-dates that version's
        // signature and stays outside its canonical form.
        false,
        &openehr_its::json::to_canonical_value(attestation),
    )
    .await?;
    Ok(Committed {
        vo_id,
        sys_version: target.sys_version,
        tree: expected,
        creating_system_id: target.creating_system_id,
        kind,
        // A 666 attestation adds no new version; it is announced in the
        // contribution's outbox envelope as a change to the existing version.
        // The code lives on the ATTESTATION's OWN inherited `change_type`, not
        // on a `commit_audit` it does not have.
        change_type: change_type::ATTESTATION.to_owned(),
        template_id: None,
        root_archetype: None,
        // The contribution's commit-act time — a 666 attestation adds no new
        // version, so this is the instant the attestation itself committed.
        time_committed,
    })
}

/// Complete the attestations committed together with a NEW version
/// (`UPDATE_VERSION.attestations`; master06 §Attestation "Signing content at
/// committal") into full canonical RM `ATTESTATION`s.
///
/// This runs BEFORE the version's signature is computed, and its output is both
/// what gets signed and what gets stored ([`insert_at_committal_attestations`])
/// — one completion, so the signed bytes and the served bytes cannot drift
/// (master06 §Digital Signature).
///
/// Infallible: each partial was decoded into its RM types when its
/// [`AttestationInput`] was built.
pub(crate) fn complete_accompanying(
    partials: &[AttestationInput],
    default_system_id: &str,
    now: jiff::Timestamp,
) -> Vec<Attestation> {
    partials
        .iter()
        .map(|partial| complete_attestation(partial, default_system_id, now))
        .collect()
}

/// Persist the already-completed attestations of a just-written version, marked
/// `at_committal` — they are inside that version's signed canonical form
/// (master06 §Digital Signature; see the module docs).
///
/// # Errors
/// The storage error of the attestation insert.
pub(crate) async fn insert_at_committal_attestations(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    contribution_id: Uuid,
    completed: &[Value],
) -> Result<(), ServiceError> {
    for full in completed {
        crate::storage::version_repo::attestation::insert_attestation(
            tx,
            vo_id,
            sys_version,
            contribution_id,
            true,
            full,
        )
        .await?;
    }
    Ok(())
}

/// The attributes `ATTESTATION` declares on top of the `AUDIT_DETAILS` it
/// inherits from — `attested_view`, `proof`, `items`, `reason`, `is_pending`
/// (RM common `UML/classes/org.openehr.rm.common.attestation.adoc`
/// §Attributes) — each decoded into the RM type its class table gives it.
///
/// This is the ONLY part of an attestation a client supplies: the inherited
/// `system_id` / `time_committed` / `change_type` are the server's (ITS-REST
/// overview `Requests_and_responses.md` §"openehr-version and
/// openehr-audit-details": "The `time_committed` attribute is always set by
/// the server"), and `committer` / `description` are shared with every audit.
#[derive(Debug, Clone)]
pub(crate) struct AttestationParts {
    /// `ATTESTATION.attested_view` (0..1).
    pub(crate) attested_view: Option<DvMultimedia>,
    /// `ATTESTATION.proof` (0..1).
    pub(crate) proof: Option<String>,
    /// `ATTESTATION.items` (0..1; empty ≙ absent — `Items_valid` forbids a
    /// present-but-empty list).
    pub(crate) items: Vec<DvEhrUri>,
    /// `ATTESTATION.reason` (1..1).
    pub(crate) reason: DvText,
    /// `ATTESTATION.is_pending` (1..1).
    pub(crate) is_pending: bool,
}

impl AttestationParts {
    /// Decode + invariant-check the `ATTESTATION`-declared attributes of a
    /// stored attestation: the canonical fragment [`Self::fragment`] writes
    /// for a commit audit that is an `ATTESTATION`.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] when an RM invariant fails:
    /// - `reason` absent (mandatory, 1..1), or a coded `reason` whose
    ///   `defining_code` is not in the openEHR `attestation reason` group
    ///   (`ATTESTATION.Reason_valid`);
    /// - `is_pending` absent or not a `Boolean` (mandatory, 1..1);
    /// - `items` present but not a non-empty list (`ATTESTATION.Items_valid`);
    /// - `reason` / `attested_view` / `proof` / `items` present but not
    ///   decodable as the RM type the class table gives them (`DV_TEXT`,
    ///   `DV_MULTIMEDIA`, `String`, `List<DV_EHR_URI>` — RM common
    ///   `UML/classes/org.openehr.rm.common.attestation.adoc` §Attributes).
    pub(crate) fn decode(partial: &Value) -> Result<Self, ServiceError> {
        // reason (1..1)
        let reason = partial.get("reason").ok_or_else(|| {
            ServiceError::content_invalid(
                Violation::new("is required (1..1)").with_path("ATTESTATION.reason"),
            )
        })?;
        // Reason_valid: a coded reason's defining_code must be a member of the
        // openEHR `attestation reason` group.
        if reason.get("_type").and_then(Value::as_str) == Some("DV_CODED_TEXT") {
            reason_code_valid(
                reason
                    .pointer("/defining_code/code_string")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )?;
        }
        // is_pending (1..1, Boolean)
        let is_pending = partial
            .get("is_pending")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                ServiceError::content_invalid(
                    Violation::new("is required (1..1 Boolean)")
                        .with_path("ATTESTATION.is_pending"),
                )
            })?;
        // items (0..1); Items_valid: non-empty when present. A JSON `null` is
        // the ABSENT encoding of an optional attribute (the one the native
        // `UPDATE_VERSION` DTO emits for `Option::None`), so it is filtered out
        // before `Items_valid`; a present-but-EMPTY list still fails.
        // NOTE: the released text's two statements of `items` SCOPE conflict
        // (master04 §Attestation vs the class table on `attestation.adoc`); the
        // class model is enforced — `Items_valid`, and NO containment check.
        let items = partial.get("items").filter(|v| !v.is_null());
        if let Some(items) = items
            && items.as_array().is_none_or(Vec::is_empty)
        {
            return Err(items_valid_violation());
        }
        Ok(Self {
            attested_view: partial
                .get("attested_view")
                .filter(|v| !v.is_null())
                .map(|v| decode(v, "ATTESTATION.attested_view", "DV_MULTIMEDIA"))
                .transpose()?,
            // Not the VERSION `signature` mechanism, where the server owns both
            // the canonicalisation and the key (master06 §Digital Signature).
            // NOTE: `proof` is an OPAQUE CLIENT FACT — no released text defines a
            // canonical form to recompute against ("The exact serialisation is
            // not yet defined by openEHR", master04 §Attestation).
            proof: partial
                .get("proof")
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_str().map(str::to_owned).ok_or_else(|| {
                        ServiceError::content_invalid(
                            Violation::new("must be a String (0..1)")
                                .with_path("ATTESTATION.proof"),
                        )
                    })
                })
                .transpose()?,
            items: match items {
                None => Vec::new(),
                Some(items) => items
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, v)| decode(v, &format!("ATTESTATION.items[{i}]"), "DV_EHR_URI"))
                    .collect::<Result<_, _>>()?,
            },
            reason: decode(reason, "ATTESTATION.reason", "DV_TEXT")?,
            is_pending,
        })
    }

    /// These attributes as a canonical JSON fragment — the storage form of a
    /// commit audit that is an `ATTESTATION` (`audit.attestation`), each
    /// attribute in the encoding `openehr-its` gives its RM type and absent
    /// optionals omitted. Round-trips through [`Self::decode`].
    pub(crate) fn fragment(&self) -> Value {
        let mut map = serde_json::Map::new();
        if let Some(attested_view) = &self.attested_view {
            map.insert(
                "attested_view".to_owned(),
                openehr_its::json::to_canonical_value(attested_view),
            );
        }
        if let Some(proof) = &self.proof {
            map.insert("proof".to_owned(), Value::String(proof.clone()));
        }
        if !self.items.is_empty() {
            map.insert(
                "items".to_owned(),
                Value::Array(
                    self.items
                        .iter()
                        .map(openehr_its::json::to_canonical_value)
                        .collect(),
                ),
            );
        }
        map.insert(
            "reason".to_owned(),
            openehr_its::json::to_canonical_value(&self.reason),
        );
        map.insert("is_pending".to_owned(), Value::Bool(self.is_pending));
        Value::Object(map)
    }
}

/// The COMPLETE client-supplied half of an `ATTESTATION`: the
/// `ATTESTATION`-declared attributes ([`AttestationParts`]) plus the inherited
/// `AUDIT_DETAILS` attributes a client states — `committer` (1..1, required by
/// ITS-REST `schemas/common/UpdateAttestation.yaml` through its `allOf` over
/// `UpdateAudit.yaml`), `description` (0..1) and `system_id` (optional on the
/// wire: "when omitted the server sets its own value", `UpdateAudit.yaml`).
/// `time_committed` and `change_type` are the server's.
///
/// Both arrival routes, `UPDATE_VERSION.attestations` on a direct write and on
/// a CONTRIBUTION member, build it with [`Self::from_update`] from the
/// generated `UpdateAttestation`, so completion ([`complete_attestation`]) is a
/// pure build with nothing left to fail.
#[derive(Debug, Clone)]
pub(crate) struct AttestationInput {
    /// The `ATTESTATION`-declared attributes.
    pub(crate) parts: AttestationParts,
    /// The inherited `AUDIT_DETAILS.committer` the client stated.
    pub(crate) committer: PartyProxy,
    /// The inherited `AUDIT_DETAILS.description` (0..1).
    pub(crate) description: Option<DvText>,
    /// The inherited `AUDIT_DETAILS.system_id` the client stated, if any.
    pub(crate) system_id: Option<String>,
}

impl AttestationInput {
    /// The carrier from the native `UPDATE_ATTESTATION`
    /// (`UPDATE_VERSION.attestations`, ITS-REST
    /// `schemas/common/UpdateAttestation.yaml`), whose attributes are already
    /// their RM types. Only the invariants the wire-partial type cannot express
    /// are evaluated here; the mandatory `reason` / `is_pending` / `committer`
    /// need no presence check, because the type makes their absence
    /// unrepresentable.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] for a coded `reason` outside the openEHR
    /// `attestation reason` group (`ATTESTATION.Reason_valid`) or a
    /// present-but-empty `items` list (`ATTESTATION.Items_valid`) — RM common
    /// `UML/classes/org.openehr.rm.common.attestation.adoc` §Invariants —, an
    /// empty stated `system_id` (`AUDIT_DETAILS.System_id_valid`), or a
    /// committer breaking its own `PARTY_PROXY` invariants.
    pub(crate) fn from_update(
        update: &openehr_its::rest::generated::common::UpdateAttestation,
    ) -> Result<Self, ServiceError> {
        reason_valid(&update.reason)?;
        if update.items.as_ref().is_some_and(Vec::is_empty) {
            return Err(items_valid_violation());
        }
        // `UpdateAudit.yaml`: a supplied `system_id` "will be validated".
        if update.system_id.as_deref() == Some("") {
            return Err(ServiceError::content_invalid(
                Violation::new("is mandatory and non-void")
                    .with_path("ATTESTATION.system_id")
                    .with_invariant("AUDIT_DETAILS.System_id_valid"),
            ));
        }
        validate_committer(&update.committer)?;
        Ok(Self {
            parts: AttestationParts {
                attested_view: update.attested_view.clone(),
                proof: update.proof.clone(),
                items: update.items.clone().unwrap_or_default(),
                reason: update.reason.clone(),
                is_pending: update.is_pending,
            },
            committer: update.committer.clone(),
            // The inherited `UPDATE_AUDIT.description`, kept WHOLE: the wire
            // types it `DV_TEXT` (`schemas/common/UpdateAudit.yaml`), whose
            // `DV_CODED_TEXT` subtype substitutes for it, and reducing it to a
            // string would drop a coded description's `defining_code`
            // permanently.
            description: update.description.clone(),
            system_id: update.system_id.clone(),
        })
    }
}

/// Completes a client-supplied attestation into a full RM `ATTESTATION`.
///
/// RM common master04 §Attestation; ITS-REST `UpdateAttestation`. The server
/// supplies the inherited `AUDIT_DETAILS` fields it owns — `time_committed`,
/// the `666|attestation|` `change_type`, and `system_id` when the partial
/// states none (`default_system_id`, the audit of the version it attests) —
/// exactly as `UPDATE_AUDIT` → `AUDIT_DETAILS` (ITS-REST `UpdateAudit.yaml`),
/// then adds the `ATTESTATION`-specific attributes. `committer` is the one the
/// partial stated.
///
/// Infallible: every client-supplied attribute was decoded into its RM type
/// when the [`AttestationInput`] was built, so completion has nothing left to
/// reject. The result is the generated `openehr-rm` [`Attestation`], which
/// serializes with `_type` first and in the BMM's own attribute order.
pub(crate) fn complete_attestation(
    partial: &AttestationInput,
    default_system_id: &str,
    now: jiff::Timestamp,
) -> Attestation {
    Attestation {
        system_id: partial
            .system_id
            .clone()
            .unwrap_or_else(|| default_system_id.to_owned()),
        time_committed: dv_date_time(&now),
        change_type: openehr_coded_text(
            change_type::ATTESTATION,
            change_type_rubric(change_type::ATTESTATION),
        ),
        description: partial.description.clone(),
        committer: partial.committer.clone(),
        attested_view: partial.parts.attested_view.clone(),
        proof: partial.parts.proof.clone(),
        items: openehr_base::containers::present_nonempty(partial.parts.items.clone()),
        reason: partial.parts.reason.clone(),
        is_pending: partial.parts.is_pending,
    }
}

/// `ATTESTATION.Reason_valid` — the ONE statement of the rule both arrival
/// routes evaluate: a coded `reason`'s `defining_code` must be a member of the
/// openEHR `attestation reason` group (RM common
/// `UML/classes/org.openehr.rm.common.attestation.adoc` §Invariants). The
/// routes differ only in how they reach the code string (a JSON pointer on a
/// body payload, the typed `DV_CODED_TEXT` on a native partial).
///
/// # Errors
/// [`ServiceError::Unprocessable`] when `code` is not a group member.
fn reason_code_valid(code: &str) -> Result<(), ServiceError> {
    if openehr_term::bundle::openehr().is_valid_attestation_reason(code) {
        return Ok(());
    }
    Err(ServiceError::content_invalid(
        Violation::new(format!(
            "{code:?} is not in the openEHR `attestation reason` group"
        ))
        .with_path("ATTESTATION.reason.defining_code")
        .with_invariant("ATTESTATION.Reason_valid"),
    ))
}

/// [`reason_code_valid`] for an already-typed `reason`; a plain `DV_TEXT`
/// carries no `defining_code` and so cannot break the invariant.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the coded reason is out of group.
fn reason_valid(reason: &DvText) -> Result<(), ServiceError> {
    match reason {
        DvText::DvText(_) => Ok(()),
        DvText::DvCodedText(coded) => reason_code_valid(&coded.defining_code.code_string),
    }
}

/// `ATTESTATION.Items_valid` (`items /= Void implies not items.is_empty`, RM
/// common `UML/classes/org.openehr.rm.common.attestation.adoc` §Invariants) —
/// the single violation both arrival routes raise for a present-but-empty list.
fn items_valid_violation() -> ServiceError {
    ServiceError::content_invalid(
        Violation::new("must be a non-empty list when present")
            .with_path("ATTESTATION.items")
            .with_invariant("ATTESTATION.Items_valid"),
    )
}

/// Decode one client-supplied `ATTESTATION` attribute into its RM type,
/// reporting the attribute by name so a `422` says which field was wrong.
fn decode<T: serde::de::DeserializeOwned>(
    value: &Value,
    attribute: &str,
    rm_type: &str,
) -> Result<T, ServiceError> {
    openehr_its::json::from_canonical_value::<T>(value).map_err(|e| {
        ServiceError::content_invalid(
            Violation::new(format!("is not a valid {rm_type}"))
                .with_path(attribute)
                .with_decode_failure(&e),
        )
    })
}

#[cfg(test)]
mod tests;
