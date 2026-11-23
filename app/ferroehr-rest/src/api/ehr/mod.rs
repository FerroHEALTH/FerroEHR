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
//! [`directory`] and [`contribution`]. [`ehr_access`] declares FerroEHR's own
//! `EHR_ACCESS` reads, which run the `EHR_STATUS` read handlers for the
//! `EHR_ACCESS` class (`EhrObjectType`).

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
pub mod ehr_access;
pub mod ehr_resource;
pub mod ehr_status;
pub mod openapi_routes;
pub mod versioned_composition;
pub mod versioned_ehr_status;

// COMPOSITION + CONTRIBUTION negotiate the Simplified-Formats (FLAT/STRUCTURED)
// representations through the shared `crate::formats::dispatch` adapter, called
// by its full path (no module alias — every import names its defining module).

use http::HeaderMap;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::runtime::ToXml;
use openehr_rm::prelude::{EhrAccess, EhrStatus, VersionedEhrAccess, VersionedEhrStatus};

use ferroehr::service::ehr::object::EhrObject;
use ferroehr::service::response::{RawServiceResponse, ReadBody};

use crate::overview::error::RestError;
use crate::overview::negotiate::{CANONICAL, WireFormat, resolve_accept};
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

// NOTE: no specification governs this: our own design. No ITS-XML bundle declares
// an element for EHR, VERSIONED_COMPOSITION, VERSIONED_EHR_STATUS, VERSIONED_EHR_ACCESS
// or REVISION_HISTORY, so each root is named after its class until #4652 settles.
/// The canonical-XML root tag of the `EHR` document.
pub(super) const EHR_ROOT_TAG: &str = "ehr";
/// The canonical-XML root tag of the `VERSIONED_COMPOSITION` document.
pub(super) const VERSIONED_COMPOSITION_ROOT_TAG: &str = "versioned_composition";
/// The canonical-XML root tag of the `VERSIONED_EHR_STATUS` document.
pub(super) const VERSIONED_EHR_STATUS_ROOT_TAG: &str = "versioned_ehr_status";
/// The canonical-XML root tag of the `VERSIONED_EHR_ACCESS` document.
pub(super) const VERSIONED_EHR_ACCESS_ROOT_TAG: &str = "versioned_ehr_access";
/// The canonical-XML root tag of the `REVISION_HISTORY` document.
pub(super) const REVISION_HISTORY_ROOT_TAG: &str = "revision_history";

/// An RM object an EHR holds exactly one of (RM ehr, EHR class), as the
/// `EHR_STATUS` reads serve it and the `EHR_ACCESS` reads serve it the same way.
pub(crate) trait EhrObjectType: DeserializeOwned + ToXml {
    /// The object the service reads.
    const OBJECT: EhrObject;
    /// The canonical-XML root tag of its versioned object.
    const VERSIONED_ROOT_TAG: &'static str;
    /// Whether the object maps onto canonical XML. `EHR_ACCESS` does not: its
    /// settings are of a scheme no published class defines, and canonical XML
    /// maps only published classes (`openehr_its::xml::runtime`).
    const CANONICAL_XML: bool;
    /// Its versioned object.
    type Versioned: DeserializeOwned + ToXml;
}

impl EhrObjectType for EhrStatus {
    const OBJECT: EhrObject = EhrObject::Status;
    const VERSIONED_ROOT_TAG: &'static str = VERSIONED_EHR_STATUS_ROOT_TAG;
    const CANONICAL_XML: bool = true;
    type Versioned = VersionedEhrStatus;
}

impl EhrObjectType for EhrAccess {
    const OBJECT: EhrObject = EhrObject::Access;
    const VERSIONED_ROOT_TAG: &'static str = VERSIONED_EHR_ACCESS_ROOT_TAG;
    const CANONICAL_XML: bool = false;
    type Versioned = VersionedEhrAccess;
}

/// Refuses with `406` a request whose `Accept` wins canonical XML for an
/// object of `T` that has no canonical-XML mapping, which the service "cannot
/// fulfill" (`Resources.md` §"XML Format").
///
/// # Errors
/// [`ApiError::NotAcceptable`] for such a request.
pub(super) fn refuse_unmapped_xml<T: EhrObjectType>(headers: &HeaderMap) -> Result<(), RestError> {
    if !T::CANONICAL_XML
        && resolve_accept(headers, CANONICAL, WireFormat::CanonicalJson)
            == Some(WireFormat::CanonicalXml)
    {
        return Err(RestError::from(ApiError::NotAcceptable(
            "the settings of this object are of a scheme no canonical-XML class maps, so it is \
             served as application/json only"
                .to_owned(),
        )));
    }
    Ok(())
}

/// Re-inlines externalized `DV_MULTIMEDIA` content when the caller asked for it
/// with `?expand_multimedia=true`, verifying each blob's integrity.
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

/// [`expand_multimedia_if_requested`] over a read whose body may still be the
/// stored text: parsed only when the caller asked for the expansion.
pub(super) async fn expand_read_if_requested(
    state: &AppState,
    query: Option<&str>,
    resp: RawServiceResponse,
) -> Result<RawServiceResponse, RestError> {
    if query_param(query, "expand_multimedia").as_deref() != Some("true") {
        return Ok(resp);
    }
    let RawServiceResponse { body, meta } = resp;
    let value = body.into_value().map_err(|e| {
        RestError::from(crate::overview::error::internal_fault(
            "re-parse the stored body",
            &e,
        ))
    })?;
    Ok(RawServiceResponse {
        body: ReadBody::Value(state.backend().expand_multimedia(value).await?),
        meta,
    })
}

/// [`ensure_served_version`] over a read whose body may still be the stored
/// text, which carries the version's own `uid`, the one its metadata names.
pub(super) fn ensure_served_read_version(
    addressed: &str,
    resp: &RawServiceResponse,
) -> Result<(), RestError> {
    match (&resp.body, &resp.meta) {
        (ReadBody::Value(body), _) => ensure_served_version(addressed, body),
        (ReadBody::RawJson(_), Some(meta)) if !meta.uid.eq_ignore_ascii_case(addressed) => Err(
            RestError::from(ApiError::NotFound(format!("version {addressed}"))),
        ),
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
