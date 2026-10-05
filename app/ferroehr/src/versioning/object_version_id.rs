// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OBJECT_VERSION_ID` / `VERSION_TREE_ID` decoding — the identification law of
//! the versioning core.
//!
//! Spec: `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
//! §"Identifying Versions" + §Syntaxes: an `OBJECT_VERSION_ID` is exactly three
//! `::`-delimited parts, `object_id '::' creating_system_id '::' version_tree_id`,
//! and a `VERSION_TREE_ID` is `trunk_version [ '.' branch_number '.'
//! branch_version ]` with every part `>= 1`
//! (`VERSION_TREE_ID.Trunk_version_valid` / `.Branch_validity`; RM common
//! `master06-change_control_package.adoc` §The 'Virtual Version Tree'). The
//! strict three-part parse lives in `openehr-base`
//! (`ObjectVersionId::from_str`); this module adds the storage-model typing on
//! top: the `object_id` must be the UUID `vo_id` key, and the
//! `version_tree_id` decodes into a [`TreeId`].
//!
//! This is the service's one strict decoder, and branch ids are first-class (RM
//! common master06 §Versioning Semantics → §Version Identification → §Local
//! Versioning: "To support branching, a further pair of numbers is added … Both
//! of these numbers also start at '1'").

use std::fmt;
use std::str::FromStr;

use openehr_base::prelude::{HierObjectId, ObjectVersionId, Uid};
use openehr_base::v1_3::base_types::identification::version_tree_id::VersionTreeId;
use openehr_its::rest::runtime::ApiError;
use uuid::Uuid;

use crate::ids::VoId;
use crate::service::error::ServiceError;

/// A decoded `VERSION_TREE_ID`.
///
/// The trunk version plus, for a branch version, the
/// `(branch_number, branch_version)` pair (both `>= 1` per BASE
/// `VERSION_TREE_ID`; RM common master06 §Local Versioning: "a further pair of
/// numbers is added … Both of these numbers also start at '1'").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeId {
    /// The trunk version this id sits on (first lexical part).
    pub(crate) trunk: i32,
    /// `None` for a trunk version; `Some((branch_number, branch_version))` for a
    /// branch version.
    pub(crate) branch: Option<(i32, i32)>,
}

impl TreeId {
    /// A trunk version id.
    pub(crate) const fn trunk(version: i32) -> Self {
        Self {
            trunk: version,
            branch: None,
        }
    }

    /// A branch version id (`trunk.branch_number.branch_version`).
    pub(crate) const fn branch(trunk: i32, branch_number: i32, branch_version: i32) -> Self {
        Self {
            trunk,
            branch: Some((branch_number, branch_version)),
        }
    }

    /// Whether this is a trunk version id.
    pub(crate) const fn is_trunk(self) -> bool {
        self.branch.is_none()
    }

    /// The storage triple `(trunk_version, branch_number, branch_version)` —
    /// `(t, 0, 0)` for a trunk row.
    pub(crate) const fn columns(self) -> (i32, i32, i32) {
        match self.branch {
            None => (self.trunk, 0, 0),
            Some((b, v)) => (self.trunk, b, v),
        }
    }

    /// A [`TreeId`] from the storage triple; `(t, 0, 0)` is a trunk id.
    pub(crate) const fn from_columns(trunk: i32, branch_number: i32, branch_version: i32) -> Self {
        if branch_number == 0 {
            Self::trunk(trunk)
        } else {
            Self::branch(trunk, branch_number, branch_version)
        }
    }

    /// Decode a BASE [`VersionTreeId`]'s lexical parts into the typed form.
    #[expect(
        clippy::map_err_ignore,
        reason = "the mapped error already echoes the rejected token; the discarded \
                  parse error adds only its own wording, which is not part of the \
                  wire contract"
    )]
    fn from_version_tree(tree: &VersionTreeId, raw: &str) -> Result<Self, VersionIdError> {
        let out_of_range = || VersionIdError::OutOfRange(raw.to_owned());
        let trunk: i32 = tree.trunk_version().parse().map_err(|_| out_of_range())?;
        if !tree.is_branch() {
            return Ok(Self::trunk(trunk));
        }
        let branch_number: i32 = tree
            .branch_number()
            .ok_or_else(out_of_range)?
            .parse()
            .map_err(|_| out_of_range())?;
        let branch_version: i32 = tree
            .branch_version()
            .ok_or_else(out_of_range)?
            .parse()
            .map_err(|_| out_of_range())?;
        Ok(Self::branch(trunk, branch_number, branch_version))
    }
}

