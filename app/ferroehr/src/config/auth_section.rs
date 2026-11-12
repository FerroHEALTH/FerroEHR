// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[auth]` table: the shared `ferrohealth-smart` section and the tables
//! FerroEHR keeps beside it: `[auth.smart]` and `[auth.scim]`.
//!
//! `ferrohealth-smart` owns every other key of `[auth]`, `[auth.idp.entra]`
//! included, and reads it with `deny_unknown_fields`. The loader takes
//! FerroEHR's keys out of the table and hands every other key to the crate's
//! section, so an unknown key is still refused with the crate's field list.
//! No openEHR spec governs configuration: our own design.

use std::fmt;

use ferrohealth_smart::config::AuthSection;
use serde::de::value::{MapAccessDeserializer, StringDeserializer};
use serde::de::{DeserializeSeed, Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::config::scim::ScimConfig;
use crate::config::smart::SmartConfig;

/// The key of the SMART App Launch table inside `[auth]`.
const SMART: &str = "smart";
/// The key of the SCIM service provider table inside `[auth]`.
const SCIM: &str = "scim";

/// The `[auth]` table.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = {"enabled": "trust", "realm": "optional", "capabilities": "optional", "scopes_supported": "optional", "product_scope": "trust", "described_issuer": "trust", "issuer": "trust", "scopeless_subjects": "trust", "basic": "trust", "basic.cache_ttl": "optional", "dpop": "trust", "tls": "trust", "federation": "trust", "attributes": "trust", "party": "trust", "idp": "infrastructure"})))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = ["issuer.issuer", "issuer.kind", "issuer.audience"])))]
pub struct AuthConfig {
    /// Every key `ferrohealth-smart` reads: the issuers, the party and
    /// federation tables, the vendor adapter tables (`[auth.idp]`), the Basic
    /// users, `[auth.dpop]` and the discovery options.
    #[serde(flatten)]
    pub shared: Box<AuthSection>,
    /// `[auth.smart]` — the SMART App Launch resource-server posture.
    pub smart: SmartConfig,
    /// `[auth.scim]` — the SCIM 2.0 service provider and its provisioning
    /// clients; absent, `/scim/v2/` is not served.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scim: Option<ScimConfig>,
}

impl<'de> Deserialize<'de> for AuthConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(AuthVisitor)
    }
}

/// Reads `[auth]`, splitting FerroEHR's tables from the crate's keys.
struct AuthVisitor;

impl<'de> Visitor<'de> for AuthVisitor {
    type Value = AuthConfig;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the [auth] table")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<AuthConfig, A::Error> {
        let mut own = Own::default();
        let shared = AuthSection::deserialize(MapAccessDeserializer::new(WithoutOwn {
            inner: map,
            own: &mut own,
        }))?;
        Ok(AuthConfig {
            shared: Box::new(shared),
            smart: own.smart.unwrap_or_default(),
            scim: own.scim,
        })
    }
}

/// FerroEHR's own tables of `[auth]`, as read.
#[derive(Default)]
struct Own {
    /// `[auth.smart]`.
    smart: Option<SmartConfig>,
    /// `[auth.scim]`.
    scim: Option<ScimConfig>,
}

/// The entries of `[auth]` with FerroEHR's tables taken out into `own`.
struct WithoutOwn<'a, A> {
    /// The table's entries.
    inner: A,
    /// Where FerroEHR's tables go.
    own: &'a mut Own,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for WithoutOwn<'_, A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        let mut seed = Some(seed);
        loop {
            match self.inner.next_key_seed(KeyOrOwn(&mut seed))? {
                None => return Ok(None),
                Some(Key::Shared(value)) => return Ok(Some(value)),
                Some(Key::Smart) => read_once(&mut self.inner, &mut self.own.smart, SMART)?,
                Some(Key::Scim) => read_once(&mut self.inner, &mut self.own.scim, SCIM)?,
            }
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.inner.next_value_seed(seed)
    }
}

/// Reads the value of the own table `key` into `slot`, refusing it twice.
fn read_once<'de, A: MapAccess<'de>, T: Deserialize<'de>>(
    map: &mut A,
    slot: &mut Option<T>,
    key: &'static str,
) -> Result<(), A::Error> {
    if slot.is_some() {
        return Err(A::Error::duplicate_field(key));
    }
    *slot = Some(map.next_value()?);
    Ok(())
}

