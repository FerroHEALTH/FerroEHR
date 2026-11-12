// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `/Users`: create, read, query, replace, modify and delete the Users of
//! the requesting client (RFC 7644 §3.3, §3.4.1, §3.4.2, §3.5.1, §3.5.2,
//! §3.6).
//!
//! A client sees only its own Users: every other id answers `404` and is
//! absent from every list. A query is always a `ListResponse`, an empty one
//! with `200` when nothing matches (§3.4.2), and its filter is `eq` and
//! `and` over `userName`, `externalId` and `id`, the subset
//! `ServiceProviderConfig` declares; anything else is `400` `invalidFilter`
//! (§3.4.2.2). Each request is an access record of the client with the
//! `PERSON` as its object. No openEHR spec governs SCIM: our own design over
//! RFC 7644.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::response::Response;
use ferroehr::ids::VoId;
use ferroehr::party_binding::provisioned::UserFilter;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::provision::{NewUser, ProvisionedUser};
use ferroehr::service::demographic::user_write::{Deactivations, UserUpdate, UserWriteError};
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use serde_json::Value;

use crate::scim::endpoint::listing::{
    ListQuery, Listing, ReadQuery, audited, equalities, json_body, listed,
};
use crate::scim::endpoint::user_map::{self, UserMap};
use crate::scim::endpoint::{ScimCaller, refusal, scim_json};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::etag::{self, ReadPrecondition};
use crate::scim::protocol::filter::Filter;
use crate::scim::protocol::list::Projection;
use crate::scim::protocol::patch::PatchRequest;
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::resource::{Container, Resource, ResourceSchema};
use crate::scim::protocol::urn;

/// What the `/Users` handlers share.
#[derive(Clone)]
pub(crate) struct Users {
    service: Arc<FerroEhrService>,
    inner: Arc<Inner>,
}

/// The configuration the handlers read.
struct Inner {
    map: UserMap,
    listing: Listing,
    limit: Deactivations,
    contact_erasure_minutes: u32,
}

impl Users {
    /// The handlers' state over `service`.
    ///
    /// # Errors
    /// The refusal naming what the mapping or the schema lacks.
    pub(crate) fn new(
        service: Arc<FerroEhrService>,
        map: UserMap,
        schema: ResourceSchema,
        scim: &ferroehr::config::scim::ScimConfig,
        base_url: &str,
    ) -> Result<Self, String> {
        let listing = Listing::new(
            schema,
            scim,
            base_url,
            urn::CORE_USER,
            &["userName", "externalId", "id"],
        )?;
        Ok(Self {
            service,
            inner: Arc::new(Inner {
                map,
                listing,
                limit: Deactivations {
                    max: scim.max_deactivations,
                    window_minutes: scim.window_minutes(),
                },
                contact_erasure_minutes: scim.erasure_minutes(),
            }),
        })
    }
}

/// `POST /Users` (RFC 7644 §3.3).
pub(crate) async fn create(
    State(users): State<Users>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    body: Bytes,
) -> Response {
    let outcome = async {
        let parsed = users.inner.listing.requested(&body)?;
        let resource = parsed.resource();
        // NOTE: RM common master06 §Version Lifecycle, a first version is complete or incomplete.
        if resource.get(Container::Core, "active") == Some(&Value::Bool(false)) {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                "A User is created active: a first version cannot be inactive.",
            ));
        }
        let map = &users.inner.map;
        let values = map
            .values(&users.service, resource, &client.binding_issuer)
            .await?;
        let user = NewUser {
            template_id: map.template_id().to_owned(),
            namespace: map.namespace().to_owned(),
            values,
        };
        users
            .service
            .provision_user(&client.binding_issuer, map.role_template(), user)
            .await
            .map_err(|error| user_map::service_refusal(&error.classified()))
    }
    .await;
    match outcome {
        Ok(user) => {
            let response = served(
                &users,
                &user,
                &client.binding_issuer,
                StatusCode::CREATED,
                &Projection::default(),
            );
            let response = users
                .inner
                .listing
                .located(response, "Users", user.row.party);
            audited(response, "scim_user_create", Some(user.row.party), None)
        }
        Err(error) => audited(refusal(&error), "scim_user_create", None, None),
    }
}

/// `GET /Users/{id}` (RFC 7644 §3.4.1), answering `304` to an
/// `If-None-Match` naming the current version (§3.14).
pub(crate) async fn read(
    State(users): State<Users>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
    headers: HeaderMap,
) -> Response {
    let outcome = async {
        let projection = query.projection()?;
        let user = find(&users, &client.binding_issuer, &id).await?;
        Ok((user, projection))
    }
    .await;
    match outcome {
        Ok((user, projection)) => {
            let party = user.row.party;
            if let Some(tag) = user_map::entity_tag(&user)
                && etag::if_none_match_read(&headers, &tag) == ReadPrecondition::NotModified
            {
                let mut response = Response::new(axum::body::Body::empty());
                *response.status_mut() = StatusCode::NOT_MODIFIED;
                set_etag(&mut response, &user);
                return audited(response, "scim_user_get", Some(party), None);
            }
            let response = served(
                &users,
                &user,
                &client.binding_issuer,
                StatusCode::OK,
                &projection,
            );
            audited(response, "scim_user_get", Some(party), None)
        }
        Err(error) => audited(refusal(&error), "scim_user_get", None, None),
    }
}

