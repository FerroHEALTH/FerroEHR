// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OAuth 2.0 client-credentials grant (RFC 6749 §4.4) an outbound
//! connection authenticates with, as configured on `[audit.fhir_feed.oauth]`
//! and `[authz.abac.remote.oauth]`.
//!
//! The server obtains a token for the upstream under its own client and sends
//! it as an RFC 6750 bearer credential, through the `ferrohealth-smart`
//! client, which caches the token, renews it before it expires, and sends a
//! call once more under a new token when the upstream answers `401`. For the
//! ITI-20 feed this is the IHE IUA model of an authorized client. No openEHR
//! spec governs outbound authentication — our own design.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::secret::{Secret, SecretUrl};

/// How the client authenticates to the token endpoint (RFC 6749 §2.3.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuthMethod {
    /// HTTP Basic over the client id and secret.
    #[default]
    ClientSecretBasic,
    /// The client id and secret in the request body.
    ClientSecretPost,
}

/// One client-credentials grant.
///
/// Exactly one of `token_url` and `issuer` names the token endpoint; the
/// issuer's metadata names it when only the issuer is given (RFC 8414).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientCredentialsConfig {
    /// The token endpoint (RFC 6749 §3.2). Any `userinfo` in it is masked in
    /// every rendering.
    pub token_url: Option<SecretUrl>,
    /// The issuer whose metadata names the token endpoint (RFC 8414 §3).
    pub issuer: Option<String>,
    /// The client identifier (RFC 6749 §2.2).
    pub client_id: String,
    /// The client secret, redacted in every rendering; or set
    /// [`Self::client_secret_file`].
    pub client_secret: Option<Secret>,
    /// A file whose contents are the client secret, resolved by the config
    /// loader into [`Self::client_secret`].
    pub client_secret_file: Option<PathBuf>,
    /// How the client authenticates to the token endpoint.
    pub client_auth: ClientAuthMethod,
    /// The `scope` request parameter (RFC 6749 §3.3).
    pub scope: Option<String>,
    /// The `audience` request parameter some authorization servers read.
    pub audience: Option<String>,
    /// The `resource` request parameter (RFC 8707 §2).
    pub resource: Option<String>,
}

impl ClientCredentialsConfig {
    /// The boot-time refusals of this grant, each naming `key`, the table it
    /// sits under.
    #[must_use]
    pub fn problems(&self, key: &str) -> Vec<String> {
        let mut out = Vec::new();
        match (
            non_blank_url(self.token_url.as_ref()),
            non_blank(self.issuer.as_ref()),
        ) {
            (Some(_), Some(_)) => {
                out.push(format!("{key}: set one of token_url and issuer, not both"));
            }
            (None, None) => out.push(format!("{key}: set token_url or issuer")),
            (Some(url), None) => {
                if url::Url::parse(url).is_err() {
                    out.push(format!("{key}.token_url is not a URL"));
                }
            }
            (None, Some(_)) => {}
        }
        if self.client_id.trim().is_empty() {
            out.push(format!("{key}.client_id must not be empty"));
        }
        if self.client_secret.as_ref().is_none_or(Secret::is_empty)
            && self.client_secret_file.is_none()
        {
            out.push(format!(
                "{key} requires client_secret or client_secret_file (the client-credentials \
                 grant authenticates the client)"
            ));
        }
        if let Some(resource) = non_blank(self.resource.as_ref())
            && url::Url::parse(resource).is_err()
        {
            out.push(format!("{key}.resource is not a URL"));
        }
        out
    }

    /// The grant as the `ferrohealth-smart` client takes it.
    ///
    /// # Errors
    /// A message naming `key` when the grant is incomplete; boot validation
    /// ([`Self::problems`]) refuses such a grant first.
    pub fn grant(&self, key: &str) -> Result<ferrohealth_smart::client::ClientCredentials, String> {
        if let Some(problem) = self.problems(key).into_iter().next() {
            return Err(problem);
        }
        let endpoint = match (
            non_blank_url(self.token_url.as_ref()),
            non_blank(self.issuer.as_ref()),
        ) {
            (Some(url), _) => ferrohealth_smart::client::Endpoint::Token(
                url::Url::parse(url).map_err(|e| format!("{key}.token_url: {e}"))?,
            ),
            (None, Some(issuer)) => ferrohealth_smart::client::Endpoint::Issuer(issuer.to_owned()),
            (None, None) => return Err(format!("{key}: set token_url or issuer")),
        };
        let secret = self
            .client_secret
            .as_ref()
            .map(|s| secrecy::SecretString::from(s.expose().to_owned()))
            .ok_or_else(|| format!("{key}.client_secret is not resolved"))?;
        let mut grant = ferrohealth_smart::client::ClientCredentials::new(
            endpoint,
            self.client_id.clone(),
            secret,
        );
        grant.client_auth = match self.client_auth {
            ClientAuthMethod::ClientSecretBasic => {
                ferrohealth_smart::client::ClientAuth::ClientSecretBasic
            }
            ClientAuthMethod::ClientSecretPost => {
                ferrohealth_smart::client::ClientAuth::ClientSecretPost
            }
        };
        grant.scope = non_blank(self.scope.as_ref()).map(str::to_owned);
        grant.audience = non_blank(self.audience.as_ref()).map(str::to_owned);
        grant.resource = non_blank(self.resource.as_ref())
            .map(|r| url::Url::parse(r).map_err(|e| format!("{key}.resource: {e}")))
            .transpose()?;
        grant.timeout = Duration::from_secs(10);
        Ok(grant)
    }
}

/// The URL when set to something other than whitespace.
fn non_blank_url(value: Option<&SecretUrl>) -> Option<&str> {
    value
        .map(SecretUrl::expose)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// The value when set to something other than whitespace.
fn non_blank(value: Option<&String>) -> Option<&str> {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}