impl fmt::Display for TreeId {
    /// The wire `version_tree_id` form: `N` or `N.B.V`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.branch {
            None => write!(f, "{}", self.trunk),
            Some((b, v)) => write!(f, "{}.{b}.{v}", self.trunk),
        }
    }
}

/// Format an `OBJECT_VERSION_ID` wire value from its three parts (`object_id ::
/// creating_system_id :: version_tree_id`; BASE master05 §Syntaxes). The single
/// place the versioning builders synthesize a version id, so its shape is
/// consistent with what [`parse_object_version_id`] accepts.
pub(crate) fn object_version_id(vo_id: VoId, creating_system_id: &str, tree: TreeId) -> String {
    format!("{vo_id}::{creating_system_id}::{tree}")
}

/// A **typed** `HIER_OBJECT_ID` from a raw string — a configured system
/// identifier, a stored container key, an imported id.
///
/// The generated `HIER_OBJECT_ID` carries a `pub(crate)` field behind a
/// validating door, so this is the platform library's one adapter from an
/// untyped string to the spec type. A value the BASE
/// `master05-identification_package.adoc` §Syntaxes grammar does not admit
/// (`hier_object_id = uid_based_id`, `root = uid`) is refused here rather than
/// serialized into a payload this CDR's own reader would then refuse.
///
/// A `HIER_OBJECT_ID` derived from a value that is a UUID *by type* needs no
/// adapter — `HierObjectId::from(uuid)` is total (BASE §Syntaxes:
/// `uid = iso_oid | uuid | internet_id`).
///
/// # Errors
/// [`VersionIdError::Malformed`] when `raw` is not a well-formed
/// `HIER_OBJECT_ID`.
pub(crate) fn hier_object_id(raw: &str) -> Result<HierObjectId, VersionIdError> {
    HierObjectId::new(raw).map_err(|source| VersionIdError::Malformed {
        raw: raw.to_owned(),
        source,
    })
}

/// The typed `OBJECT_VERSION_ID` for a version this CDR minted or stored: the
/// same three parts [`object_version_id`] formats, composed through the BASE
/// grammar rather than a struct literal.
///
/// The generated `OBJECT_VERSION_ID` carries a `pub(crate)` field behind a
/// validating door, so this is the one place the platform library turns its
/// storage triple into the spec type, and a malformed identifier cannot be
/// smuggled into a served payload.
///
/// Two of the three parts are valid by their Rust type ([`VoId`] is a UUID,
/// [`TreeId`] renders `N` / `N.B.V` with every part `>= 1`). The third,
/// `creating_system_id`, is a `String` — a config value for a version this
/// deployment mints, a stored column for one it read back — and BASE
/// `master05-identification_package.adoc` §Syntaxes types it `creating_system_id
/// = uid`. It is therefore the only part that can fail, and it fails LOUDLY
/// rather than producing an identifier the CDR's own reader would refuse.
///
/// # Errors
/// [`VersionIdError::Malformed`] when `creating_system_id` is not a legal
/// `uid`, so the three parts do not compose into a well-formed
/// `OBJECT_VERSION_ID`.
pub(crate) fn version_id(
    vo_id: VoId,
    creating_system_id: &str,
    tree: TreeId,
) -> Result<ObjectVersionId, VersionIdError> {
    let raw = object_version_id(vo_id, creating_system_id, tree);
    ObjectVersionId::new(raw.clone()).map_err(|source| VersionIdError::Malformed { raw, source })
}

/// Why a version-id string was rejected.
///
/// Converts into [`ApiError`] (`400`, path/header parameters),
/// [`ServiceError`] (`422`, payload fields) and
/// [`crate::service::status::SmError`] (`400`, SM catalog arguments) at each
/// call site's natural severity.
#[derive(Debug, thiserror::Error)]
pub enum VersionIdError {
    /// Not a well-formed BASE `OBJECT_VERSION_ID` (wrong part count, empty
    /// component, malformed `version_tree_id`).
    #[error("malformed OBJECT_VERSION_ID {raw:?}: {source}")]
    Malformed {
        /// The rejected wire value, echoed so the client can see what was read.
        raw: String,
        /// The BASE lexical-form error that rejected it.
        source: openehr_base::v1_3::base_types::identification::lexical::IdError,
    },
    /// The `object_id` part is not a UUID (this CDR keys versioned objects by
    /// UUID `vo_id`).
    #[error("object_id is not a UUID: {0:?}")]
    NotAUuid(String),
    /// A `version_tree_id` part does not fit the storage columns (`i32`).
    #[error("version_tree_id out of range: {0:?}")]
    OutOfRange(String),
}

