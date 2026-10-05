// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST ehr API (Release-1.1.0, STABLE).
//!
//! Governing spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` and
//! the `ehr-*.openapi.yaml` OAS group generated into `openehr_its::rest`.
//!
//! [`dispatch`] routes an operation id to one of seven resource modules, one per
//! spec resource boundary: [`ehr_resource`], [`ehr_status`],
//! [`versioned_ehr_status`], [`composition`], [`versioned_composition`],
//! [`directory`] and [`contribution`].
//!
//! Each arm rebuilds the operation's `*Params`, decodes wire strings into the SM
//! catalog's native argument types through [`crate::overview::version_id`],
//! decodes any body (RM-typed bodies accept JSON or canonical XML), calls the
//! SM catalog methods on the platform service `S`, and rebuilds a
//! [`ServiceResponse`] from the result, which the `negotiate::*` helpers render
//! with the spec's `ETag`/`Location`/`Prefer` behaviour. The shared write, read
//! and committal helpers below back all seven modules; the `ITEM_TAG`
//! write-wrapper headers they accept are handled by `crate::api::item_tags`,
//! the one implementation this group shares with the demographic group.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

pub mod composition;
pub mod contribution;
pub mod directory;
pub mod dispatch;
pub mod ehr_resource;
pub mod ehr_status;
pub mod openapi_routes;
pub mod versioned_composition;
pub mod versioned_ehr_status;

// COMPOSITION + CONTRIBUTION negotiate the Simplified-Formats (FLAT/STRUCTURED)
// representations through the shared `crate::formats::dispatch` adapter, called
// by its full path (no module alias — every import names its defining module).

use http::HeaderMap;
use serde_json::Value;
use uuid::Uuid;

use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::{DvIdentifier, PartyIdentified, PartyIdentifiedData, PartyProxy};

use ferroehr::service::response::{ResourceMeta, ServiceResponse};
use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded, plain_text};

use crate::extensions::access::authn::AuthMethod;
use crate::overview::error::RestError;
use crate::overview::params::query_param;
use crate::state::AppState;

/// openEHR *audit change type* codes (the `openehr` terminology group).
pub(super) const CHANGE_CREATION: &str = "249";
pub(super) const CHANGE_MODIFICATION: &str = "251";
/// `532|complete|` — the lifecycle state stamped on synthesized version updates.
pub(super) const LIFECYCLE_COMPLETE: &str = "532";

/// The `uid`/`value` of a returned RM object → the resource metadata a read
/// with an `ETag`/`Last-Modified` needs (the version id is the object's own
/// `uid`).
///
/// When the served body is a VERSION envelope, its `commit_audit.time_committed`
/// is also read as the `Last-Modified` instant: "For openEHR resources, this
/// value should be derived from `VERSION.commit_audit.time_committed.value`"
/// (`Requests_and_responses.md` §"`ETag` and Last-Modified").
///
/// A bare RM body carries no commit audit, so those routes take their metadata
/// from the service layer, which reads the commit instant off the version row.
fn resource_meta_from(ehr_id: &str, body: &Value) -> Option<ResourceMeta> {
    let uid = body
        .pointer("/uid/value")
        .or_else(|| body.pointer("/item/uid/value"))
        .and_then(Value::as_str)?;
    let meta = ResourceMeta::new(ehr_id.to_owned(), uid.to_owned());
    Some(match commit_instant(body) {
        Some(at) => meta.with_last_modified(at),
        None => meta,
    })
}

/// The commit instant of a served VERSION envelope —
/// `VERSION.commit_audit.time_committed.value` (RM common master04
/// §Audit Details; the `Last-Modified` source named by ITS-REST overview
/// §"`ETag` and Last-Modified"). `None` for a body that is not a VERSION.
fn commit_instant(body: &Value) -> Option<jiff::Timestamp> {
    body.get("commit_audit")
        .and_then(|a| a.get("time_committed"))
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str)?
        .parse::<jiff::Timestamp>()
        .ok()
}

/// The canonical-XML root tag for a served VERSION envelope: the document
/// element the published ITS-XML schemas declare for the resource.
///
/// `Resources.md` §"XML Format" requires responses to "conform to the
/// [published XSDs]", and the schemas publish exactly one document element for a
/// VERSION, spelled identically in both lineages. `VERSION` is abstract there,
/// so the concrete subtype is named by `xsi:type` on that same root, which is
/// how an `IMPORTED_VERSION` stays distinguishable from an `ORIGINAL_VERSION`
/// (RM common master06 §Version and its Subtypes).
pub(super) const VERSION_ROOT_TAG: &str = "version";

