// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The declared deployment posture: what this deployment is allowed to hold,
//! and which separations it has actually made (#3226).
//!
//! FerroEHR has a set of properties a deployment either has or does not have:
//! separated database credentials, separated database clusters, a declared
//! subject pseudonym namespace, an audit trail with a durable sink, schema
//! preparation on its own credential. Every one is a configuration key an
//! operator can leave unset, and a deployment that quietly has none of them
//! looks, from its own logs and its own API, exactly like one that has all of
//! them. The top-level `deployment_profile` key gives the server a declared
//! posture: `production` refuses to start when a property is missing and not
//! explicitly accepted, `sandbox` says which are missing, loudly and on every
//! surface, so it cannot be mistaken for the other.
//!
//! **This is FerroEHR's own posture, not a legal requirement.** GDPR Art. 4(5)
//! asks that the additional information be "kept separately and … subject to
//! technical and organisational measures"
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>), not that it sit on a
//! separate server; one cluster with separated schemas, roles and no role that
//! spans them is a defensible reading. Two clusters are materially stronger
//! all the same, because a superuser, an instance-wide point-in-time recovery
//! and a single compromise are bridges no grant can close, and that is a
//! choice a deployment should make deliberately. There is no `research`
//! value: the flag controls RIGOUR, not purpose, and EHDS Chapter IV secondary
//! use runs on real patient data under a data permit, which is production
//! rigour whatever it is for.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::FerroEhrConfig;
use crate::config::server::binds_loopback;
use crate::db::MigrationMode;

/// How rigorously this deployment is held to the separations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentProfile {
    /// Must not hold real personal data. One cluster and one credential are
    /// fine, and the server says so on every surface. The default, so no
    /// existing deployment changes behaviour by upgrading.
    #[default]
    Sandbox,
    /// Holds real personal data. The server refuses to start while a
    /// [`DeploymentGap`] is open and not accepted by name.
    Production,
}

impl DeploymentProfile {
    /// The configuration token (`"sandbox"` / `"production"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

impl fmt::Display for DeploymentProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A `deployment_profile` token that names no profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentProfileParseError {
    /// The token as written.
    pub unrecognized: String,
}

impl fmt::Display for DeploymentProfileParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown deployment_profile `{}` (expected `sandbox` or `production`)",
            self.unrecognized
        )
    }
}

impl std::error::Error for DeploymentProfileParseError {}

impl std::str::FromStr for DeploymentProfile {
    type Err = DeploymentProfileParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sandbox" => Ok(Self::Sandbox),
            "production" => Ok(Self::Production),
            other => Err(DeploymentProfileParseError {
                unrecognized: other.to_owned(),
            }),
        }
    }
}

/// A separation a production deployment is expected to have made.
///
/// Each is a real, checkable property; none is a box ticked by being present.
/// The token is what `deployment_accepts` names to run `production` without
/// it, which is then said loudly at boot and on `/management/status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentGap {
    /// The clinical, party and linkage domains do not each connect on their own
    /// database credential.
    SharedCredential,
    /// Two of the four domains reach the same `PostgreSQL` cluster, read from
    /// `pg_control_system().system_identifier` on each pool, never from the
    /// DSN text: two DSNs can reach one cluster through different names, a
    /// proxy or a pooler.
    ///
    /// The audit trail counts, because the separation it needs is its own:
    /// `docs/law/ch/dpo/text-de.html Art. 4 Abs. 5` keeps the log "getrennt
    /// vom System, in welchem die Personendaten bearbeitet werden".
    SharedCluster,
    /// `privacy.subject_namespaces` is empty, so the rule that refuses a real
    /// identifier as an `EHR_STATUS` subject is out of force.
    OpenSubjectNamespace,
    /// Auditing is off, or on with no durable sink, so access is not recorded
    /// (EHDS Annex II 3.2 is not satisfiable by a deployment that logs nothing).
    AuditOff,
    /// Auditing is on and `audit.fail_mode` is `open`, so a record the queue
    /// cannot take is dropped and the request succeeds: the access happened and
    /// nothing says so.
    AuditFailsOpen,
    /// `authz.rbac.ehr_access_default` is `open`, so an EHR carrying no
    /// `ACCESS_CONTROL_SETTINGS` — which every new EHR does — admits any caller
    /// the coarse layers already let through.
    ///
    /// `docs/law/eu/gdpr/text.html Art. 25(2)`: measures "shall ensure that by
    /// default personal data are not made accessible without the individual's
    /// intervention to an indefinite number of natural persons". RM ehr
    /// master04 §EHR Access makes `EHR_ACCESS` the gateway for access decisions
    /// and defines no default, so which one to ship is ours to choose.
    OpenEhrAccessDefault,
    /// Schema preparation runs on a credential that also serves requests —
    /// `db.migrate = "apply"` with no `db.migrate_url`, or a relocated domain
    /// whose database is prepared on the domain's own runtime DSN.
    MigrateOnRuntimeCredential,
    /// `auth.enabled` is `false`, so every request is served unauthenticated.
    ///
    /// `docs/law/eu/cra/text.html Annex I Part I(2)(d)`: products shall "ensure
    /// protection from unauthorised access by appropriate control mechanisms,
    /// including but not limited to authentication".
    AuthOff,
    /// The main listener speaks plaintext (`server.tls.enabled = false`) on an
    /// address reachable off the host; a loopback bind is exempt.
    ///
    /// `docs/law/eu/cra/text.html Annex I Part I(2)(e)`: products shall "protect
    /// the confidentiality of stored, transmitted or otherwise processed data
    /// … by encrypting relevant data … in transit". Accepting the gap by name
    /// records that a TLS-terminating ingress fronts the port.
    PlaintextListener,
}

