// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[auth]` table: the shared `ferrohealth-smart` section and the tables
//! FerroEHR keeps beside it: `[auth.smart]` and `[auth.scim]`.
//!
//! `ferrohealth-smart` owns every other key of `[auth]`, `[auth.idp.entra]`
//! included, and reads it with `deny_unknown_fields`; `config::beside` reads the
//! two tables beside it. The section is read as the crate's
//! `ProductStandingSection`, without the client lists the policy grants
//! answer (`party_binding::grants::standing`), so `scopeless_subjects` and an
//! issuer's `first_party_clients` and `delegates` are unknown keys. No
//! openEHR spec governs configuration: our own design.

use std::fmt;

use ferrohealth_smart::config::{AuthSection, ProductStandingSection};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::config::beside::{self, read_once, split};
use crate::config::scim::ScimConfig;
use crate::config::smart::SmartConfig;

/// The key of the SMART App Launch table inside `[auth]`.
const SMART: &str = "smart";
/// The key of the SCIM service provider table inside `[auth]`.
const SCIM: &str = "scim";

/// The `[auth]` table.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = {"enabled": "trust", "realm": "optional", "capabilities": "optional", "scopes_supported": "optional", "product_scope": "trust", "described_issuer": "trust", "issuer": "trust", "basic": "trust", "basic.cache_ttl": "optional", "dpop": "trust", "tls": "trust", "federation": "trust", "attributes": "trust", "party": "trust", "idp": "infrastructure"})))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = ["issuer.issuer", "issuer.kind", "issuer.audience"])))]
pub struct AuthConfig {
    /// Every key `ferrohealth-smart` reads: the issuers, the party and
    /// federation tables, the vendor adapter tables (`[auth.idp]`), the Basic
    /// users, `[auth.dpop]` and the discovery options.
    #[serde(flatten)]
    #[cfg_attr(feature = "schema", schemars(with = "ProductStandingSection"))]
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
        let (shared, own) = split::<ProductStandingSection, Own, A>(map)?;
        Ok(AuthConfig {
            shared: Box::new(shared.0),
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

impl beside::Own for Own {
    const TABLE: &'static str = "[auth]";

    fn own(key: &str) -> Option<&'static str> {
        [SMART, SCIM].into_iter().find(|own| *own == key)
    }

    fn read<'de, A: MapAccess<'de>>(
        &mut self,
        key: &'static str,
        map: &mut A,
    ) -> Result<(), A::Error> {
        if key == SMART {
            read_once(map, &mut self.smart, SMART)
        } else {
            read_once(map, &mut self.scim, SCIM)
        }
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
    fn the_client_lists_the_policy_grants_answer_are_unknown_keys() {
        for body in [
            "scopeless_subjects = []\n",
            "[[issuer]]\nissuer = \"https://idp.example\"\nfirst_party_clients = [\"portal\"]\n",
            "[[issuer]]\nissuer = \"https://idp.example\"\ndelegates = [{ client = \"bridge\" }]\n",
        ] {
            let err = toml::from_str::<AuthConfig>(body).expect_err("a removed key");
            assert!(err.to_string().contains("unknown field"), "{body}: {err}");
        }
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
