// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/demographics` section — the CDR's demographic space.
//!
//! Five screens over one API group: the per-kind party browser
//! ([`browse`]), the party detail with its edit / history / relationships /
//! tags tabs ([`party`], [`history`], [`relationship`], [`tags`]), the
//! `PARTY_RELATIONSHIP` resource ([`relationship`]), and the demographic
//! CONTRIBUTION viewer ([`contribution`]).
//!
//! NOTE: the Demographic API is `DEVELOPMENT`-state within ITS-REST
//! Release-1.1.0 (`specifications/docs/demographic/Description.md` §Status),
//! so the wire these screens read can change in the next release.
//!
//! Three wire facts shape every screen here, all verified against the vendored
//! release:
//!
//! 1. **A party is reached by id, never by listing.** The released Demographic
//!    API publishes `POST /demographic/{kind}` plus
//!    `GET`/`PUT`/`DELETE /demographic/{kind}/{uid_based_id}` and no collection
//!    `GET` (`specifications/demographic.openapi.yaml` + the per-kind
//!    `operations/person_*.yaml` quintet), and AQL's `FROM` is EHR-scoped, so
//!    there is no query that enumerates parties either. The browser screen is
//!    therefore a lookup + create surface, and the one enumerable demographic
//!    collection the release does publish — `GET /demographic/tags`
//!    (`operations/demographic_tags_get.yaml`) — is its listing.
//! 2. **`uid_based_id` has two forms and they are not interchangeable.** An
//!    update addresses the version CONTAINER ("can take only a form of an
//!    `HIER_OBJECT_ID` identifier", `operations/person_update.yaml`), a delete
//!    addresses the version to supersede ("MUST be in a form of an
//!    `OBJECT_VERSION_ID` … representing the `preceding_version_uid` to be
//!    deleted", `operations/person_delete.yaml`), and a read takes either. The
//!    viewer routes on the container uid
//!    ([`container_uid_of`]) and takes the
//!    served document's own `uid.value` for `If-Match` and for the delete path.
//! 3. **`PARTY_RELATIONSHIP` has no released wire.** The vendored Demographic
//!    API defines no `party_relationship` path at all; those routes are the
//!    CDR's own extension realizing SM `I_PARTY_RELATIONSHIP`
//!    (`docs/specs/openehr/SM/docs/UML/classes/i_party_relationship.adoc`), and
//!    a CDR that does not serve them answers `404` — which this section reports
//!    inline rather than hiding.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! Every co-located `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

pub mod browse;
pub mod contribution;
pub mod history;
pub mod party;
pub mod relationship;
pub mod tags;

use leptos::server;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssr")]
use serde_json::Value;

use crate::components::wire::{VersionEntry, VersionedObjectFacts};
use crate::error::ViewerError;
use crate::uid::container_uid_of;

/// The five concrete PARTY families the Demographic API routes by.
///
/// The segments are the released path segments themselves
/// (`/demographic/agent`, `/demographic/group`,
/// `/demographic/organisation`, `/demographic/person`, `/demographic/role` —
/// `specifications/demographic.openapi.yaml`), and each RM type is the
/// payload `_type` that family stores. This is a route key, not a re-model of
/// the RM: a party document itself is only ever carried verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartyKind {
    /// `AGENT` — `/demographic/agent`.
    Agent,
    /// `GROUP` — `/demographic/group`.
    Group,
    /// `ORGANISATION` — `/demographic/organisation`.
    Organisation,
    /// `PERSON` — `/demographic/person`.
    Person,
    /// `ROLE` — `/demographic/role`.
    Role,
}

impl PartyKind {
    /// Every kind, in the order the switcher lists them.
    pub const ALL: [Self; 5] = [
        Self::Person,
        Self::Organisation,
        Self::Group,
        Self::Agent,
        Self::Role,
    ];

