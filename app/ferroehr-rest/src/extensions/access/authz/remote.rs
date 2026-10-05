// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`RemotePdp`] — the external policy-decision-point client.
//!
//! The wire contract is deliberately minimal, in the NIST SP 800-162 sense of a
//! PDP the PEP consults: `POST {server}{policy-name}` with a **flat** JSON body
//! carrying only the configured, resolved attribute keys
//! (`organization`/`patient`/`template`); HTTP **200 = permit**, any 4xx =
//! deny, and the response body is ignored so no policy language is imposed on
//! the deployment. A 5xx or an IO failure is not a decision at all: it is
//! fail-closed as [`AuthzError::Unreachable`], which the PEP renders 500 rather
//! than a silent permit or a misleading 403. Multi-valued attributes fan out as
//! the full cartesian product, all-must-permit, first deny short-circuits.
//! Connect and request timeouts are explicit — without them a blackholed PDP
//! parks the request until the client gives up.
//!
//! NOTE: no openEHR spec governs external authorization — our own
//! design/extension; the SM places authorization out of band
//! (SM `openehr_platform/master02-overview.adoc` §General Assumptions).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): the external policy-server request body \
              is an open operational contract — its resolved attribute keys are \
              configuration-driven"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::extensions::access::authz::engine::{AuthzError, PolicyEngine};
use crate::extensions::access::authz::request::{AuthzRequest, Combination, Decision};
use ferroehr::config::authz::{AbacConfig, AbacParam, PolicyRule};

/// The remote policy-decision-point client.
#[derive(Debug)]
pub struct RemotePdp {
    client: reqwest::Client,
    /// The base URL, guaranteed to end with `/` (config boot validation).
    server: String,
    /// Per-resource-kind policy bindings, keyed by [`ResourceKind::config_key`].
    ///
    /// [`ResourceKind::config_key`]: crate::extensions::access::authz::request::ResourceKind::config_key
    policies: BTreeMap<String, PolicyRule>,
}

impl RemotePdp {
    /// Build the client from the ABAC config (already boot-validated: `server`
    /// present and slash-terminated).
    ///
    /// # Errors
    /// [`AuthzError::PolicyLoad`] if the reqwest client cannot be built or the
    /// server URL is absent (defensive — config validation catches this first).
    pub fn new(config: &AbacConfig) -> Result<Self, AuthzError> {
        let server = config
            .remote
            .server
            .clone()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AuthzError::PolicyLoad("abac.remote.server is not set".to_owned()))?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(config.remote.connect_timeout_ms))
            .timeout(Duration::from_millis(config.remote.request_timeout_ms))
            .build()
            .map_err(|e| AuthzError::PolicyLoad(format!("building remote PDP client: {e}")))?;
        Ok(Self {
            client,
            server,
            policies: config.policy.clone(),
        })
    }

    /// Build the flat request body for one combination: only the configured
    /// parameters that have a resolved value.
    fn body(rule: &PolicyRule, combo: &Combination<'_>) -> Value {
        let mut map = Map::new();
        for param in &rule.parameters {
            let value = match param {
                AbacParam::Organization => combo.organization,
                AbacParam::Patient => combo.patient,
                AbacParam::Template => combo.template,
            };
            if let Some(v) = value {
                map.insert(param.wire_key().to_owned(), Value::String(v.to_owned()));
            }
        }
        Value::Object(map)
    }

    /// POST one combination and return whether the PDP permitted it.
    ///
    /// `200` is a permit and a `4xx` is a deny — both are DECISIONS the policy
    /// server made. A `5xx` is not: it says the PDP failed, so it produced no
    /// decision at all, and reading it as a deny would let a broken policy
    /// server silently refuse clinical access while looking like policy. It
    /// becomes [`AuthzError::Unreachable`], which the PEP renders `500`
    /// (RFC 9110 §15.6: a 5xx means "the server is aware that it has erred").
    /// A `3xx` is equally not a decision: an authorization endpoint that
    /// redirects is misconfigured, and following it would send the attribute
    /// body somewhere the deployment did not name.
    ///
    /// # Errors
    /// [`AuthzError::Unreachable`] when the request fails, or when the PDP
    /// answers with any status that is not a decision.
    async fn permits(
        &self,
        rule: &PolicyRule,
        combo: &Combination<'_>,
    ) -> Result<bool, AuthzError> {
        let url = format!("{}{}", self.server, rule.name);
        let body = Self::body(rule, combo);
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthzError::Unreachable(format!("POST {url}: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::OK {
            return Ok(true);
        }
        if status.is_client_error() {
            return Ok(false);
        }
        Err(AuthzError::Unreachable(format!(
            "POST {url}: the policy server answered {status}, which is not an \
             authorization decision"
        )))
    }
}

#[async_trait]
impl PolicyEngine for RemotePdp {
    async fn decide(&self, req: &AuthzRequest<'_>) -> Result<Decision, AuthzError> {
        let Some(rule) = self.policies.get(req.kind.config_key()) else {
            // NOTE: no openEHR spec governs authorization — our own fail-closed
            // design; config boot validation makes this unreachable.
            tracing::error!(
                kind = req.kind.config_key(),
                operation_id = req.operation_id,
                "no remote PDP policy is configured for this resource kind — denying"
            );
            return Ok(Decision::Deny);
        };
        for combo in req.combinations() {
            if !self.permits(rule, &combo).await? {
                ferroehr::telemetry::metrics::metrics()
                    .authz_remote_pdp_calls
                    .add(1, &[opentelemetry::KeyValue::new("result", "deny")]);
                return Ok(Decision::Deny);
            }
            ferroehr::telemetry::metrics::metrics()
                .authz_remote_pdp_calls
                .add(1, &[opentelemetry::KeyValue::new("result", "permit")]);
        }
        Ok(Decision::Permit)
    }
}
