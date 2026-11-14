// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The instances of one deployment and whether they share what they must.
//!
//! Any number of instances serve one deployment behind a load balancer, on
//! one PostgreSQL primary. Each signs versions and access-log links with the
//! keyring `[record.signing]` configures, and a link another instance signed
//! with another key cannot be vouched for. Each instance therefore writes the
//! fingerprint of its signing key and the digest of the configuration every
//! instance must share into its row when it starts, touches the row every
//! [`HEARTBEAT`], and compares its own with every row touched within
//! [`LIVE_WITHIN`]. A difference is logged and reported in health from the
//! start ([`crate::telemetry::indicators::InstancesHealth`]). No openEHR spec
//! governs the process model: our own design (#5589).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::FerroEhrConfig;
use crate::config::loader::ConfigError;
use crate::storage::error::StorageError;
use crate::storage::instance_repo::{self, InstanceRow};
use crate::versioning::signature::signer::Signer;

/// How often an instance touches its row and compares again.
pub const HEARTBEAT: Duration = Duration::from_secs(30);

/// How recently another instance must have touched its row to be compared:
/// three heartbeats, so one late heartbeat never drops an instance.
pub const LIVE_WITHIN: Duration = Duration::from_secs(90);

/// The sections whose values are each instance's own, left out of the
/// configuration digest: its listener and TLS, its telemetry and management
/// surface, its usage report, and its database connections and pool sizes.
const OWN_SECTIONS: [&str; 5] = [
    "server",
    "telemetry",
    "management",
    "usage_report",
    "database",
];

/// What every instance of a deployment must share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// The fingerprint of the signing key's `OpenPGP` certificate, upper-case
    /// hex, or `digest` for a keyring that holds none.
    pub signing_key: String,
    /// The SHA-256, as hex, of the configuration tree with
    /// the sections each instance holds its own left out and every secret
    /// redacted.
    pub configuration: String,
}

impl Fingerprint {
    /// The fingerprint of an instance running `config` with `signer`.
    ///
    /// # Errors
    /// [`ConfigError`] when the configuration does not render as JSON.
    pub fn of(config: &FerroEhrConfig, signer: &Signer) -> Result<Self, ConfigError> {
        let mut tree = config.to_redacted_json()?;
        if let Some(sections) = tree.as_object_mut() {
            for own in OWN_SECTIONS {
                sections.remove(own);
            }
        }
        let digest = Sha256::digest(tree.to_string().as_bytes());
        Ok(Self {
            signing_key: signer
                .key_fingerprint()
                .unwrap_or_else(|| "digest".to_owned()),
            configuration: digest.iter().fold(String::new(), |mut hex, byte| {
                use std::fmt::Write as _;
                // NOTE: writing to a String cannot fail (std::fmt::Write for String).
                let _written = write!(hex, "{byte:02x}");
                hex
            }),
        })
    }
}

/// Another instance that differs from this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The other instance.
    pub instance: Uuid,
    /// When it started.
    pub started_at: jiff::Timestamp,
    /// The FerroEHR release it runs.
    pub version: String,
    /// Whether it signs with another key.
    pub signing_key: bool,
    /// Whether it runs another configuration.
    pub configuration: bool,
}

/// What this instance found when it last compared itself with the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agreement {
    /// It has not compared yet.
    Unread,
    /// Every other live instance shares its fingerprint; `others` counts
    /// them.
    Agrees {
        /// The other live instances.
        others: usize,
    },
    /// These live instances differ from it.
    Differs(Vec<Difference>),
    /// The last comparison could not read the rows.
    Unknown,
}

/// This instance among the deployment's instances.
#[derive(Debug)]
pub struct Instances {
    pool: PgPool,
    own: InstanceRow,
    agreement: Mutex<Agreement>,
}