/// Re-inlines externalized `DV_MULTIMEDIA` content when the caller asked for it
/// with `?expand_multimedia=true`, verifying each blob's integrity.
///
/// Every read that can return externalized content routes through here, because
/// externalization is applied on the way in by the generic versioning path, so a
/// read that could not restore it would leave clinical content in the object
/// store with no API that returns it.
///
/// NOTE: no openEHR spec governs this — our own design/extension, so the
/// parameter is read off the raw query string rather than a generated params
/// struct.
///
/// # Errors
/// Propagates the service's failure when a referenced blob cannot be fetched or
/// fails its integrity check — never a silent fall back to the stored form.
pub(super) async fn expand_multimedia_if_requested(
    state: &AppState,
    query: Option<&str>,
    body: Value,
) -> Result<Value, RestError> {
    if query_param(query, "expand_multimedia").as_deref() == Some("true") {
        return Ok(state.backend().expand_multimedia(body).await?);
    }
    Ok(body)
}

/// Wrap a read body as a [`ServiceResponse`], attaching resource metadata drawn
/// from the body's own `uid` (and, for a VERSION envelope, its commit instant)
/// when present.
pub(super) fn read_resp(ehr_id: &str, body: Value) -> ServiceResponse {
    match resource_meta_from(ehr_id, &body) {
        Some(m) => ServiceResponse::new(body, m),
        None => ServiceResponse::plain(body),
    }
}

/// The committer `PARTY_PROXY` for a write, from the authenticated principal
/// published by the auth middleware. With no authenticated principal the write
/// is attributed to this CDR's own system identity
/// ([`ferroehr::service::SYSTEM_COMMITTER_NAME`]) — the same constant the
/// platform library uses, so both layers name the system committer identically.
/// The SM service impl re-derives the committer from the same principal, so
/// this rides in the [`UpdateVersion`] envelope for completeness.
pub(crate) fn committer_proxy() -> PartyProxy {
    let party = match crate::extensions::access::authn::current_principal() {
        Some(principal) => {
            let id_type = match principal.method {
                AuthMethod::Basic => "basic",
                AuthMethod::Bearer => "oauth2",
            };
            PartyIdentifiedData {
                external_ref: None,
                name: Some(principal.subject.clone()),
                identifiers: Some(openehr_base::containers::NonEmptyVec::of(DvIdentifier {
                    issuer: Some("ferroehr".to_owned()),
                    assigner: None,
                    id: principal.subject,
                    r#type: Some(id_type.to_owned()),
                })),
            }
        }
        None => PartyIdentifiedData {
            external_ref: None,
            name: Some(ferroehr::service::SYSTEM_COMMITTER_NAME.to_owned()),
            identifiers: None,
        },
    };
    PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party))
}

/// Synthesizes the SM `UPDATE_VERSION` commit envelope for a bare-RM-body write
/// route: the RM object is the `data`, the `If-Match` is the
/// `preceding_version_uid`, and the audit carries the change type and committer.
///
/// The server defaults are then merged with any committal request headers the
/// client supplied, the ITS-REST MUST of overview §"openehr-version and
/// openehr-audit-details" (`crate::overview::committal`).
///
/// # Errors
/// [`ApiError::BadRequest`] when a committal header carries a malformed
/// identifier (`crate::overview::committal::build_committer`).
pub(super) fn mk_update_version<T>(
    headers: &HeaderMap,
    data: T,
    change_code: &str,
    description: &str,
    preceding: Option<ObjectVersionId>,
) -> Result<UpdateVersion<T>, ApiError> {
    let mut uv = UpdateVersion {
        preceding_version_uid: preceding,
        signature: None,
        lifecycle_state: lifecycle_state_coded(LIFECYCLE_COMPLETE),
        attestations: None,
        data: Some(data),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: Some(plain_text(description)),
            committer: committer_proxy(),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        additional_properties: std::collections::BTreeMap::new(),
    };
    crate::overview::committal::merge_committal_headers(&mut uv, headers)?;
    Ok(uv)
}

/// Decomposes an [`ObjectVersionId`] into the `(versioned-object uuid,
/// version_tree_id)` pair the SM `*_at_version` reads take.
///
/// Branch version ids are first-class (RM common master06 §The 'Virtual Version
/// Tree').
/// Verify a version-addressed read served the VERSION the path named: the
/// addressed `version_uid` must equal the served body's `uid.value` — the
/// stored full `object_id :: creating_system_id :: version_tree_id` identity
/// (ITS-REST overview `Resources.md` §Identifier types) — compared
/// case-insensitively (BASE master05 §"Composite Identifiers and Case").
/// A tree-only fetch would satisfy a fabricated `creating_system_id`; that
/// names no VERSION in this repository → 404. A body without a served uid
/// (nothing to verify against) passes.
pub(super) fn ensure_served_version(addressed: &str, body: &Value) -> Result<(), RestError> {
    match body
        .get("uid")
        .and_then(|u| u.get("value"))
        .and_then(Value::as_str)
    {
        Some(served) if !served.eq_ignore_ascii_case(addressed) => Err(RestError(
            ApiError::NotFound(format!("version {addressed}")),
        )),
        _ => Ok(()),
    }
}

pub(super) fn version_components(ovid: &ObjectVersionId) -> Result<(Uuid, String), ApiError> {
    let vo = crate::overview::version_id::object_id_uuid(ovid).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "OBJECT_VERSION_ID object_id is not a UUID: {}",
            ovid.value()
        ))
    })?;
    Ok((vo, ovid.version_tree_id().value().to_owned()))
}
