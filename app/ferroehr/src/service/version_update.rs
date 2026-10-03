// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version-commit envelope (`master03-common_package.adoc` §Version Update
//! Semantics; `update_version.adoc`, `update_audit.adoc`).
//!
//! A call that creates or updates a versioned top-level object implicitly
//! creates a `CONTRIBUTION`, one or more `ORIGINAL_VERSION`s and, on creation,
//! a `VERSIONED_OBJECT`; `UPDATE_VERSION<T>` is the structure through which the
//! caller supplies the meta-data the service does not generate.
//!
//! The envelope types are the generated ITS-REST contract,
//! [`openehr_its::rest::generated::common`]'s `UpdateVersion<T>`,
//! `UpdateAudit`, `UpdateAuditData` and `UpdateAttestation`, consumed directly.
//! This module carries only what the contract does not: the header-merge
//! carrier [`Committal`], the server-side envelope constructor
//! [`direct_envelope`], and the base-attribute accessors every commit path
//! reads through.
//!
//! Four wire facts this seam depends on:
//!
//! - `UPDATE_AUDIT` is polymorphic. `UpdateAudit.yaml` carries both an
//!   `x-discriminator-value` of its own and a `discriminator.mapping` to
//!   `UPDATE_ATTESTATION`, the `AUDIT_DETAILS`/`ATTESTATION` pair of RM common
//!   `master06-change_control_package.adoc` §Committal and Audits, so
//!   [`audit_base`] reads the inherited attributes of whichever form arrived.
//! - `description` is a `DV_TEXT` object on the wire (`UpdateAudit.yaml`) where
//!   SM `UPDATE_AUDIT.description` is `String [0..1]`, so a body-borne commit
//!   audit can carry a coded description. The `openehr-audit-details` header
//!   cannot: its grammar carries the attribute pre-flattened as the
//!   `description.value` subkey (ITS-REST overview `Requests_and_responses.md`),
//!   so the header merge always produces a plain `DV_TEXT`.
//! - `lifecycle_state` and `change_type` are `DV_CODED_TEXT` (`UpdateVersion.yaml`,
//!   `UpdateAudit.yaml`), the ITS-REST docs text being silent, so the numeric
//!   group code lives at `defining_code.code_string`.
//! - `UPDATE_ATTESTATION` is a wire partial the server completes into a full RM
//!   `ATTESTATION` (adding `time_committed` and `system_id`), and `signature` is
//!   wire-only: `UpdateVersion.yaml` carries it, the SM class does not, and it
//!   is fed to the version signer.

use openehr_base::prelude::TerminologyId;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_rm::prelude::{DvCodedText, DvText, PartyProxy};
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_text::DvTextData;

use crate::versioning::audit::{OPENEHR, change_type};
use crate::versioning::lifecycle;

/// A `DV_CODED_TEXT` over an openEHR-terminology code, carrying `rubric` as
/// its `value` — "the rubric of the complete term as provided by the
/// terminology service" (RM `data_types`
/// `UML/classes/org.openehr.rm.data_types.dv_coded_text.adoc`
/// §Attributes).
fn openehr_coded(rubric: String, code: &str) -> DvCodedText {
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

/// The `UPDATE_VERSION.lifecycle_state` value for a `version_lifecycle_state`
/// group code, with the bundle's rubric (RM common master06 §Version
/// Lifecycle).
///
/// The one place this server builds the member, so no write path spells a
/// rubric as a literal.
#[must_use]
pub fn lifecycle_state_coded(code: &str) -> DvCodedText {
    openehr_coded(lifecycle::lifecycle_rubric(code), code)
}

/// The `UPDATE_AUDIT.change_type` value for an `audit_change_type` group code,
/// with the bundle's rubric (RM common master04 §Audit Details —
/// `AUDIT_DETAILS.Change_type_valid`).
#[must_use]
pub fn change_type_coded(code: &str) -> DvCodedText {
    openehr_coded(crate::versioning::audit::change_type_rubric(code), code)
}

/// The "the client stated no code" placeholder a **header-derived** envelope
/// starts from, so that only a header-carried code reads as supplied.
///
/// The committal headers are optional in every attribute ("None of these
/// headers are mandatory, but whatever is provided it MUST be merged" —
/// ITS-REST overview `Requests_and_responses.md` §"openehr-version and
/// openehr-audit-details"), and the service resolves an empty code to the
/// operation's own default (`versioning::audit`, `versioning::lifecycle`). It
/// therefore never reaches storage: an unresolved placeholder is replaced
/// before the commit.
#[must_use]
pub fn unstated_code() -> DvCodedText {
    openehr_coded(String::new(), "")
}

/// The `UPDATE_AUDIT`-declared attributes of a commit audit, whichever
/// concrete form the wire carried.
///
/// The OAS models `UPDATE_ATTESTATION` as an `allOf` extension of
/// `UPDATE_AUDIT`, so both forms carry these four members; this borrow lets a
/// commit path read them without caring which arrived.
#[derive(Debug)]
pub struct AuditBase<'a> {
    /// `AUDIT_DETAILS.change_type` (1..1), coded from the openEHR *audit
    /// change type* group (`AUDIT_DETAILS.Change_type_valid` — enforced at the
    /// service boundary against `openehr-term`, not by the type).
    pub change_type: &'a DvCodedText,
    /// `AUDIT_DETAILS.description` (0..1).
    pub description: Option<&'a DvText>,
    /// `AUDIT_DETAILS.committer` (1..1).
    pub committer: &'a PartyProxy,
    /// `AUDIT_DETAILS.system_id`, when the client supplied one. ITS-REST
    /// overview §"openehr-version and openehr-audit-details": "clients MAY
    /// supply values for the `AUDIT_DETAILS` attributes … and `system_id`";
    /// when absent "the server MUST set it to its own configured system
    /// identifier". The server default is asserted at the versioning seam.
    pub system_id: Option<&'a str>,
}

