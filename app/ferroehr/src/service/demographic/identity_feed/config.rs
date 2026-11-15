// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[pix]` table: the PMIR Patient Identity Registry FerroEHR subscribes
//! to (ITI-94) and the client whose ITI-93 messages it applies.
//!
//! The table holds the Registry's endpoint and the trust in its feed, nothing
//! else: the channel FerroEHR names in the subscription is its own feed route
//! under `[server] public_base_url`, and what an applied message may change is
//! decided by the policy grants of the feed's client in the CDR. No openEHR
//! spec governs configuration: our own design.

use ferrohealth_smart::config::endpoint::EndpointSection;
use serde::{Deserialize, Serialize};

/// The `[pix]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct PixConfig {
    /// `[pix.endpoint]`, the family's outbound endpoint: the FHIR base URL of
    /// the Patient Identity Registry (`url`, the `[base]` of
    /// `[base]/Subscription`, PMIR 1.6.0 §2:3.94.4.1.2), the timeout of one
    /// ITI-94 exchange, its `retry`, the TLS and the credential by which the
    /// Registry authorizes the subscription (§2:3.94.5), such as the OAuth 2.0
    /// client credentials grant (IUA ITI-72 §3.72.4.2).
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
    pub endpoint: EndpointSection,
    /// The issuer of the access token the Registry presents with every ITI-93
    /// message (§2:3.93.5, IUA): one of the `[auth]` issuers.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub feed_issuer: String,
    /// The `client_id` that access token names: the only client whose ITI-93
    /// messages are applied.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub feed_client_id: String,
    /// `[[pix.identifier_domain]]`: the `DV_IDENTIFIER.type` a patient
    /// identifier of each domain is written with when the feed links it to a
    /// `PERSON` no other `PERSON` hands it from. A domain named nowhere takes
    /// the one type the patient `PERSON`s already carry it with; with none, a
    /// message linking one is refused and names the domain.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
    pub identifier_domain: Vec<IdentifierDomain>,
}

/// One `[[pix.identifier_domain]]` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct IdentifierDomain {
    /// The domain's `Identifier.system`, the `DV_IDENTIFIER.issuer` its
    /// identifiers are carried under, such as `urn:oid:2.999.1.1`.
    pub system: String,
    /// The `DV_IDENTIFIER.type` they are written with, such as `MR`; a
    /// protected scheme's code keeps the identifier sealed.
    pub identifier_type: String,
}

/// The path of the ITI-93 feed route under the server's public root.
pub const FEED_PATH: &str = "/pmir/feed";

impl PixConfig {
    /// Every rule the table breaks, as boot refusals: the endpoint's own
    /// rules, a feed issuer that is an absolute URL, a feed client, and a
    /// public base URL, which the subscription's channel endpoint is written
    /// under (`public_root`).
    #[must_use]
    pub fn problems(&self, public_root: bool) -> Vec<String> {
        let mut problems = crate::outbound::problems("pix.endpoint", &self.endpoint);
        if url::Url::parse(self.feed_issuer.trim()).is_err() {
            problems.push(
                "pix.feed_issuer must be the issuer URL of the Registry's access tokens, one of \
                 the [auth] issuers"
                    .to_owned(),
            );
        }
        if self.feed_client_id.trim().is_empty() {
            problems.push(
                "pix.feed_client_id must name the client the Registry's access tokens are \
                 issued to"
                    .to_owned(),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, domain) in self.identifier_domain.iter().enumerate() {
            if url::Url::parse(domain.system.trim()).is_err() {
                problems.push(format!(
                    "pix.identifier_domain[{index}].system must be an absolute URI, the domain's \
                     Identifier.system"
                ));
            } else if !seen.insert(domain.system.trim()) {
                problems.push(format!(
                    "pix.identifier_domain[{index}].system names a domain an earlier entry names"
                ));
            }
            if domain.identifier_type.trim().is_empty() {
                problems.push(format!(
                    "pix.identifier_domain[{index}].identifier_type must name the DV_IDENTIFIER type"
                ));
            }
        }
        if !public_root {
            problems.push(
                "[pix] needs [server] public_base_url: the subscription names the feed route \
                 under it as its channel endpoint"
                    .to_owned(),
            );
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pix() -> PixConfig {
        PixConfig {
            endpoint: EndpointSection {
                url: ferrohealth_secret::secret_url::SecretUrl::new("https://pix.example/fhir"),
                ..EndpointSection::default()
            },
            feed_issuer: "https://idp.example/realms/hospital".to_owned(),
            feed_client_id: "ferropix".to_owned(),
            identifier_domain: vec![IdentifierDomain {
                system: "urn:oid:2.999.1.1".to_owned(),
                identifier_type: "MR".to_owned(),
            }],
        }
    }

    #[test]
    fn a_complete_table_has_no_problem() {
        assert_eq!(pix().problems(true), Vec::<String>::new());
    }

    #[test]
    fn every_missing_piece_is_named_at_once() {
        let problems = PixConfig {
            feed_issuer: "not a url".to_owned(),
            feed_client_id: " ".to_owned(),
            ..pix()
        }
        .problems(false);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("pix.feed_issuer"), "{problems:?}");
        assert!(problems[1].contains("pix.feed_client_id"), "{problems:?}");
        assert!(problems[2].contains("public_base_url"), "{problems:?}");
    }

    #[test]
    fn an_identifier_domain_needs_a_uri_a_type_and_no_twin() {
        let mut config = pix();
        config.identifier_domain.push(IdentifierDomain {
            system: "urn:oid:2.999.1.1".to_owned(),
            identifier_type: "MR".to_owned(),
        });
        config.identifier_domain.push(IdentifierDomain {
            system: "no uri".to_owned(),
            identifier_type: " ".to_owned(),
        });
        let problems = config.problems(true);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("identifier_domain[1]"), "{problems:?}");
        assert!(
            problems[1].contains("identifier_domain[2].system"),
            "{problems:?}"
        );
        assert!(
            problems[2].contains("identifier_domain[2].identifier_type"),
            "{problems:?}"
        );
    }

    #[test]
    fn the_domains_map_from_the_environment() {
        let env = std::collections::HashMap::<String, String>::from([
            (
                "FERROEHR__PIX__FEED_ISSUER".to_owned(),
                "https://idp.example".to_owned(),
            ),
            (
                "FERROEHR__PIX__IDENTIFIER_DOMAIN__0__SYSTEM".to_owned(),
                "urn:oid:2.999.1.1".to_owned(),
            ),
            (
                "FERROEHR__PIX__IDENTIFIER_DOMAIN__0__IDENTIFIER_TYPE".to_owned(),
                "MR".to_owned(),
            ),
        ]);
        let (_, config) =
            crate::config::loader::assemble_from(None, &env, &[]).expect("the tree assembles");
        let pix = config.pix.expect("the [pix] table");
        assert_eq!(
            pix.identifier_domain,
            vec![IdentifierDomain {
                system: "urn:oid:2.999.1.1".to_owned(),
                identifier_type: "MR".to_owned(),
            }]
        );
    }
}