impl From<VersionIdError> for ApiError {
    fn from(e: VersionIdError) -> Self {
        ApiError::BadRequest(e.to_string())
    }
}

impl From<VersionIdError> for ServiceError {
    fn from(e: VersionIdError) -> Self {
        ServiceError::content_invalid(crate::service::error::Violation::new(e.to_string()))
    }
}

impl From<VersionIdError> for crate::service::status::SmError {
    /// A malformed version id in an SM catalog argument is an argument-validity
    /// precondition failure (→ `400` at the wire, matching the
    /// [`From<VersionIdError> for ApiError`] `BadRequest` row).
    fn from(e: VersionIdError) -> Self {
        crate::service::status::SmError::precondition(e.to_string())
    }
}

/// Parse a bare `VERSION_TREE_ID` lexical value (`N` or `N.B.V`) into a
/// [`TreeId`] — the SM catalog's version argument form.
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already echoes the rejected token; the discarded \
              parse error adds only its own wording, which is not part of the \
              wire contract"
)]
pub(crate) fn parse_tree_id(raw: &str) -> Result<TreeId, VersionIdError> {
    let tree =
        VersionTreeId::from_str(raw).map_err(|_| VersionIdError::OutOfRange(raw.to_owned()))?;
    TreeId::from_version_tree(&tree, raw)
}

/// Parse a full `OBJECT_VERSION_ID` (`{object_id}::{creating_system_id}::{version_tree_id}`)
/// into the storage key pair (`vo_id`, [`TreeId`]).
pub(crate) fn parse_version_uid(raw: &str) -> Result<(VoId, TreeId), VersionIdError> {
    let (vo_id, _, tree) = parse_object_version_id(raw)?;
    Ok((VoId(vo_id), tree))
}

/// Parse a full `OBJECT_VERSION_ID` into all three components:
/// (`vo_id` = the `object_id` UUID, `creating_system_id`, [`TreeId`]).
/// The EHR-Extract import path preserves the wrapped `ORIGINAL_VERSION`'s
/// **complete** 3-part identity — object id, originating system, and version
/// tree id — so the imported version keeps its source identity (RM common
/// master06 §"Distributed Versioning": "if the version was imported,
/// `creating_system_id` will already have been set to the identifier of the
/// system of original creation").
pub(crate) fn parse_object_version_id(raw: &str) -> Result<(Uuid, String, TreeId), VersionIdError> {
    let ovid = ObjectVersionId::from_str(raw).map_err(|source| VersionIdError::Malformed {
        raw: raw.to_owned(),
        source,
    })?;
    components_of(&ovid, raw)
}

/// Decompose an already-validated [`ObjectVersionId`] into this CDR's storage
/// typing. `raw` is the wire spelling the errors echo (it equals
/// `ovid.value()`; the callers already hold it).
///
/// The ONE decomposition every caller shares, so a version id can never be
/// decomposed two different ways.
fn components_of(
    ovid: &ObjectVersionId,
    raw: &str,
) -> Result<(Uuid, String, TreeId), VersionIdError> {
    let Uid::Uuid(object_id) = ovid.object_id() else {
        return Err(VersionIdError::NotAUuid(raw.to_owned()));
    };
    let tree = TreeId::from_version_tree(&ovid.version_tree_id(), raw)?;
    // The VERBATIM second component, not the typed `creating_system_id()`:
    // an imported originating-system id must survive byte-for-byte
    // (BASE master05 §"Composite Identifiers and Case", case-PRESERVING), and
    // the typed accessor renders a UUID-shaped system id in the normalised
    // RFC 4122 lower case.
    let creating_system_id = ovid.creating_system_id_str().to_owned();
    Ok((*object_id.value(), creating_system_id, tree))
}

/// Decompose an already-parsed [`ObjectVersionId`] (the SM catalog's native
/// version-id argument) into the storage key pair (`vo_id`, [`TreeId`]).
pub(crate) fn components(ovid: &ObjectVersionId) -> Result<(VoId, TreeId), VersionIdError> {
    let (object_id, _, tree) = components_of(ovid, ovid.value())?;
    Ok((VoId(object_id), tree))
}