/// The mutable twin of [`AuditBase`] — what the committal-header merge writes
/// through.
#[derive(Debug)]
pub struct AuditBaseMut<'a> {
    /// `AUDIT_DETAILS.change_type` (1..1).
    pub change_type: &'a mut DvCodedText,
    /// `AUDIT_DETAILS.description` (0..1).
    pub description: &'a mut Option<DvText>,
    /// `AUDIT_DETAILS.committer` (1..1).
    pub committer: &'a mut PartyProxy,
    /// `AUDIT_DETAILS.system_id` (0..1 on the wire).
    pub system_id: &'a mut Option<String>,
}

/// Borrow the inherited `UPDATE_AUDIT` attributes of a commit audit.
#[must_use]
pub fn audit_base(audit: &UpdateAudit) -> AuditBase<'_> {
    match audit {
        UpdateAudit::UpdateAudit(a) => AuditBase {
            change_type: &a.change_type,
            description: a.description.as_ref(),
            committer: &a.committer,
            system_id: a.system_id.as_deref(),
        },
        UpdateAudit::UpdateAttestation(a) => AuditBase {
            change_type: &a.change_type,
            description: a.description.as_ref(),
            committer: &a.committer,
            system_id: a.system_id.as_deref(),
        },
    }
}

/// Borrow the inherited `UPDATE_AUDIT` attributes of a commit audit mutably.
#[must_use]
pub fn audit_base_mut(audit: &mut UpdateAudit) -> AuditBaseMut<'_> {
    match audit {
        UpdateAudit::UpdateAudit(a) => AuditBaseMut {
            change_type: &mut a.change_type,
            description: &mut a.description,
            committer: &mut a.committer,
            system_id: &mut a.system_id,
        },
        UpdateAudit::UpdateAttestation(a) => AuditBaseMut {
            change_type: &mut a.change_type,
            description: &mut a.description,
            committer: &mut a.committer,
            system_id: &mut a.system_id,
        },
    }
}

/// Returns the displayable `value` of a `DV_TEXT`, in whichever concrete form.
///
/// `DV_CODED_TEXT` declares the same attribute as "the rubric of the complete
/// term" (RM `data_types`
/// `UML/classes/org.openehr.rm.data_types.dv_coded_text.adoc`).
#[must_use]
pub fn text_value(text: &DvText) -> &str {
    match text {
        DvText::DvText(t) => &t.value,
        DvText::DvCodedText(t) => &t.value,
    }
}

/// A plain (uncoded) `DV_TEXT` — the only `AUDIT_DETAILS.description` form a
/// server default or a committal header can produce.
#[must_use]
pub fn plain_text(value: &str) -> DvText {
    DvText::DvText(DvTextData {
        value: value.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
    })
}

/// The `PARTY_PROXY` this server attributes a commit to when nothing else
/// names a committer: its own system identity.
///
/// Built with the generated constructor rather than decoded from a JSON
/// literal: the value is a compile-time constant of the RM model, so there is
/// no fallible step to handle.
#[must_use]
pub fn system_committer() -> PartyProxy {
    PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(PartyIdentifiedData {
        external_ref: None,
        name: Some(crate::service::SYSTEM_COMMITTER_NAME.to_owned()),
        identifiers: None,
    }))
}

/// A plain server-side commit envelope around `data`: `532|complete|`
/// lifecycle, the server's own audit identity, no client signature, no
/// attestations.
///
/// The internal commit paths (FHIR ingest, TDD import) that synthesize
/// content use this; wire commits carry the caller's merged envelope instead.
/// It is a free function because `UPDATE_VERSION` is the generated ITS-REST
/// contract type — foreign to this crate, so it can carry no inherent
/// constructor of ours.
#[must_use]
pub fn direct_envelope<T>(data: T) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: None,
        signature: None,
        // 532|complete| (RM common master06 §Version Lifecycle).
        lifecycle_state: lifecycle_state_coded(lifecycle::state::COMPLETE),
        attestations: None,
        data: Some(data),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            // 249|creation| — a placeholder. A caller-supplied `change_type`
            // is honoured after group + operation validation
            // (`versioning::audit`, ITS-REST overview §"openehr-version and
            // openehr-audit-details" MUST-merge); header-derived envelopes
            // blank this first so only a header-carried value reads as
            // supplied (`overview::committal`).
            change_type: change_type_coded(change_type::CREATION),
            description: None,
            committer: system_committer(),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// The client-supplied committal metadata of a **direct** commit.