    /// The URL path segment of this family (`person`, `organisation`, …) — the
    /// released route segment and the viewer's own route segment alike.
    #[must_use]
    pub fn segment(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Group => "group",
            Self::Organisation => "organisation",
            Self::Person => "person",
            Self::Role => "role",
        }
    }

    /// The RM `_type` this family stores (`PERSON`, `ROLE`, …).
    #[must_use]
    pub fn rm_type(self) -> &'static str {
        match self {
            Self::Agent => "AGENT",
            Self::Group => "GROUP",
            Self::Organisation => "ORGANISATION",
            Self::Person => "PERSON",
            Self::Role => "ROLE",
        }
    }

    /// The plural label the kind switcher shows.
    #[must_use]
    pub fn plural(self) -> &'static str {
        match self {
            Self::Agent => "Agents",
            Self::Group => "Groups",
            Self::Organisation => "Organisations",
            Self::Person => "People",
            Self::Role => "Roles",
        }
    }

    /// The kind a URL segment names, or `None` for anything outside the closed
    /// five-kind set (the browser screen answers that with its not-found view).
    #[must_use]
    pub fn from_segment(segment: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.segment() == segment)
    }

    /// The kind a `PARTY_REF.type` names, or `None` when the ref points at an
    /// abstract type (`PARTY`, `ACTOR`) or anything else — `PARTY_REF` admits
    /// the abstract supertypes on purpose (BASE
    /// `org.openehr.base.base_types.party_ref.adoc` §`Type_validity`), and no
    /// per-kind route addresses those.
    #[must_use]
    pub fn from_rm_type(rm_type: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.rm_type() == rm_type)
    }
}

/// A demographic versioned-object family, as the `versioned_*` read routes
/// segment it.
///
/// `versioned_party` is released (`operations/versioned_party_get.yaml`);
/// `versioned_party_relationship` is the CDR's own extension (module docs
/// fact 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionedFamily {
    /// `VERSIONED_PARTY` — `/demographic/versioned_party/{versioned_object_uid}`.
    Party,
    /// The relationship container — `/demographic/versioned_party_relationship/…`
    /// (extension).
    PartyRelationship,
}

impl VersionedFamily {
    /// The route segment of this family.
    #[must_use]
    pub fn segment(self) -> &'static str {
        match self {
            Self::Party => "versioned_party",
            Self::PartyRelationship => "versioned_party_relationship",
        }
    }

    /// The family a route segment names, or `None` for anything else.
    ///
    /// Server functions are a public HTTP API, so the segment they
    /// interpolate into a CDR path is validated back into this closed set
    /// rather than trusted.
    #[must_use]
    pub fn from_segment(segment: &str) -> Option<Self> {
        match segment {
            "versioned_party" => Some(Self::Party),
            "versioned_party_relationship" => Some(Self::PartyRelationship),
            _ => None,
        }
    }
}

/// Which demographic resource a document read addresses: one of the five party
/// families, or the `PARTY_RELATIONSHIP` extension resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemographicResource {
    /// A party of this kind — `/demographic/{kind}/{uid_based_id}`.
    Party(PartyKind),
    /// A relationship — `/demographic/party_relationship/{uid_based_id}`
    /// (extension).
    Relationship,
}

impl DemographicResource {
    /// The route segment this resource lives under.
    #[must_use]
    pub fn segment(self) -> &'static str {
        match self {
            Self::Party(kind) => kind.segment(),
            Self::Relationship => "party_relationship",
        }
    }

    /// The versioned-object family this resource's versions live in.
    #[must_use]
    pub fn family(self) -> VersionedFamily {
        match self {
            Self::Party(_) => VersionedFamily::Party,
            Self::Relationship => VersionedFamily::PartyRelationship,
        }
    }

    /// The resource a route segment names, or `None` for anything else — the
    /// same public-endpoint validation [`VersionedFamily::from_segment`] does.
    #[must_use]
    pub fn from_segment(segment: &str) -> Option<Self> {
        if segment == "party_relationship" {
            return Some(Self::Relationship);
        }
        PartyKind::from_segment(segment).map(Self::Party)
    }
}

/// The typed refusal a server function answers when a caller hands it a
/// segment outside the closed demographic route set.
#[cfg(feature = "ssr")]
fn unknown_segment(segment: &str, expected: &str) -> ViewerError {
    ViewerError::Invalid(format!(
        "{segment:?} is not a {expected} of the demographic API"
    ))
}