/// The storage address a `uid_based_id` / `versioned_object_uid` value names.
///
/// It carries the versioned-object key, plus the exact VERSION when the wire
/// value carried one (BASE `base_types` master05 §Syntaxes — a bare
/// `HIER_OBJECT_ID` addresses the container, a three-part
/// `OBJECT_VERSION_ID` addresses one version of it).
///
/// Produced by [`parse_uid_based_id`], the ONE decoder for this wire shape:
/// the platform library owns the policy (this CDR keys versioned objects by
/// UUID, and a version resolves to a [`TreeId`] storage position), so the
/// protocol adapter reads a path segment through this type rather than
/// re-deriving the same rules at the edge.
#[derive(Debug, Clone)]
pub struct UidAddress {
    /// The versioned-object UUID key (the id's `object_id`).
    pub vo_id: VoId,
    /// The full `OBJECT_VERSION_ID` when the wire value named one version;
    /// `None` for a bare container address.
    pub version: Option<ObjectVersionId>,
    /// The decoded version-tree position — `Some` exactly when
    /// [`version`](Self::version) is.
    tree: Option<TreeId>,
}

impl UidAddress {
    /// The addressed version's storage position in the version tree, or `None`
    /// when the wire value addressed the versioned object as a whole.
    pub(crate) const fn tree(&self) -> Option<TreeId> {
        self.tree
    }
}

/// Parse a `uid_based_id`/`versioned_object_uid` wire value: either a bare
/// `HIER_OBJECT_ID` (a UUID, → no version) or a full `OBJECT_VERSION_ID`
/// (strict three-part, → its [`TreeId`]).
///
/// # Errors
/// [`VersionIdError::Malformed`] when a `::`-carrying value is not a
/// well-formed `OBJECT_VERSION_ID`, [`VersionIdError::NotAUuid`] when the
/// addressed `object_id` is not a UUID, [`VersionIdError::OutOfRange`] when the
/// version-tree numbers do not fit the storage columns.
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already names the resource and echoes the \
              rejected token; the discarded `uuid::Error` adds only its own \
              wording, which is not part of the wire contract"
)]
pub fn parse_uid_based_id(raw: &str) -> Result<UidAddress, VersionIdError> {
    if raw.contains("::") {
        // ONE parse, ONE grammar: the strict `OBJECT_VERSION_ID` decode both
        // validates the wire value and yields the typed id, so the address can
        // never carry a version id the BASE grammar did not accept.
        let ovid = ObjectVersionId::from_str(raw).map_err(|source| VersionIdError::Malformed {
            raw: raw.to_owned(),
            source,
        })?;
        let (object_id, _, tree) = components_of(&ovid, raw)?;
        Ok(UidAddress {
            vo_id: VoId(object_id),
            version: Some(ovid),
            tree: Some(tree),
        })
    } else {
        let vo_id = Uuid::parse_str(raw).map_err(|_| VersionIdError::NotAUuid(raw.to_owned()))?;
        // A bare UID-based id names a versioned object.
        Ok(UidAddress {
            vo_id: VoId(vo_id),
            version: None,
            tree: None,
        })
    }
}

/// The bare token inside an `If-Match` value: surrounding whitespace and the
/// entity-tag double quotes stripped (RFC 9110 §8.8.3 — an entity-tag is a
/// quoted string).
///
/// This is the ONE place the versioning core unwraps an `If-Match` value, so
/// the demographic precondition comparison (`ensure_full_ovid_if_match`) and
/// the version-tree extraction ([`expected_from_if_match`]) always judge the
/// same token. The `W/` weakness indicator the ITS-REST overview §"`ETag` and
/// Last-Modified" mandates on emitted `ETag`s is decoded one layer up, at the
/// protocol adapter (`ferroehr-rest::overview::version_id::strip_etag`), where
/// HTTP header syntax belongs; this function is the library-boundary tolerance
/// for a direct caller that hands the quoted wire value straight through.
pub(crate) fn if_match_token(if_match: &str) -> &str {
    if_match.trim().trim_matches('"')
}

