// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST ehr API (Release-1.1.0, STABLE).
//!
//! Governing spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` and
//! the `ehr-*.openapi.yaml` OAS group generated into `openehr_its::rest`.
//!
//! Each operation's handler lives in one of seven resource modules, one per
//! spec resource boundary: [`ehr_resource`], [`ehr_status`],
//! [`versioned_ehr_status`], [`composition`], [`versioned_composition`],
//! [`directory`] and [`contribution`].
//!
//! Each arm rebuilds the operation's `*Params`, decodes wire strings into the
//! service's native argument types through [`crate::overview::version_id`],
//! decodes any body (RM-typed bodies accept JSON or canonical XML), calls the
//! service methods on the platform service `S`, and rebuilds a
//! [`ServiceResponse`](ferroehr::service::response::ServiceResponse) from the
//! result, which the `negotiate::*` helpers render with the spec's
//! `ETag`/`Location`/`Prefer` behaviour. The shared write, read and committal
//! helpers below back all seven modules; the `ITEM_TAG`
//! write-wrapper headers they accept are handled by `crate::api::item_tags`,
//! the one implementation this group shares with the demographic group.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

pub mod access_delay;
pub mod access_log;
pub mod composition;
pub mod contribution;
pub mod directory;
pub mod ehr_resource;
pub mod ehr_status;
pub mod openapi_routes;
pub mod versioned_composition;
pub mod versioned_ehr_status;

// COMPOSITION + CONTRIBUTION negotiate the Simplified-Formats (FLAT/STRUCTURED)
// representations through the shared `crate::formats::dispatch` adapter, called
// by its full path (no module alias — every import names its defining module).

use serde_json::Value;
use uuid::Uuid;

use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::runtime::ApiError;

use crate::overview::error::RestError;
use crate::overview::params::query_param;
use crate::state::AppState;

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

// NOTE: no specification governs this: our own design. No ITS-XML bundle
// declares an element for EHR, VERSIONED_COMPOSITION, VERSIONED_EHR_STATUS or
// REVISION_HISTORY, so each root is named after its class until #4652 settles.
/// The canonical-XML root tag of the `EHR` document.
pub(super) const EHR_ROOT_TAG: &str = "ehr";
/// The canonical-XML root tag of the `VERSIONED_COMPOSITION` document.
pub(super) const VERSIONED_COMPOSITION_ROOT_TAG: &str = "versioned_composition";
/// The canonical-XML root tag of the `VERSIONED_EHR_STATUS` document.
pub(super) const VERSIONED_EHR_STATUS_ROOT_TAG: &str = "versioned_ehr_status";
/// The canonical-XML root tag of the `REVISION_HISTORY` document.
pub(super) const REVISION_HISTORY_ROOT_TAG: &str = "revision_history";

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

/// Decomposes an [`ObjectVersionId`] into the `(versioned-object uuid,
/// version_tree_id)` pair the service's `*_at_version` reads take.
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
        Some(served) if !served.eq_ignore_ascii_case(addressed) => Err(RestError::from(
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
