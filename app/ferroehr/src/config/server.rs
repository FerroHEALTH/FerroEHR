// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[server]` table: `ferrohealth-server`'s listener section and the one
//! key FerroEHR keeps beside it, `public_base_url`, the URL clients reach the
//! listener at.
//!
//! Every public URL the server names derives from that one key and
//! `server.base_path`, the path the server mounts every route under
//! ([`PublicRoot`]): the SMART discovery document's `baseUrl`, every SCIM
//! `meta.location`, and the URL a `DPoP` proof's `htu` is compared with. No
//! openEHR spec governs configuration: our own design.

use std::fmt;

use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_server::section::ServerSection;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

use crate::config::beside::{self, read_once, split};

/// The key of the public base URL inside `[server]`.
const PUBLIC_BASE_URL: &str = "public_base_url";

/// The `[server]` table.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct ServerConfig {
    /// Every key `ferrohealth-server` reads: the listen address, the base
    /// path, the timeouts, the body limit, the overload bound, the drain,
    /// TLS, the trusted proxies, the caller rate and compression.
    #[serde(flatten)]
    pub shared: ServerSection,
    /// The URL clients reach this listener at, through any proxy in front
    /// of it: `https://cdr.example.com`, or with a path when a proxy serves
    /// the server under one. Every public URL derives from it, after
    /// `server.base_path`: the SMART discovery document's `baseUrl` (it,
    /// `base_path` and `api.base_path`), every SCIM `meta.location` (it,
    /// `base_path` and `/scim/v2`) and the URL a `DPoP` proof's `htu` is
    /// compared with (it, `base_path` and the request path, RFC 9449 §4.3).
    /// An absolute `http(s)` URL with no userinfo, query or fragment;
    /// required with `auth.smart.discovery`, `[auth.scim]` or `[auth.dpop]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_base_url: Option<SecretUrl>,
}

impl<'de> Deserialize<'de> for ServerConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ServerVisitor)
    }
}

/// Reads `[server]`, splitting FerroEHR's key from the crate's keys.
struct ServerVisitor;

impl<'de> Visitor<'de> for ServerVisitor {
    type Value = ServerConfig;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the [server] table")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<ServerConfig, A::Error> {
        let (shared, own) = split::<ServerSection, Own, A>(map)?;
        Ok(ServerConfig {
            shared,
            public_base_url: own.public_base_url,
        })
    }
}

/// FerroEHR's own key of `[server]`, as read.
#[derive(Default)]
struct Own {
    /// `public_base_url`.
    public_base_url: Option<SecretUrl>,
}

impl beside::Own for Own {
    const TABLE: &'static str = "[server]";

    fn own(key: &str) -> Option<&'static str> {
        (key == PUBLIC_BASE_URL).then_some(PUBLIC_BASE_URL)
    }

    fn read<'de, A: MapAccess<'de>>(
        &mut self,
        key: &'static str,
        map: &mut A,
    ) -> Result<(), A::Error> {
        read_once(map, &mut self.public_base_url, key)
    }
}

impl ServerConfig {
    /// The URL every route is reached at, when `public_base_url` is set and
    /// holds its rule.
    #[must_use]
    pub fn public_root(&self) -> Option<PublicRoot> {
        let raw = self.public_base_url.as_ref()?;
        PublicRoot::new(raw.expose(), self.shared.base_path.as_str()).ok()
    }

    /// The boot refusal of `public_base_url`, when it is set and breaks its
    /// rule; the message never quotes the value, which may carry a
    /// credential.
    #[must_use]
    pub fn public_base_url_error(&self) -> Option<String> {
        let raw = self.public_base_url.as_ref()?;
        PublicRoot::new(raw.expose(), self.shared.base_path.as_str())
            .err()
            .map(|rule| format!("server.public_base_url {rule}"))
    }
}

/// The URL every route of the server is reached at: `[server]
/// public_base_url` followed by `server.base_path`.
///
/// The SMART discovery document's `baseUrl`, the SCIM base URL and the base
/// a `DPoP` proof's `htu` is compared with all hang off it, so a deployment
/// names its public URL once. No openEHR spec governs configuration: our own
/// design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoot(Url);

