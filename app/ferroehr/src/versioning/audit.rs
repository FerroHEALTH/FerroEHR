// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `AUDIT_DETAILS` — the commit provenance every version and contribution
//! carries, and the `audit_change_type` terminology codes it is
//! coded from.
//!
//! Spec: RM common `master04-generic_package.adoc` §Audit Details
//! (`AUDIT_DETAILS` = `system_id`, `committer: PARTY_PROXY`, `time_committed`,
//! `change_type`, `description?`) and RM common
//! `master06-change_control_package.adoc` §Contributions (the change-type
//! group). The wire form of the change type is its **numeric group code**
//! (`AUDIT_DETAILS.Change_type_valid` requires `change_type.defining_code` to
//! be a member of the openEHR `audit_change_type` group, e.g. `"249"`, not the
//! rubric `"creation"`); the numeric code is stored and the human rubric is
//! resolved from the `openehr-term` bundle at the render edge, never a
//! hardcoded rubric.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use openehr_base::prelude::TerminologyId;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData};
use openehr_rm::prelude::{
    Attestation, AuditDetails, AuditDetailsData, CodePhrase, DvCodedText, DvDateTime, DvText,
    DvTextData, PartyProxy,
};
use openehr_term::bundle::openehr;
use serde_json::Value;

use crate::service::error::{ServiceError, Violation};
use crate::versioning::attestation::AttestationParts;

/// The openEHR internal terminology id (`Terminology_id_openehr`).
pub(crate) const OPENEHR: &str = "openehr";

/// The `audit_change_type` openEHR terminology group id.
const AUDIT_CHANGE_TYPE: &str = "audit_change_type";

/// The COMPLETE `audit_change_type` group (TERM 3.1.0
/// `openehr_terminology.xml` group `audit_change_type`; RM common master06
/// §Contributions), one named constant per member. Membership checks still go
/// through [`change_type_code`] (the bundle is the authority); the constants
/// exist so no write path or compatibility rule ever spells a code as a bare
/// string literal.
pub(crate) mod change_type {
    /// `249|creation|` — first version of a versioned object.
    pub(crate) const CREATION: &str = "249";
    /// `250|amendment|` — a correcting content change to an existing object
    /// (a legal client choice on an update, alongside `251`).
    pub(crate) const AMENDMENT: &str = "250";
    /// `251|modification|` — a content change to an existing object.
    pub(crate) const MODIFICATION: &str = "251";
    /// `252|synthesis|` — a version synthesized from other sources.
    pub(crate) const SYNTHESIS: &str = "252";
    /// `253|unknown|` — provenance of the change is unknown.
    pub(crate) const UNKNOWN: &str = "253";
    /// `523|deleted|` — a logical deletion.
    pub(crate) const DELETED: &str = "523";
    /// `666|attestation|` — attaches an `ATTESTATION` to an existing
    /// `ORIGINAL_VERSION` (adds no new version — RM common master06
    /// §Contributions; the contribution path's `Action::Attest`).
    pub(crate) const ATTESTATION: &str = "666";
    /// `816|restoration|` — restores earlier content as a new version.
    pub(crate) const RESTORATION: &str = "816";
    /// `817|format conversion|` — the same content re-encoded in another
    /// format, committed as a new version.
    pub(crate) const FORMAT_CONVERSION: &str = "817";
}

/// Resolve an inbound audit `change_type` token — either a numeric group code
/// (`"249"`) or a rubric (`"creation"`) — to its canonical numeric group code.
/// `None` when the token is not a member of the `audit_change_type` group
/// (RM common master04 `AUDIT_DETAILS.Change_type_valid` — callers must reject,
/// never store, an out-of-group change type).
pub(crate) fn change_type_code(token: &str) -> Option<String> {
    let t = openehr();
    if t.is_valid_audit_change_type(token) {
        return Some(token.to_owned());
    }
    t.concepts_in_group(AUDIT_CHANGE_TYPE)
        .iter()
        .find(|c| c.rubric.eq_ignore_ascii_case(token))
        .map(|c| c.id.clone())
}

