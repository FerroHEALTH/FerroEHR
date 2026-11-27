// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[auth]` table: the shared `ferrohealth-smart` section and what
//! FerroEHR keeps beside it: `[auth.smart]`, `[auth.scim]` and
//! `post_quantum_only`.
//!
//! `ferrohealth-smart` owns every other key of `[auth]` and reads it with
//! `deny_unknown_fields`; `config::beside` reads FerroEHR's keys beside it.
//! No openEHR spec governs configuration: our own design.

use std::fmt;

use ferrohealth_smart::config::{AuthMode, AuthSection, IssuerEntry};
use ferrohealth_smart::jws::algorithm::Algorithm;
use ferrohealth_smart::sender::proof::DpopSection;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::config::beside::{self, read_once, split};
use crate::config::scim::ScimConfig;
use crate::config::smart::SmartConfig;

/// The key of the SMART App Launch table inside `[auth]`.
const SMART: &str = "smart";
/// The key of the SCIM service provider table inside `[auth]`.
const SCIM: &str = "scim";
/// The key of the switch that verifies JOSE signatures with ML-DSA alone.
const POST_QUANTUM_ONLY: &str = "post_quantum_only";

/// The ML-DSA parameter sets of JOSE (RFC 9964 §5), the algorithms
/// `post_quantum_only` admits.
pub const ML_DSA: [Algorithm; 3] = [Algorithm::MlDsa44, Algorithm::MlDsa65, Algorithm::MlDsa87];

/// The `[auth]` table.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = {"enabled": "trust", "realm": "optional", "capabilities": "optional", "scopes_supported": "optional", "product_scope": "trust", "described_issuer": "trust", "issuer": "trust", "basic": "trust", "basic.cache_ttl": "optional", "dpop": "trust", "tls": "trust", "post_quantum_only": "trust"})))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = ["issuer.<name>.issuer"])))]
pub struct AuthConfig {
    /// Every key `ferrohealth-smart` reads: the issuers, the party and
    /// federation tables, the Basic users, `[auth.dpop]` and the discovery
    /// options.
    #[serde(flatten)]
    pub shared: Box<AuthSection>,
    /// `[auth.smart]` — the SMART App Launch resource-server posture.
    pub smart: SmartConfig,
    /// `[auth.scim]` — the SCIM 2.0 service provider and its provisioning
    /// clients; absent, `/scim/v2/` is not served.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scim: Option<ScimConfig>,
    /// Whether access token and `DPoP` proof signatures verify with ML-DSA
    /// alone (RFC 9964 §5). On, every issuer's `algorithms` and
    /// `[auth.dpop] algorithms` left at their default become ML-DSA-44,
    /// ML-DSA-65 and ML-DSA-87, and a list naming any other algorithm stops
    /// the start.
    #[serde(default)]
    pub post_quantum_only: bool,
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
        let (mut shared, own) = split::<AuthSection, Own, A>(map)?;
        let post_quantum_only = own.post_quantum_only.unwrap_or_default();
        if post_quantum_only {
            post_quantum(&mut shared).map_err(A::Error::custom)?;
        }
        Ok(AuthConfig {
            shared: Box::new(shared),
            smart: own.smart.unwrap_or_default(),
            scim: own.scim,
            post_quantum_only,
        })
    }
}

/// FerroEHR's own keys of `[auth]`, as read.
#[derive(Default)]
struct Own {
    /// `[auth.smart]`.
    smart: Option<SmartConfig>,
    /// `[auth.scim]`.
    scim: Option<ScimConfig>,
    /// `post_quantum_only`.
    post_quantum_only: Option<bool>,
}

impl beside::Own for Own {
    const TABLE: &'static str = "[auth]";

    fn own(key: &str) -> Option<&'static str> {
        [SMART, SCIM, POST_QUANTUM_ONLY]
            .into_iter()
            .find(|own| *own == key)
    }

    fn read<'de, A: MapAccess<'de>>(
        &mut self,
        key: &'static str,
        map: &mut A,
    ) -> Result<(), A::Error> {
        match key {
            SMART => read_once(map, &mut self.smart, SMART),
            SCIM => read_once(map, &mut self.scim, SCIM),
            _ => read_once(map, &mut self.post_quantum_only, POST_QUANTUM_ONLY),
        }
    }
}

/// An algorithm list `post_quantum_only` refuses.
#[derive(Debug, thiserror::Error)]
#[error(
    "{key} names {algorithm}, which auth.post_quantum_only = true refuses: list ML-DSA-44, \
     ML-DSA-65 or ML-DSA-87 alone, or leave the key unset"
)]
pub struct ClassicalAlgorithm {
    /// The dotted key of the list.
    pub key: String,
    /// The first algorithm of it that is no ML-DSA parameter set.
    pub algorithm: Algorithm,
}

/// Narrows the signature algorithms of `section` to ML-DSA (RFC 9964 §5):
/// a list at its default becomes [`ML_DSA`], and a list naming any other
/// algorithm is refused.
///
/// An issuer under `mode = "introspection"` keeps its list, since the
/// authorization server checks its tokens.
///
/// # Errors
/// [`ClassicalAlgorithm`] naming the first list that names another
/// algorithm.
pub fn post_quantum(section: &mut AuthSection) -> Result<(), ClassicalAlgorithm> {
    let issuer_default = IssuerEntry::default().algorithms;
    for (name, entry) in &mut section.issuer {
        if entry.mode == AuthMode::Introspection {
            continue;
        }
        narrow(&mut entry.algorithms, &issuer_default, || {
            format!("auth.issuer.{name}.algorithms")
        })?;
    }
    if let Some(dpop) = &mut section.dpop {
        narrow(
            &mut dpop.algorithms,
            &DpopSection::default().algorithms,
            || "auth.dpop.algorithms".to_owned(),
        )?;
    }
    Ok(())
}

