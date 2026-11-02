// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/demographics` section — the CDR's demographic space.
//!
//! Five screens over one API group: the per-kind party browser
//! (`browse`), the party detail with its edit / history / relationships /
//! tags tabs ([`party`], `history`, [`relationship`], [`PartyKind::tags_path`]), the
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
//!    CDR's own extension, and a CDR that does not serve them answers `404` — which this section reports
//!    inline rather than hiding.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! Every read guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches the page.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

pub mod contribution;
pub mod party;
pub mod relationship;

use serde_json::Value;

use crate::bff::wire::{VersionEntry, VersionedObjectFacts};
use crate::error::ViewerError;
use crate::locale::Msg;
use crate::locale::tr_served;
use crate::uid::container_uid_of;

/// The five concrete PARTY families the Demographic API routes by.
///
/// The segments are the released path segments themselves
/// (`/demographic/agent`, `/demographic/group`,
/// `/demographic/organisation`, `/demographic/person`, `/demographic/role` —
/// `specifications/demographic.openapi.yaml`), and each RM type is the
/// payload `_type` that family stores. This is a route key, not a re-model of
/// the RM: a party document itself is only ever carried verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The path of the tag collection of party `uid`
    /// (`demographic/{kind}/{uid_based_id}/tags`, `operations/person_tags_*.yaml`).
    ///
    /// The path addresses the version CONTAINER: the container and a single
    /// VERSION are two disjoint collections of the same object
    /// (`operations/person_tags_get.yaml`), and a container tag survives the
    /// party's next version, which is what an operator marking a party means.
    #[must_use]
    pub fn tags_path(self, uid: &str) -> String {
        format!(
            "demographic/{}/{}/tags",
            self.segment(),
            urlencoding::encode(&container_uid_of(uid))
        )
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The CDR URL of `path` (a `demographic/...` path of this family): under
    /// the openEHR API for the released `versioned_party`, under the CDR's
    /// extensions root for the relationship container.
    #[must_use]
    pub fn url(self, cdr: &crate::cdr::CdrClient, path: &str) -> String {
        match self {
            Self::Party => cdr.rest_v1(path),
            Self::PartyRelationship => cdr.extensions_v1(path),
        }
    }

    /// The family a route segment names, or `None` for anything else.
    ///
    /// A route segment is caller-supplied text, so the segment a page
    /// interpolates into a CDR path is validated back into this closed set
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The CDR URL of `path` (a `demographic/...` path of this resource):
    /// under the openEHR API for a party, under the CDR's extensions root for
    /// a relationship.
    #[must_use]
    pub fn url(self, cdr: &crate::cdr::CdrClient, path: &str) -> String {
        self.family().url(cdr, path)
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

/// The typed refusal a page answers when a caller hands it a
/// segment outside the closed demographic route set.
fn unknown_segment(segment: &str, expected: &str) -> ViewerError {
    ViewerError::Invalid(format!(
        "{segment:?} is not a {expected} of the demographic API"
    ))
}

/// The `/parties/{kind}/{uid}` page of a party.
///
/// The id is percent-encoded — an id carrying `/`, `#`, `?` or `%` would
/// otherwise address a different route, and the encoded form is also what makes
/// the value safe to hand a server-side redirect.
#[must_use]
pub fn party_href(kind: PartyKind, uid: &str) -> String {
    format!(
        "/parties/{}/{}",
        kind.segment(),
        urlencoding::encode(&container_uid_of(uid))
    )
}

/// The `/parties/relationships/{uid}` page of a relationship (same encoding rule as
/// [`party_href`]).
#[must_use]
pub fn relationship_href(uid: &str) -> String {
    format!(
        "/parties/relationships/{}",
        urlencoding::encode(&container_uid_of(uid))
    )
}

/// The `/parties/contributions/{uid}` page of a demographic contribution.
#[must_use]
pub fn contribution_href(uid: &str) -> String {
    format!("/parties/contributions/{}", urlencoding::encode(uid.trim()))
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
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); a non-2xx CDR answer
/// (the `404` for an unknown container or version included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok); [`ViewerError::Internal`]
/// when either body is not valid JSON.
pub async fn fetch_versioned_object(
    // Which versioned-object family the container belongs to, as its route
    // segment ([`VersionedFamily::segment`]).
    family: String,
    // The container id (`versioned_object_uid`).
    versioned_object_uid: String,
    // The version whose envelope facts to read; empty reads the latest.
    version_uid: String,
) -> Result<VersionedObjectFacts, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let versioned = VersionedFamily::from_segment(&family)
        .ok_or_else(|| unknown_segment(&family, "versioned-object family"))?;
    let object_url = versioned.url(
        &cdr,
        &format!(
            "demographic/{}/{}",
            versioned.segment(),
            urlencoding::encode(&container_uid_of(&versioned_object_uid))
        ),
    );
    crate::bff::wire::read_versioned_facts(&cdr, &object_url, &version_uid).await
}

/// A demographic versioned object's revision history, newest-first
/// (`GET /demographic/{family}/{versioned_object_uid}/revision_history` —
/// `operations/versioned_party_revision_history.yaml`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); a non-2xx CDR answer (a
/// `404` for an unknown container included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok); [`ViewerError::Internal`]
/// when the history is not valid JSON.
pub async fn fetch_demographic_revision_history(
    // Which versioned-object family the container belongs to.
    family: VersionedFamily,
    // The container id (`versioned_object_uid`).
    versioned_object_uid: &str,
) -> Result<Vec<VersionEntry>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = family.url(
        &cdr,
        &format!(
            "demographic/{}/{}/revision_history",
            family.segment(),
            urlencoding::encode(&container_uid_of(versioned_object_uid))
        ),
    );
    crate::bff::wire::revision_history(&cdr, &url).await
}