/// The `/demographics/{kind}` browser href.
#[must_use]
pub fn browse_href(kind: PartyKind) -> String {
    format!("/demographics/{}", kind.segment())
}

/// The `/demographics/{kind}/{uid}` detail href.
///
/// The id is percent-encoded — an id carrying `/`, `#`, `?` or `%` would
/// otherwise address a different route, and the encoded form is also what makes
/// the value safe to hand a server-side redirect.
#[must_use]
pub fn party_href(kind: PartyKind, uid: &str) -> String {
    format!(
        "/demographics/{}/{}",
        kind.segment(),
        urlencoding::encode(&container_uid_of(uid))
    )
}

/// The `/demographics/relationship/{uid}` detail href (same encoding rule as
/// [`party_href`]).
#[must_use]
pub fn relationship_href(uid: &str) -> String {
    format!(
        "/demographics/relationship/{}",
        urlencoding::encode(&container_uid_of(uid))
    )
}

/// The `/demographics/contribution/{uid}` viewer href.
#[must_use]
pub fn contribution_href(uid: &str) -> String {
    format!(
        "/demographics/contribution/{}",
        urlencoding::encode(uid.trim())
    )
}

/// Read a demographic versioned-object container and one of its VERSIONs.
///
/// Two reads, one resource: `GET /demographic/{family}/{versioned_object_uid}`
/// for the container (`operations/versioned_party_get.yaml`) and the VERSION
/// read for the envelope — `…/version/{version_uid}` for an explicitly
/// selected version (`operations/versioned_party_version_get_by_id.yaml`) or
/// `…/version` for the current one when nothing is selected, which "retrieves
/// the _latest_ VERSION" when `version_at_time` is omitted
/// (`operations/versioned_party_version_get_at_time.yaml`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (the `404` for an unknown
/// container or version included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when either body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_versioned_object(
    /// Which versioned-object family the container belongs to, as its route
    /// segment ([`VersionedFamily::segment`]).
    family: String,
    /// The container id (`versioned_object_uid`).
    versioned_object_uid: String,
    /// The version whose envelope facts to read; empty reads the latest.
    version_uid: String,
) -> Result<VersionedObjectFacts, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let family = VersionedFamily::from_segment(&family)
        .ok_or_else(|| unknown_segment(&family, "versioned-object family"))?
        .segment();
    let container = urlencoding::encode(&container_uid_of(&versioned_object_uid)).into_owned();
    let object_url = state
        .cdr
        .rest_v1(&format!("demographic/{family}/{container}"));
    let object_response = state
        .cdr
        .get(&session.credential, &object_url, "application/json")
        .await?;
    let object_body = crate::cdr::CdrClient::expect_success(object_response)?.body;

    let version_uid = version_uid.trim();
    let version_url = if version_uid.is_empty() {
        state
            .cdr
            .rest_v1(&format!("demographic/{family}/{container}/version"))
    } else {
        state.cdr.rest_v1(&format!(
            "demographic/{family}/{container}/version/{}",
            urlencoding::encode(version_uid)
        ))
    };
    let version_response = state
        .cdr
        .get(&session.credential, &version_url, "application/json")
        .await?;
    let version_body = crate::cdr::CdrClient::expect_success(version_response)?.body;
    parse_versioned_object(&object_body, &version_body)
}