/// Replaces `list` with [`ML_DSA`] when it equals `default`, and refuses it
/// under `key` when it names an algorithm that is no ML-DSA parameter set.
fn narrow(
    list: &mut Vec<Algorithm>,
    default: &[Algorithm],
    key: impl FnOnce() -> String,
) -> Result<(), ClassicalAlgorithm> {
    if list.as_slice() == default {
        *list = ML_DSA.to_vec();
        return Ok(());
    }
    match list.iter().find(|algorithm| !algorithm.is_ml_dsa()) {
        Some(algorithm) => Err(ClassicalAlgorithm {
            key: key(),
            algorithm: *algorithm,
        }),
        None => Ok(()),
    }
}

/// Returns the table of one issuer `issuer` for the `ferroehr` audience, the
/// table the configuration tests trust.
#[cfg(test)]
pub(crate) fn machine(issuer: &str) -> IssuerEntry {
    IssuerEntry {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer.to_owned()),
        audience: vec!["ferroehr".to_owned()],
        ..IssuerEntry::default()
    }
}

/// Returns the issuer key holding [`machine`] `issuer` as the table
/// `[auth.issuer.machines]`.
#[cfg(test)]
pub(crate) fn machines(issuer: &str) -> std::collections::BTreeMap<String, IssuerEntry> {
    [("machines".to_owned(), machine(issuer))].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_key_is_refused_with_the_crate_s_field_list() {
        let err = toml::from_str::<AuthConfig>("isser = \"x\"\n").expect_err("unknown key");
        let text = err.to_string();
        assert!(text.contains("unknown field `isser`"), "{text}");
        assert!(text.contains("`issuer`"), "{text}");
    }

    #[test]
    fn the_section_renders_flat() {
        let rendered = serde_json::to_value(AuthConfig::default()).expect("renders");
        assert!(rendered.get("enabled").is_some(), "{rendered}");
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

    /// The `[auth.issuer.idp]` table head of the post-quantum cases.
    const IDP: &str = "[issuer.idp]\nissuer = \"https://idp.example\"\n";

    /// The algorithms of the `idp` issuer of `auth`.
    fn idp(auth: &AuthConfig) -> &[Algorithm] {
        &auth
            .shared
            .issuer
            .get("idp")
            .expect("the idp issuer")
            .algorithms
    }

    #[test]
    fn post_quantum_only_narrows_the_default_lists_to_ml_dsa() {
        let auth: AuthConfig =
            toml::from_str(&format!("post_quantum_only = true\n{IDP}[dpop]\n")).expect("parses");
        assert!(auth.post_quantum_only);
        assert_eq!(idp(&auth), ML_DSA);
        assert_eq!(auth.shared.dpop.expect("the dpop table").algorithms, ML_DSA);
    }

    #[test]
    fn post_quantum_only_keeps_an_ml_dsa_list_and_off_keeps_the_crate_defaults() {
        let chosen: AuthConfig = toml::from_str(&format!(
            "post_quantum_only = true\n{IDP}algorithms = [\"ML-DSA-87\"]\n"
        ))
        .expect("parses");
        assert_eq!(idp(&chosen), [Algorithm::MlDsa87]);
        let off: AuthConfig = toml::from_str(&format!("{IDP}[dpop]\n")).expect("parses");
        assert!(!off.post_quantum_only);
        assert_eq!(idp(&off), IssuerEntry::default().algorithms);
        assert_eq!(
            off.shared.dpop.expect("the dpop table").algorithms,
            DpopSection::default().algorithms
        );
    }

    #[test]
    fn post_quantum_only_refuses_a_list_naming_a_classical_algorithm() {
        let cases = [
            (
                format!("{IDP}algorithms = [\"ES256\"]\n"),
                "auth.issuer.idp.algorithms names ES256",
            ),
            (
                format!("{IDP}algorithms = [\"ML-DSA-65\", \"RS256\"]\n"),
                "auth.issuer.idp.algorithms names RS256",
            ),
            (
                "[dpop]\nalgorithms = [\"EdDSA\"]\n".to_owned(),
                "auth.dpop.algorithms names EdDSA",
            ),
        ];
        for (body, expected) in cases {
            let err = toml::from_str::<AuthConfig>(&format!("post_quantum_only = true\n{body}"))
                .expect_err("a classical algorithm under post_quantum_only");
            assert!(err.to_string().contains(expected), "{body}: {err}");
        }
    }

    #[test]
    fn none_an_hmac_and_an_unknown_algorithm_are_refused_with_or_without_the_switch() {
        // NOTE: RFC 8725 §3.1 and §3.2, neither `none` nor an HMAC algorithm verifies a token.
        for switch in ["", "post_quantum_only = true\n"] {
            for name in ["none", "HS256", "ML-DSA-128", "ml-dsa-65"] {
                let body = format!("{switch}{IDP}algorithms = [\"{name}\"]\n");
                assert!(toml::from_str::<AuthConfig>(&body).is_err(), "{body}");
            }
        }
    }
}