///
/// Such a commit's `UPDATE_VERSION` envelope never travels in the request body
/// — the bare `POST`/`PUT`/`DELETE` writes on a change-controlled resource,
/// where the only channel for it is the `openehr-version` /
/// `openehr-audit-details` request headers.
///
/// ITS-REST overview `Requests_and_responses.md` §"openehr-version and
/// openehr-audit-details": "services MUST accept `openehr-version` and
/// `openehr-audit-details` custom request headers", and "None of these headers
/// are mandatory, but whatever is provided it MUST be merged with the default
/// VERSION and `VERSION.audit_details` attributes on commit runtime." This type
/// carries the two halves of that merge — the `UPDATE_AUDIT` attributes and
/// the VERSION `lifecycle_state` — for the commit paths whose arguments are
/// the bare RM content (EHR creation) rather than a full envelope.
#[derive(Debug, Clone)]
pub struct Committal {
    /// The merged `UPDATE_AUDIT` attributes (`change_type`, `description`,
    /// `committer`, `system_id`), already seeded with the server defaults the
    /// client did not override.
    pub audit: UpdateAudit,
    /// `UPDATE_VERSION.lifecycle_state` as its numeric
    /// `version_lifecycle_state` code, when the client supplied one through
    /// `openehr-version`; `None` leaves the server default (`532|complete|`,
    /// RM common master06 §Version Lifecycle).
    pub lifecycle_state: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_its::rest::generated::ehr::Versionable;

    /// The generated envelope reads the ITS-REST wire shape: `commit_audit`,
    /// `DV_CODED_TEXT` codes, partial attestations, `signature` — and an
    /// `UPDATE_AUDIT` whose `_type` is absent still reads as the base form
    /// (the OAS leaves `_type` out of `UpdateAudit.required` and gives it a
    /// `default: UPDATE_AUDIT`).
    #[test]
    fn update_version_deserializes_the_wire_shape() {
        let v: UpdateVersion<Versionable> = openehr_its::json::from_canonical_value(
            &serde_json::json!({
                "preceding_version_uid": { "value": "8849182c-82ad-4088-a07f-48ead4180515::sys::1" },
                "lifecycle_state": {
                    "value": "complete",
                    "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "532" }
                },
                "data": { "_type": "COMPOSITION", "name": { "value": "enc" },
                          "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                          "language": { "terminology_id": { "value": "ISO_639-1" }, "code_string": "en" },
                          "territory": { "terminology_id": { "value": "ISO_3166-1" }, "code_string": "NL" },
                          "category": { "value": "event",
                                        "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "433" } },
                          "composer": { "_type": "PARTY_IDENTIFIED", "name": "A user" } },
                "signature": "sig-bytes",
                "commit_audit": {
                    "change_type": {
                        "value": "modification",
                        "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "251" }
                    },
                    "committer": { "_type": "PARTY_IDENTIFIED", "name": "A user" }
                }
            }),
        )
        .expect("wire-shaped UPDATE_VERSION deserializes");
        assert!(v.preceding_version_uid.is_some());
        assert_eq!(v.signature.as_deref(), Some("sig-bytes"));
        assert!(v.attestations.is_none());
        assert!(matches!(v.data, Some(Versionable::Composition(_))));
        let base = audit_base(&v.commit_audit);
        assert_eq!(base.change_type.defining_code.code_string, "251");
    }

    /// `_type: "UPDATE_ATTESTATION"` on `commit_audit` selects the
    /// attestation form, and its inherited `UPDATE_AUDIT` attributes read
    /// through the same accessor (the OAS `allOf` inheritance).
    #[test]
    fn commit_audit_accepts_the_attestation_subtype() {
        let audit: UpdateAudit = openehr_its::json::from_canonical_value(&serde_json::json!({
            "_type": "UPDATE_ATTESTATION",
            "change_type": {
                "value": "attestation",
                "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "666" }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Attester" },
            "reason": {
                "_type": "DV_CODED_TEXT",
                "value": "signed",
                "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "240" }
            },
            "is_pending": false
        }))
        .expect("an UPDATE_ATTESTATION commit audit deserializes");
        assert!(matches!(audit, UpdateAudit::UpdateAttestation(_)));
        assert_eq!(
            audit_base(&audit).change_type.defining_code.code_string,
            "666"
        );
    }

    /// The server-side envelope: `532|complete|`, `249|creation|`, the
    /// system committer, no signature, no attestations.
    #[test]
    fn direct_envelope_is_the_server_default_commit() {
        let uv = direct_envelope(serde_json::json!({ "_type": "COMPOSITION" }));
        assert_eq!(uv.lifecycle_state.defining_code.code_string, "532");
        assert!(uv.signature.is_none());
        assert!(uv.attestations.is_none());
        assert!(uv.preceding_version_uid.is_none());
        let base = audit_base(&uv.commit_audit);
        assert_eq!(base.change_type.defining_code.code_string, "249");
        assert!(base.system_id.is_none());
    }
}
