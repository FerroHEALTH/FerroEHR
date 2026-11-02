// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/ehrs/{ehr_id}/compositions/{uid}` screen — the composition viewer.
//!
//! A view of one COMPOSITION: a format toggle (canonical JSON/XML and the
//! Simplified FLAT/STRUCTURED renderings — the CDR converts, the BFF forwards
//! and pretty-prints), a version selector fed by the versioned object's
//! revision history, a per-version audit card, the `VERSIONED_COMPOSITION`
//! container + selected-VERSION envelope card, and the two write paths (commit
//! a new version, logically delete the latest one). The document resource is
//! keyed on `(version, format)` so either switch refetches, under a
//! `<Transition>`.
//!
//! One reader per claim: the document CONTENT comes from the COMPOSITION
//! resource (the only one that negotiates the simplified formats), the commit
//! history from the revision history, and the VERSION's own envelope facts from
//! the `VERSIONED_COMPOSITION` version read.
//!
//! No openEHR spec governs the viewer — our own design; the wire it reads is
//! spec-bound (`Accept` negotiation per
//! `docs/specs/openehr/ITS-REST/docs/simplified_formats/`). Every read takes
//! the session's handle ([`cdr`](crate::bff::cdr)) first, and the CDR
//! credential never reaches the page.

use crate::bff::wire::{VersionEntry, VersionedObjectFacts, revision_history};
use crate::error::ViewerError;
use crate::format::ReprFormat;
use crate::locale::Msg;
use crate::locale::{tr_served, tr_served_with};

/// The revision history of a versioned composition, newest-first.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Internal`] when the history is not valid JSON.
pub async fn fetch_versions(
    // The EHR holding the versioned composition.
    ehr_id: String,
    // The `VERSIONED_COMPOSITION` uid whose revision history to read.
    uid: String,
) -> Result<Vec<VersionEntry>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1(&format!(
        "ehr/{}/versioned_composition/{}/revision_history",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&uid)
    ));
    revision_history(&cdr, &url).await
}

/// One representation of a composition version, pretty-printed for display, or
/// `None` when the addressed version is deleted.
///
/// `version_uid` may be the bare versioned-object id (the latest version) or a
/// full `OBJECT_VERSION_ID` (that exact version).
///
/// The `204` branch is `composition_get`'s own declared answer for a version
/// deleted at the specified time
/// (`docs/specs/openehr/ITS-REST/specifications/responses/204_deleted_at_time.yaml`,
/// referenced from `operations/composition_get.yaml`), so it is a rendered
/// absence rather than a failure.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); a non-2xx CDR answer (a
/// `406` for a declined representation included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn fetch_composition(
    // The EHR holding the composition.
    ehr_id: String,
    // The version to read: a full `OBJECT_VERSION_ID`, or the object uid
    // for the latest version.
    version_uid: String,
    // Which representation to negotiate for the document body.
    format: ReprFormat,
) -> Result<Option<String>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1(&format!(
        "ehr/{}/composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&version_uid)
    ));
    let response = cdr.get(&url).accept(format.media_type()).send().await?;
    if response.is(http::StatusCode::NO_CONTENT) {
        return Ok(None);
    }
    let response = response.ok()?;
    Ok(Some(crate::bff::pretty_body(&response.body, format)))
}