/// The expected (current) version from an `If-Match` header value: a quoted or
/// bare `OBJECT_VERSION_ID` (strict BASE parse — the `object_id` need not be a
/// UUID for precondition purposes), or a bare trunk integer.
///
/// Returns `Ok(None)` only for `If-Match: *` — RFC 9110 §If-Match's "matches any
/// current representation" wildcard: a must-exist precondition with no specific
/// version to compare, so no version-tree precondition is extracted and the
/// versioning path enforces existence alone.
///
/// A value that is neither a well-formed `OBJECT_VERSION_ID`, a bare
/// `VERSION_TREE_ID` trunk integer, nor `*` is **rejected** as
/// [`VersionIdError::Malformed`], never silently discarded: ITS-REST overview
/// §"If-Match and accidental overwrites" requires the precondition be honoured
/// ("if a service receives this header, and the condition evaluates to `false`,
/// it MUST NOT perform the requested method"), so a header that cannot be
/// evaluated must not run as if no precondition was sent (the lost-update
/// window). The spec does not name a code for a *malformed* `If-Match`; we map
/// it to `400 Bad Request` (the general "malformed request syntax" rule), the
/// same choice `ferroehr-rest::overview::version_id::require_if_match` makes for
/// the required-`If-Match` endpoints. `VersionIdError` converts into that `400`
/// at each caller's error type.
pub(crate) fn expected_from_if_match(if_match: &str) -> Result<Option<TreeId>, VersionIdError> {
    let token = if_match_token(if_match);
    // RFC 9110 §If-Match: `*` matches any current representation — no specific
    // version precondition to extract.
    if token == "*" {
        return Ok(None);
    }
    match ObjectVersionId::from_str(token) {
        // A full OBJECT_VERSION_ID names the version in its `version_tree_id` part.
        Ok(ovid) => Ok(Some(TreeId::from_version_tree(
            &ovid.version_tree_id(),
            token,
        )?)),
        // Lenient fallback: a bare VERSION_TREE_ID trunk integer.
        Err(source) => match token.parse().map(TreeId::trunk) {
            Ok(tree) => Ok(Some(tree)),
            Err(_) => Err(VersionIdError::Malformed {
                raw: token.to_owned(),
                source,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VO: &str = "018f4a5e-9df1-7d1e-8b6f-2b8c00000001";

    #[test]
    fn version_uid_strict_three_part() {
        let raw = format!("{VO}::ferroehr.local::3");
        let (vo_id, tree) = parse_version_uid(&raw).unwrap();
        assert_eq!(vo_id, VoId(Uuid::parse_str(VO).unwrap()));
        assert_eq!(tree, TreeId::trunk(3));

        // Two parts is not a valid lexical form.
        assert!(matches!(
            parse_version_uid(&format!("{VO}::2")),
            Err(VersionIdError::Malformed { .. })
        ));
        // Four parts is not a valid lexical form either.
        assert!(matches!(
            parse_version_uid(&format!("{VO}::a::b::2")),
            Err(VersionIdError::Malformed { .. })
        ));
        // Non-integer version tail.
        assert!(matches!(
            parse_version_uid(&format!("{VO}::sys::latest")),
            Err(VersionIdError::Malformed { .. })
        ));
        // Non-UUID object id.
        assert!(matches!(
            parse_version_uid("not-a-uuid::sys::1"),
            Err(VersionIdError::NotAUuid(_))
        ));
    }

    #[test]
    fn object_version_id_preserves_all_three_parts() {
        // The import path keeps the source's full 3-part identity.
        let raw = format!("{VO}::sourceSystem.example.org::4");
        let (vo_id, csid, tree) = parse_object_version_id(&raw).unwrap();
        assert_eq!(vo_id, Uuid::parse_str(VO).unwrap());
        assert_eq!(csid, "sourceSystem.example.org");
        assert_eq!(tree, TreeId::trunk(4));
        // Round-trips through the wire formatter.
        assert_eq!(object_version_id(VoId(vo_id), &csid, tree), raw);

        assert!(matches!(
            parse_object_version_id("not-a-uuid::sys::1"),
            Err(VersionIdError::NotAUuid(_))
        ));
    }

    /// Branch `version_tree_id`s decode into their `(trunk, branch, version)`
    /// triple and round-trip through the wire form (RM common master06 §Local
    /// Versioning — "version numbers like '1.1.1' … '2.3.3' … are possible").
    #[test]
    fn branch_ids_are_first_class() {
        let (vo_id, tree) = parse_version_uid(&format!("{VO}::sys::2.1.4")).unwrap();
        assert_eq!(vo_id, VoId(Uuid::parse_str(VO).unwrap()));
        assert_eq!(tree, TreeId::branch(2, 1, 4));
        assert_eq!(tree.columns(), (2, 1, 4));
        assert_eq!(tree.to_string(), "2.1.4");
        assert_eq!(TreeId::from_columns(2, 1, 4), tree);
        assert_eq!(TreeId::from_columns(3, 0, 0), TreeId::trunk(3));
        assert_eq!(TreeId::trunk(3).to_string(), "3");

        // A malformed branch tail (two parts) is not a valid VERSION_TREE_ID.
        assert!(parse_version_uid(&format!("{VO}::sys::2.1")).is_err());
    }

    #[test]
    fn uid_based_id_accepts_bare_hier_object_id() {
        let bare = parse_uid_based_id(VO).unwrap();
        assert_eq!(bare.vo_id, VoId(Uuid::parse_str(VO).unwrap()));
        assert_eq!(bare.tree(), None);
        assert!(bare.version.is_none());

        let versioned = parse_uid_based_id(&format!("{VO}::sys::2")).unwrap();
        assert_eq!(versioned.vo_id, VoId(Uuid::parse_str(VO).unwrap()));
        assert_eq!(versioned.tree(), Some(TreeId::trunk(2)));
        // The full three-part identity is carried through verbatim.
        assert_eq!(
            versioned.version.map(|o| o.value().to_owned()),
            Some(format!("{VO}::sys::2"))
        );

        // A `::`-carrying id must be a *valid* OBJECT_VERSION_ID.
        assert!(parse_uid_based_id(&format!("{VO}::sys")).is_err());
        assert!(parse_uid_based_id("garbage").is_err());
    }

    /// The verbatim `creating_system_id` survives the decode byte-for-byte —
    /// the case-PRESERVING half of BASE master05 §"Composite Identifiers and
    /// Case" (the typed `Uid` accessor would normalise a UUID-shaped system id).
    #[test]
    fn creating_system_id_is_preserved_verbatim() {
        let raw = format!("{VO}::87284370-2D4B-4e3d-A3F3-F303D2F4F34B::1");
        let (_, csid, _) = parse_object_version_id(&raw).unwrap();
        assert_eq!(csid, "87284370-2D4B-4e3d-A3F3-F303D2F4F34B");
        let mixed = format!("{VO}::SourceSystem.Example.ORG::1");
        let (_, csid, _) = parse_object_version_id(&mixed).unwrap();
        assert_eq!(csid, "SourceSystem.Example.ORG");
    }

    #[test]
    fn if_match_extraction() {
        assert_eq!(
            expected_from_if_match("\"abc::sys::3\"").unwrap(),
            Some(TreeId::trunk(3))
        );
        assert_eq!(
            expected_from_if_match("abc::sys::3").unwrap(),
            Some(TreeId::trunk(3))
        );
        // A branch precondition is honoured, not dropped.
        assert_eq!(
            expected_from_if_match("abc::sys::2.1.1").unwrap(),
            Some(TreeId::branch(2, 1, 1))
        );
        // Bare integer.
        assert_eq!(expected_from_if_match("2").unwrap(), Some(TreeId::trunk(2)));
        // RFC 9110 `If-Match: *` — match any current representation: no specific
        // version precondition, request proceeds (must-exist enforced downstream).
        assert_eq!(expected_from_if_match("*").unwrap(), None);
        // A malformed `If-Match` is REJECTED (400), never silently discarded as
        // "no precondition" — ITS-REST overview §"If-Match and accidental
        // overwrites".
        assert!(matches!(
            expected_from_if_match("garbage"),
            Err(VersionIdError::Malformed { .. })
        ));
        // Malformed OVID shapes do not leak a version out of the wrong slot —
        // and are rejected rather than dropped.
        assert!(matches!(
            expected_from_if_match("a::b::c::3"),
            Err(VersionIdError::Malformed { .. })
        ));
        assert!(matches!(
            expected_from_if_match("abc::3"),
            Err(VersionIdError::Malformed { .. })
        ));
    }

    /// The one `If-Match` token normalizer strips whitespace and the
    /// entity-tag quotes (RFC 9110 §8.8.3), leaving the bare value both the
    /// precondition compare and the version-tree extraction judge.
    #[test]
    fn if_match_token_strips_quotes_and_whitespace() {
        assert_eq!(if_match_token("\"abc::sys::3\""), "abc::sys::3");
        assert_eq!(if_match_token("  \"abc::sys::3\"  "), "abc::sys::3");
        assert_eq!(if_match_token("abc::sys::3"), "abc::sys::3");
        assert_eq!(if_match_token("\"\""), "");
    }
}