/// A key of `[auth]`: one of the crate's, read by its seed, or one of
/// FerroEHR's tables.
enum Key<T> {
    /// A key the crate's section reads.
    Shared(T),
    /// The `smart` key.
    Smart,
    /// The `scim` key.
    Scim,
}

/// Reads a key of `[auth]` inside the table's own key read, so a refused key
/// is reported at its path; the seed is taken by the first key that is not
/// one of FerroEHR's.
struct KeyOrOwn<'s, K>(&'s mut Option<K>);

impl<'de, K: DeserializeSeed<'de>> DeserializeSeed<'de> for KeyOrOwn<'_, K> {
    type Value = Key<K::Value>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl<'de, K: DeserializeSeed<'de>> Visitor<'de> for KeyOrOwn<'_, K> {
    type Value = Key<K::Value>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a key of the [auth] table")
    }

    fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<Self::Value, E> {
        match key {
            SMART => return Ok(Key::Smart),
            SCIM => return Ok(Key::Scim),
            _ => {}
        }
        let seed = self
            .0
            .take()
            .ok_or_else(|| E::custom("the [auth] key was read twice"))?;
        seed.deserialize(StringDeserializer::<E>::new(key.to_owned()))
            .map(Key::Shared)
    }
}

/// Returns the issuer key of one `machines` issuer `issuer` for the
/// `ferroehr` audience, the table the configuration tests trust.
#[cfg(test)]
pub(crate) fn machines(issuer: &str) -> Vec<ferrohealth_smart::config::IssuerEntry> {
    vec![ferrohealth_smart::config::IssuerEntry {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer.to_owned()),
        kind: Some(ferrohealth_smart::kind::IssuerKind::Machines),
        audience: vec!["ferroehr".to_owned()],
        ..ferrohealth_smart::config::IssuerEntry::default()
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idp_table_is_read_beside_the_crate_s_keys() {
        let auth: AuthConfig = toml::from_str(
            "realm = \"r\"\n[idp.entra]\nbase_url = \"https://graph.example/v1.0\"\n\
             [idp.entra.credentials]\nclient_id = \"reader\"\n",
        )
        .expect("parses");
        assert_eq!(auth.shared.realm, "r");
        let entra = auth.shared.idp.entra.expect("the entra table");
        assert_eq!(entra.base_url, "https://graph.example/v1.0");
        assert_eq!(entra.credentials.client_id, "reader");
    }

    #[test]
    fn an_unknown_key_is_refused_with_the_crate_s_field_list() {
        let err = toml::from_str::<AuthConfig>("isser = \"x\"\n").expect_err("unknown key");
        let text = err.to_string();
        assert!(text.contains("unknown field `isser`"), "{text}");
        assert!(text.contains("`issuer`"), "{text}");
        let err = toml::from_str::<AuthConfig>("[idp.okta]\n").expect_err("unknown adapter");
        assert!(err.to_string().contains("unknown field `okta`"), "{err}");
    }

    #[test]
    fn the_section_renders_flat_with_the_crate_s_idp_table() {
        let rendered = serde_json::to_value(AuthConfig::default()).expect("renders");
        assert!(rendered.get("enabled").is_some(), "{rendered}");
        assert_eq!(
            rendered.get("idp"),
            Some(&serde_json::json!({})),
            "{rendered}"
        );
        assert!(rendered.get("scim").is_none(), "{rendered}");
    }

    #[test]
    fn the_smart_and_scim_tables_are_read_beside_the_crate_s_keys() {
        let auth: AuthConfig = toml::from_str(
            "realm = \"r\"\n[smart]\ndiscovery = true\n[scim]\nfilter_max_results = 7\n",
        )
        .expect("parses");
        assert_eq!(auth.shared.realm, "r");
        assert!(auth.smart.discovery);
        assert_eq!(auth.scim.expect("the scim table").filter_max_results, 7);
        let twice = toml::from_str::<AuthConfig>("[smart]\n[smart]\n");
        assert!(twice.is_err());
    }
}
