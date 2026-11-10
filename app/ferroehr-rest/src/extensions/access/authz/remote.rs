// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The remote policy decision point of the ABAC gate (`[authz.abac.remote]`).

use ferroehr::config::authz::AbacConfig;
use ferrohealth_authz::engine::AuthzError;
use ferrohealth_authz::remote::{Binding, RemotePdp, RemoteSettings};

use crate::extensions::access::authz::request;

/// The remote decision point `[authz.abac.remote]` and `[authz.abac.policy]`
/// describe: each resource kind bound to its policy, and the client
/// credentials token of `[authz.abac.remote.oauth]` when one is set.
///
/// # Errors
/// [`AuthzError::Load`] when the server is unset or no URL, a policy names an
/// unknown resource kind, or the grant does not resolve; [`AuthzError::Remote`]
/// when the settings are refused.
pub fn remote_pdp(config: &AbacConfig) -> Result<RemotePdp, AuthzError> {
    let server = config
        .remote
        .url
        .as_ref()
        .filter(|s| !s.expose().trim().is_empty())
        .ok_or_else(|| AuthzError::Load("authz.abac.remote.url is not set".into()))?;
    let server = url::Url::parse(server.expose()).map_err(|e| AuthzError::Load(Box::new(e)))?;
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
    let token = config
        .remote
        .oauth
        .as_ref()
        .map(|oauth| {
            let grant = oauth
                .resolve_with_tls(&ferrohealth_smart::config::ServerTls)
                .map_err(|e| AuthzError::Load(Box::new(e)))?;
            ferrohealth_smart::client::cache::TokenSource::new(grant)
                .map_err(|e| AuthzError::Load(Box::new(e)))
        })
        .transpose()?;
    RemotePdp::new(
        RemoteSettings {
            server,
            bindings,
            connect_timeout: config.remote.connect_timeout.get(),
            request_timeout: config.remote.timeout.get(),
        },
        token,
    )
}
