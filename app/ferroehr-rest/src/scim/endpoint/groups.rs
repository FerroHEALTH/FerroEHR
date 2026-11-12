// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `/Groups`: create, read, query, replace, modify and delete the Groups of
//! the requesting client (RFC 7644 §3.3, §3.4.1, §3.4.2, §3.5.1, §3.5.2,
//! §3.6).
//!
//! A client sees only the Groups it created: every other id answers `404`
//! and is absent from every list. A query's filter is `eq` and `and` over
//! `displayName`, `externalId` and `id`, and `excludedAttributes=members`
//! leaves the members out, the request Microsoft Entra ID sends (RFC 7644
//! §3.4.2.5). A PATCH answers `204` with the new entity tag (§3.5.2: "the
//! server MAY return HTTP status code 204"), as Microsoft's Group Operations
//! advise, since a Group's members can number thousands. A `remove` of
//! `members` with a value list removes only the listed members, the Entra
//! deviation the protocol layer labels. Each request is an access record of
//! the client with the `GROUP` as its object. No openEHR spec governs SCIM:
//! our own design over RFC 7644.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::response::Response;
use ferroehr::config::scim::ScimClient;
use ferroehr::ids::VoId;
use ferroehr::party_binding::provisioned::GroupFilter;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::group_provision::{GroupPage, ProvisionedGroup};
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use serde_json::Value;

use crate::scim::endpoint::group_map::{self, GroupMap};
use crate::scim::endpoint::listing::{
    ListQuery, Listing, ReadQuery, audited, equalities, json_body, listed,
};
use crate::scim::endpoint::user_map;
use crate::scim::endpoint::{ScimCaller, refusal, scim_json};
use crate::scim::protocol::error::ScimError;
use crate::scim::protocol::etag::{self, ReadPrecondition};
use crate::scim::protocol::filter::Filter;
use crate::scim::protocol::list::Projection;
use crate::scim::protocol::patch::PatchRequest;
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::resource::{Resource, ResourceSchema};
use crate::scim::protocol::urn;

/// What the `/Groups` handlers share.
#[derive(Clone)]
pub(crate) struct Groups {
    service: Arc<FerroEhrService>,
    inner: Arc<Inner>,
}

/// The configuration the handlers read.
struct Inner {
    map: GroupMap,
    listing: Listing,
}

impl Groups {
    /// The handlers' state over `service`.
    ///
    /// # Errors
    /// The refusal naming what the schema lacks.
    pub(crate) fn new(
        service: Arc<FerroEhrService>,
        map: GroupMap,
        schema: ResourceSchema,
        scim: &ferroehr::config::scim::ScimConfig,
        base_url: &str,
    ) -> Result<Self, String> {
        let listing = Listing::new(
            schema,
            scim,
            base_url,
            urn::CORE_GROUP,
            &["displayName", "externalId", "id"],
        )?;
        Ok(Self {
            service,
            inner: Arc::new(Inner { map, listing }),
        })
    }
}

/// `POST /Groups` (RFC 7644 §3.3).
pub(crate) async fn create(
    State(groups): State<Groups>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    body: Bytes,
) -> Response {
    let outcome = async {
        let parsed = groups.inner.listing.requested(&body)?;
        let state = group_map::state(parsed.resource())?;
        groups
            .service
            .provision_group(groups.inner.map.writer(&client), state)
            .await
            .map_err(|error| user_map::service_refusal(&error.classified()))
    }
    .await;
    match outcome {
        Ok(group) => {
            let response = served(&groups, &group, StatusCode::CREATED, &Projection::default());
            let response = groups
                .inner
                .listing
                .located(response, "Groups", group.row.party);
            audited(response, "scim_group_create", Some(group.row.party), None)
        }
        Err(error) => audited(refusal(&error), "scim_group_create", None, None),
    }
}

