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
use crate::service::status::CallStatusType;
use crate::versioning::Kind;
use crate::versioning::audit::{
    change_type, change_type_rubric, decode_description, dv_date_time, dv_text, openehr_coded_text,
    required_committer,
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
        return Err(ServiceError::sm(
            CallStatusType::ObjectVersionDoesNotExist,
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
    system_id: &str,
    now: jiff::Timestamp,
) -> Vec<Attestation> {
    partials
        .iter()
        .map(|partial| complete_attestation(partial, system_id, now))
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
    /// client-supplied attestation payload — a wire `UPDATE_ATTESTATION`, an
    /// RM `ATTESTATION` used as a version's `commit_audit`, or the canonical
    /// fragment [`Self::fragment`] stored for one (the three carry the same
    /// attribute names in the same encodings, so one decoder serves all).
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
/// `ATTESTATION`-declared attributes ([`AttestationParts`]) plus the two
/// inherited `AUDIT_DETAILS` attributes a client states — `committer` (1..1,
/// required by ITS-REST `schemas/common/UpdateAttestation.yaml` through its
/// `allOf` over `UpdateAudit.yaml`) and `description` (0..1). The rest of
/// `AUDIT_DETAILS` — `system_id`, `time_committed`, `change_type` — is the
/// server's.
///
/// This is the carrier the two arrival routes converge on
/// ([`Self::decode`] for a CONTRIBUTION-body attestation, [`Self::from_update`]
/// for `UPDATE_VERSION.attestations`), so completion
/// ([`complete_attestation`]) is a pure build with nothing left to fail.
#[derive(Debug, Clone)]
pub(crate) struct AttestationInput {
    /// The `ATTESTATION`-declared attributes.
    pub(crate) parts: AttestationParts,
    /// The inherited `AUDIT_DETAILS.committer` the client stated.
    pub(crate) committer: PartyProxy,
    /// The inherited `AUDIT_DETAILS.description` (0..1).
    pub(crate) description: Option<DvText>,
}

impl AttestationInput {
    /// Decodes a CONTRIBUTION-body attestation payload: a wire
    /// `UPDATE_ATTESTATION`, or an RM `ATTESTATION` used as a version's
    /// `commit_audit`.
    ///
    /// The payload states its own `committer`, which ITS-REST
    /// `schemas/common/UpdateAttestation.yaml` requires through its `allOf`
    /// over `UpdateAudit.yaml`; the server never supplies one.
    ///
    /// # Errors
    /// [`ServiceError::BadRequest`] when `committer` is absent or is not a
    /// canonical `PARTY_PROXY` (the [`required_committer`] refusals); the
    /// [`AttestationParts::decode`] rejections; [`ServiceError::Unprocessable`]
    /// when `description` is neither a string nor a canonical `DV_TEXT`.
    pub(crate) fn decode(partial: &Value) -> Result<Self, ServiceError> {
        let committer = required_committer(Some(partial), "UPDATE_ATTESTATION")?;
        // description: the inherited AUDIT_DETAILS.description (0..1). Both
        // released spellings are read — the ITS-REST `UDvText` object and the SM
        // `String [0..1]` (`update_audit.adoc`) — and the object is decoded
        // WHOLE: reducing a `DV_CODED_TEXT` to its `value` would drop the
        // `defining_code` of a committed attestation.
        let description = partial
            .get("description")
            .filter(|d| !d.is_null())
            .map(|d| match d {
                Value::String(s) => Ok(dv_text(s)),
                other => decode_description(other),
            })
            .transpose()?;
        Ok(Self {
            parts: AttestationParts::decode(partial)?,
            committer,
            description,
        })
    }

    /// The same carrier from the native `UPDATE_ATTESTATION`
    /// (`UPDATE_VERSION.attestations`, SM `UML/classes/update_version.adoc`
    /// §Attributes), whose attributes are already their RM types. Only the two
    /// invariants the wire-partial type cannot express are evaluated here; the
    /// mandatory `reason` / `is_pending` need no check at all, because the type
    /// makes their absence unrepresentable.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] for a coded `reason` outside the openEHR
    /// `attestation reason` group (`ATTESTATION.Reason_valid`) or a
    /// present-but-empty `items` list (`ATTESTATION.Items_valid`) — RM common
    /// `UML/classes/org.openehr.rm.common.attestation.adoc` §Invariants.
    pub(crate) fn from_update(
        update: &openehr_its::rest::generated::common::UpdateAttestation,
    ) -> Result<Self, ServiceError> {
        reason_valid(&update.reason)?;
        if update.items.as_ref().is_some_and(Vec::is_empty) {
            return Err(items_valid_violation());
        }
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
        })
    }
}

/// Completes a client-supplied attestation into a full RM `ATTESTATION`.
///
/// RM common master04 §Attestation; ITS-REST `UpdateAttestation`. The server
/// supplies the inherited `AUDIT_DETAILS` fields it owns — `system_id`,
/// `time_committed`, and the `666|attestation|` `change_type` — exactly as
/// `UPDATE_AUDIT` → `AUDIT_DETAILS` (SM master03 §Version Update Semantics),
/// then adds the `ATTESTATION`-specific attributes. `committer` is the one the
/// partial stated.
///
/// Infallible: every client-supplied attribute was decoded into its RM type
/// when the [`AttestationInput`] was built, so completion has nothing left to
/// reject. The result is the generated `openehr-rm` [`Attestation`], which
/// serializes with `_type` first and in the BMM's own attribute order.
pub(crate) fn complete_attestation(
    partial: &AttestationInput,
    system_id: &str,
    now: jiff::Timestamp,
) -> Attestation {
    Attestation {
        system_id: system_id.to_owned(),
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
mod tests {
    use serde_json::json;

    use super::*;

    fn now() -> jiff::Timestamp {
        "2026-07-07T10:11:12Z"
            .parse()
            .expect("the literal is a valid RFC 3339 instant")
    }

    /// Decode a CONTRIBUTION-body attestation payload and complete it, as the
    /// `666|attestation|` route does, returning the canonical form the store
    /// and the wire carry.
    ///
    /// The fixture states the committer every `UPDATE_ATTESTATION` requires
    /// (`PARTY_IDENTIFIED` with a name satisfies `Basic_validity` +
    /// `Name_valid`), so each test exercises only the attribute it names;
    /// [`missing_committer_is_refused`] pins the refusal of a partial without
    /// one.
    fn complete(partial: &Value) -> Result<Value, ServiceError> {
        let mut stated = partial.clone();
        stated
            .as_object_mut()
            .expect("the fixture is an object")
            .insert(
                "committer".to_owned(),
                json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }),
            );
        let input = AttestationInput::decode(&stated)?;
        Ok(openehr_its::json::to_canonical_value(
            &complete_attestation(&input, "ferroehr.local", now()),
        ))
    }

    /// An `UPDATE_ATTESTATION` without a `committer` is refused as the
    /// schema-invalid body it is (400), never completed with a committer the
    /// client did not state: ITS-REST `schemas/common/UpdateAttestation.yaml`
    /// inherits `required: [change_type, committer]` from `UpdateAudit.yaml`
    /// through `allOf`. A `null` committer is the same absence.
    #[test]
    fn missing_committer_is_refused() {
        let base = json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false
        });
        let mut null_committer = base.clone();
        null_committer
            .as_object_mut()
            .expect("built as an object above")
            .insert("committer".to_owned(), Value::Null);
        for partial in [base, null_committer] {
            match AttestationInput::decode(&partial) {
                Err(ServiceError::BadRequest(e)) => {
                    assert!(
                        e.message
                            .contains("UPDATE_ATTESTATION.committer is required"),
                        "{e:?}"
                    );
                }
                other => panic!("expected the 400 refusal, got {other:?}"),
            }
        }
    }

    /// The completed `ATTESTATION` is the canonical serialization of the
    /// generated RM type: `_type` first, then the BMM's own attribute order
    /// (the inherited `AUDIT_DETAILS` attributes, then `attested_view`,
    /// `proof`, `items`, `reason`, `is_pending` — RM common
    /// `UML/classes/org.openehr.rm.common.attestation.adoc` §Attributes).
    #[test]
    fn completed_attestation_is_in_bmm_attribute_order() {
        let att = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "proof": "signed-by-hand",
            "items": [{ "_type": "DV_EHR_URI", "value": "ehr://x/y" }]
        }))
        .expect("a complete, well-typed UPDATE_ATTESTATION partial");
        let keys: Vec<&str> = att
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(
            keys,
            vec![
                "_type",
                "system_id",
                "time_committed",
                "change_type",
                "committer",
                "proof",
                "items",
                "reason",
                "is_pending",
            ]
        );
        assert_eq!(
            att.get("_type").and_then(Value::as_str),
            Some("ATTESTATION")
        );
    }

    /// A `DV_CODED_TEXT` `description` survives completion whole. The
    /// inherited `AUDIT_DETAILS.description` is typed `DV_TEXT` at 0..1 (RM
    /// common `UML/classes/org.openehr.rm.common.audit_details.adoc`
    /// §Attributes, inherited by `…org.openehr.rm.common.attestation.adoc`)
    /// and `DV_CODED_TEXT` is a substitutable subtype — so the `_type`, the
    /// display `value` and the whole `defining_code` come back unchanged
    /// rather than being flattened to the plain text they share.
    #[test]
    fn coded_description_round_trips() {
        let description = json!({
            "_type": "DV_CODED_TEXT",
            "value": "amended after multidisciplinary review",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": "at0004"
            }
        });
        let att = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "description": description.clone()
        }))
        .expect("a coded description is a valid DV_TEXT");
        assert_eq!(att.get("description"), Some(&description));
    }

    /// The SM spelling of the same attribute — `UPDATE_AUDIT.description` is
    /// `String [0..1]` (`SM/docs/UML/classes/update_audit.adoc` §Attributes) —
    /// still completes into the plain `DV_TEXT` it denotes.
    #[test]
    fn string_description_becomes_plain_dv_text() {
        let att = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "description": "countersigned"
        }))
        .expect("the SM string spelling is accepted");
        assert_eq!(
            att.get("description"),
            Some(&json!({ "_type": "DV_TEXT", "value": "countersigned" }))
        );
    }

    /// A `description` that is neither a string nor a canonical `DV_TEXT` is
    /// refused (422) instead of silently dropped.
    #[test]
    fn malformed_description_is_refused() {
        let err = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "description": { "_type": "DV_CODED_TEXT", "value": "no defining_code" }
        }))
        .expect_err("a DV_CODED_TEXT without its mandatory defining_code must be refused");
        // Asserted as DATA: the refused attribute path, and the decode
        // failure's own JSON path carried in the causes.
        match err {
            ServiceError::Unprocessable { violation: v, .. } => {
                assert_eq!(v.path(), Some("AUDIT_DETAILS.description"));
                assert_eq!(v.causes().len(), 1, "the decode failure is the cause");
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    /// `ATTESTATION.attested_view` is a `DV_MULTIMEDIA` (0..1). A value that
    /// does not decode as one is refused (422) naming the attribute, rather
    /// than stored verbatim: the completed attestation is built as the RM type,
    /// so every client-supplied attribute is read.
    #[test]
    fn malformed_attested_view_is_refused() {
        // DV_MULTIMEDIA.media_type is 1..1 (attestation.adoc §Attributes →
        // dv_multimedia.adoc), so this object cannot be one.
        for bad in [json!({ "_type": "DV_MULTIMEDIA" }), json!("a screenshot")] {
            let err = complete(&json!({
                "reason": { "_type": "DV_TEXT", "value": "witness" },
                "is_pending": false,
                "attested_view": bad
            }))
            .expect_err("a malformed attested_view must be refused");
            match err {
                ServiceError::Unprocessable { violation: v, .. } => {
                    assert_eq!(v.path(), Some("ATTESTATION.attested_view"));
                    assert_eq!(v.detail(), "is not a valid DV_MULTIMEDIA");
                }
                other => panic!("expected Unprocessable, got {other:?}"),
            }
        }
    }

    /// `ATTESTATION.proof` is a `String` (0..1) — a non-string is refused.
    #[test]
    fn non_string_proof_is_refused() {
        let err = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "proof": { "value": "not-a-string" }
        }))
        .expect_err("a non-string proof must be refused");
        match err {
            ServiceError::Unprocessable { violation: v, .. } => {
                assert_eq!(v.path(), Some("ATTESTATION.proof"));
                assert_eq!(v.detail(), "must be a String (0..1)");
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    /// `ATTESTATION.items` is a `List<DV_EHR_URI>` — a member that is not one
    /// is refused, naming its index.
    #[test]
    fn malformed_items_member_is_refused() {
        let err = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "items": [{ "_type": "DV_EHR_URI", "value": "ehr://x/y" }, 7]
        }))
        .expect_err("a malformed items member must be refused");
        match err {
            // The offending INDEX is data on the path, not a substring hunt.
            ServiceError::Unprocessable { violation: v, .. } => {
                assert_eq!(v.path(), Some("ATTESTATION.items[1]"));
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    /// A coded `reason` whose `defining_code` is outside the openEHR
    /// `attestation reason` group is refused (`ATTESTATION.Reason_valid`,
    /// `org.openehr.rm.common.attestation.adoc` §Invariants).
    #[test]
    fn out_of_group_coded_reason_is_refused() {
        let err = complete(&json!({
            "reason": {
                "_type": "DV_CODED_TEXT",
                "value": "signed",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    // 249 is `creation` (audit change type), not an
                    // `attestation reason` member ({240 signed, 648 witnessed}).
                    "code_string": "249"
                }
            },
            "is_pending": false
        }))
        .expect_err("an out-of-group coded reason must be refused");
        match err {
            ServiceError::Unprocessable { violation: v, .. } => {
                assert_eq!(v.path(), Some("ATTESTATION.reason.defining_code"));
                assert_eq!(v.invariant(), Some("ATTESTATION.Reason_valid"));
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    /// A JSON `null` `items` is the ABSENT encoding of the 0..1 attribute, so
    /// it is accepted and yields no list — `ATTESTATION.Items_valid` (`items
    /// /= Void implies not items.is_empty`) constrains a PRESENT list, and
    /// Void is exactly what a null spells. This is the shape the native
    /// `UPDATE_VERSION.attestations` DTO emits for `Option::None`, so a
    /// rejection here would make the at-committal attestation route
    /// unreachable through the typed API.
    #[test]
    fn null_items_is_absent_not_invalid() {
        let att = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "items": Value::Null,
            "proof": Value::Null,
            "attested_view": Value::Null,
            "description": Value::Null
        }))
        .expect("a null optional is absent, not invalid");
        assert_eq!(att.get("items"), None, "no items list is serialized");
    }

    /// The native `UPDATE_VERSION.attestations` partial, as the direct-write
    /// route supplies it.
    ///
    /// The inherited `change_type` is a `DV_CODED_TEXT`: ITS-REST
    /// `schemas/common/UpdateAudit.yaml` `$ref`s `DvCodedText` for it, and the
    /// ITS-REST docs text is silent on the member's shape, so the released OAS
    /// grounds it (`.claude/rules/spec-adherence.md` §the ITS-REST
    /// wire-oracle order). The flat SM `Terminology_code` spelling this
    /// fixture used to send is not that shape — see
    /// [`flat_terminology_code_change_type_is_refused`] for the twin that
    /// pins its refusal.
    fn update_attestation(body: &Value) -> openehr_its::rest::generated::common::UpdateAttestation {
        let mut wire = json!({
            "change_type": {
                "value": "attestation",
                "defining_code": {
                    "terminology_id": { "value": "openehr" },
                    "code_string": "666"
                }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
        });
        for (k, v) in body.as_object().expect("the fixture is an object") {
            wire.as_object_mut()
                .expect("built as an object above")
                .insert(k.clone(), v.clone());
        }
        openehr_its::json::from_canonical_value(&wire)
            .expect("the fixture is a well-typed UPDATE_ATTESTATION")
    }

    /// The invalid twin of [`update_attestation`]'s `change_type`: the flat SM
    /// `Terminology_code` spelling (`{terminology_id, code_string}`) is NOT
    /// the released wire shape of `UPDATE_AUDIT.change_type`
    /// (`schemas/common/UpdateAudit.yaml` → `DvCodedText`), so the strict
    /// canonical reader refuses it — the PARSE class, answered `400` on the
    /// wire. A reader that started accepting it would silently admit a member
    /// with no `defining_code`, which is the attribute
    /// `AUDIT_DETAILS.Change_type_valid` is stated over.
    #[test]
    fn flat_terminology_code_change_type_is_refused() {
        let refused = openehr_its::json::from_canonical_value::<
            openehr_its::rest::generated::common::UpdateAttestation,
        >(&json!({
            "change_type": { "terminology_id": "openehr", "code_string": "666" },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" },
            "reason": { "value": "signed" },
            "is_pending": false
        }));
        let err = refused.expect_err("a flat TERMINOLOGY_CODE change_type must be refused");
        assert!(
            err.to_string().contains("terminology_id"),
            "the refusal names the offending member: {err}"
        );
    }

    /// The native route reaches the SAME invariants as the body route: a coded
    /// `reason` outside the openEHR `attestation reason` group is refused
    /// (`ATTESTATION.Reason_valid`).
    #[test]
    fn native_partial_refuses_out_of_group_coded_reason() {
        let update = update_attestation(&json!({
            "reason": {
                "_type": "DV_CODED_TEXT", "value": "signed",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "249"
                }
            },
            "is_pending": false
        }));
        match AttestationInput::from_update(&update) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert_eq!(v.path(), Some("ATTESTATION.reason.defining_code"));
                assert_eq!(v.invariant(), Some("ATTESTATION.Reason_valid"));
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    /// `ATTESTATION.Items_valid` on the native route: a present-but-empty list
    /// is refused, an absent one is not.
    #[test]
    fn native_partial_enforces_items_valid() {
        let base = json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": true
        });
        let mut empty = base.clone();
        empty
            .as_object_mut()
            .expect("built as an object above")
            .insert("items".to_owned(), json!([]));
        match AttestationInput::from_update(&update_attestation(&empty)) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert_eq!(v.path(), Some("ATTESTATION.items"));
                assert_eq!(v.invariant(), Some("ATTESTATION.Items_valid"));
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
        let absent = AttestationInput::from_update(&update_attestation(&base))
            .expect("an absent items list is legal");
        assert!(absent.parts.items.is_empty());
    }

    /// A native partial completes into the same `ATTESTATION` shape the body
    /// route produces: the committer is the one the partial stated, and the
    /// description object is kept whole.
    #[test]
    fn native_partial_completes_into_an_attestation() {
        // `description` is an OBJECT on the typed wire: ITS-REST
        // `schemas/common/UpdateAudit.yaml` `$ref`s `DvText` (whose
        // `DV_CODED_TEXT` subtype substitutes for it). The bare-string SM
        // spelling (`UML/classes/update_audit.adoc`: `String [0..1]`) is
        // accepted only on the raw-body CONTRIBUTION lane —
        // [`string_description_becomes_plain_dv_text`] pins that twin.
        let update = update_attestation(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "description": { "_type": "DV_TEXT", "value": "countersigned" }
        }));
        let input =
            AttestationInput::from_update(&update).expect("a well-typed partial is accepted");
        let att = openehr_its::json::to_canonical_value(&complete_attestation(
            &input,
            "ferroehr.local",
            now(),
        ));
        assert_eq!(
            att.get("description"),
            Some(&json!({ "_type": "DV_TEXT", "value": "countersigned" }))
        );
        // The committer the partial stated.
        assert_eq!(
            att.pointer("/committer/_type").and_then(Value::as_str),
            Some("PARTY_IDENTIFIED")
        );
        assert_eq!(att.get("items"), None);
    }

    /// A present-but-empty `items` list is refused (`ATTESTATION.Items_valid`:
    /// `items /= Void implies not items.is_empty`,
    /// `org.openehr.rm.common.attestation.adoc` §Invariants).
    #[test]
    fn present_but_empty_items_is_refused() {
        let err = complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "items": []
        }))
        .expect_err("a present-but-empty items list must be refused");
        match err {
            ServiceError::Unprocessable { violation: v, .. } => {
                assert_eq!(v.path(), Some("ATTESTATION.items"));
                assert_eq!(v.invariant(), Some("ATTESTATION.Items_valid"));
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }
}
