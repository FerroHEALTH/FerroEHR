// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The remote policy decision point of the ABAC gate (`[authz.abac.remote]`).

use ferroehr::config::authz::AbacConfig;
use ferrohealth_authz::engine::AuthzError;
use ferrohealth_authz::remote::{Binding, RemotePdp};
use ferrohealth_smart::client::outbound::OutboundClient;
use ferrohealth_smart::config::ServerTls;

use crate::extensions::access::authz::request;

/// The remote decision point `[authz.abac.remote]` and `[authz.abac.policy]`
/// describe.
///
/// Each resource kind is bound to its policy, and the point is called over
/// `ferrohealth-smart`'s outbound client of `[authz.abac.remote.endpoint]`:
/// its HTTP client, its `retry`, and its Basic credential or client
/// credentials token.
///
/// # Errors
/// [`AuthzError::Load`] when the server is unset or no URL, a policy names an
/// unknown resource kind, or the grant does not resolve; [`AuthzError::Remote`]
/// when the settings are refused.
pub fn remote_pdp(config: &AbacConfig) -> Result<RemotePdp, AuthzError> {
    let outbound = OutboundClient::build(
        &config.remote.endpoint,
        "authz.abac.remote.endpoint",
        &ServerTls,
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
    RemotePdp::from_outbound(bindings, &outbound)
}
