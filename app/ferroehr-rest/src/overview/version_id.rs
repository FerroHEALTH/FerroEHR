// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Wire-string decoding for the EHR-core routes: path/header identifiers into
//! the SM catalog's native argument types (`uuid::Uuid`,
//! [`openehr_base::prelude::ObjectVersionId`]).
//!
//! The SM native API is protocol-free: the catalog takes RM/BASE
//! identifier types, not raw wire strings. Turning a path parameter such as
//! `{ehr_id}` or `{version_uid}` into those types is the protocol adapter's
//! decode job — it lives here, at the ITS-REST edge, and every malformed value
//! surfaces as `400 Bad Request` ([`ApiError::BadRequest`]).
//!
//! Spec: BASE 1.3.0 `object_version_id.adoc` — an `OBJECT_VERSION_ID` is exactly
//! three `::`-delimited parts (`object_id '::' creating_system_id '::'
//! version_tree_id`); the strict parse is `ObjectVersionId::from_str`. A
//! `uid_based_id`/`versioned_object_uid` path segment is either a bare
//! `HIER_OBJECT_ID` (a UUID) or a full `OBJECT_VERSION_ID` — that one is
//! decoded by the platform library's single decoder
//! ([`ferroehr::versioning::object_version_id::parse_uid_based_id`]), because
//! the POLICY it applies (a versioned object is keyed by its `object_id` UUID,
//! and a version resolves to a storage tree position) belongs to the storage
//! model, not to the protocol edge.

use std::str::FromStr;

use ferroehr::ids::EhrId;
use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::runtime::ApiError;
use uuid::Uuid;

/// Parse an `{ehr_id}` path parameter into its UUID.
///
/// # Errors
/// [`ApiError::BadRequest`] if `raw` is not a UUID.
#[expect(
    clippy::map_err_ignore,
    reason = "`uuid::Error` carries only \"this is not a UUID\", which the wire \
              message already states; the body text is pinned by the conformance \
              catalogue and must not gain parser detail"
)]
pub(crate) fn parse_ehr_id(raw: &str) -> Result<EhrId, ApiError> {
    Uuid::parse_str(raw)
        .map(EhrId)
        .map_err(|_| ApiError::BadRequest(format!("invalid EHR id: {raw}")))
}

/// Parse a UUID path parameter (a bare `HIER_OBJECT_ID`: a versioned-object or
/// contribution uid).
///
/// # Errors
/// [`ApiError::BadRequest`] if `raw` is not a UUID.
#[expect(
    clippy::map_err_ignore,
    reason = "`uuid::Error` carries only \"this is not a UUID\", which the wire \
              message already states; the body text is pinned by the conformance \
              catalogue and must not gain parser detail"
)]
pub(crate) fn parse_uuid(raw: &str, what: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::BadRequest(format!("invalid {what}: {raw}")))
}

/// Parse a full `{version_uid}` path parameter into a BASE
/// [`openehr_base::prelude::ObjectVersionId`].
///
/// # Errors
/// [`ApiError::BadRequest`] if `raw` is not a well-formed `OBJECT_VERSION_ID`.
pub(crate) fn parse_version_uid(raw: &str) -> Result<ObjectVersionId, ApiError> {
    ObjectVersionId::from_str(raw)
        .map_err(|e| ApiError::BadRequest(format!("malformed OBJECT_VERSION_ID {raw:?}: {e}")))
}

/// The `object_id` of an `OBJECT_VERSION_ID` as a UUID (this CDR keys versioned
/// objects by UUID), or `None` if the `object_id` is some other `UID` form.
pub(crate) fn object_id_uuid(ovid: &ObjectVersionId) -> Option<Uuid> {
    use openehr_base::prelude::Uid;
    match ovid.object_id() {
        Uid::Uuid(u) => Some(*u.value()),
        _ => None,
    }
}