/// The effective `change_type` of a direct commit: the caller's supplied
/// token when it names a legal, operation-compatible `audit_change_type`
/// group member; the operation's default when the caller supplied none
/// (empty) or restated the default.
///
/// ITS-REST overview §"openehr-version and openehr-audit-details" lists
/// `change_type` among the attributes clients may supply and requires that
/// "whatever is provided it MUST be merged", so a legal divergent value such as
/// `250|amendment|` on an update is honoured. Operation compatibility mirrors
/// the CONTRIBUTION path (RM common master06 §Contributions): a direct create
/// commits a first version (`249` only), a direct delete a logical deletion
/// (`523` only), and a direct update any group code except `249`, `523` and
/// `666`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] — the token is not a member of the
/// `audit_change_type` group (`AUDIT_DETAILS.Change_type_valid`);
/// [`ServiceError::BadRequest`] — a group code that does not match the
/// operation (a change-control mismatch, not content validation — the same
/// split the CONTRIBUTION path draws).
fn merged_change_type(supplied: &str, operation: &str) -> Result<String, ServiceError> {
    let token = supplied.trim();
    if token.is_empty() || token == operation {
        return Ok(operation.to_owned());
    }
    let code = change_type_code(token).ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "{token:?} is not a code in the openEHR audit_change_type group"
            ))
            .with_path("change_type")
            .with_invariant("AUDIT_DETAILS.Change_type_valid"),
        )
    })?;
    let compatible = match operation {
        change_type::CREATION => code == change_type::CREATION,
        change_type::DELETED => code == change_type::DELETED,
        // A direct update: a content-carrying new version of an existing
        // object — the full modification family, never
        // creation/deleted/attestation.
        _ => matches!(
            code.as_str(),
            change_type::AMENDMENT
                | change_type::MODIFICATION
                | change_type::SYNTHESIS
                | change_type::UNKNOWN
                | change_type::RESTORATION
                | change_type::FORMAT_CONVERSION
        ),
    };
    if compatible {
        Ok(code)
    } else {
        Err(ServiceError::precondition(format!(
            "change_type {code} does not match the operation (its change type is \
             {operation}) — the modification type must match the operation \
             (RM change_control §Contributions; ITS-REST overview \
             §\"openehr-version and openehr-audit-details\")"
        )))
    }
}

/// The rubric (English display text) for an `audit_change_type` code; falls
/// back to the code itself if the code is unknown to the bundle. The single
/// code→rubric mapping every presentation surface uses (the by-uid
/// CONTRIBUTION rendering and the contribution-list extension) — consumers
/// never hardcode a rubric table (RM common master04 §Audit Details; the
/// `audit_change_type` group, TERM 3.1.0).
pub(crate) fn change_type_rubric(code: &str) -> String {
    openehr()
        .rubric(AUDIT_CHANGE_TYPE, code, "en")
        .unwrap_or(code)
        .to_owned()
}

/// What an audit row records about a committed change — the `AUDIT_DETAILS`
/// attributes the service owns at write time (RM common master04 §Audit
/// Details), plus the `ATTESTATION`-declared attributes when the commit audit
/// is of that subtype (master06 §Attestation: "`ORIGINAL_VERSION._commit_audit_`
/// is of type `ATTESTATION` rather than `AUDIT_DETAILS`").
///
/// This is the ONE commit-audit carrier: the write paths build it, storage
/// persists it ([`Self::row`]), and the read paths rebuild the served
/// `AUDIT_DETAILS` from it ([`Self::typed`] / [`Self::canonical`]) — so what a
/// client commits is what a reader gets back.
#[derive(Debug, Clone)]
pub(crate) struct AuditInput {
    /// `AUDIT_DETAILS.system_id` (1..1, non-empty — `System_id_valid`).
    pub(crate) system_id: String,
    /// The numeric `audit_change_type` group code (`249`/`251`/`523`/…) — never
    /// a rubric string (`AUDIT_DETAILS.Change_type_valid`).
    pub(crate) change_type: String,
    /// `AUDIT_DETAILS.description` (0..1) as its RM value — whole, because the
    /// attribute's `DV_CODED_TEXT` subtype carries a `defining_code` a bare
    /// string would discard (RM common
    /// `UML/classes/org.openehr.rm.common.audit_details.adoc` §Attributes).
    pub(crate) description: Option<DvText>,
    /// The committer (`AUDIT_DETAILS.committer`, 1..1).
    pub(crate) committer: PartyProxy,
    /// The `ATTESTATION`-declared attributes
    /// ([`crate::versioning::attestation::AttestationParts`]) when this commit
    /// audit is an `ATTESTATION`, else `None`. `ATTESTATION` is the only
    /// `AUDIT_DETAILS` subtype RM 1.2.0 declares, so presence IS the concrete
    /// class.
    ///
    /// Boxed because it is the rare case and by far the widest: it carries a
    /// `DV_MULTIMEDIA` (`attested_view`), which would otherwise put hundreds of
    /// bytes of attestation payload on the stack of every commit — including
    /// the overwhelming majority that are plain `AUDIT_DETAILS`.
    pub(crate) attestation: Option<Box<AttestationParts>>,
}

