// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-side `ITEM_TAG` surfaces: the per-target tag panels the composition
//! viewer and the EHR-status tab mount, and the EHR-wide tag browser.
//!
//! Seven released operations, all in the EHR API
//! (`docs/specs/openehr/ITS-REST/specifications/ehr.openapi.yaml`): the
//! aggregate `ehr_tags_get`, and the `composition_tags_*` /
//! `ehr_status_tags_*` get-update-delete trios. There is no directory tag
//! route — a FOLDER's tags are written with the `openehr-item-tag` commit
//! headers instead (`specifications/docs/overview/Requests_and_responses.md`
//! §`openehr-item-tag and openehr-version-item-tag`) — but the aggregate list
//! still reports them, so the browser resolves and links a directory target
//! without offering to edit it.
//!
//! Four wire facts this module is built around, each verified against the
//! released operation files:
//!
//! 1. **No tag route is conditional.** The three write operations declare
//!    `Prefer` and the two content headers and NOTHING else — no `If-Match` —
//!    and a tag collection carries neither `ETag` nor `Last-Modified`, which
//!    `Requests_and_responses.md` §"`ETag` and `Last-Modified`" reserves for
//!    resources "that have versioning or unique state identifiers". A tag write
//!    is last-writer-wins and commits no CONTRIBUTION; every panel says so.
//! 2. **The container form and the VERSION form address DISJOINT collections.**
//!    A `uid_based_id` is "an `OBJECT_VERSION_ID` … used to get the tags of a
//!    particular (target) version … whereas the latter … is be used to get the
//!    tags of the target `VERSIONED_COMPOSITION` container"
//!    (`operations/composition_tags_get.yaml`), and an `ITEM_TAG` has exactly
//!    one `target` (RM
//!    `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`),
//!    so neither collection is a view of the other.
//! 3. **An aggregate tag row names its target but not its target's KIND.** The
//!    RM types `target` as a bare `UID_BASED_ID`, which carries no `type`
//!    member, so opening a row asks the CDR which object holds that id
//!    ([`resolve_ehr_target`]) exactly as the demographic tag index does.
//! 4. **`GET /ehr/{ehr_id}/tags` filters, and an empty answer is `200 []`** —
//!    "This will return an empty list when there is no matching `ITEM_TAG`"
//!    (`operations/ehr_tags_get.yaml`); a `404` means the EHR itself is unknown.
//!
//! The tag client is the shared one ([`crate::bff::item_tags`]), working on
//! the paths this module builds; every path segment is percent-encoded.
//!
//! No openEHR spec governs the viewer — our own design / product extension.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use crate::error::ViewerError;
use crate::locale::tr_served;
use crate::locale::{Msg, tr_with};
use crate::uid::container_uid_of;

/// Which EHR-side object an `ITEM_TAG` target turned out to be.
///
/// The three taggable kinds inside an EHR. Two of them —
/// [`Composition`](Self::Composition) and [`EhrStatus`](Self::EhrStatus) — have
/// the released tag route trios; [`Directory`](Self::Directory) has none (fact
/// 1 in the module docs), so it is a navigation answer only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EhrTargetKind {
    /// A COMPOSITION or its `VERSIONED_COMPOSITION`.
    Composition,
    /// An `EHR_STATUS` or its `VERSIONED_EHR_STATUS`.
    EhrStatus,
    /// The EHR's FOLDER directory.
    Directory,
}

impl EhrTargetKind {
    /// The viewer screen that owns an object of this kind.
    ///
    /// A composition has its own route; the status and the directory are tabs
    /// of the EHR detail screen. Every id is percent-encoded.
    #[must_use]
    pub fn href(self, ehr_id: &str, container: &str) -> String {
        let ehr = urlencoding::encode(ehr_id);
        match self {
            Self::Composition => format!(
                "/ehrs/{ehr}/compositions/{}",
                urlencoding::encode(container)
            ),
            Self::EhrStatus => format!("/ehrs/{ehr}/status"),
            Self::Directory => format!("/ehrs/{ehr}/directory"),
        }
    }
}

/// The path of the tag collection of composition `uid` inside `ehr_id`
/// (`ehr/{ehr_id}/composition/{uid_based_id}/tags`, `composition_tags_*`).
///
/// `uid` addresses either the version container or one VERSION, two disjoint
/// collections (module docs fact 2).
#[must_use]
pub fn composition_tags_path(ehr_id: &str, uid: &str) -> String {
    format!(
        "ehr/{}/composition/{}/tags",
        urlencoding::encode(ehr_id),
        urlencoding::encode(uid)
    )
}

/// The path of the aggregate tag list of `ehr_id` (`ehr_tags_get`), each
/// filter sent only when it is filled.
#[must_use]
pub fn ehr_tags_path(ehr_id: &str, key: &str, value: &str) -> String {
    format!(
        "ehr/{}/tags{}",
        urlencoding::encode(ehr_id),
        crate::bff::item_tags::tag_filter_query(key, value, "")
    )
}