/// `GET /Groups/{id}` (RFC 7644 §3.4.1), answering `304` to an
/// `If-None-Match` naming the current version (§3.14).
pub(crate) async fn read(
    State(groups): State<Groups>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
    headers: HeaderMap,
) -> Response {
    let found = match find(&groups, &client, &id).await {
        Ok(group) => query.projection().map(|projection| (group, projection)),
        Err(error) => Err(error),
    };
    match found {
        Ok((group, projection)) => {
            let party = group.row.party;
            if let Some(tag) = group_map::entity_tag(&group)
                && etag::if_none_match_read(&headers, &tag) == ReadPrecondition::NotModified
            {
                let response = with_etag(no_body(StatusCode::NOT_MODIFIED), &group);
                return audited(response, "scim_group_get", Some(party), None);
            }
            let response = served(&groups, &group, StatusCode::OK, &projection);
            audited(response, "scim_group_get", Some(party), None)
        }
        Err(error) => audited(refusal(&error), "scim_group_get", None, None),
    }
}

/// `GET /Groups` (RFC 7644 §3.4.2).
pub(crate) async fn list(
    State(groups): State<Groups>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Query(query): Query<ListQuery>,
) -> Response {
    let outcome = async {
        let (page, shown, parsed) = groups.inner.listing.asked(&query)?;
        let filter = match parsed {
            Some(parsed) => criteria(&parsed)?,
            None => Some(GroupFilter::default()),
        };
        let mut matched = GroupPage {
            total: 0,
            groups: Vec::new(),
        };
        if let Some(filter) = filter {
            let client = groups.inner.map.client(&client);
            matched = groups
                .service
                .provisioned_groups(client, &filter, page.offset(), page.count())
                .await
                .map_err(|error| user_map::service_refusal(&error.classified()))?;
        }
        Ok((page, shown, matched))
    }
    .await;
    match outcome {
        Ok((page, shown, matched)) => {
            let ids: Vec<VoId> = matched.groups.iter().map(|group| group.row.party).collect();
            let resources: Vec<Value> = matched
                .groups
                .iter()
                .map(|group| represented(&groups, group, &shown))
                .collect();
            listed(resources, (matched.total, &page), "scim_group_list", &ids)
        }
        Err(error) => audited(refusal(&error), "scim_group_list", None, None),
    }
}

/// `PUT /Groups/{id}` (RFC 7644 §3.5.1) and `PATCH /Groups/{id}` (§3.5.2).
///
/// A PUT replaces the `displayName` and the members and keeps an omitted
/// `externalId`; a PATCH applies each operation to the current Group.
/// Either commits the result as one version of the `GROUP`.
pub(crate) async fn modify(
    State(groups): State<Groups>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let patching = method == Method::PATCH;
    let outcome = async {
        let body = json_body(&body)?;
        let request = patching.then(|| PatchRequest::parse(&body)).transpose()?;
        let group = find(&groups, &client, &id).await?;
        if let Some(tag) = group_map::entity_tag(&group) {
            etag::if_match(&headers, &tag)?;
        }
        let schema = &groups.inner.listing.schema;
        let current = Resource::from_representation(
            &group_map::representation(&group, &groups.inner.listing.base_url),
            schema,
        )?;
        let after = if let Some(request) = request {
            request.apply(&current, schema)?.into_resource()
        } else {
            let keep = [AttrPath::parse("externalId").map_err(|error| {
                ScimError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            })?];
            current.replace(&body, schema, &keep)?.into_resource()
        };
        let state = group_map::state(&after)?;
        groups
            .service
            .update_group(
                groups.inner.map.writer(&client),
                group.row.party,
                state,
                Some(&group.version_uid),
            )
            .await
            .map_err(|error| user_map::service_refusal(&error.classified()))?
            .ok_or_else(|| missing(&id))
    }
    .await;
    let op = if patching {
        "scim_group_patch"
    } else {
        "scim_group_replace"
    };
    match outcome {
        Ok(group) => {
            let party = group.row.party;
            let response = if patching {
                with_etag(no_body(StatusCode::NO_CONTENT), &group)
            } else {
                served(&groups, &group, StatusCode::OK, &Projection::default())
            };
            audited(response, op, Some(party), None)
        }
        Err(error) => audited(refusal(&error), op, None, None),
    }
}