/// `GET /Users` (RFC 7644 §3.4.2).
pub(crate) async fn list(
    State(users): State<Users>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Query(query): Query<ListQuery>,
) -> Response {
    let outcome = async {
        let (page, projection, parsed) = users.inner.listing.asked(&query)?;
        let filter = match parsed {
            Some(parsed) => criteria(&parsed)?,
            None => Some(UserFilter::default()),
        };
        let found = match filter {
            Some(filter) => users
                .service
                .provisioned_users(&client.binding_issuer, &filter, page.offset(), page.count())
                .await
                .map_err(|error| user_map::service_refusal(&error.classified()))?,
            None => ferroehr::service::demographic::provision::UserPage {
                total: 0,
                users: Vec::new(),
            },
        };
        Ok((page, projection, found))
    }
    .await;
    match outcome {
        Ok((page, projection, found)) => {
            let ids: Vec<VoId> = found.users.iter().map(|user| user.row.party).collect();
            let resources: Vec<Value> = found
                .users
                .iter()
                .map(|user| represented(&users, user, &client.binding_issuer, &projection))
                .collect();
            listed(resources, (found.total, &page), "scim_user_list", &ids)
        }
        Err(error) => audited(refusal(&error), "scim_user_list", None, None),
    }
}

/// `PUT /Users/{id}` (RFC 7644 §3.5.1) and `PATCH /Users/{id}` (§3.5.2).
///
/// A PUT clears a mapped attribute the body omits, except `externalId`, and
/// an omitted `active` leaves the lifecycle state as it is. A PATCH applies
/// each operation to the current User. Either commits the result as one
/// version.
pub(crate) async fn modify(
    State(users): State<Users>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let issuer = client.binding_issuer.as_str();
    let patching = method == Method::PATCH;
    let outcome = async {
        let body = json_body(&body)?;
        let request = patching.then(|| PatchRequest::parse(&body)).transpose()?;
        let (user, current) = current(&users, issuer, &id, &headers).await?;
        let active_of = |resource: &Resource| {
            resource
                .get(Container::Core, "active")
                .and_then(Value::as_bool)
        };
        let (after, active) = if let Some(request) = request {
            let patched = request
                .apply(&current, &users.inner.listing.schema)?
                .into_resource();
            let after = active_of(&patched);
            let active = (after != active_of(&current)).then_some(after).flatten();
            (patched, active)
        } else {
            let keep = [AttrPath::parse("externalId").map_err(|error| {
                ScimError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            })?];
            let replaced = current
                .replace(&body, &users.inner.listing.schema, &keep)?
                .into_resource();
            let active = active_of(&replaced);
            (replaced, active)
        };
        write(&users, issuer, user, (&current, &after), active).await
    }
    .await;
    let op = if patching {
        "scim_user_patch"
    } else {
        "scim_user_replace"
    };
    match outcome {
        Ok(user) => {
            let party = user.row.party;
            let response = served(
                &users,
                &user,
                issuer,
                StatusCode::OK,
                &Projection::default(),
            );
            audited(response, op, Some(party), None)
        }
        Err(error) => refused(&users, &error, op),
    }
}

/// The User `id` of the client of `issuer` with its current representation,
/// once an `If-Match` the request carries names its version (RFC 7644
/// §3.14).
async fn current(
    users: &Users,
    issuer: &str,
    id: &str,
    headers: &HeaderMap,
) -> Result<(ProvisionedUser, Resource), ScimError> {
    let user = find(users, issuer, id).await?;
    if let Some(tag) = user_map::entity_tag(&user) {
        etag::if_match(headers, &tag)?;
    }
    let current = Resource::from_representation(
        &users
            .inner
            .map
            .representation(&user, &users.inner.listing.base_url, issuer),
        &users.inner.listing.schema,
    )?;
    Ok((user, current))
}

/// Commits the User `user` going from `old` to `new` with the request's
/// `active`, returning the User after.
async fn write(
    users: &Users,
    issuer: &str,
    user: ProvisionedUser,
    (old, new): (&Resource, &Resource),
    active: Option<bool>,
) -> Result<ProvisionedUser, ScimError> {
    let (set, clear) = users
        .inner
        .map
        .changes(&users.service, &user.body, (old, new), issuer)
        .await?;
    let update = UserUpdate {
        set,
        clear,
        active,
        expected: Some(user.version_uid.clone()),
    };
    let missing = || ScimError::new(StatusCode::NOT_FOUND, "Resource not found.");
    users
        .service
        .update_user(issuer, user.row.party, update, users.inner.limit)
        .await
        .map_err(write_refusal)?
        .ok_or_else(missing)
}