impl PublicRoot {
    /// The root of `public_base_url` with the mount path `base_path`, the
    /// path's trailing `/` dropped.
    ///
    /// # Errors
    /// The rule `public_base_url` breaks, as text that names no part of it:
    /// it is no absolute `http(s)` URL with a host, or it carries userinfo, a
    /// query or a fragment.
    pub fn new(public_base_url: &str, base_path: &str) -> Result<Self, &'static str> {
        let mut url = Url::parse(public_base_url.trim())
            .map_err(|_unparsed| "is no absolute URL, e.g. 'https://cdr.example.com'")?;
        if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
            return Err("is no absolute http(s) URL with a host, e.g. 'https://cdr.example.com'");
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(
                "must carry no userinfo: the discovery document and every SCIM resource \
                 publish it",
            );
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("must carry no query or fragment: every public URL hangs off its path");
        }
        let path = format!("{}{base_path}", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Ok(Self(url))
    }

    /// The URL clients address the server's root at, which a `DPoP` proof's
    /// `htu` is compared with, the request path appended (RFC 9449 §4.3,
    /// check 9).
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.0
    }

    /// The absolute URL of the route at `path`, a path from the server's
    /// root starting with `/`.
    #[must_use]
    pub fn at(&self, path: &str) -> String {
        format!("{}{path}", self.0.as_str().trim_end_matches('/'))
    }
}

impl fmt::Display for PublicRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str().trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_read_beside_the_crate_s_keys() {
        let server: ServerConfig = toml::from_str(
            "listen = \"127.0.0.1:9000\"\npublic_base_url = \"https://cdr.example.com\"\n",
        )
        .expect("parses");
        assert_eq!(server.shared.listen.to_string(), "127.0.0.1:9000");
        assert_eq!(
            server.public_root().map(|root| root.to_string()).as_deref(),
            Some("https://cdr.example.com")
        );
        let twice =
            toml::from_str::<ServerConfig>("public_base_url = \"a\"\npublic_base_url = \"b\"\n");
        assert!(twice.is_err());
    }

    #[test]
    fn an_unknown_key_is_refused_with_the_crate_s_field_list() {
        let err = toml::from_str::<ServerConfig>("lissen = \"x\"\n").expect_err("unknown key");
        let text = err.to_string();
        assert!(text.contains("unknown field `lissen`"), "{text}");
        assert!(text.contains("`listen`"), "{text}");
    }

    #[test]
    fn the_section_renders_flat_and_without_an_unset_url() {
        let rendered = serde_json::to_value(ServerConfig::default()).expect("renders");
        assert!(rendered.get("listen").is_some(), "{rendered}");
        assert!(rendered.get("public_base_url").is_none(), "{rendered}");
    }

    #[test]
    fn every_public_url_hangs_off_the_root_and_the_mount_path() {
        let root = PublicRoot::new("https://cdr.example.com/", "").expect("a root");
        assert_eq!(root.url().as_str(), "https://cdr.example.com/");
        assert_eq!(root.at("/scim/v2"), "https://cdr.example.com/scim/v2");
        let mounted = PublicRoot::new("https://gw.example.com/cdr/", "/ehr").expect("a root");
        assert_eq!(mounted.url().as_str(), "https://gw.example.com/cdr/ehr");
        assert_eq!(
            mounted.at("/ferroehr/rest/openehr/v1"),
            "https://gw.example.com/cdr/ehr/ferroehr/rest/openehr/v1"
        );
        assert_eq!(mounted.to_string(), "https://gw.example.com/cdr/ehr");
    }

    #[test]
    fn a_url_that_breaks_the_rule_is_refused_without_quoting_it() {
        for raw in [
            "/ferroehr",
            "cdr.example.com",
            "ftp://cdr.example.com",
            "https://user:SENTINEL_PW@cdr.example.com",
            "https://cdr.example.com/?q=1",
            "https://cdr.example.com/#f",
        ] {
            let server = ServerConfig {
                public_base_url: Some(SecretUrl::new(raw)),
                ..ServerConfig::default()
            };
            assert!(server.public_root().is_none(), "{raw}");
            let error = server.public_base_url_error().expect("refused");
            assert!(error.starts_with("server.public_base_url "), "{error}");
            assert!(!error.contains("SENTINEL_PW"), "{error}");
        }
        assert!(ServerConfig::default().public_base_url_error().is_none());
    }
}
