// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FerroEHR's SCIM endpoint at `/scim/v2/`: the routes, the gate that admits
//! only a declared provisioning client, and the discovery documents.
//!
//! `/ServiceProviderConfig` is readable without a token, so a client can learn
//! the authentication scheme before it holds one (RFC 7644 §4,
//! `authenticationSchemes`). Every other path needs a bearer token of a
//! `[[auth.scim.client]]`: a caller acting for nobody (a `System` caller), whose
//! issuer and client the table names, holding the table's `scope`. A person's
//! token is refused, since provisioning is a client's act. The client's
//! requests count against `[server.caller_rate]` like any other caller's. No
//! openEHR spec governs SCIM: our own design over RFC 7644.

#[expect(
    clippy::disallowed_types,
    reason = "SCIM resource content (#1694 boundary class): a Group is the RFC 7644 §3.8 JSON \
              representation of an attribute-based schema, built from the GROUP's canonical body"
)]
mod group_map;
#[expect(
    clippy::disallowed_types,
    reason = "SCIM resource content (#1694 boundary class): the RFC 7644 §3.8 JSON \
              representation a request carries and a response serves"
)]
mod groups;
#[expect(
    clippy::disallowed_types,
    reason = "SCIM resource content (#1694 boundary class): the RFC 7644 §3.8 JSON \
              representation a request carries and a response serves"
)]
mod listing;
pub(crate) mod schemas;
#[expect(
    clippy::disallowed_types,
    reason = "SCIM resource content (#1694 boundary class): a User is the RFC 7644 §3.8 JSON \
              representation of an attribute-based schema, built from the PERSON's canonical body"
)]
mod user_map;
#[expect(
    clippy::disallowed_types,
    reason = "SCIM resource content (#1694 boundary class): the RFC 7644 §3.8 JSON \
              representation a request carries and a response serves"
)]
mod users;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ferroehr::config::scim::ScimClient;
use ferroehr::service::FerroEhrService;
use ferrohealth_server::caller_rate::{self, CallerLimit};
use ferrohealth_smart::kind::CallerKind;
use http::{HeaderValue, StatusCode, header};
use serde::Deserialize;

use crate::config::AppConfig;
use crate::extensions::access::authn::{self, AuthError, Authenticator};
use crate::scim::protocol::MEDIA_TYPE;
use crate::scim::protocol::discovery::{self, Discovery, DiscoveryError};
use crate::scim::protocol::error::ScimError;
use crate::state::AppState;

/// The path the endpoint is served under.
pub const BASE_PATH: &str = "/scim/v2";

/// The provisioning client a request was admitted as, on its extensions.
#[derive(Debug, Clone)]
pub struct ScimCaller(pub Arc<ScimClient>);

/// What the gate admits against.
#[derive(Clone)]
struct Gate {
    authenticator: Arc<Authenticator>,
    clients: Arc<[Arc<ScimClient>]>,
}

/// Why a SCIM endpoint cannot be served.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// A mapped attribute is no attribute of its SCIM schema.
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// The party mapping lacks what a User is written with.
    #[error("{0}")]
    Mapping(String),
}

/// The SCIM base URL every `meta.location` and discovery document names:
/// [`BASE_PATH`] under the server's public root (`[server] public_base_url`
/// and `server.base_path`; RFC 7643 §3.1: `location` is "The URI of the
/// resource being returned"). Boot validation requires the root with
/// `[auth.scim]`.
fn base_url(config: &AppConfig) -> String {
    config
        .public_root
        .as_ref()
        .map_or_else(String::new, |root| root.at(BASE_PATH))
}

/// Checks that `config` describes a SCIM endpoint that can be served.
///
/// # Errors
/// The [`EndpointError`] of a mapped attribute the schemas do not define, or
/// of a party mapping that lacks what a User is written with.
pub fn check(config: &AppConfig) -> Result<(), EndpointError> {
    if let Some(scim) = &config.auth.scim {
        schemas::discovery(scim, base_url(config), &config.party_binding)?;
        user_map::UserMap::new(&config.party_binding).map_err(EndpointError::Mapping)?;
        group_map::GroupMap::new(config, scim).map_err(EndpointError::Mapping)?;
    }
    Ok(())
}