impl Instances {
    /// This instance, newly started over the clinical `pool`, with `own` as
    /// its fingerprint.
    #[must_use]
    pub fn new(pool: PgPool, own: Fingerprint) -> Self {
        Self {
            pool,
            own: InstanceRow {
                instance_id: Uuid::now_v7(),
                started_at: jiff::Timestamp::now(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                signing_key: own.signing_key,
                configuration: own.configuration,
            },
            agreement: Mutex::new(Agreement::Unread),
        }
    }

    /// This instance's id.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.own.instance_id
    }

    /// What the last comparison found.
    #[must_use]
    pub fn agreement(&self) -> Agreement {
        self.agreement
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Writes or touches this instance's row and compares it with every live
    /// row; what it found, which [`Self::agreement`] then answers.
    ///
    /// # Errors
    /// The storage error of the statement; the agreement is then
    /// [`Agreement::Unknown`].
    pub async fn compare(&self) -> Result<Agreement, StorageError> {
        let live = i64::try_from(LIVE_WITHIN.as_secs()).unwrap_or(i64::MAX);
        let found = match instance_repo::touch(&self.pool, &self.own, live).await {
            Ok(others) => self.judge(&others),
            Err(error) => {
                self.set(Agreement::Unknown);
                return Err(error);
            }
        };
        if let Agreement::Differs(differences) = &found {
            for difference in differences {
                tracing::error!(
                    instance = %difference.instance,
                    version = %difference.version,
                    signing_key = difference.signing_key,
                    configuration = difference.configuration,
                    "another instance of this deployment signs with another key or runs \
                     another configuration; every instance must share both"
                );
            }
        }
        self.set(found.clone());
        Ok(found)
    }

    /// The agreement of this instance with `others`.
    fn judge(&self, others: &[InstanceRow]) -> Agreement {
        let differences: Vec<Difference> = others
            .iter()
            .filter_map(|other| {
                let signing_key = other.signing_key != self.own.signing_key;
                let configuration = other.configuration != self.own.configuration;
                (signing_key || configuration).then(|| Difference {
                    instance: other.instance_id,
                    started_at: other.started_at,
                    version: other.version.clone(),
                    signing_key,
                    configuration,
                })
            })
            .collect();
        if differences.is_empty() {
            Agreement::Agrees {
                others: others.len(),
            }
        } else {
            Agreement::Differs(differences)
        }
    }

    /// Stores what a comparison found.
    fn set(&self, found: Agreement) {
        *self
            .agreement
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = found;
    }

    /// Compares again every [`HEARTBEAT`], in a detached task that ends with
    /// the runtime.
    pub fn spawn_heartbeat(self: &Arc<Self>) {
        let instances = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(HEARTBEAT);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tick.tick().await;
                if let Err(error) = instances.compare().await {
                    let cause: &(dyn std::error::Error + 'static) = &error;
                    tracing::warn!(error = cause, "the instances could not be compared");
                }
            }
        });
    }

    /// Deletes this instance's row, as it stops.
    ///
    /// # Errors
    /// The storage error of the statement.
    pub async fn leave(&self) -> Result<(), StorageError> {
        instance_repo::leave(&self.pool, self.own.instance_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fingerprint of `config` under the default `digest` keyring.
    fn of(config: &FerroEhrConfig) -> Fingerprint {
        Fingerprint::of(config, &Signer::digest_default()).expect("the configuration renders")
    }

    /// An instance's own listener, pool sizes and telemetry change nothing;
    /// a shared setting changes the digest.
    #[test]
    fn only_a_shared_setting_changes_the_configuration_digest() {
        let base = FerroEhrConfig::default();
        let mut own = base.clone();
        own.server.shared.listen = "127.0.0.1:18081".parse().expect("an address");
        own.database.max_connections = 7;
        assert_eq!(of(&base), of(&own));
        let mut shared = base.clone();
        shared.api.system_id = "another.example.org".to_owned();
        assert_ne!(of(&base).configuration, of(&shared).configuration);
        assert_eq!(of(&base).signing_key, "digest");
    }
}