impl DeploymentGap {
    /// The configuration token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SharedCredential => "shared_credential",
            Self::SharedCluster => "shared_cluster",
            Self::OpenSubjectNamespace => "open_subject_namespace",
            Self::AuditOff => "audit_off",
            Self::AuditFailsOpen => "audit_fails_open",
            Self::OpenEhrAccessDefault => "open_ehr_access_default",
            Self::MigrateOnRuntimeCredential => "migrate_on_runtime_credential",
            Self::AuthOff => "auth_off",
            Self::PlaintextListener => "plaintext_listener",
        }
    }

    /// What is missing, what was found, and what to change: the sentence the
    /// banner, the boot log, the refusal and `/management/status` all carry.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::SharedCredential => {
                "the clinical, party and linkage domains share a database credential; set \
                 [storage.party] url and [storage.linkage] url (or [storage.clinical] url) to \
                 roles that hold one domain each"
            }
            Self::SharedCluster => {
                "two domains reach the same PostgreSQL cluster (pg_control_system().system_identifier \
                 is equal); give the party, linkage and audit domains their own clusters through \
                 [storage.party], [storage.linkage] and [storage.audit] url"
            }
            Self::OpenSubjectNamespace => {
                "[privacy] subject_namespaces is empty, so an EHR_STATUS subject may carry a real \
                 identifier; declare the pseudonym namespaces this deployment issues"
            }
            Self::AuditOff => {
                "no access log is written: [audit] is disabled or has no durable sink; enable it \
                 with the local store, syslog or the FHIR feed on"
            }
            Self::AuditFailsOpen => {
                "[audit] fail_mode is `open`, so a record the audit queue cannot take is dropped \
                 and the request succeeds; set [audit] fail_mode = \"closed\" to refuse the \
                 operation instead"
            }
            Self::OpenEhrAccessDefault => {
                "[authz.rbac] ehr_access_default is `open`, so an EHR carrying no \
                 ACCESS_CONTROL_SETTINGS admits every caller the coarse layers already let \
                 through; set [authz.rbac] ehr_access_default = \"restricted\", or author the \
                 settings per EHR"
            }
            Self::MigrateOnRuntimeCredential => {
                "[db] migrate is `apply` on a credential that also serves requests; set [db] \
                 migrate_url to the credential that prepares the schema, give a relocated domain \
                 a migrator of its own, or run migrate = \"verify\""
            }
            Self::AuthOff => {
                "[auth] enabled is false, so every request is served unauthenticated; set [auth] \
                 enabled = true with [auth.oidc] or [auth.basic] configured"
            }
            Self::PlaintextListener => {
                "[server] bind is a routable address and [server.tls] is off, so clinical data and \
                 credentials cross the network unencrypted; enable [server.tls], bind to loopback, \
                 or accept plaintext_listener by name when a TLS-terminating ingress fronts this \
                 port"
            }
        }
    }
}