/// A demographic versioned object's revision history, newest-first
/// (`GET /demographic/{family}/{versioned_object_uid}/revision_history` —
/// `operations/versioned_party_revision_history.yaml`).
///
/// The rows are the shared [`VersionEntry`] every History tab renders, parsed by
/// the same `parse_versions`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (a `404` for an unknown container
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the history is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_demographic_revision_history(
    /// Which versioned-object family the container belongs to, as its route
    /// segment ([`VersionedFamily::segment`]).
    family: String,
    /// The container id (`versioned_object_uid`).
    versioned_object_uid: String,
) -> Result<Vec<VersionEntry>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let family = VersionedFamily::from_segment(&family)
        .ok_or_else(|| unknown_segment(&family, "versioned-object family"))?;
    let url = state.cdr.rest_v1(&format!(
        "demographic/{}/{}/revision_history",
        family.segment(),
        urlencoding::encode(&container_uid_of(&versioned_object_uid))
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    // NOTE: REVISION_HISTORY.items arrives most-recent-LAST on the wire (RM
    // common `org.openehr.rm.common.revision_history.adoc` §items); the table
    // presents newest-first, so reverse here.
    crate::pages::composition::parse_versions(&response.body).map(|mut entries| {
        entries.reverse();
        entries
    })
}

/// Resolve the `OBJECT_VERSION_ID` of the VERSION extant at `at_time` (a
/// browser `datetime-local` value):
/// `GET /demographic/{family}/{versioned_object_uid}/version?version_at_time=…`
/// — "If `version_at_time` is supplied, retrieves the VERSION extant _at
/// specified time_" (`operations/versioned_party_version_get_at_time.yaml`).
/// The `200` body is a VERSION envelope whose `uid.value` is that
/// `OBJECT_VERSION_ID`; the string is returned so the caller can pin it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when `at_time` is empty; CDR transport errors pass
/// through; a non-2xx CDR answer (the `404` for no version at that time
/// included, which the UI renders as an inline note) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn resolve_demographic_version_at_time(
    /// Which versioned-object family the container belongs to, as its route
    /// segment ([`VersionedFamily::segment`]).
    family: String,
    /// The container id (`versioned_object_uid`).
    versioned_object_uid: String,
    /// The instant to resolve, as a `datetime-local` input value.
    at_time: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let family = VersionedFamily::from_segment(&family)
        .ok_or_else(|| unknown_segment(&family, "versioned-object family"))?;
    let at_time = crate::format::datetime_local_to_rfc3339(&at_time);
    if at_time.is_empty() {
        return Err(ViewerError::Invalid(
            "pick a date and time to travel to".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/{}/{}/version?version_at_time={}",
        family.segment(),
        urlencoding::encode(&container_uid_of(&versioned_object_uid)),
        urlencoding::encode(&at_time),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Read one version of a demographic resource as its own document, pretty-
/// printed for the pane.
///
/// The document CONTENT reader is the RESOURCE route, not the VERSION route —
/// the composition viewer's split: `GET /demographic/{segment}/{uid_based_id}`
/// serves the party (or relationship) itself, and a `uid_based_id` "in the form
/// of an OBJECT_VERSION_ID … is used to retrieve a specific known version"
/// (`operations/person_get.yaml`). The VERSION envelope facts come from
/// [`fetch_versioned_object`] instead.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when no version uid is given; CDR transport errors
/// pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_demographic_version_document(
    /// Which resource family the version belongs to, as its route segment
    /// ([`DemographicResource::segment`]).
    resource: String,
    /// The version to read, as a full `OBJECT_VERSION_ID`.
    version_uid: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let resource = DemographicResource::from_segment(&resource)
        .ok_or_else(|| unknown_segment(&resource, "resource"))?;
    let version_uid = version_uid.trim();
    if version_uid.is_empty() {
        return Err(ViewerError::Invalid(
            "a version uid is required to read a past version".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/{}/{}",
        resource.segment(),
        urlencoding::encode(version_uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    Ok(crate::components::format_view::pretty_body(
        &body,
        crate::format::ReprFormat::CanonicalJson,
    ))
}

/// The party kind that holds `uid`, or `None` when no kind does.
///
/// The demographic space has no kind-agnostic party read, so the only honest
/// answer is to ask each family's own read route in turn
/// (`GET /demographic/{kind}/{uid_based_id}`) and take the first that answers:
/// a `200` is the party, a `204` is a party whose current version is deleted
/// ("`204` … deleted at time", `operations/person_get.yaml`) — both mean "this
/// family holds it" — and a `404` means "not here, try the next".
///
/// It exists because the one enumerable demographic collection,
/// `GET /demographic/tags`, reports each tag's target as a bare
/// `UID_BASED_ID` with no kind attached, so a tag row cannot link anywhere
/// without this.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when `uid` is empty; CDR transport errors pass
/// through; a refusal or any non-`404` failure normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success) rather
/// than being read as "not found".
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn resolve_party_kind(
    /// The party id to place, in either `uid_based_id` form.
    uid: String,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let uid = container_uid_of(&uid);
    if uid.is_empty() {
        return Err(ViewerError::Invalid("a party id is required".to_owned()));
    }
    let encoded = urlencoding::encode(&uid).into_owned();
    for kind in PartyKind::ALL {
        let url = state
            .cdr
            .rest_v1(&format!("demographic/{}/{encoded}", kind.segment()));
        let response = state
            .cdr
            .get(&session.credential, &url, "application/json")
            .await?;
        if response.is(http::StatusCode::OK) || response.is(http::StatusCode::NO_CONTENT) {
            return Ok(Some(kind.segment().to_owned()));
        }
        if response.is(http::StatusCode::NOT_FOUND) {
            continue;
        }
        // A refusal or a server fault is NOT "this kind does not hold it":
        // swallowing it would report a reachable party as missing.
        drop(crate::cdr::CdrClient::expect_success(response)?);
    }
    Ok(None)
}

#[cfg(feature = "ssr")]
/// Flatten a container body plus a VERSION body into [`VersionedObjectFacts`].
/// Defensive throughout — an absent attribute reads as empty rather than
/// failing the card.
///
/// # Errors
/// [`ViewerError::Internal`] when either body is not valid JSON.
fn parse_versioned_object(
    object_body: &str,
    version_body: &str,
) -> Result<VersionedObjectFacts, ViewerError> {
    let object: Value = serde_json::from_str(object_body)
        .map_err(|e| ViewerError::Internal(format!("versioned demographic object JSON: {e}")))?;
    let version: Value = serde_json::from_str(version_body)
        .map_err(|e| ViewerError::Internal(format!("demographic version JSON: {e}")))?;
    Ok(VersionedObjectFacts {
        object_uid: crate::uid::uid_value_of_document(&object),
        owner_id: json_str(&object, &["owner_id", "id", "value"]),
        time_created: json_str(&object, &["time_created", "value"]),
        version_id: crate::uid::uid_value_of_document(&version),
        lifecycle_state: json_str(&version, &["lifecycle_state", "value"]),
        preceding_version_uid: json_str(&version, &["preceding_version_uid", "value"]),
        contribution_uid: json_str(&version, &["contribution", "id", "value"]),
        signed: version
            .get("signature")
            .and_then(Value::as_str)
            .is_some_and(|signature| !signature.is_empty()),
    })
}

#[cfg(feature = "ssr")]
/// Follow a chain of object keys to a string leaf, or an empty string when any
/// hop is absent or not a string.
pub(crate) fn json_str(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        DemographicResource, PartyKind, VersionedFamily, browse_href, contribution_href,
        party_href, relationship_href,
    };

    #[test]
    fn every_kind_round_trips_through_its_route_segment() {
        for kind in PartyKind::ALL {
            assert_eq!(PartyKind::from_segment(kind.segment()), Some(kind));
            assert_eq!(PartyKind::from_rm_type(kind.rm_type()), Some(kind));
        }
        // The five released segments, exactly (demographic.openapi.yaml).
        let segments: Vec<&str> = PartyKind::ALL.iter().map(|k| k.segment()).collect();
        assert_eq!(
            segments,
            vec!["person", "organisation", "group", "agent", "role"]
        );
    }

    #[test]
    fn a_segment_outside_the_closed_set_is_not_a_kind() {
        // The viewer's own reserved segments must never read as a kind, or the
        // relationship/contribution routes would be shadowed.
        for segment in [
            "",
            "relationship",
            "contribution",
            "PERSON",
            "people",
            "party",
        ] {
            assert_eq!(PartyKind::from_segment(segment), None, "{segment}");
        }
        // PARTY_REF admits the abstract supertypes; no per-kind route serves
        // them (BASE party_ref.adoc §Type_validity).
        for rm_type in ["PARTY", "ACTOR", "ANY", ""] {
            assert_eq!(PartyKind::from_rm_type(rm_type), None, "{rm_type}");
        }
    }

    #[test]
    fn hrefs_route_on_the_container_and_percent_encode_the_id() {
        assert_eq!(browse_href(PartyKind::Person), "/demographics/person");
        assert_eq!(
            party_href(PartyKind::Role, "8849182c::sys::3"),
            "/demographics/role/8849182c"
        );
        // A hostile id can never break out of its path segment.
        assert_eq!(
            party_href(PartyKind::Person, "a/b?c#d"),
            "/demographics/person/a%2Fb%3Fc%23d"
        );
        assert_eq!(
            relationship_href("7d44aa01::sys::1"),
            "/demographics/relationship/7d44aa01"
        );
        // A contribution uid is NOT a version id — it keeps every character.
        assert_eq!(contribution_href(" c9 "), "/demographics/contribution/c9");
    }

    #[test]
    fn resource_and_family_segments_are_the_wire_segments() {
        assert_eq!(
            DemographicResource::Party(PartyKind::Organisation).segment(),
            "organisation"
        );
        assert_eq!(
            DemographicResource::Relationship.segment(),
            "party_relationship"
        );
        assert_eq!(
            DemographicResource::Party(PartyKind::Person).family(),
            VersionedFamily::Party
        );
        assert_eq!(
            DemographicResource::Relationship.family(),
            VersionedFamily::PartyRelationship
        );
        assert_eq!(VersionedFamily::Party.segment(), "versioned_party");
        assert_eq!(
            VersionedFamily::PartyRelationship.segment(),
            "versioned_party_relationship"
        );
    }

    #[test]
    fn a_segment_a_server_function_receives_is_validated_back_into_the_closed_set() {
        // Every segment the viewer itself can spell round-trips…
        for family in [VersionedFamily::Party, VersionedFamily::PartyRelationship] {
            assert_eq!(
                VersionedFamily::from_segment(family.segment()),
                Some(family)
            );
        }
        for resource in PartyKind::ALL
            .into_iter()
            .map(DemographicResource::Party)
            .chain(std::iter::once(DemographicResource::Relationship))
        {
            assert_eq!(
                DemographicResource::from_segment(resource.segment()),
                Some(resource)
            );
        }
        // …and nothing else does, so no caller-supplied string can steer a CDR
        // path (a server function is a public HTTP endpoint).
        for hostile in ["", "..", "ehr", "versioned_composition", "party", "/"] {
            assert_eq!(VersionedFamily::from_segment(hostile), None, "{hostile}");
            assert_eq!(
                DemographicResource::from_segment(hostile),
                None,
                "{hostile}"
            );
        }
    }
}

#[cfg(all(test, feature = "ssr"))]
mod parse_tests {
    use super::parse_versioned_object;

    #[test]
    fn parses_a_demographic_container_and_its_version_envelope() {
        let object = r#"{
            "_type": "VERSIONED_PARTY",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "8849182c"},
            "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
        }"#;
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::2"},
            "contribution": {"_type": "OBJECT_REF", "id": {"value": "c9"}},
            "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "complete"},
            "preceding_version_uid": {"value": "8849182c::example.org::1"},
            "signature": "-----BEGIN PGP SIGNATURE-----"
        }"#;
        let facts = parse_versioned_object(object, version).expect("valid bodies");
        assert_eq!(facts.object_uid, "8849182c");
        assert_eq!(facts.time_created, "2026-07-12T10:00:00Z");
        assert_eq!(facts.version_id, "8849182c::example.org::2");
        assert_eq!(facts.lifecycle_state, "complete");
        assert_eq!(facts.preceding_version_uid, "8849182c::example.org::1");
        assert_eq!(facts.contribution_uid, "c9");
        assert!(facts.signed);
        // A demographic container has no owning EHR, so `owner_id` is empty.
        assert_eq!(facts.owner_id, "");
    }

    #[test]
    fn a_first_version_has_no_preceding_version_and_no_signature() {
        // RM common `org.openehr.rm.common.version.adoc` invariant
        // `Preceding_version_uid_validity`: absent exactly for a first version.
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"value": "8849182c::example.org::1"},
            "lifecycle_state": {"value": "complete"}
        }"#;
        let facts = parse_versioned_object("{}", version).expect("valid bodies");
        assert_eq!(facts.preceding_version_uid, "");
        assert!(!facts.signed);
        assert_eq!(facts.object_uid, "");
        assert!(parse_versioned_object("not json", "{}").is_err());
        assert!(parse_versioned_object("{}", "not json").is_err());
    }
}
