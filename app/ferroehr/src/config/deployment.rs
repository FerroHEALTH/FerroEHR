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
//! them. The top-level `deployment.profile` key gives the server a declared
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
use crate::config::rest::binds_loopback;
use crate::db::MigrationMode;

/// How rigorously this deployment is held to the separations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

/// A `deployment.profile` token that names no profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentProfileParseError {
    /// The token as written.
    pub unrecognized: String,
}

impl fmt::Display for DeploymentProfileParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown deployment.profile `{}` (expected `sandbox` or `production`)",
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeploymentGap {
    /// The clinical and party domains do not each connect on their own
    /// database credential.
    SharedCredential,
    /// Two of the three domains reach the same `PostgreSQL` cluster, read from
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
    /// Auditing is on and no category map applies: `[audit.categories]`
    /// declares no template and no archetype and `shipped_map` is `false`, so
    /// every access is recorded `unclassified`.
    ///
    /// `docs/law/eu/ehds/text.html Annex II 3.2(c)`: the logging records "the
    /// categories of data accessed".
    NoCategoryMap,
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
    /// `database.migrations = "apply"` with no `database.migrate_url`, or a relocated domain
    /// whose database is prepared on the domain's own runtime DSN because no
    /// `[database.<domain>] migrate_url` credential is named for it.
    MigrateOnRuntimeCredential,
    /// `auth.enabled` is `false`, so every request is served unauthenticated.
    ///
    /// `docs/law/eu/cra/text.html Annex I Part I(2)(d)`: products shall "ensure
    /// protection from unauthorised access by appropriate control mechanisms,
    /// including but not limited to authentication".
    AuthOff,
    /// Authentication is on and `authz.rbac.enabled` is `false`, so every
    /// authenticated caller may use every enabled surface, the admin and
    /// management groups included.
    ///
    /// `docs/law/eu/cra/text.html Annex I Part I(2)(d)`: products shall "ensure
    /// protection from unauthorised access by appropriate control mechanisms,
    /// including but not limited to authentication, identity or access
    /// management systems". Not reported while `auth_off` is, which already
    /// names the wider exposure.
    RbacOff,
    /// A listener speaks plaintext on an address reachable off the host: the
    /// main listener with no `[server.tls]` and a non-loopback `server.listen`,
    /// or the separate management listener (`management.listen`), which always
    /// binds every interface in plain HTTP.
    ///
    /// The management listener is covered by this gap rather than given TLS of
    /// its own, because serving it over TLS is a larger change than the risk it
    /// carries; accepting the gap by name stays the escape where the port is
    /// unreachable from outside the pod or host.
    ///
    /// `docs/law/eu/cra/text.html Annex I Part I(2)(e)`: products shall "protect
    /// the confidentiality of stored, transmitted or otherwise processed data
    /// … by encrypting relevant data … in transit". Accepting the gap by name
    /// records that a TLS-terminating ingress fronts the port.
    PlaintextListener,
    /// The enabled RESTful-ATNA feed (`[audit.fhir_feed] url`) posts the access
    /// records to a plain `http://` Audit Record Repository.
    ///
    /// `docs/law/eu/cra/text.html Annex I Part I(2)(e)`: the records name the
    /// patient and the professional, so they are relevant data in transit.
    PlaintextAuditFeed,
    /// The enabled multimedia store accepts a plain-HTTP S3 endpoint
    /// (`[record.multimedia] allow_http = true`), so the externalised blobs cross the
    /// network unencrypted (`docs/law/eu/cra/text.html Annex I Part I(2)(e)`).
    PlaintextObjectStore,
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
            Self::NoCategoryMap => "no_category_map",
            Self::OpenEhrAccessDefault => "open_ehr_access_default",
            Self::MigrateOnRuntimeCredential => "migrate_on_runtime_credential",
            Self::AuthOff => "auth_off",
            Self::RbacOff => "rbac_off",
            Self::PlaintextListener => "plaintext_listener",
            Self::PlaintextAuditFeed => "plaintext_audit_feed",
            Self::PlaintextObjectStore => "plaintext_object_store",
        }
    }

    /// What is missing, what was found, and what to change: the sentence the
    /// banner, the boot log, the refusal and `/management/status` all carry.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::SharedCredential => {
                "the clinical and party domains share a database credential; set \
                 [database.party] url (or [database.clinical] url) to roles that hold one \
                 domain each"
            }
            Self::SharedCluster => {
                "two domains reach the same PostgreSQL cluster (pg_control_system().system_identifier \
                 is equal); give the party and audit domains their own clusters through \
                 [database.party] and [database.audit] url"
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
            Self::NoCategoryMap => {
                "no category map applies: [audit.categories] declares no template and no \
                 archetype and shipped_map is false, so every access is recorded unclassified; \
                 declare this deployment's templates and archetypes, or set shipped_map = true"
            }
            Self::OpenEhrAccessDefault => {
                "[authz.rbac] ehr_access_default is `open`, so an EHR carrying no \
                 ACCESS_CONTROL_SETTINGS admits every caller the coarse layers already let \
                 through; set [authz.rbac] ehr_access_default = \"restricted\", or author the \
                 settings per EHR"
            }
            Self::MigrateOnRuntimeCredential => {
                "[database] migrate is `apply` on a credential that also serves requests; set [database] \
                 migrate_url to the credential that prepares the schema, give a relocated domain \
                 its database's migrator in [database.<domain>] migrate_url, or run migrate = \
                 \"verify\""
            }
            Self::AuthOff => {
                "[auth] enabled is false, so every request is served unauthenticated; set [auth] \
                 enabled = true with an [auth] issuer or [auth.basic] users configured"
            }
            Self::RbacOff => {
                "[authz.rbac] enabled is false, so every authenticated caller may use every \
                 enabled surface, the admin API and the management endpoints included; set \
                 [authz.rbac] enabled = true and give each user the roles it needs"
            }
            Self::PlaintextListener => {
                "a listener serves plain HTTP on a routable address ([server] listen with \
                 no [server.tls], or [management] port, which binds every interface without \
                 TLS), so clinical data and credentials cross the network unencrypted; enable \
                 [server.tls] or listen on loopback, unset [management] port so the surface shares \
                 the main listener, or accept plaintext_listener by name when a TLS-terminating \
                 ingress fronts the port or the management port is unreachable off the host"
            }
            Self::PlaintextAuditFeed => {
                "the enabled audit FHIR feed ([audit.fhir_feed] url) posts the access records to a \
                 plain http:// repository, so who accessed which patient crosses the network \
                 unencrypted; use an https:// URL, or accept plaintext_audit_feed by name where \
                 the repository is reached on a trusted segment"
            }
            Self::PlaintextObjectStore => {
                "the enabled multimedia store is a plain-HTTP S3 endpoint (an http:// \
                 record.multimedia.url), so the externalised clinical blobs cross the network \
                 unencrypted; use an https:// url, or accept plaintext_object_store by name \
                 where the store is reached on a trusted segment"
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
    /// The audit pool's cluster.
    ///
    /// Measured like the other two, because the audit trail's separation is
    /// its own duty rather than a consequence of theirs:
    /// `docs/law/ch/dpo/text-de.html Art. 4 Abs. 5` keeps the log "getrennt vom
    /// System, in welchem die Personendaten bearbeitet werden".
    pub audit: Option<String>,
}

impl ClusterIdentities {
    /// Whether any two known identifiers are equal.
    #[must_use]
    pub fn any_shared(&self) -> bool {
        let known: Vec<&String> = [&self.clinical, &self.party, &self.audit]
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
    /// database, because `[database] migrate_url` names one database and the domain
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
        if !config.database.pseudonymisation_domains_are_separated() {
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
            || !(audit.store.enabled || audit.syslog.is_some() || audit.fhir_feed.is_some())
        {
            gaps.push(DeploymentGap::AuditOff);
        } else if audit.fail_mode == crate::system_log::config::FailMode::Open {
            // Only while a trail exists: a disabled trail makes the fail mode
            // moot, and naming both would report one posture twice.
            gaps.push(DeploymentGap::AuditFailsOpen);
        }
        if audit.enabled
            && audit.categories.source() == crate::system_log::categories::MapSource::None
        {
            gaps.push(DeploymentGap::NoCategoryMap);
        }
        // NOTE: no openEHR spec governs this, our own design/extension; the configured default is the
        // effective one: the REST layer applies it with RBAC off, and validation refuses it unenforced.
        if config.authz.rbac.ehr_access_default == crate::config::authz::EhrAccessDefault::Open {
            gaps.push(DeploymentGap::OpenEhrAccessDefault);
        }
        if config.database.migrations == MigrationMode::Apply
            && (!config.database.migrator_is_separated()
                || !databases.prepared_on_runtime_credential.is_empty())
        {
            gaps.push(DeploymentGap::MigrateOnRuntimeCredential);
        }
        if !config.auth.shared.enabled {
            gaps.push(DeploymentGap::AuthOff);
        } else if !config.authz.rbac.enabled {
            gaps.push(DeploymentGap::RbacOff);
        }
        let plaintext_main = config.server.tls.is_none() && !binds_loopback(config.server.listen);
        let plaintext_management = config.management.enabled && config.management.listen.is_some();
        if plaintext_main || plaintext_management {
            gaps.push(DeploymentGap::PlaintextListener);
        }
        if config.audit.enabled
            && config
                .audit
                .fhir_feed
                .as_ref()
                .is_some_and(|feed| is_plaintext_http(feed.endpoint.url.expose()))
        {
            gaps.push(DeploymentGap::PlaintextAuditFeed);
        }
        if config.record.multimedia.enabled && config.record.multimedia.allows_http() {
            gaps.push(DeploymentGap::PlaintextObjectStore);
        }
        let accepted = gaps
            .iter()
            .copied()
            .filter(|gap| config.deployment.accepts.contains(gap))
            .collect();
        Self {
            profile: config.deployment.profile,
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
            "deployment.profile = \"production\" refuses to start: this deployment has not made \
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

/// Whether a URL names the plaintext `http` scheme, compared case-insensitively
/// (RFC 3986 §3.1).
fn is_plaintext_http(url: &str) -> bool {
    url.split_once("://")
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("http"))
}

#[cfg(test)]
mod tests;