impl AuditInput {
    /// Build the commit audit from the caller's `UPDATE_VERSION.audit`
    /// envelope, merged with the server rules (ITS-REST overview
    /// §"openehr-version and openehr-audit-details": `change_type` is the
    /// FIRST attribute the clients "MAY supply values for", and "whatever is
    /// provided it MUST be merged"; RM common master06 §Committal and Audits
    /// for the audit attributes):
    ///
    /// - `change_type` — the caller's when supplied, honoured verbatim after
    ///   [`merged_change_type`] validates it against the `audit_change_type`
    ///   group and the operation; absent or empty takes the operation's
    ///   default;
    /// - `description` — the caller's when supplied, else the server default;
    /// - `committer` — the caller's `PARTY_PROXY` (defaulted by the adapter to
    ///   the principal / system identity), or the person's own `PARTY_SELF`
    ///   ([`crate::service::committer::attributed`]);
    /// - `system_id` — the caller's when supplied, else this server's.
    ///
    /// The audit is a plain `UPDATE_AUDIT` ([`plain_audit`]), as on the
    /// CONTRIBUTION path: an attestation travels in
    /// `UPDATE_VERSION.attestations`.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] for a `change_type` outside the
    /// `audit_change_type` group (`AUDIT_DETAILS.Change_type_valid`) or a
    /// stated empty `system_id` ([`validate_system_id`]);
    /// [`ServiceError::BadRequest`] for a group code that contradicts the
    /// operation (mirroring the CONTRIBUTION path's change-control mismatch,
    /// `versioning/contribution.rs`) or an `UPDATE_ATTESTATION` audit.
    pub(crate) fn from_update(
        update: &UpdateAudit,
        operation_change_type: &str,
        default_description: &str,
        fallback_system_id: &str,
    ) -> Result<Self, ServiceError> {
        let base = plain_audit(update, "commit_audit")?;
        let change_type = merged_change_type(
            &base.change_type.defining_code.code_string,
            operation_change_type,
        )?;
        // `UpdateAudit.yaml`: a `system_id` "When provided it will be validated;
        // when omitted the server sets its own value".
        let system_id = base
            .system_id
            .clone()
            .unwrap_or_else(|| fallback_system_id.to_owned());
        validate_system_id(&system_id)?;
        Ok(Self {
            system_id,
            change_type,
            // The wire types `description` as `DV_TEXT`
            // (`schemas/common/UpdateAudit.yaml`), whose `DV_CODED_TEXT` subtype
            // substitutes for it, so it is kept WHOLE: reducing it to its
            // `value` would permanently drop a coded description's
            // `defining_code`.
            description: Some(
                base.description
                    .as_ref()
                    .filter(|d| !crate::service::version_update::text_value(d).is_empty())
                    .cloned()
                    .unwrap_or_else(|| dv_text(default_description)),
            ),
            committer: crate::service::committer::attributed(base.committer.clone()),
            attestation: None,
        })
    }

    /// Build a CONTRIBUTION commit audit (the CONTRIBUTION's own or a member's
    /// `commit_audit`) from its generated `UPDATE_AUDIT`, with the client's
    /// already-resolved numeric `audit_change_type` code.
    ///
    /// The `committer` and `description` are the ones the audit states, kept
    /// whole; an omitted `system_id` is `default_system_id` (this server's for
    /// the CONTRIBUTION audit, the CONTRIBUTION audit's for a member: RM common
    /// master06 §Committal and Audits). A stated one is kept as stated, so an
    /// empty one reaches [`validate_commit_audit`]'s `System_id_valid`.
    ///
    /// ITS-REST `schemas/common/UpdateAudit.yaml` leaves `additionalProperties`
    /// open (`schemas/common/ItemTag.yaml` closes its object with `false`), so a
    /// member it does not declare (`reason` or `is_pending` on a plain audit) is
    /// valid content that is not part of the audit.
    pub(crate) fn from_commit_audit(
        update: &UpdateAuditData,
        change_type: String,
        default_system_id: &str,
    ) -> Self {
        Self {
            system_id: update
                .system_id
                .clone()
                .unwrap_or_else(|| default_system_id.to_owned()),
            change_type,
            description: update.description.clone(),
            committer: crate::service::committer::attributed(update.committer.clone()),
            attestation: None,
        }
    }

