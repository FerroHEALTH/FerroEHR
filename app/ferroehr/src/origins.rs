// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[origins]` register: the origins of data this deployment recognises.
//!
//! Regulation (EU) 2025/327 Annex II 3.4 asks the storing components to
//! "support different retention periods and access rights that take into
//! account the origins and categories of electronic health data"
//! (`docs/law/eu/ehds/text.html`). FerroEHR records the origin of every version
//! at commit (`version.origins`, [`crate::versioning::origins`]): each
//! `FEEDER_AUDIT.originating_system_audit.system_id` the body carries, as the
//! sending system chose it (RM common, `FEEDER_AUDIT_DETAILS.system_id`). A
//! free-text value can key neither a retention period nor an access rule, so
//! the deployment declares here the origins it recognises, each by the id its
//! data carry, and the clients whose direct commits come from it. A body with
//! no feeder audit, committed by a registered client, is stamped with that
//! client's origin; any other direct commit keeps this server's `system_id`.
//!
//! **No openEHR spec governs this — our own design/extension.**

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The `[origins]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OriginsConfig {
    /// The recognised origins (`[[origins.registered]]`). Empty by default:
    /// no retention period or access rule can then name an origin.
    pub registered: Vec<RegisteredOrigin>,
}

/// One recognised origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredOrigin {
    /// The id the origin's data carry: the
    /// `FEEDER_AUDIT.originating_system_audit.system_id` its feeds commit
    /// under, and the origin a direct commit by one of [`Self::clients`] is
    /// stamped with. Compared exactly.
    pub id: String,
    /// The authenticated clients whose direct commits come from this origin:
    /// a Basic user name, or the `client_id` (failing that the `azp`) of an
    /// OAuth 2.0 access token (RFC 9068 §2.2). Each client belongs to one
    /// origin at most.
    #[serde(default)]
    pub clients: Vec<String>,
}

impl OriginsConfig {
    /// The boot validation's findings, one message per defect: a blank id or
    /// one carrying whitespace, an id registered twice, a blank client, and a
    /// client named by two origins.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();
        let mut clients: BTreeMap<&str, &str> = BTreeMap::new();
        for origin in &self.registered {
            if origin.id.is_empty() || origin.id.chars().any(char::is_whitespace) {
                errors.push(format!(
                    "origins.registered id {:?} is blank or carries whitespace",
                    origin.id
                ));
            }
            if !ids.insert(origin.id.as_str()) {
                errors.push(format!(
                    "origins.registered names the id {:?} twice",
                    origin.id
                ));
            }
            for client in &origin.clients {
                if client.trim().is_empty() {
                    errors.push(format!(
                        "origins.registered id {:?} lists a blank client",
                        origin.id
                    ));
                } else if let Some(other) = clients.insert(client.as_str(), origin.id.as_str())
                    && other != origin.id
                {
                    errors.push(format!(
                        "origins.registered client {client:?} belongs to both {other:?} and {:?}; \
                         a client's direct commits come from one origin",
                        origin.id
                    ));
                }
            }
        }
        errors
    }

    /// Whether `id` is a registered origin.
    #[must_use]
    pub fn is_registered(&self, id: &str) -> bool {
        self.registered.iter().any(|origin| origin.id == id)
    }

    /// The registered origin whose clients name `client`.
    #[must_use]
    pub fn origin_of_client(&self, client: &str) -> Option<&str> {
        self.registered
            .iter()
            .find(|origin| origin.clients.iter().any(|c| c == client))
            .map(|origin| origin.id.as_str())
    }

    /// The origin a direct commit by `client` is stamped with: the client's
    /// registered origin, or `system_id`, this server, for any other caller.
    #[must_use]
    pub fn direct_origin(&self, client: Option<&str>, system_id: &str) -> String {
        client
            .and_then(|client| self.origin_of_client(client))
            .unwrap_or(system_id)
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register() -> OriginsConfig {
        toml::from_str(
            r#"
            [[registered]]
            id = "lab.example"
            clients = ["lab-gateway"]
            [[registered]]
            id = "pacs.example"
            "#,
        )
        .expect("the register parses")
    }

    #[test]
    fn a_registered_client_stamps_its_origin_and_any_other_the_server() {
        let origins = register();
        assert_eq!(origins.errors(), Vec::<String>::new());
        assert!(origins.is_registered("pacs.example"));
        assert!(!origins.is_registered("PACS.example"));
        assert_eq!(
            origins.direct_origin(Some("lab-gateway"), "ferroehr"),
            "lab.example"
        );
        assert_eq!(
            origins.direct_origin(Some("someone"), "ferroehr"),
            "ferroehr"
        );
        assert_eq!(origins.direct_origin(None, "ferroehr"), "ferroehr");
    }

    #[test]
    fn the_boot_validation_names_every_defect() {
        let origins: OriginsConfig = toml::from_str(
            r#"
            [[registered]]
            id = "a b"
            clients = ["x", " "]
            [[registered]]
            id = "dup"
            clients = ["x"]
            [[registered]]
            id = "dup"
            "#,
        )
        .expect("the register parses");
        let errors = origins.errors();
        assert_eq!(errors.len(), 4, "{errors:#?}");
        assert!(errors.iter().any(|e| e.contains("carries whitespace")));
        assert!(errors.iter().any(|e| e.contains("blank client")));
        assert!(errors.iter().any(|e| e.contains("belongs to both")));
        assert!(errors.iter().any(|e| e.contains("twice")));
    }
}