/// `DELETE /Groups/{id}` (RFC 7644 §3.6): the Group's `GROUP` is logically
/// deleted with its memberships, and every later request on the id is
/// `404`.
pub(crate) async fn delete(
    State(groups): State<Groups>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let outcome = async {
        let group = find(&groups, &client, &id).await?;
        if let Some(tag) = group_map::entity_tag(&group) {
            etag::if_match(&headers, &tag)?;
        }
        let party = group.row.party;
        let deleted = groups
            .service
            .delete_group(
                groups.inner.map.writer(&client),
                party,
                Some(&group.version_uid),
            )
            .await
            .map_err(|error| user_map::service_refusal(&error.classified()))?;
        if deleted {
            Ok(party)
        } else {
            Err(missing(&id))
        }
    }
    .await;
    match outcome {
        Ok(party) => audited(
            no_body(StatusCode::NO_CONTENT),
            "scim_group_delete",
            Some(party),
            None,
        ),
        Err(error) => audited(refusal(&error), "scim_group_delete", None, None),
    }
}

/// The `404` of an id naming no Group of the client (RFC 7644 §3.12).
fn missing(id: &str) -> ScimError {
    ScimError::new(StatusCode::NOT_FOUND, format!("Resource {id} not found."))
}

/// The Group `id` of `client`, or the `404` of an id naming none.
async fn find(
    groups: &Groups,
    client: &ScimClient,
    id: &str,
) -> Result<ProvisionedGroup, ScimError> {
    // NOTE: a Group's id is its container's UUID, so any other id names none.
    let Ok(party) = id.parse::<uuid::Uuid>() else {
        return Err(missing(id));
    };
    groups
        .service
        .provisioned_group(groups.inner.map.client(client), VoId(party))
        .await
        .map_err(|error| user_map::service_refusal(&error.classified()))?
        .ok_or_else(|| missing(id))
}

/// The side-table criteria of the supported filter `parsed`: `None` when an
/// `id` it compares with is no UUID, so nothing matches.
///
/// # Errors
/// A `400` `invalidFilter` when it compares with a value that is not a
/// string.
fn criteria(parsed: &Filter) -> Result<Option<GroupFilter>, ScimError> {
    let mut filter = GroupFilter::default();
    for (name, value) in equalities(parsed)? {
        if name.eq_ignore_ascii_case("displayName") {
            filter.display_names.push(value.to_owned());
        } else if name.eq_ignore_ascii_case("externalId") {
            filter.external_ids.push(value.to_owned());
        } else {
            // NOTE: a Group's id is its container's UUID, so any other id matches none.
            let Ok(id) = value.parse::<uuid::Uuid>() else {
                return Ok(None);
            };
            filter.ids.push(id);
        }
    }
    Ok(Some(filter))
}

/// `group` as the response `status`, with its `ETag` (RFC 7644 §3.14).
fn served(
    groups: &Groups,
    group: &ProvisionedGroup,
    status: StatusCode,
    projection: &Projection,
) -> Response {
    with_etag(
        scim_json(status, &represented(groups, group, projection)),
        group,
    )
}

/// The representation of `group` under `projection`.
fn represented(groups: &Groups, group: &ProvisionedGroup, projection: &Projection) -> Value {
    let listing = &groups.inner.listing;
    let full = group_map::representation(group, &listing.base_url);
    listing.projected(full, projection, "Group")
}

/// An empty response of `status`.
fn no_body(status: StatusCode) -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = status;
    response
}

/// `response` with `group`'s entity tag.
fn with_etag(mut response: Response, group: &ProvisionedGroup) -> Response {
    if let Some(tag) = group_map::entity_tag(group)
        && let Ok(value) = HeaderValue::from_str(&tag.to_string())
    {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}