    /// The commit audit of a stored version-metadata row: the three jsonb
    /// columns decoded back into the RM values they hold.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] when a stored fragment is not the RM
    /// value its column holds — the [`party_proxy`] / [`decode_description`]
    /// rejections, or the
    /// [`crate::versioning::attestation::AttestationParts::decode`] ones.
    pub(crate) fn from_meta(
        meta: &crate::storage::version_repo::meta::VersionMeta,
    ) -> Result<Self, ServiceError> {
        Ok(Self {
            system_id: meta.audit_system_id.clone(),
            change_type: meta.audit_change_type.clone(),
            description: meta
                .audit_description
                .as_ref()
                .map(decode_description)
                .transpose()?,
            committer: party_proxy(&meta.audit_committer)?,
            attestation: meta
                .audit_attestation
                .as_ref()
                .map(AttestationParts::decode)
                .transpose()?
                .map(Box::new),
        })
    }

    /// The storage row shape ([`crate::storage::version_repo::commit::AuditRow`])
    /// this audit persists as — the three jsonb columns encoded ONCE here, at
    /// the versioning→storage boundary, because storage is plumbing that takes
    /// plain values (`crate::storage::version_repo` module docs) and the
    /// `ATTESTATION`-declared subset has no RM class of its own to hand it.
    pub(crate) fn row(&self) -> crate::storage::version_repo::commit::AuditRow<'_> {
        crate::storage::version_repo::commit::AuditRow {
            system_id: &self.system_id,
            change_type: &self.change_type,
            description: self
                .description
                .as_ref()
                .map(openehr_its::json::to_canonical_value),
            committer: openehr_its::json::to_canonical_value(&self.committer),
            attestation: self.attestation.as_ref().map(|parts| parts.fragment()),
        }
    }

    /// This commit audit as its typed RM value at the recorded commit instant:
    /// an [`AuditDetails::Attestation`] when the audit carries the
    /// `ATTESTATION`-declared attributes (RM common master06 §Attestation),
    /// else a plain [`AuditDetails::AuditDetails`]. `change_type` is the stored
    /// numeric `audit_change_type` group code; the `DV_CODED_TEXT` carries it as
    /// `defining_code.code_string` (`AUDIT_DETAILS.Change_type_valid`) with the
    /// group rubric — resolved from the `openehr-term` bundle — as its `value`.
    ///
    /// Infallible: every attribute is already the RM value it will be served
    /// as, so building the `AUDIT_DETAILS` is a move, never a decode.
    pub(crate) fn typed(&self, time_committed: &jiff::Timestamp) -> AuditDetails {
        let system_id = self.system_id.clone();
        let time_committed = dv_date_time(time_committed);
        let change_type =
            openehr_coded_text(&self.change_type, change_type_rubric(&self.change_type));
        let description = self.description.clone();
        let committer = self.committer.clone();
        match &self.attestation {
            None => AuditDetails::AuditDetails(AuditDetailsData {
                system_id,
                time_committed,
                change_type,
                description,
                committer,
            }),
            Some(parts) => AuditDetails::Attestation(Attestation {
                system_id,
                time_committed,
                change_type,
                description,
                committer,
                attested_view: parts.attested_view.clone(),
                proof: parts.proof.clone(),
                items: openehr_base::containers::present_nonempty(parts.items.clone()),
                reason: parts.reason.clone(),
                is_pending: parts.is_pending,
            }),
        }
    }

    /// The canonical-JSON form of [`Self::typed`], serialized through the
    /// native codec so the wire body carries `_type` first — `ATTESTATION` when
    /// the version was committed with one — and the BMM's own attribute order.
    pub(crate) fn canonical(&self, time_committed: &jiff::Timestamp) -> Value {
        openehr_its::json::to_canonical_value(&self.typed(time_committed))
    }
}