impl fmt::Display for DeploymentGap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The cluster each domain pool reached, as `pg_control_system().system_identifier`.
///
/// `None` where the identifier could not be read; a gap is never inferred from
/// an unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterIdentities {
    /// The clinical pool's cluster.
    pub clinical: Option<String>,
    /// The party pool's cluster.
    pub party: Option<String>,
    /// The linkage pool's cluster.
    pub linkage: Option<String>,
    /// The audit pool's cluster.
    ///
    /// Measured like the other three, because the audit trail's separation is
    /// its own duty rather than a consequence of theirs:
    /// `docs/law/ch/dpo/text-de.html Art. 4 Abs. 5` keeps the log "getrennt vom
    /// System, in welchem die Personendaten bearbeitet werden".
    pub audit: Option<String>,
}

impl ClusterIdentities {
    /// Whether any two known identifiers are equal.
    #[must_use]
    pub fn any_shared(&self) -> bool {
        let known: Vec<&String> = [&self.clinical, &self.party, &self.linkage, &self.audit]
            .into_iter()
            .flatten()
            .collect();
        known
            .iter()
            .enumerate()
            .any(|(i, a)| known.iter().skip(i + 1).any(|b| a == b))
    }
}

/// What the database layer measured about this deployment's databases.
///
/// Both members are facts a connection produced, not configuration: the
/// clusters the pools reached, and the domains whose database schema
/// preparation will touch on a credential that also serves requests. Both are
/// empty in the banner's pre-pool evaluation, which is what makes that reading
/// the configuration's own posture and this one the measured posture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseFacts {
    /// The cluster each domain pool reached.
    pub clusters: ClusterIdentities,
    /// The domains whose database is prepared on a runtime credential, as the
    /// preparation plan resolved it — a relocated domain prepares its own
    /// database, because `[db].migrate_url` names one database and the domain
    /// is not in it.
    pub prepared_on_runtime_credential: Vec<crate::db::domain::Domain>,
}

/// The evaluated posture: what the profile asserts, what is open, what was
/// accepted by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeploymentPosture {
    /// The declared profile.
    pub profile: DeploymentProfile,
    /// The separations this deployment has not made, whether or not accepted.
    pub gaps: Vec<DeploymentGap>,
    /// The open gaps `deployment_accepts` named, so the server runs with them.
    pub accepted: Vec<DeploymentGap>,
}

impl DeploymentPosture {
    /// Evaluate the posture from the resolved configuration and the clusters
    /// the pools reached.
    #[must_use]
    pub fn evaluate(config: &FerroEhrConfig, databases: &DatabaseFacts) -> Self {
        let clusters = &databases.clusters;
        let mut gaps = Vec::new();
        if !config
            .storage
            .pseudonymisation_domains_are_separated(&config.db)
        {
            gaps.push(DeploymentGap::SharedCredential);
        }
        if clusters.any_shared() {
            gaps.push(DeploymentGap::SharedCluster);
        }
        if config.privacy.subject_namespaces.is_empty() {
            gaps.push(DeploymentGap::OpenSubjectNamespace);
        }
        let audit = &config.audit;
        if !audit.enabled
            || !(audit.store.enabled || audit.syslog.enabled || audit.fhir_feed.enabled)
        {
            gaps.push(DeploymentGap::AuditOff);
        } else if audit.fail_mode == crate::system_log::config::FailMode::Open {
            // Only while a trail exists: a disabled trail makes the fail mode
            // moot, and naming both would report one posture twice.
            gaps.push(DeploymentGap::AuditFailsOpen);
        }
        if config.authz.rbac.ehr_access_default == crate::config::authz::EhrAccessDefault::Open {
            gaps.push(DeploymentGap::OpenEhrAccessDefault);
        }
        if config.db.migrate == MigrationMode::Apply
            && (!config.db.migrator_is_separated()
                || !databases.prepared_on_runtime_credential.is_empty())
        {
            gaps.push(DeploymentGap::MigrateOnRuntimeCredential);
        }
        if !config.auth.enabled {
            gaps.push(DeploymentGap::AuthOff);
        }
        if !config.server.tls.enabled && !binds_loopback(&config.server.bind) {
            gaps.push(DeploymentGap::PlaintextListener);
        }
        let accepted = gaps
            .iter()
            .copied()
            .filter(|gap| config.deployment_accepts.contains(gap))
            .collect();
        Self {
            profile: config.deployment_profile,
            gaps,
            accepted,
        }
    }

