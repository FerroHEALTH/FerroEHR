// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The remote policy decision point of the ABAC gate (`[authz.abac.remote]`).

use ferroehr::config::authz::AbacConfig;
use ferrohealth_authz::engine::AuthzError;
use ferrohealth_authz::remote::{Binding, RemotePdp, RemoteSettings};
use ferrohealth_config::units::Duration;

use crate::extensions::access::authz::request;

/// The remote decision point `[authz.abac.remote]` and `[authz.abac.policy]`
/// describe.
///
/// Each resource kind is bound to its policy, and the point is called over
/// the endpoint's HTTP client and `retry`, with the client credentials token
/// of `[authz.abac.remote.endpoint.credentials.oauth]` when one is set.
///
/// # Errors
/// [`AuthzError::Load`] when the server is unset or no URL, a policy names an
/// unknown resource kind, or the grant does not resolve; [`AuthzError::Remote`]
/// when the settings are refused.
pub fn remote_pdp(config: &AbacConfig) -> Result<RemotePdp, AuthzError> {
    let endpoint = ferroehr::outbound::Endpoint::connect(
        "authz.abac.remote.endpoint",
        &config.remote.endpoint,
    )
    .map_err(|e| AuthzError::Load(Box::new(e)))?;
    let mut bindings = std::collections::BTreeMap::new();
    for (key, rule) in &config.policy {
        let kind = request::ResourceKind::from_config_key(key).ok_or_else(|| {
            AuthzError::Load(format!("abac.policy names the unknown resource kind {key}").into())
        })?;
        bindings.insert(
            kind.resource_type().to_owned(),
            Binding {
                policy: rule.name.clone(),
                parameters: rule
                    .parameters
                    .iter()
                    .map(|p| p.wire_key().to_owned())
                    .collect(),
            },
        );
    }
    RemotePdp::with_client(
        RemoteSettings {
            server: endpoint.url().clone(),
            bindings,
            connect_timeout: config
                .remote
                .endpoint
                .connect_timeout
                .map_or(endpoint.timeout(), Duration::get),
            request_timeout: endpoint.timeout(),
        },
        endpoint.http(),
        endpoint.retry(),
        endpoint.token(),
    )
}