/// Decode a stored/wire `AUDIT_DETAILS.description` fragment into its RM type,
/// so a `DV_CODED_TEXT` description returns as one.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the value is not a canonical `DV_TEXT`
/// (RM common `UML/classes/org.openehr.rm.common.audit_details.adoc`
/// §Attributes).
pub(crate) fn decode_description(fragment: &Value) -> Result<DvText, ServiceError> {
    openehr_its::json::from_canonical_value::<DvText>(fragment).map_err(|e| {
        ServiceError::content_invalid(
            Violation::new("is not a canonical DV_TEXT")
                .with_path("AUDIT_DETAILS.description")
                .with_decode_failure(&e),
        )
    })
}

/// An openEHR-terminology `DV_CODED_TEXT`: the group `code` as
/// `defining_code.code_string` under the `openehr` terminology, with `rubric`
/// as its displayable `value`. The one shared constructor for the coded texts
/// this module and its callers mint (`AUDIT_DETAILS.change_type`,
/// `ORIGINAL_VERSION.lifecycle_state`), so no site rebuilds the `CODE_PHRASE`
/// by hand.
pub(crate) fn openehr_coded_text(code: &str, rubric: String) -> DvCodedText {
    DvCodedText {
        value: rubric,
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: OPENEHR.to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        },
    }
}

/// A `DV_DATE_TIME` carrying an instant's ISO 8601 form — the shape
/// `AUDIT_DETAILS.time_committed` (1..1) takes.
pub(crate) fn dv_date_time(instant: &jiff::Timestamp) -> DvDateTime {
    DvDateTime {
        normal_status: None,
        normal_range: None,
        other_reference_ranges: None,
        magnitude_status: None,
        accuracy: None,
        value: instant.to_string(),
    }
}

/// A plain `DV_TEXT` — the shape `AUDIT_DETAILS.description` (0..1) takes when
/// the caller supplied only a string.
pub(crate) fn dv_text(value: &str) -> DvText {
    DvText::DvText(DvTextData {
        value: value.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
    })
}

/// Decode a stored/assembled canonical `PARTY_PROXY` value into its RM type.
///
/// The committer travels between the service layer and storage as canonical
/// JSON (an `audit.committer` jsonb column), so the typed builders below must
/// read it back to place it in an `AUDIT_DETAILS`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the value is not a canonical
/// `PARTY_PROXY` (`AUDIT_DETAILS.committer` is 1..1 — RM common
/// `UML/classes/org.openehr.rm.common.audit_details.adoc` §Attributes).
pub(crate) fn party_proxy(committer: &Value) -> Result<PartyProxy, ServiceError> {
    openehr_its::json::from_canonical_value::<PartyProxy>(committer).map_err(|e| {
        ServiceError::content_invalid(
            Violation::new("is not a canonical PARTY_PROXY")
                .with_path("AUDIT_DETAILS.committer")
                .with_decode_failure(&e),
        )
    })
}

/// The plain `UPDATE_AUDIT` of a commit audit, refusing the attestation
/// subtype.
///
/// ITS-REST `schemas/common/UpdateAudit.yaml` names what a server accepts for
/// an audit: "Clients SHOULD send `_type: \"UPDATE_AUDIT\"`. For
/// interoperability with existing implementations, servers SHOULD additionally
/// accept `_type: \"AUDIT_DETAILS\"` or an omitted `_type` for this attribute."
/// An attestation travels in its own member, `UpdateVersion.attestations[]`
/// (`schemas/ehr/UpdateVersion.yaml`, items `schemas/common/UpdateAttestation.yaml`),
/// as RM common master06 §Contributions states it: "a new `ATTESTATION` is
/// added to the attestations list of an existing `ORIGINAL_VERSION`".
///
/// # Errors
/// [`ServiceError::BadRequest`] when the audit is an `UPDATE_ATTESTATION`.
pub(crate) fn plain_audit<'a>(
    audit: &'a UpdateAudit,
    owner: &str,
) -> Result<&'a UpdateAuditData, ServiceError> {
    match audit {
        UpdateAudit::UpdateAudit(data) => Ok(data),
        UpdateAudit::UpdateAttestation(_) => Err(ServiceError::precondition(format!(
            "{owner} is an UPDATE_ATTESTATION: an audit is sent with _type UPDATE_AUDIT, \
             AUDIT_DETAILS or no _type (ITS-REST schemas/common/UpdateAudit.yaml), and an \
             attestation travels in versions[].attestations as UPDATE_ATTESTATION \
             (schemas/ehr/UpdateVersion.yaml)"
        ))),
    }
}