    /// The open gaps `production` refuses on: those not accepted by name.
    #[must_use]
    pub fn refusals(&self) -> Vec<DeploymentGap> {
        self.gaps
            .iter()
            .copied()
            .filter(|gap| !self.accepted.contains(gap))
            .collect()
    }

    /// Whether the server may start under this posture.
    #[must_use]
    pub fn permits_boot(&self) -> bool {
        self.profile == DeploymentProfile::Sandbox || self.refusals().is_empty()
    }

    /// The refusal, when [`Self::permits_boot`] is false: every open gap, its
    /// finding and its remedy, so one boot fixes them all.
    #[must_use]
    pub fn refusal_message(&self) -> String {
        let mut out = String::from(
            "deployment_profile = \"production\" refuses to start: this deployment has not made \
             the separations production asserts. Make them, or accept each one by name in \
             deployment_accepts (which is then stated on every boot and on /management/status):",
        );
        for gap in self.refusals() {
            out.push_str("\n  - ");
            out.push_str(gap.as_str());
            out.push_str(": ");
            out.push_str(gap.describe());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::secret::SecretUrl;

    fn separated() -> FerroEhrConfig {
        FerroEhrConfig {
            db: crate::db::DbConfig {
                migrate_url: Some(SecretUrl::new("postgres://m@h/x")),
                ..crate::db::DbConfig::default()
            },
            storage: crate::db::domain::StorageConfig {
                clinical: crate::db::domain::DomainDsn {
                    url: Some(SecretUrl::new("postgres://c@h/x")),
                    url_file: None,
                },
                party: crate::db::domain::DomainDsn {
                    url: Some(SecretUrl::new("postgres://d@h/x")),
                    url_file: None,
                },
                linkage: crate::db::domain::DomainDsn {
                    url: Some(SecretUrl::new("postgres://l@h/x")),
                    url_file: None,
                },
                ..crate::db::domain::StorageConfig::default()
            },
            privacy: crate::privacy::config::PrivacyConfig {
                subject_namespaces: vec!["urn:example:pseudonym".to_owned()],
                ..crate::privacy::config::PrivacyConfig::default()
            },
            audit: crate::system_log::config::AuditConfig {
                fail_mode: crate::system_log::config::FailMode::Closed,
                ..crate::system_log::config::AuditConfig::default()
            },
            authz: crate::config::authz::AuthzConfig {
                rbac: crate::config::authz::RbacConfig {
                    ehr_access_default: crate::config::authz::EhrAccessDefault::Restricted,
                    ..crate::config::authz::RbacConfig::default()
                },
                ..crate::config::authz::AuthzConfig::default()
            },
            server: crate::config::server::ServerConfig {
                tls: crate::config::server::TlsConfig {
                    enabled: true,
                    ..crate::config::server::TlsConfig::default()
                },
                ..crate::config::server::ServerConfig::default()
            },
            ..FerroEhrConfig::default()
        }
    }

    fn production(base: FerroEhrConfig) -> FerroEhrConfig {
        FerroEhrConfig {
            deployment_profile: DeploymentProfile::Production,
            ..base
        }
    }

    fn distinct_clusters() -> DatabaseFacts {
        DatabaseFacts {
            clusters: ClusterIdentities {
                clinical: Some("1".to_owned()),
                party: Some("2".to_owned()),
                linkage: Some("3".to_owned()),
                audit: Some("4".to_owned()),
            },
            prepared_on_runtime_credential: Vec::new(),
        }
    }

    /// The clusters alone, with nothing measured about preparation.
    fn on(clusters: ClusterIdentities) -> DatabaseFacts {
        DatabaseFacts {
            clusters,
            prepared_on_runtime_credential: Vec::new(),
        }
    }

    /// The shipped default is `sandbox`, and a stock configuration on one
    /// cluster is exactly the deployment that must not hold real data: every
    /// gap but the audit-off one is open, and boot is permitted.
    #[test]
    fn the_default_is_sandbox_and_a_stock_deployment_names_its_gaps() {
        let config = FerroEhrConfig::default();
        assert_eq!(config.deployment_profile, DeploymentProfile::Sandbox);
        let clusters = ClusterIdentities {
            clinical: Some("1".to_owned()),
            party: Some("1".to_owned()),
            linkage: Some("1".to_owned()),
            audit: Some("1".to_owned()),
        };
        let posture = DeploymentPosture::evaluate(&config, &on(clusters));
        assert_eq!(
            posture.gaps,
            [
                DeploymentGap::SharedCredential,
                DeploymentGap::SharedCluster,
                DeploymentGap::OpenSubjectNamespace,
                DeploymentGap::AuditFailsOpen,
                DeploymentGap::OpenEhrAccessDefault,
                DeploymentGap::MigrateOnRuntimeCredential,
                DeploymentGap::PlaintextListener,
            ],
            "auditing and authentication are on by default, so neither is a gap"
        );
        assert!(posture.permits_boot());
    }

    /// `production` refuses every open gap by name and lists the remedy.
    #[test]
    fn production_refuses_each_open_gap_and_names_the_remedy() {
        let config = production(FerroEhrConfig {
            audit: crate::system_log::config::AuditConfig {
                enabled: false,
                ..crate::system_log::config::AuditConfig::default()
            },
            ..FerroEhrConfig::default()
        });
        let posture = DeploymentPosture::evaluate(&config, &DatabaseFacts::default());
        assert!(!posture.permits_boot());
        let message = posture.refusal_message();
        for gap in [
            DeploymentGap::SharedCredential,
            DeploymentGap::OpenSubjectNamespace,
            DeploymentGap::AuditOff,
            DeploymentGap::OpenEhrAccessDefault,
            DeploymentGap::MigrateOnRuntimeCredential,
        ] {
            assert!(message.contains(gap.as_str()), "{message}");
            assert!(message.contains(gap.describe()), "{message}");
        }
        assert!(
            !posture.gaps.contains(&DeploymentGap::SharedCluster),
            "an unknown cluster identity is never read as a gap"
        );
        assert!(
            !posture.gaps.contains(&DeploymentGap::AuditFailsOpen),
            "a disabled trail makes the fail mode moot, so it is named once"
        );
    }

    /// A fully separated deployment on four clusters boots as `production`
    /// with nothing open.
    #[test]
    fn a_separated_deployment_boots_as_production() {
        let config = production(separated());
        let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(posture.gaps.is_empty(), "{posture:?}");
        assert!(posture.permits_boot());
    }

    /// Two DSNs reaching one cluster are refused by the cluster identity, not
    /// by the host string; accepting the gap by name permits boot and keeps
    /// it listed.
    #[test]
    fn a_shared_cluster_is_refused_by_identity_and_accepted_only_by_name() {
        let config = production(separated());
        let same_cluster = on(ClusterIdentities {
            clinical: Some("7".to_owned()),
            party: Some("7".to_owned()),
            linkage: Some("8".to_owned()),
            audit: Some("9".to_owned()),
        });
        let posture = DeploymentPosture::evaluate(&config, &same_cluster);
        assert_eq!(posture.gaps, [DeploymentGap::SharedCluster]);
        assert!(!posture.permits_boot());

        let config = FerroEhrConfig {
            deployment_accepts: vec![DeploymentGap::SharedCluster],
            ..config
        };
        let accepted = DeploymentPosture::evaluate(&config, &same_cluster);
        assert!(accepted.permits_boot());
        assert_eq!(accepted.accepted, [DeploymentGap::SharedCluster]);
        assert_eq!(
            accepted.gaps,
            [DeploymentGap::SharedCluster],
            "accepting hides nothing"
        );
    }

    /// `Display`/`FromStr` round-trip the tokens, and serde spells them the
    /// same way.
    #[test]
    fn tokens_round_trip() {
        for profile in [DeploymentProfile::Sandbox, DeploymentProfile::Production] {
            let token = profile.to_string();
            assert_eq!(token.parse::<DeploymentProfile>().ok(), Some(profile));
            assert_eq!(
                serde_json::to_string(&profile).ok(),
                Some(format!("\"{token}\""))
            );
        }
        assert!("research".parse::<DeploymentProfile>().is_err());
        for gap in [
            DeploymentGap::SharedCredential,
            DeploymentGap::SharedCluster,
            DeploymentGap::OpenSubjectNamespace,
            DeploymentGap::AuditOff,
            DeploymentGap::AuditFailsOpen,
            DeploymentGap::OpenEhrAccessDefault,
            DeploymentGap::MigrateOnRuntimeCredential,
            DeploymentGap::AuthOff,
            DeploymentGap::PlaintextListener,
        ] {
            assert_eq!(
                serde_json::to_string(&gap).ok(),
                Some(format!("\"{}\"", gap.as_str()))
            );
        }
    }

    /// The two compatibility defaults are open gaps under `production`, each
    /// named with its key, its value and its remedy, and each accepted only by
    /// name.
    ///
    /// The shipped defaults do not change: they serve the conformance
    /// instrument and the development stacks, where a closed `EHR_ACCESS` default
    /// would refuse every read of a fresh EHR. What changes is that a
    /// production deployment has to say it wants them.
    /// `docs/law/eu/gdpr/text.html Art. 25(2)` is the ground for the access
    /// default: measures "shall ensure that by default personal data are not
    /// made accessible without the individual's intervention to an indefinite
    /// number of natural persons".
    #[test]
    fn production_refuses_the_open_access_default_and_the_open_audit_fail_mode() {
        let config = production(FerroEhrConfig {
            audit: crate::system_log::config::AuditConfig {
                fail_mode: crate::system_log::config::FailMode::Open,
                ..crate::system_log::config::AuditConfig::default()
            },
            authz: crate::config::authz::AuthzConfig {
                rbac: crate::config::authz::RbacConfig {
                    ehr_access_default: crate::config::authz::EhrAccessDefault::Open,
                    ..crate::config::authz::RbacConfig::default()
                },
                ..crate::config::authz::AuthzConfig::default()
            },
            ..separated()
        });
        let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert_eq!(
            posture.gaps,
            [
                DeploymentGap::AuditFailsOpen,
                DeploymentGap::OpenEhrAccessDefault
            ]
        );
        assert!(!posture.permits_boot());
        let message = posture.refusal_message();
        for gap in [
            DeploymentGap::AuditFailsOpen,
            DeploymentGap::OpenEhrAccessDefault,
        ] {
            assert!(message.contains(gap.as_str()), "{message}");
            assert!(message.contains(gap.describe()), "{message}");
        }
        assert!(message.contains("ehr_access_default"), "{message}");
        assert!(message.contains("fail_mode"), "{message}");
        assert!(message.contains("restricted"), "{message}");
        assert!(message.contains("closed"), "{message}");

        // Accepted by name: the deployment boots and both stay listed.
        let accepted = DeploymentPosture::evaluate(
            &FerroEhrConfig {
                deployment_accepts: vec![
                    DeploymentGap::AuditFailsOpen,
                    DeploymentGap::OpenEhrAccessDefault,
                ],
                ..config
            },
            &distinct_clusters(),
        );
        assert!(accepted.permits_boot());
        assert_eq!(
            accepted.accepted,
            [
                DeploymentGap::AuditFailsOpen,
                DeploymentGap::OpenEhrAccessDefault
            ]
        );
    }

    /// A `sandbox` deployment runs on the shipped defaults and says so; the two
    /// gaps refuse nothing there.
    #[test]
    fn sandbox_runs_the_shipped_defaults_and_only_names_them() {
        let config = FerroEhrConfig::default();
        assert_eq!(
            config.authz.rbac.ehr_access_default,
            crate::config::authz::EhrAccessDefault::Open,
            "the shipped access default is unchanged"
        );
        assert_eq!(
            config.audit.fail_mode,
            crate::system_log::config::FailMode::Open,
            "the shipped audit fail mode is unchanged"
        );
        let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(posture.gaps.contains(&DeploymentGap::AuditFailsOpen));
        assert!(posture.gaps.contains(&DeploymentGap::OpenEhrAccessDefault));
        assert!(posture.permits_boot(), "sandbox refuses nothing");
    }

    /// An audit database sharing a cluster with a pseudonymisation domain is a
    /// `SharedCluster` gap.
    ///
    /// `docs/law/ch/dpo/text-de.html Art. 4 Abs. 5`: "Die Protokolle müssen
    /// während mindestens einem Jahr getrennt vom System, in welchem die
    /// Personendaten bearbeitet werden, aufbewahrt werden." The posture reads
    /// the audit pool's own cluster identity, so an audit database co-located
    /// with the clinical one is visible rather than silently uncounted.
    #[test]
    fn the_audit_cluster_counts_towards_the_shared_cluster_gap() {
        let config = production(separated());
        let audit_with_clinical = on(ClusterIdentities {
            clinical: Some("1".to_owned()),
            party: Some("2".to_owned()),
            linkage: Some("3".to_owned()),
            audit: Some("1".to_owned()),
        });
        let posture = DeploymentPosture::evaluate(&config, &audit_with_clinical);
        assert_eq!(posture.gaps, [DeploymentGap::SharedCluster]);
        assert!(!posture.permits_boot());
        let message = posture.refusal_message();
        assert!(
            message.contains("[storage.audit]"),
            "the remedy names the audit domain's own DSN: {message}"
        );
    }

    /// A domain whose database schema preparation reaches on its own runtime
    /// DSN opens `MigrateOnRuntimeCredential`, even with `[db].migrate_url`
    /// set.
    #[test]
    fn a_relocated_domain_prepared_on_its_runtime_credential_is_a_gap() {
        let config = production(separated());
        let separated_migrator = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(
            separated_migrator.gaps.is_empty(),
            "[db].migrate_url alone closes it while nothing is relocated: {separated_migrator:?}"
        );

        let relocated = DatabaseFacts {
            prepared_on_runtime_credential: vec![crate::db::domain::Domain::Party],
            ..distinct_clusters()
        };
        let posture = DeploymentPosture::evaluate(&config, &relocated);
        assert_eq!(posture.gaps, [DeploymentGap::MigrateOnRuntimeCredential]);
        assert!(!posture.permits_boot());
    }

    /// `production` refuses unauthenticated serving, and accepting `auth_off`
    /// by name is the only way to boot with it (CRA Annex I Part I(2)(d)).
    #[test]
    fn production_refuses_auth_off_unless_accepted_by_name() {
        let mut config = production(separated());
        config.auth.enabled = false;
        let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert_eq!(posture.gaps, [DeploymentGap::AuthOff]);
        assert!(!posture.permits_boot());
        let message = posture.refusal_message();
        assert!(message.contains("auth_off"), "{message}");
        assert!(
            message.contains(DeploymentGap::AuthOff.describe()),
            "{message}"
        );

        config.deployment_accepts = vec![DeploymentGap::AuthOff];
        let accepted = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(accepted.permits_boot());
        assert_eq!(accepted.accepted, [DeploymentGap::AuthOff]);
        assert_eq!(
            accepted.gaps,
            [DeploymentGap::AuthOff],
            "accepting hides nothing"
        );

        config.deployment_accepts = vec![DeploymentGap::PlaintextListener];
        let other_name = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(
            !other_name.permits_boot(),
            "accepting a different gap does not accept this one"
        );
    }

    /// `production` refuses a plaintext listener on a routable or ambiguous
    /// address, exempts a loopback bind, and boots once `plaintext_listener`
    /// is accepted by name (CRA Annex I Part I(2)(e)).
    #[test]
    fn production_refuses_a_routable_plaintext_listener_and_exempts_loopback() {
        let mut config = production(separated());
        config.server.tls.enabled = false;
        for bind in [
            "0.0.0.0:8080",
            "10.0.0.4:8080",
            "[::]:8080",
            "ferroehr.internal:8080",
        ] {
            config.server.bind = bind.to_owned();
            let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
            assert_eq!(posture.gaps, [DeploymentGap::PlaintextListener], "{bind}");
            assert!(!posture.permits_boot(), "{bind}");
        }

        for bind in ["127.0.0.1:8080", "[::1]:8080"] {
            config.server.bind = bind.to_owned();
            let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
            assert!(posture.gaps.is_empty(), "{bind} is loopback: {posture:?}");
            assert!(posture.permits_boot(), "{bind}");
        }

        config.server.bind = "0.0.0.0:8080".to_owned();
        config.deployment_accepts = vec![DeploymentGap::PlaintextListener];
        let accepted = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(accepted.permits_boot());
        assert_eq!(accepted.accepted, [DeploymentGap::PlaintextListener]);
        assert_eq!(accepted.gaps, [DeploymentGap::PlaintextListener]);
    }
}