/// Which object inside `ehr_id` holds `uid`, or `None` when none does.
///
/// An aggregate tag row reports its target as a bare `UID_BASED_ID` with no RM
/// type attached (module docs fact 3), so the only honest way to open a row is
/// to ask the CDR.
///
/// Three steps, and the ORDER is load-bearing: the two IDENTITY comparisons run
/// first — the EHR resource's own `ehr_status` reference, then the directory's
/// `uid` — because each names its object exactly. Only then is the COMPOSITION
/// resource read, the one step that infers a kind from a route answering at
/// all, since inference would otherwise claim every id that route happens to
/// serve. Any answer other than `404` from that read is raised rather than read
/// as "not a composition".
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when `uid` is empty.
pub async fn resolve_ehr_target(
    // The EHR that owns the tagged object.
    ehr_id: String,
    // The tag's target, in either `uid_based_id` form.
    uid: String,
) -> Result<Option<EhrTargetKind>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let container = container_uid_of(&uid);
    if container.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::TagTargetIdRequired)));
    }
    if ehr_status_container(&cdr, &ehr_id).await? == container {
        return Ok(Some(EhrTargetKind::EhrStatus));
    }
    let ehr = urlencoding::encode(&ehr_id).into_owned();
    let directory = cdr.rest_v1(&format!("ehr/{ehr}/directory"));
    let response = cdr.get(&directory).send().await?;
    if response.is(http::StatusCode::OK)
        && container_uid_of(&crate::uid::uid_value_of(&response.body)) == container
    {
        return Ok(Some(EhrTargetKind::Directory));
    }
    let composition = cdr.rest_v1(&format!(
        "ehr/{ehr}/composition/{}",
        urlencoding::encode(&container)
    ));
    let response = cdr.get(&composition).send().await?;
    if response.is(http::StatusCode::OK) || response.is(http::StatusCode::NO_CONTENT) {
        return Ok(Some(EhrTargetKind::Composition));
    }
    if !response.is(http::StatusCode::NOT_FOUND) {
        response.ok()?;
    }
    Ok(None)
}

/// The `VERSIONED_EHR_STATUS` container of an EHR, read from the EHR resource's
/// own `ehr_status` reference (`GET /ehr/{ehr_id}`).
///
/// The `EHR_STATUS` tag routes take a `uid_based_id` and there is no
/// status-tags route keyed on the `ehr_id` alone, so the container has to be
/// resolved before the collection can be addressed. `EHR.ehr_status` is an
/// `OBJECT_REF` whose `id` addresses the CURRENT `EHR_STATUS` version (RM
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr.adoc`), so
/// its container part is the versioned object the panel edits.
///
/// This is a second WINDOW of the endpoint the EHR-detail header already reads,
/// for an identifier rather than for a fact the screen shows.
///
/// # Errors
/// CDR transport errors pass through; a non-2xx answer normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
async fn ehr_status_container(cdr: &crate::bff::Cdr, ehr_id: &str) -> Result<String, ViewerError> {
    let url = cdr.rest_v1(&format!("ehr/{}", urlencoding::encode(ehr_id)));
    let response = cdr.get(&url).ok().await?;
    let doc: serde_json::Value = serde_json::from_str(&response.body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::EhrJson2, &[("e", (e).to_string().into())]))
    })?;
    Ok(container_uid_of(
        doc.get("ehr_status")
            .and_then(|status| status.get("id"))
            .and_then(|id| id.get("value"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{EhrTargetKind, composition_tags_path};

    #[test]
    fn a_composition_tag_path_encodes_every_id() {
        // The release publishes composition_tags_* and ehr_status_tags_* and no
        // directory tag route at all (ehr.openapi.yaml).
        assert_eq!(
            composition_tags_path("7d44", "a/b"),
            "ehr/7d44/composition/a%2Fb/tags"
        );
    }

    #[test]
    fn a_targets_href_opens_the_screen_that_owns_it_and_encodes_every_id() {
        assert_eq!(
            EhrTargetKind::Composition.href("7d44b88c", "8849182c"),
            "/ehrs/7d44b88c/compositions/8849182c"
        );
        assert_eq!(
            EhrTargetKind::EhrStatus.href("7d44b88c", "8849182c"),
            "/ehrs/7d44b88c/status"
        );
        assert_eq!(
            EhrTargetKind::Directory.href("7d44b88c", "8849182c"),
            "/ehrs/7d44b88c/directory"
        );
        // A hostile id can never break out of its path segment.
        assert_eq!(
            EhrTargetKind::Composition.href("a/b", "c?d#e"),
            "/ehrs/a%2Fb/compositions/c%3Fd%23e"
        );
    }
}
