// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version-commit envelope (ITS-REST `UpdateVersion.yaml`,
//! `UpdateAudit.yaml`).
//!
//! A call that creates or updates a versioned top-level object implicitly
//! creates a `CONTRIBUTION`, one or more `ORIGINAL_VERSION`s and, on creation,
//! a `VERSIONED_OBJECT`; `UPDATE_VERSION<T>` is the structure through which the
//! caller supplies the meta-data the service does not generate.
//!
//! The envelope types are the generated ITS-REST contract,
//! [`openehr_its::rest::generated::common`]'s `UpdateVersion<T>`,
//! `UpdateAudit`, `UpdateAuditData` and `UpdateAttestation`, consumed directly.
//! This module carries only what the contract does not: the service-boundary
//! [`WriteRequest`] a direct write arrives as, and the base-attribute
//! accessors every commit path reads through.
//!
//! Four wire facts this seam depends on:
//!
//! - `UPDATE_AUDIT` is polymorphic. `UpdateAudit.yaml` carries both an
//!   `x-discriminator-value` of its own and a `discriminator.mapping` to
//!   `UPDATE_ATTESTATION`, the `AUDIT_DETAILS`/`ATTESTATION` pair of RM common
//!   `master06-change_control_package.adoc` §Committal and Audits, so
//!   [`audit_base`] reads the inherited attributes of whichever form arrived.
//! - `description` is a `DV_TEXT` object on the wire (`UpdateAudit.yaml`), so a
//!   body-borne commit audit can carry a coded description. The
//!   `openehr-audit-details` header
//!   cannot: its grammar carries the attribute pre-flattened as the
//!   `description.value` subkey (ITS-REST overview `Requests_and_responses.md`),
//!   so the header merge always produces a plain `DV_TEXT`.
//! - `lifecycle_state` and `change_type` are `DV_CODED_TEXT` (`UpdateVersion.yaml`,
//!   `UpdateAudit.yaml`), the ITS-REST docs text being silent, so the numeric
//!   group code lives at `defining_code.code_string`.
//! - `UPDATE_ATTESTATION` is a wire partial the server completes into a full RM
//!   `ATTESTATION` (adding `time_committed` and `system_id`), and `signature` is
//!   wire-only: `UpdateVersion.yaml` carries it, and it is fed to the version
//!   signer.

use openehr_base::prelude::{ObjectVersionId, TerminologyId};
use openehr_its::rest::generated::common::{UpdateAttestation, UpdateAudit};
use openehr_rm::prelude::{DvCodedText, DvText, PartyProxy};
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_text::DvTextData;

use crate::versioning::audit::OPENEHR;
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
        name: Some(crate::versioning::committer::SYSTEM_COMMITTER_NAME.to_owned()),
        identifiers: None,
    }))
}

/// The caller's side of one direct write: the content and the commit metadata
/// the service does not generate.
///
/// A direct write on a change-controlled resource implicitly creates a
/// `CONTRIBUTION` and an `ORIGINAL_VERSION` (`UpdateVersion.yaml`), and its
/// metadata reaches the server through the `openehr-version` and
/// `openehr-audit-details` request headers: "None of these headers are
/// mandatory, but whatever is provided it MUST be merged with the default
/// VERSION and `VERSION.audit_details` attributes on commit runtime" (ITS-REST
/// overview `Requests_and_responses.md` §"openehr-version and
/// openehr-audit-details"). Every member the caller leaves out is the server
/// default, so [`WriteRequest::new`] is the plain server commit. A delete
/// carries no content (RM common master06 §Logical Deletion), so its `data` is
/// `()`.
#[derive(Debug, Clone)]
pub struct WriteRequest<T = ()> {
    /// The RM content the write commits.
    pub data: T,
    /// The caller's `UPDATE_AUDIT` attributes, merged over the server
    /// defaults; `None` commits under the server's own audit.
    pub audit: Option<UpdateAudit>,
    /// `UPDATE_VERSION.lifecycle_state` as its numeric
    /// `version_lifecycle_state` code; `None` is `532|complete|` (RM common
    /// master06 §Version Lifecycle).
    pub lifecycle_state: Option<String>,
    /// A client-supplied `VERSION.signature`, stored verbatim (RM common
    /// master06 §Digital Signature).
    pub signature: Option<String>,
    /// `UPDATE_VERSION.attestations` committed with the version.
    pub attestations: Vec<UpdateAttestation>,
    /// The version the caller names as current (`If-Match`, or the
    /// `preceding_version_uid` of `UpdateVersion.yaml`), checked once against
    /// the tip the write locks.
    pub preceding: Option<ObjectVersionId>,
}

impl<T> WriteRequest<T> {
    /// A write of `data` with every commit attribute at the server default.
    #[must_use]
    pub fn new(data: T) -> Self {
        Self {
            data,
            audit: None,
            lifecycle_state: None,
            signature: None,
            attestations: Vec::new(),
            preceding: None,
        }
    }

    /// The same request carrying `f(data)`.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> WriteRequest<U> {
        WriteRequest {
            data: f(self.data),
            audit: self.audit,
            lifecycle_state: self.lifecycle_state,
            signature: self.signature,
            attestations: self.attestations,
            preceding: self.preceding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioning::audit::audit_base;
    use openehr_its::rest::generated::common::UpdateVersion;
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
}
