// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `/Users`: create, read and query the Users of the requesting client
//! (RFC 7644 §3.3, §3.4.1, §3.4.2).
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
use ferroehr::system_log::event::AccessDomain;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Deserialize;
use serde_json::Value;

use crate::scim::endpoint::user_map::{self, UserMap};
use crate::scim::endpoint::{ScimCaller, refusal, scim_json};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::etag::{self, ReadPrecondition};
use crate::scim::protocol::filter::{
    self, CompValue, CompareOp, Filter, FilterSupport, Logic, Term, Test,
};
use crate::scim::protocol::list::{ListResponse, Page, Projection};
use crate::scim::protocol::resource::{Resource, ResourceSchema};
use crate::scim::protocol::urn;
use crate::system_log::middleware::{AuditObject, AuditOpId};

/// What the `/Users` handlers share.
#[derive(Clone)]
pub(crate) struct Users {
    service: Arc<FerroEhrService>,
    inner: Arc<Inner>,
}

/// The configuration the handlers read.
struct Inner {
    map: UserMap,
    schema: ResourceSchema,
    base_url: String,
    max_results: u64,
    filter: FilterSupport,
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
        base_url: String,
        max_results: u64,
    ) -> Result<Self, String> {
        let filter =
            FilterSupport::equality_conjunction(urn::CORE_USER, &["userName", "externalId", "id"])
                .map_err(|error| error.to_string())?;
        Ok(Self {
            service,
            inner: Arc::new(Inner {
                map,
                schema,
                base_url,
                max_results,
                filter,
            }),
        })
    }
}

/// The query of `GET /Users` (RFC 7644 §3.4.2).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    filter: Option<String>,
    start_index: Option<String>,
    count: Option<String>,
    attributes: Option<String>,
    excluded_attributes: Option<String>,
}

/// The attribute projection of `GET /Users/{id}` (RFC 7644 §3.9).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadQuery {
    attributes: Option<String>,
    excluded_attributes: Option<String>,
}

/// `POST /Users` (RFC 7644 §3.3).
pub(crate) async fn create(
    State(users): State<Users>,
    Extension(ScimCaller(client)): Extension<ScimCaller>,
    body: Bytes,
) -> Response {
    let outcome = async {
        let body: Value = serde_json::from_slice(&body).map_err(|_error| {
            ScimError::bad_request(ScimType::InvalidSyntax, "The body is not a JSON object.")
        })?;
        let parsed = Resource::from_request(&body, &users.inner.schema)?;
        let resource = parsed.resource();
        // NOTE: RM common master06 §Version Lifecycle, a first version is complete or incomplete.
        if resource.get(crate::scim::protocol::resource::Container::Core, "active")
            == Some(&Value::Bool(false))
        {
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
            let mut response = served(&users, &user, StatusCode::CREATED, &Projection::default());
            if let Ok(location) = HeaderValue::from_str(&format!(
                "{}/Users/{}",
                users.inner.base_url, user.row.party
            )) {
                response.headers_mut().insert(header::LOCATION, location);
            }
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
        let projection = Projection::parse(
            query.attributes.as_deref(),
            query.excluded_attributes.as_deref(),
        )?;
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
            let response = served(&users, &user, StatusCode::OK, &projection);
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
        let page = Page::parse(
            query.start_index.as_deref(),
            query.count.as_deref(),
            users.inner.max_results,
        )?;
        let projection = Projection::parse(
            query.attributes.as_deref(),
            query.excluded_attributes.as_deref(),
        )?;
        let filter = match query.filter.as_deref() {
            Some(text) => {
                let parsed = filter::parse(text)?;
                users.inner.filter.check(&parsed)?;
                criteria(&parsed)?
            }
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
            let only = match found.users.as_slice() {
                [one] => Some(one.row.party),
                _ => None,
            };
            let count = u64::try_from(found.users.len()).unwrap_or(u64::MAX);
            let resources: Vec<Value> = found
                .users
                .iter()
                .map(|user| represented(&users, user, &projection))
                .collect();
            let response = scim_json(
                StatusCode::OK,
                &ListResponse::new(resources, found.total, page.start_index()),
            );
            audited(response, "scim_user_list", only, Some(count))
        }
        Err(error) => audited(refusal(&error), "scim_user_list", None, None),
    }
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
    let mut leaves = Vec::new();
    collect(parsed, &mut leaves);
    for (name, value) in leaves {
        let Some(value) = value else {
            return Err(ScimError::bad_request(
                ScimType::InvalidFilter,
                format!("'{name}' is compared with a string."),
            ));
        };
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

/// The `eq` comparisons of the supported filter `parsed`, each its attribute
/// name and its string value.
fn collect<'a>(parsed: &'a Filter, leaves: &mut Vec<(&'a str, Option<&'a str>)>) {
    match parsed {
        Logic::Leaf(Term::Attr(expression)) => {
            let value = match &expression.test {
                Test::Compare(CompareOp::Eq, CompValue::String(value)) => Some(value.as_str()),
                _ => None,
            };
            leaves.push((expression.path.name.as_str(), value));
        }
        Logic::And(parts) => {
            for part in parts {
                collect(part, leaves);
            }
        }
        // NOTE: `FilterSupport::check` admitted only `eq` and `and`.
        Logic::Leaf(Term::ValuePath(..)) | Logic::Or(_) | Logic::Not(_) => {}
    }
}

/// `user` as the response `status`, with its `ETag` (RFC 7644 §3.14).
fn served(
    users: &Users,
    user: &ProvisionedUser,
    status: StatusCode,
    projection: &Projection,
) -> Response {
    let mut response = scim_json(status, &represented(users, user, projection));
    set_etag(&mut response, user);
    response
}

/// The representation of `user` under `projection`.
fn represented(users: &Users, user: &ProvisionedUser, projection: &Projection) -> Value {
    let full = users.inner.map.representation(user, &users.inner.base_url);
    match Resource::from_representation(&full, &users.inner.schema) {
        Ok(resource) => projection.apply(&resource, &users.inner.schema),
        Err(error) => {
            tracing::error!(%error, "a User representation does not meet its schema");
            full
        }
    }
}

/// Puts `user`'s entity tag on `response`.
fn set_etag(response: &mut Response, user: &ProvisionedUser) {
    if let Some(tag) = user_map::entity_tag(user)
        && let Ok(value) = HeaderValue::from_str(&tag.to_string())
    {
        response.headers_mut().insert(header::ETAG, value);
    }
}

/// `response` with the access record's operation and object: the `PERSON`
/// `party`, and the number of Users a query served.
fn audited(
    mut response: Response,
    op: &'static str,
    party: Option<VoId>,
    result_count: Option<u64>,
) -> Response {
    response.extensions_mut().insert(AuditOpId(op));
    response.extensions_mut().insert(AuditObject {
        uid: party.map(|party| party.to_string()),
        result_count,
        domain: Some(AccessDomain::Demographic),
        ..AuditObject::default()
    });
    response
}