/// The endpoint's routes under [`BASE_PATH`], or `None` when `[auth.scim]` is absent.
///
/// [`check`] has accepted the configuration at boot; a configuration it
/// would refuse mounts nothing and says so in the log.
pub(crate) fn router(
    config: &AppConfig,
    authenticator: &Arc<Authenticator>,
    service: Arc<FerroEhrService>,
) -> Option<Router<AppState>> {
    let scim = config.auth.scim.as_ref()?;
    let discovery = match schemas::discovery(scim, base_url(config), &config.party_binding) {
        Ok(discovery) => Arc::new(discovery),
        Err(error) => {
            tracing::error!(%error, "the SCIM endpoint is not mounted: its schemas do not build");
            return None;
        }
    };
    // NOTE: GDPR Art. 5(1)(e), a deleted User's contacts are kept no longer than necessary;
    // the sweep runs while the endpoint that deletes Users is served.
    service.spawn_contact_erasure(
        scim.binding_issuers(),
        scim.erasure_minutes(),
        Duration::from_secs(60),
    );
    let resources = match resources(config, scim, &discovery, service) {
        Ok(resources) => resources,
        Err(error) => {
            tracing::error!(%error, "the SCIM endpoint is not mounted: its resource mapping does not build");
            return None;
        }
    };
    let gate = Gate {
        authenticator: Arc::clone(authenticator),
        clients: scim.client.iter().cloned().map(Arc::new).collect(),
    };
    let gated: Router<AppState> = Router::new()
        .route("/ResourceTypes", get(resource_types))
        .route("/ResourceTypes/{id}", get(resource_type))
        .route("/Schemas", get(schema_list))
        .route("/Schemas/{id}", get(schema))
        .with_state(Arc::clone(&discovery))
        .merge(resources);
    let gated = match config.server.caller_rate {
        Some(rate) => gated
            .route_layer(from_fn_with_state(
                CallerLimit::new(rate),
                caller_rate::limit,
            ))
            .route_layer(from_fn(crate::router::caller_key)),
        None => gated,
    };
    let gated = gated.route_layer(from_fn_with_state(gate, admit));
    let open: Router<AppState> = Router::new()
        .route("/ServiceProviderConfig", get(service_provider_config))
        .with_state(discovery);
    Some(Router::new().nest(
        BASE_PATH,
        open.merge(gated).fallback(|| async {
            refusal(&ScimError::new(
                StatusCode::NOT_FOUND,
                "no SCIM resource is served at this path",
            ))
        }),
    ))
}

/// The `/Users` routes, with the `/Groups` routes when the party mapping
/// maps a group.
///
/// # Errors
/// The refusal naming what the User or the Group mapping lacks.
fn resources(
    config: &AppConfig,
    scim: &ferroehr::config::scim::ScimConfig,
    discovery: &Discovery,
    service: Arc<FerroEhrService>,
) -> Result<Router<AppState>, String> {
    let schema = |name: &str| {
        discovery
            .resource_schema(name)
            .cloned()
            .ok_or_else(|| format!("the discovery documents define no {name}"))
    };
    let users = users::Users::new(
        Arc::clone(&service),
        user_map::UserMap::new(&config.party_binding)?,
        schema("User")?,
        scim,
        &base_url(config),
    )?;
    let routes = Router::new()
        .route("/Users", get(users::list).post(users::create))
        .route(
            "/Users/{id}",
            get(users::read)
                .put(users::modify)
                .patch(users::modify)
                .delete(users::delete),
        )
        .with_state(users);
    let Some(map) = group_map::GroupMap::new(config, scim)? else {
        return Ok(routes);
    };
    let groups = groups::Groups::new(service, map, schema("Group")?, scim, &base_url(config))?;
    Ok(routes.merge(
        Router::new()
            .route("/Groups", get(groups::list).post(groups::create))
            .route(
                "/Groups/{id}",
                get(groups::read)
                    .put(groups::modify)
                    .patch(groups::modify)
                    .delete(groups::delete),
            )
            .with_state(groups),
    ))
}