/// Refuses an empty `AUDIT_DETAILS.system_id`.
///
/// RM `UML/classes/org.openehr.rm.common.audit_details.adoc` §Invariants:
/// `System_id_valid: not system_id.is_empty`. A stated one is validated, never
/// replaced (ITS-REST `schemas/common/UpdateAudit.yaml`: "When provided it will
/// be validated; when omitted the server sets its own value").
///
/// NOTE: an empty string is schema-valid content that breaks an RM invariant,
/// the `422` row of ITS-REST `docs/overview/Requests_and_responses.md`
/// §HTTP status codes; a non-string `system_id` fails the schema and is `400`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming `AUDIT_DETAILS.System_id_valid`.
pub(crate) fn validate_system_id(system_id: &str) -> Result<(), ServiceError> {
    if system_id.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("is mandatory and non-void")
                .with_path("AUDIT_DETAILS.system_id")
                .with_invariant("AUDIT_DETAILS.System_id_valid"),
        ));
    }
    Ok(())
}

/// Validate a client-supplied commit `AUDIT_DETAILS`' non-terminology RM
/// invariants before it is persisted (a CONTRIBUTION audit or a version
/// `commit_audit`).
///
/// `change_type` is validated separately ([`change_type_code`]).
///
/// # Errors
/// [`ServiceError::Unprocessable`] when either enforced invariant fails:
///
/// - `AUDIT_DETAILS.System_id_valid`: `not system_id.is_empty` (RM
///   `UML/classes/org.openehr.rm.common.audit_details.adoc` §Invariants).
///   Unguarded, an empty client-supplied `system_id` reaches the DB
///   `System_id_valid` CHECK and surfaces as a `500` where a validation failure
///   belongs at `422`.
/// - the committer `PARTY_PROXY`'s own `PARTY_IDENTIFIED`/`PARTY_RELATED`
///   invariants `Basic_validity` and `Name_valid`, plus `Relationship_valid` for
///   `PARTY_RELATED` (the §Invariants sections of
///   `org.openehr.rm.common.party_identified.adoc` and `…party_related.adoc`).
///   A PARTY appearing as content is validated by the RM-invariant pass, but the
///   audit committer is stored verbatim, so it is checked here.
pub(crate) fn validate_commit_audit(audit: &AuditInput) -> Result<(), ServiceError> {
    validate_system_id(&audit.system_id)?;
    validate_committer(&audit.committer)
}

/// Run the committer `PARTY_PROXY`'s OWN RM class invariants + terminology
/// bindings — `PARTY_IDENTIFIED.Basic_validity` / `.Name_valid`
/// (`RM/docs/UML/classes/org.openehr.rm.common.party_identified.adoc`
/// §Invariants), `PARTY_RELATED.Relationship_valid`
/// (`…org.openehr.rm.common.party_related.adoc` §Invariants), and the
/// structural conformance of the concrete `PARTY_PROXY` subtype.
///
/// The rules are not restated here: the value goes through the unified
/// dispatcher [`openehr_its::wire_validate::validate_rm_value`], the same one
/// the whole-instance commit pass runs on every node of a committed RM document,
/// which runs the generated structural check, the typed invariant cores, the
/// model-driven mandatory-container bounds and the terminology-backed invariants
/// (all defined in `openehr_rm::v1_2::validate`).
///
/// It is invoked explicitly because a commit audit is not part of any committed
/// RM document: the `AUDIT_DETAILS` is written to its own row and never walked
/// by the content pass, so without this call the committer would be the one
/// `PARTY_PROXY` reaching storage unvalidated.
///
/// NOTE: the dispatcher is canonical-JSON-valued, so a typed committer is
/// serialized here to be judged; running only
/// [`openehr_base::validate::Validate`] on the typed value would drop the
/// terminology tier that decides `PARTY_RELATED.Relationship_valid`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] carrying the dispatcher's violations.
pub(crate) fn validate_committer(committer: &PartyProxy) -> Result<(), ServiceError> {
    let mut violations = Vec::new();
    openehr_its::wire_validate::validate_rm_value(
        &openehr_its::json::to_canonical_value(committer),
        &mut violations,
    );
    if violations.is_empty() {
        return Ok(());
    }
    // The dispatcher's own `InvariantViolation`s travel on as DATA — the
    // service never flattens them into a sentence here.
    Err(ServiceError::content_invalid(
        Violation::new("is not a valid PARTY_PROXY")
            .with_path("AUDIT_DETAILS.committer")
            .with_causes(violations),
    ))
}

#[cfg(test)]
mod tests;