/// The `preceding_version_uid` for an operation whose `If-Match` header is
/// **required** (`ehr_status_update`, `composition_update`, `directory_update`,
/// `directory_delete` — all `required: true`, `parameters/If-Match`). The value
/// is the full quoted `OBJECT_VERSION_ID`; a malformed or empty value is a
/// client error rather than a silently-skipped precondition.
///
/// NOTE (wire, spec-silent): ITS-REST defines only the "received and the
/// condition evaluates to false → `412`" case; it says nothing about a
/// syntactically invalid `If-Match`. We map an unparseable required `If-Match`
/// to `400 Bad Request` (the general "malformed request syntax" rule), never to
/// a silent bypass of the optimistic-concurrency guard.
///
/// Both the weak (`W/"…"`) and the bare quoted (`"…"`) forms are accepted:
/// the overview §"`ETag` and Last-Modified" now emits the weak form, but a client
/// that echoes the deprecated bare form "MAY still" be supported
/// (§"Deprecated headers") — [`strip_etag`] normalizes either into the inner
/// `OBJECT_VERSION_ID`.
pub(crate) fn require_if_match(if_match: &str) -> Result<ObjectVersionId, ApiError> {
    let token = strip_etag(if_match);
    if token.is_empty() {
        return Err(ApiError::BadRequest(
            "If-Match is required for this operation but was empty".to_owned(),
        ));
    }
    ObjectVersionId::from_str(token).map_err(|e| {
        ApiError::BadRequest(format!(
            "If-Match must be a quoted OBJECT_VERSION_ID; {token:?} is malformed: {e}"
        ))
    })
}

/// Strip an `ETag`/`If-Match` wrapper down to its opaque value: an optional
/// leading weakness indicator `W/` (case-insensitive) then the surrounding
/// double quotes. Accepts the weak form the server now emits and the deprecated
/// bare quoted form alike (overview §"`ETag` and Last-Modified").
pub(crate) fn strip_etag(raw: &str) -> &str {
    let trimmed = raw.trim();
    let unweak = trimmed
        .strip_prefix("W/")
        .or_else(|| trimmed.strip_prefix("w/"))
        .unwrap_or(trimmed);
    unweak.trim().trim_matches('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every ITS-REST path identifier has a parser here for its class: a
    /// `HIER_OBJECT_ID` is read as the UUID the OAS declares (`format: uuid`,
    /// [`parse_ehr_id`], [`parse_uuid`]), an `OBJECT_VERSION_ID` by
    /// [`parse_version_uid`], and a `UID_BASED_ID` by the platform's
    /// `parse_uid_based_id`. A re-vendored OAS that gives a `HIER_OBJECT_ID`
    /// another form fails here before it reaches the UUID-keyed storage.
    #[test]
    fn every_path_identifier_class_has_its_parser() {
        use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
        use openehr_its::rest::routes::{IdentifierClass, Param, ParamKind, ParamLocation};
        let tables: [&[&[Param]]; 6] = [
            admin::ROUTE_PARAMS,
            definition::ROUTE_PARAMS,
            demographic::ROUTE_PARAMS,
            ehr::ROUTE_PARAMS,
            query::ROUTE_PARAMS,
            system::ROUTE_PARAMS,
        ];
        let mut classes = std::collections::BTreeSet::new();
        for row in tables.into_iter().flatten() {
            for p in row.iter().filter(|p| p.location == ParamLocation::Path) {
                let Some(class) = p.identifier else { continue };
                classes.insert(class.as_str());
                match class {
                    IdentifierClass::HierObject => {
                        assert_eq!(p.kind, ParamKind::Uuid, "{}", p.name);
                    }
                    IdentifierClass::ObjectVersion => {
                        assert!(parse_version_uid(UID).is_ok(), "{}", p.name);
                    }
                    IdentifierClass::UidBased => {
                        assert!(
                            ferroehr::versioning::object_version_id::parse_uid_based_id(UID)
                                .is_ok(),
                            "{}",
                            p.name
                        );
                    }
                }
            }
        }
        assert_eq!(
            classes.into_iter().collect::<Vec<_>>(),
            ["HIER_OBJECT_ID", "OBJECT_VERSION_ID", "UID_BASED_ID"]
        );
    }

    const UID: &str = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2";

    #[test]
    fn strip_etag_handles_weak_and_bare() {
        assert_eq!(strip_etag(&format!("W/\"{UID}\"")), UID);
        assert_eq!(strip_etag(&format!("\"{UID}\"")), UID);
        assert_eq!(strip_etag(&format!("  w/\"{UID}\"  ")), UID);
        assert_eq!(strip_etag(UID), UID);
    }

    #[test]
    fn require_if_match_accepts_weak_and_bare() {
        // The server now emits the weak form; a client echoing either shape
        // must parse to the same OBJECT_VERSION_ID.
        let weak = require_if_match(&format!("W/\"{UID}\"")).expect("weak");
        let bare = require_if_match(&format!("\"{UID}\"")).expect("bare");
        assert_eq!(weak.value(), UID);
        assert_eq!(bare.value(), UID);
    }

    #[test]
    fn require_if_match_empty_is_bad_request() {
        let err = require_if_match("\"\"").expect_err("empty");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    #[test]
    fn require_if_match_malformed_is_bad_request() {
        let err = require_if_match("W/\"not-a-version-id\"").expect_err("malformed");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }
}