/// Admits the request of a declared provisioning client holding its scope.
async fn admit(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let authenticated = match gate.authenticator.authenticate(&parts).await {
        Ok(authenticated) => authenticated,
        Err(error) => return refused(&error),
    };
    let principal = authenticated.principal;
    let caller = principal.access.caller();
    if !caller.is_some_and(|caller| matches!(caller.kind(), CallerKind::System)) {
        return refusal(&ScimError::new(
            StatusCode::FORBIDDEN,
            "SCIM provisioning is a client's act: the token must be a client-credentials token \
             acting for nobody",
        ));
    }
    let issuer = caller.and_then(|caller| caller.issuer());
    let client_id = authn::client_of(&principal);
    let Some(client) = gate.clients.iter().find(|client| {
        Some(client.issuer.expose()) == issuer && client_id.as_deref() == Some(&client.client_id)
    }) else {
        return refusal(&ScimError::new(
            StatusCode::FORBIDDEN,
            "the token's client is no [[auth.scim.client]] of this deployment",
        ));
    };
    if !principal.scopes.iter().any(|scope| *scope == client.scope) {
        let mut response = refusal(&ScimError::new(
            StatusCode::FORBIDDEN,
            "the token does not hold the scope its [[auth.scim.client]] table names",
        ));
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer error=\"insufficient_scope\""),
        );
        return response;
    }
    let mut request = Request::from_parts(parts, body);
    request
        .extensions_mut()
        .insert(ScimCaller(Arc::clone(client)));
    request.extensions_mut().insert(principal.clone());
    gate.authenticator
        .run_admitted(principal, next.run(request))
        .await
}

/// The SCIM answer to an authentication refusal: `401` with the RFC 6750 §3
/// challenge, and the authentication layer's own status otherwise.
fn refused(error: &AuthError) -> Response {
    let status = error.status();
    if status == StatusCode::UNAUTHORIZED {
        let mut response = refusal(&ScimError::new(status, "authentication failed"));
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    refusal(&ScimError::new(status, error.to_string()))
}

/// `error` as its RFC 7644 §3.12 `Error` message.
pub(crate) fn refusal(error: &ScimError) -> Response {
    scim_json(error.status(), &error.body())
}

/// `body` as `application/scim+json` with `status` (RFC 7644 §8.1).
pub(crate) fn scim_json<T: serde::Serialize>(status: StatusCode, body: &T) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(MEDIA_TYPE));
    response
}

/// The query of a discovery endpoint: only `filter` is read, to refuse it.
#[derive(Debug, Default, Deserialize)]
struct DiscoveryQuery {
    filter: Option<String>,
}

async fn service_provider_config(State(discovery): State<Arc<Discovery>>) -> Response {
    scim_json(StatusCode::OK, discovery.service_provider_config())
}

async fn resource_types(
    State(discovery): State<Arc<Discovery>>,
    Query(query): Query<DiscoveryQuery>,
) -> Response {
    listed(&query, &discovery.resource_types())
}

async fn resource_type(
    State(discovery): State<Arc<Discovery>>,
    Path(id): Path<String>,
) -> Response {
    found(discovery.resource_type(&id), "resource type", &id)
}

async fn schema_list(
    State(discovery): State<Arc<Discovery>>,
    Query(query): Query<DiscoveryQuery>,
) -> Response {
    listed(&query, &discovery.schemas())
}

/// `list`, unless the query carries a `filter`, which the discovery
/// endpoints refuse (RFC 7644 §4).
fn listed<T: serde::Serialize>(query: &DiscoveryQuery, list: &T) -> Response {
    match discovery::filter_refusal(query.filter.as_deref()) {
        Some(refused) => refusal(&refused),
        None => scim_json(StatusCode::OK, list),
    }
}

async fn schema(State(discovery): State<Arc<Discovery>>, Path(id): Path<String>) -> Response {
    found(discovery.schema(&id), "schema", &id)
}

/// `document`, or the `404` naming the `kind` `id` names no document of.
fn found<T: serde::Serialize>(document: Option<&T>, kind: &str, id: &str) -> Response {
    match document {
        Some(document) => scim_json(StatusCode::OK, document),
        None => refusal(&ScimError::new(
            StatusCode::NOT_FOUND,
            format!("no {kind} {id}"),
        )),
    }
}