/// Resolve the version of a `VERSIONED_COMPOSITION` extant at `at_time`.
///
/// The `OBJECT_VERSION_ID`, for a browser `datetime-local` value: `GET
/// /ehr/{ehr}/versioned_composition/{uid}/version?version_at_time=…` (ITS-REST
/// `VERSIONED_COMPOSITION` API `versioned_composition_version_get_at_time` —
/// "if `version_at_time` is supplied, retrieves the VERSION extant at specified
/// time").
///
/// The 200 body is a VERSION envelope whose `uid.value` is the
/// `OBJECT_VERSION_ID` (RM common — a VERSION's `uid` is an
/// `OBJECT_VERSION_ID`); that string is returned so the caller can select it.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when `at_time` is empty; a non-2xx CDR answer (a
/// `404` for no version at that time included, which the UI renders as an
/// inline note) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn fetch_version_at_time(
    // The EHR holding the versioned composition.
    ehr_id: String,
    // The `VERSIONED_COMPOSITION` uid to read a version of.
    versioned_object_uid: String,
    // The instant to resolve, as a `datetime-local` input value.
    at_time: String,
) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let at_time = crate::format::datetime_local_to_rfc3339(&at_time);
    if at_time.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::PickDateTimeTravel)));
    }
    let url = cdr.rest_v1(&format!(
        "ehr/{}/versioned_composition/{}/version?version_at_time={}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&versioned_object_uid),
        urlencoding::encode(&at_time),
    ));
    let response = cdr.get(&url).ok().await?;
    // The VERSION envelope's `uid.value` is the `OBJECT_VERSION_ID`.
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Commit a new version of a COMPOSITION (`PUT
/// /ehr/{ehr_id}/composition/{versioned_object_uid}`).
///
/// `If-Match` carries the CURRENT (latest) `version_uid` — the
/// `preceding_version_uid` — so the update is conditional (ITS-REST COMPOSITION
/// API `composition_update`; the header value is the `version_uid` enclosed in
/// double quotes). Canonical JSON body; `Prefer: return=representation` yields
/// the new `uid.value`.
///
/// This is the one update whose precondition is NOT a served `ETag`: the
/// document pane may be showing an older version, so the value comes from the
/// revision history's newest entry instead — the `ETag` of the read that filled
/// the editor would name the wrong version.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] on an empty body, a missing current version, or a
/// `412` mid-air collision (prefixed with a reload hint, the CDR diagnostic
/// appended); any other non-2xx CDR answer (its validation diagnostics
/// included) normalizes via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn update_composition(
    // The EHR holding the composition.
    ehr_id: String,
    // The `VERSIONED_COMPOSITION` uid being updated.
    versioned_object_uid: String,
    // The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    // The replacement composition document, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::CompositionBodyEmpty)));
    }
    let current_version_uid = current_version_uid.trim();
    if current_version_uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::NoCurrentVersionKnownReloadPage,
        )));
    }
    // If-Match value is the version_uid in double quotes (composition_update).
    let if_match = format!("\"{current_version_uid}\"");
    let url = cdr.rest_v1(&format!(
        "ehr/{}/composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&versioned_object_uid)
    ));
    let response = cdr
        .put(&url)
        .body("application/json", body)
        .headers(&[("Prefer", "return=representation"), ("If-Match", &if_match)])
        .send()
        .await?;
    // 412 Precondition Failed = a mid-air collision (the latest version moved
    // since this page loaded); give it a friendly prefix, the CDR diagnostic
    // appended verbatim.
    if response.is(http::StatusCode::PRECONDITION_FAILED) {
        let detail = response
            .ok()
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        return Err(ViewerError::Invalid(tr_served_with(
            Msg::CompositionChangedSincePageLoadedReload,
            &[("detail", detail.clone().into())],
        )));
    }
    let response = response.ok()?;
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Logically delete a COMPOSITION
/// (`DELETE /ehr/{ehr_id}/composition/{version_uid}`).
///
/// `version_uid` MUST be a full `OBJECT_VERSION_ID` — the path id is "in a form
/// of an `OBJECT_VERSION_ID` identifier taken from the last (most recent)
/// `VERSION.uid.value`, representing the `preceding_version_uid` to be deleted"
/// (`docs/specs/openehr/ITS-REST/specifications/operations/composition_delete.yaml`)
/// — so a bare versioned-object id is rejected here rather than sent. No
/// `If-Match` is sent: the operation declares none, and latest-ness is enforced
/// by its own `409`, "supplied `uid_based_id` doesn't match the latest version".
/// The success answer is `204`; an already-deleted version is `400`.
///
/// This is the openEHR LOGICAL delete: a new deleted-lifecycle version is
/// committed and the history stays readable.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when no full `OBJECT_VERSION_ID` is known; a
/// non-2xx CDR answer (the `409` stale-uid branch and the `400` for an
/// already-deleted version included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn delete_composition(
    // The EHR holding the composition.
    ehr_id: String,
    // The latest version's full `OBJECT_VERSION_ID`, which the delete requires.
    version_uid: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let version_uid = version_uid.trim();
    if version_uid.is_empty() || !version_uid.contains("::") {
        return Err(ViewerError::Invalid(tr_served(
            Msg::LatestVersionSFullObjectVersion,
        )));
    }
    let url = cdr.rest_v1(&format!(
        "ehr/{}/composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(version_uid)
    ));
    cdr.delete(&url).ok().await?;
    Ok(())
}

/// Read the `VERSIONED_COMPOSITION` container and one of its VERSIONS.
///
/// Two reads, one resource: `GET /ehr/{ehr}/versioned_composition/{uid}` for the
/// container (`versioned_composition_get`) and the direct VERSION read for the
/// envelope — `…/version/{version_uid}` for an explicitly selected version
/// (`versioned_composition_version_get_by_id`) or `…/version` for the current
/// one when the selector says "Latest": with no `version_at_time`, that
/// operation "retrieves the _latest_ VERSION"
/// (`docs/specs/openehr/ITS-REST/specifications/operations/versioned_composition_version_get_at_time.yaml`
/// §description).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); a non-2xx CDR answer (a
/// `404` for an unknown object/version included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok); [`ViewerError::Internal`]
/// when either body is not valid JSON.
pub async fn fetch_versioned_composition(
    // The EHR holding the versioned composition.
    ehr_id: String,
    // The `VERSIONED_COMPOSITION` uid whose envelope facts to read.
    versioned_object_uid: String,
    // The specific version within that versioned object.
    version_uid: String,
) -> Result<VersionedObjectFacts, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let object_url = cdr.rest_v1(&format!(
        "ehr/{}/versioned_composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&versioned_object_uid)
    ));
    crate::bff::wire::read_versioned_facts(&cdr, &object_url, &version_uid).await
}