/// Resolve the `OBJECT_VERSION_ID` of the VERSION extant at `at_time`.
///
/// For a browser `datetime-local` value:
/// `GET /demographic/{family}/{versioned_object_uid}/version?version_at_time=…`
/// — "If `version_at_time` is supplied, retrieves the VERSION extant _at
/// specified time_" (`operations/versioned_party_version_get_at_time.yaml`).
///
/// The `200` body is a VERSION envelope whose `uid.value` is that
/// `OBJECT_VERSION_ID`; the string is returned so the caller can pin it.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when `at_time` is empty; a non-2xx CDR answer (the
/// `404` for no version at that time included, which the UI renders as an
/// inline note) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn resolve_demographic_version_at_time(
    // Which versioned-object family the container belongs to, as its route
    // segment ([`VersionedFamily::segment`]).
    family: String,
    // The container id (`versioned_object_uid`).
    versioned_object_uid: String,
    // The instant to resolve, as a `datetime-local` input value.
    at_time: String,
) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let family = VersionedFamily::from_segment(&family)
        .ok_or_else(|| unknown_segment(&family, "versioned-object family"))?;
    let at_time = crate::format::datetime_local_to_rfc3339(&at_time);
    if at_time.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::PickDateTimeTravel)));
    }
    let url = family.url(
        &cdr,
        &format!(
            "demographic/{}/{}/version?version_at_time={}",
            family.segment(),
            urlencoding::encode(&container_uid_of(&versioned_object_uid)),
            urlencoding::encode(&at_time),
        ),
    );
    let response = cdr.get(&url).ok().await?;
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Read one version of a demographic resource as its own document, pretty-
/// printed for the pane.
///
/// The document CONTENT reader is the RESOURCE route, not the VERSION route —
/// the composition viewer's split: `GET /demographic/{segment}/{uid_based_id}`
/// serves the party (or relationship) itself, and a `uid_based_id` "in the form
/// of an `OBJECT_VERSION_ID` … is used to retrieve a specific known version"
/// (`operations/person_get.yaml`). The VERSION envelope facts come from
/// [`fetch_versioned_object`] instead.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when no version uid is given.
pub async fn fetch_demographic_version_document(
    // Which resource family the version belongs to, as its route segment
    // ([`DemographicResource::segment`]).
    resource: String,
    // The version to read, as a full `OBJECT_VERSION_ID`.
    version_uid: String,
) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let resource = DemographicResource::from_segment(&resource)
        .ok_or_else(|| unknown_segment(&resource, "resource"))?;
    let version_uid = version_uid.trim();
    if version_uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::VersionUidRequiredReadPastVersion,
        )));
    }
    let url = resource.url(
        &cdr,
        &format!(
            "demographic/{}/{}",
            resource.segment(),
            urlencoding::encode(version_uid)
        ),
    );
    let body = cdr.get(&url).ok().await?.body;
    Ok(crate::bff::pretty_body(
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
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when `uid` is empty; a refusal or any non-`404`
/// failure normalizes via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok)
/// rather than being read as "not found".
pub async fn resolve_party_kind(
    // The party id to place, in either `uid_based_id` form.
    uid: String,
) -> Result<Option<String>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let uid = container_uid_of(&uid);
    if uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::PartyIdRequired)));
    }
    let encoded = urlencoding::encode(&uid).into_owned();
    for kind in PartyKind::ALL {
        let url = cdr.rest_v1(&format!("demographic/{}/{encoded}", kind.segment()));
        let response = cdr.get(&url).send().await?;
        if response.is(http::StatusCode::OK) || response.is(http::StatusCode::NO_CONTENT) {
            return Ok(Some(kind.segment().to_owned()));
        }
        if response.is(http::StatusCode::NOT_FOUND) {
            continue;
        }
        // A refusal or a server fault is NOT "this kind does not hold it":
        // swallowing it would report a reachable party as missing.
        response.ok()?;
    }
    Ok(None)
}

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
        DemographicResource, PartyKind, VersionedFamily, contribution_href, party_href,
        relationship_href,
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
        assert_eq!(
            party_href(PartyKind::Role, "8849182c::sys::3"),
            "/parties/role/8849182c"
        );
        // A hostile id can never break out of its path segment.
        assert_eq!(
            party_href(PartyKind::Person, "a/b?c#d"),
            "/parties/person/a%2Fb%3Fc%23d"
        );
        assert_eq!(
            relationship_href("7d44aa01::sys::1"),
            "/parties/relationships/7d44aa01"
        );
        // A contribution uid is NOT a version id — it keeps every character.
        assert_eq!(contribution_href(" c9 "), "/parties/contributions/c9");
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
        // path (every route is a public HTTP endpoint).
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