/// `DELETE /Users/{id}` (RFC 7644 §3.6): the User's `PERSON` and its
/// professional `ROLE` are logically deleted, and every later request on the
/// id is `404`.
pub(crate) async fn delete(
    State(users): State<Users>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let issuer = client.binding_issuer.as_str();
    let outcome = async {
        let user = find(&users, issuer, &id).await?;
        if let Some(tag) = user_map::entity_tag(&user) {
            etag::if_match(&headers, &tag)?;
        }
        let party = user.row.party;
        let deleted = users
            .service
            .delete_user(
                issuer,
                party,
                Some(&user.version_uid),
                users.inner.limit,
                users.inner.contact_erasure_minutes,
            )
            .await
            .map_err(write_refusal)?;
        if deleted {
            Ok(party)
        } else {
            Err(ScimError::new(
                StatusCode::NOT_FOUND,
                format!("Resource {id} not found."),
            ))
        }
    }
    .await;
    match outcome {
        Ok(party) => {
            let mut response = Response::new(axum::body::Body::empty());
            *response.status_mut() = StatusCode::NO_CONTENT;
            audited(response, "scim_user_delete", Some(party), None)
        }
        Err(error) => refused(&users, &error, "scim_user_delete"),
    }
}

/// The SCIM error a refused User write answers: `429` past
/// `max_deactivations` (RFC 6585 §4), and the demographic refusal otherwise.
fn write_refusal(error: UserWriteError) -> ScimError {
    match error {
        UserWriteError::Service(error) => user_map::service_refusal(&error.classified()),
        limited @ UserWriteError::Limited { .. } => {
            ScimError::new(StatusCode::TOO_MANY_REQUESTS, limited.to_string())
        }
    }
}

/// Puts the window of `limit` on a `429` as its `Retry-After` (RFC 6585 §4),
/// the longest the client waits for the window to end.
fn retry_after(response: &mut Response, limit: Deactivations) {
    let seconds = u64::from(limit.window_minutes).saturating_mul(60);
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
}

/// The refusal `error` of a write, with the `Retry-After` of a `429` and
/// the access record of `op`.
fn refused(users: &Users, error: &ScimError, op: &'static str) -> Response {
    let mut response = refusal(error);
    if error.status() == StatusCode::TOO_MANY_REQUESTS {
        retry_after(&mut response, users.inner.limit);
    }
    audited(response, op, None, None)
}

/// The User `id` of the client of `issuer`, or the `404` of an id naming
/// none (RFC 7644 §3.12).
async fn find(users: &Users, issuer: &str, id: &str) -> Result<ProvisionedUser, ScimError> {
    let missing = || ScimError::new(StatusCode::NOT_FOUND, format!("Resource {id} not found."));
    // NOTE: a User's id is its container's UUID, so any other id names none.
    let Ok(party) = id.parse::<uuid::Uuid>() else {
        return Err(missing());
    };
    users
        .service
        .provisioned_user(issuer, VoId(party))
        .await
        .map_err(|error| user_map::service_refusal(&error.classified()))?
        .ok_or_else(missing)
}

/// The side-table criteria of the supported filter `parsed`: `None` when
/// an `id` it compares with is no UUID, so nothing matches.
///
/// # Errors
/// A `400` `invalidFilter` when it compares with a value that is not a
/// string.
fn criteria(parsed: &Filter) -> Result<Option<UserFilter>, ScimError> {
    let mut filter = UserFilter::default();
    for (name, value) in equalities(parsed)? {
        if name.eq_ignore_ascii_case("userName") {
            filter.user_names.push(value.to_owned());
        } else if name.eq_ignore_ascii_case("externalId") {
            filter.external_ids.push(value.to_owned());
        } else {
            // NOTE: a User's id is its container's UUID, so any other id matches none.
            let Ok(id) = value.parse::<uuid::Uuid>() else {
                return Ok(None);
            };
            filter.ids.push(id);
        }
    }
    Ok(Some(filter))
}

/// `user` as the response `status`, with its `ETag` (RFC 7644 §3.14).
fn served(
    users: &Users,
    user: &ProvisionedUser,
    issuer: &str,
    status: StatusCode,
    projection: &Projection,
) -> Response {
    let mut response = scim_json(status, &represented(users, user, issuer, projection));
    set_etag(&mut response, user);
    response
}

/// The representation of `user` under `projection`.
fn represented(
    users: &Users,
    user: &ProvisionedUser,
    issuer: &str,
    projection: &Projection,
) -> Value {
    let full = users
        .inner
        .map
        .representation(user, &users.inner.listing.base_url, issuer);
    users.inner.listing.projected(full, projection, "User")
}

/// Puts `user`'s entity tag on `response`.
fn set_etag(response: &mut Response, user: &ProvisionedUser) {
    if let Some(tag) = user_map::entity_tag(user)
        && let Ok(value) = HeaderValue::from_str(&tag.to_string())
    {
        response.headers_mut().insert(header::ETAG, value);
    }
}
