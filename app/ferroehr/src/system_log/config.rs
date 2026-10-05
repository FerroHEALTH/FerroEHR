// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Audit configuration ([`AuditConfig`]) — the `[audit]` section of the one
//! config tree ([`crate::config::FerroEhrConfig`]); no loader of its own.
//!
//! No openEHR spec governs configuration — our own design. The tree is
//! sink-structured: the shared event/queue settings at the root, one
//! sub-table per sink — `[audit.store]` (the local Audit Record Repository,
//! the durability anchor, **on by default**), `[audit.syslog]` (the classic
//! IHE ITI-20 DICOM-over-syslog feed, opt-in), `[audit.fhir_feed]` (the
//! RESTful-ATNA ITI-20 ATX:FHIR Feed, opt-in). Auditing itself is **on by
//! default** with only the local store active: compliance out of the box,
//! nothing leaves the node.

use serde::{Deserialize, Serialize};

use crate::config::secret::SecretUrl;

/// The syslog transport to the Audit Record Repository (ARR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// RFC 5426 UDP (the reference Elastic/Logstash stack default, port 514).
    #[default]
    Udp,
    /// RFC 5425 TLS (the IHE-recommended secure transport).
    Tls,
}

/// Behaviour when an audit record cannot be enqueued/delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FailMode {
    /// Log + meter the drop and let the request succeed (common ATNA default).
    #[default]
    Open,
    /// Reject auditable operations with `503` when auditing cannot be
    /// delivered — a full queue, or (with the store on) a store that stopped
    /// accepting writes. No un-audited PHI access.
    Closed,
}

/// The local Audit Record Repository (`[audit.store]`) — the PG-backed store
/// ([`crate::system_log::store`]), the durability anchor of the subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoreConfig {
    /// Persist every record locally (`FERROEHR__AUDIT__STORE__ENABLED`).
    /// **On by default** (owner posture: compliance out of the box).
    pub enabled: bool,
    /// Days to keep records; `0` = keep forever
    /// (`FERROEHR__AUDIT__STORE__RETENTION_DAYS`). Applied hourly by the
    /// retention reaper.
    pub retention_days: u32,
    /// Whether this deployment is one of the controllers SGB V § 307 names for
    /// a German telematics-infrastructure application
    /// (`FERROEHR__AUDIT__STORE__SGB_V_309_CONTROLLER`). **Off by default.**
    ///
    /// SGB V § 309 Abs. 1 binds "die Verantwortlichen nach § 307" of the
    /// applications under §§ 327 and 334 Abs. 1, and Abs. 3 then requires the
    /// log data to be deleted "unverzüglich" once the three-year limitation
    /// period has run (`docs/law/de/sgb-v/BJNR024820988.xml`). That is a
    /// CEILING on access-log retention, and it reaches a CDR only when the
    /// deploying organisation actually is, or acts for, one of those
    /// controllers — which the software cannot tell from its configuration.
    /// So the deployment declares it here, beside the horizon it bounds, and the
    /// DE ceiling applies from then on.
    pub sgb_v_309_controller: bool,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 0,
            sgb_v_309_controller: false,
        }
    }
}

/// The classic ATNA feed (`[audit.syslog]`): the DICOM PS3.15 §A.5 XML
/// record over syslog (IHE ITI TF-2 ITI-20; RFC 5424 message, RFC 5426 UDP
/// or RFC 5425 TLS transport).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyslogConfig {
    /// Ship records to an external ARR over syslog
    /// (`FERROEHR__AUDIT__SYSLOG__ENABLED`).
    pub enabled: bool,
    /// ARR host (`FERROEHR__AUDIT__SYSLOG__HOST`).
    pub host: String,
    /// ARR port (`FERROEHR__AUDIT__SYSLOG__PORT`).
    pub port: u16,
    /// Transport (`FERROEHR__AUDIT__SYSLOG__TRANSPORT`): `udp` | `tls`.
    pub transport: Transport,
    /// PEM file with the ARR CA to trust for TLS
    /// (`FERROEHR__AUDIT__SYSLOG__TLS_CA_FILE`).
    pub tls_ca_file: Option<String>,
    /// Client-certificate PEM file for mutual TLS
    /// (`FERROEHR__AUDIT__SYSLOG__TLS_IDENTITY_CERT_FILE`).
    pub tls_identity_cert_file: Option<String>,
    /// Client-key PEM file for mutual TLS
    /// (`FERROEHR__AUDIT__SYSLOG__TLS_IDENTITY_KEY_FILE`).
    pub tls_identity_key_file: Option<String>,
}

impl Default for SyslogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "localhost".to_owned(),
            port: 514,
            transport: Transport::default(),
            tls_ca_file: None,
            tls_identity_cert_file: None,
            tls_identity_key_file: None,
        }
    }
}

/// The RESTful-ATNA feed (`[audit.fhir_feed]`): ITI-20 **ATX: FHIR Feed** —
/// HTTP `POST {url}/AuditEvent` of the FHIR R4 `AuditEvent` (IHE BALP shape)
/// to an external Audit Record Repository.
///
/// When the local store is on, the feed drains the store's outbox
/// (`delivered_fhir_feed_at IS NULL`), so a down ARR loses nothing; with the
/// store off it ships in-drain with bounded retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FhirFeedConfig {
    /// Ship records to an external FHIR ARR
    /// (`FERROEHR__AUDIT__FHIR_FEED__ENABLED`).
    pub enabled: bool,
    /// The ARR's FHIR base URL (`FERROEHR__AUDIT__FHIR_FEED__URL`); the
    /// `AuditEvent` endpoint is `{url}/AuditEvent`. Credentials in the URL
    /// (basic auth) are redacted from every rendering.
    pub url: SecretUrl,
    /// Outbox rows shipped per poll (`FERROEHR__AUDIT__FHIR_FEED__BATCH_SIZE`).
    pub batch_size: i64,
    /// Outbox poll interval when idle, in milliseconds
    /// (`FERROEHR__AUDIT__FHIR_FEED__POLL_INTERVAL_MS`).
    pub poll_interval_ms: u64,
    /// Per-record POST retries before the record is left pending (store on)
    /// or dropped + metered (store off)
    /// (`FERROEHR__AUDIT__FHIR_FEED__MAX_RETRIES`).
    pub max_retries: usize,
}

impl Default for FhirFeedConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: SecretUrl::new("http://localhost:8080/fhir"),
            batch_size: 64,
            poll_interval_ms: 2000,
            max_retries: 3,
        }
    }
}

/// ATNA audit configuration (`[audit]`). Every field has a default; the
/// all-defaults tree is **auditing on with only the local store active**.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuditConfig {
    /// Master switch (`FERROEHR__AUDIT__ENABLED`). **On by default** — every
    /// deployment gets a queryable audit trail with zero external
    /// dependencies (the sinks decide where records go).
    pub enabled: bool,
    /// Enterprise/site id → `AuditEnterpriseSiteID`
    /// (`FERROEHR__AUDIT__ENTERPRISE_SITE_ID`).
    pub enterprise_site_id: Option<String>,
    /// Audit source id → `AuditSourceID` and the destination participant
    /// (`FERROEHR__AUDIT__SOURCE_ID`).
    pub source_id: String,
    /// Fill value for empty mandatory fields
    /// (`FERROEHR__AUDIT__VALUE_IF_MISSING`).
    pub value_if_missing: String,
    /// Skip successful-login records (`FERROEHR__AUDIT__SUPPRESS_LOGIN_EVENTS`).
    /// Rejected accesses (401/403) are always recorded.
    pub suppress_login_events: bool,
    /// Failure mode (`FERROEHR__AUDIT__FAIL_MODE`): `open` | `closed`.
    pub fail_mode: FailMode,
    /// Enrich the patient participant via a background indexed lookup of
    /// `ehr.subject_id` (`FERROEHR__AUDIT__RESOLVE_SUBJECT`). On by default —
    /// the IHE BALP `Patient*` patterns and the patient-centric audit search
    /// need the subject; the lookup runs only on the background drain.
    pub resolve_subject: bool,
    /// Bounded audit queue capacity (`FERROEHR__AUDIT__QUEUE_CAPACITY`).
    /// Sized for write-path bursts: the drain persists in multi-row batches,
    /// so the queue only needs to ride out sink latency spikes, but a loaded
    /// write path can enqueue thousands per second.
    pub queue_capacity: usize,
    /// This node's advertised network address → the destination
    /// network-access-point (`FERROEHR__AUDIT__SERVER_HOST`); the
    /// `value_if_missing` fill when unset.
    pub server_host: Option<String>,
    /// The request header carrying the caller's declared purpose of use
    /// (`FERROEHR__AUDIT__PURPOSE_HEADER`), recorded on every access record.
    ///
    /// NEN 7513 asks on whose authority a record was accessed, and EHDS Art. 9
    /// asks why; neither is derivable from the request, so the caller declares
    /// it and the trail records what was declared. No openEHR spec governs
    /// this and IHE carries the equivalent in a SAML attribute rather than a
    /// header, so the header is our own design.
    pub purpose_header: String,
    /// The purpose codes this deployment accepts
    /// (`FERROEHR__AUDIT__PURPOSE_CODES`).
    ///
    /// Empty (the default) records whatever the caller declares. A non-empty
    /// list records a declared code only when it is on the list, so a
    /// deployment that has agreed a vocabulary does not accumulate a trail of
    /// free text that means nothing at review time.
    pub purpose_codes: Vec<String>,
    /// The legal basis this deployment processes under
    /// (`FERROEHR__AUDIT__LEGAL_BASIS`), recorded on every access record.
    ///
    /// A deployment-level fact, not a per-request one: the controller
    /// establishes the GDPR Art. 6/9 condition once
    /// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>) and every access under
    /// this deployment carries it. Unset records nothing rather than a guess.
    pub legal_basis: Option<String>,
    /// `[audit.store]` — the local Audit Record Repository.
    pub store: StoreConfig,
    /// `[audit.syslog]` — the classic DICOM-over-syslog feed.
    pub syslog: SyslogConfig,
    /// `[audit.fhir_feed]` — the RESTful-ATNA FHIR `AuditEvent` feed.
    pub fhir_feed: FhirFeedConfig,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            enterprise_site_id: None,
            source_id: "ferroehr".to_owned(),
            value_if_missing: "UNKNOWN".to_owned(),
            suppress_login_events: true,
            fail_mode: FailMode::default(),
            resolve_subject: true,
            queue_capacity: 8192,
            server_host: None,
            purpose_header: "x-purpose-of-use".to_owned(),
            purpose_codes: Vec::new(),
            legal_basis: None,
            store: StoreConfig::default(),
            syslog: SyslogConfig::default(),
            fhir_feed: FhirFeedConfig::default(),
        }
    }
}

/// The audit posture a deployment runs under, as boot, `/health/readiness` and
/// `/management/info` report it.
///
/// Two of its states are legitimate to run and wrong to run silently (#3238):
/// auditing off leaves no access log and no EHDS logging component, and
/// `fail_mode = "open"` drops a record the queue cannot take while the request
/// succeeds. Each is stated once as a [caution](Self::cautions), the same
/// sentence on every surface, so a collector can alert on it without parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AuditPosture {
    /// The `[audit] enabled` master switch.
    pub enabled: bool,
    /// The behaviour when a record cannot be taken.
    pub fail_mode: FailMode,
    /// Whether the local Audit Record Repository is on.
    pub local_store: bool,
    /// Days the local store keeps records; `0` = forever.
    pub retention_days: u32,
}

impl AuditPosture {
    /// The posture a configuration describes.
    #[must_use]
    pub fn of(config: &AuditConfig) -> Self {
        Self {
            enabled: config.enabled,
            fail_mode: config.fail_mode,
            local_store: config.store.enabled,
            retention_days: config.store.retention_days,
        }
    }

    /// The postures worth a warning, as the one sentence every surface shows.
    ///
    /// Empty when auditing is on and fails closed. Order is severity: a
    /// disabled trail makes the fail mode moot, so it is the only caution then.
    #[must_use]
    pub fn cautions(self) -> Vec<&'static str> {
        if !self.enabled {
            return vec![
                "auditing is disabled: no access log is written and the EHDS logging \
                 component is off (set [audit] enabled = true)",
            ];
        }
        match self.fail_mode {
            FailMode::Open => vec![
                "fail_mode is open: a record the audit queue cannot take is dropped and \
                 metered while the request succeeds (set [audit] fail_mode = \"closed\" to \
                 refuse an unrecorded access with 503)",
            ],
            FailMode::Closed => Vec::new(),
        }
    }

    /// The one-line summary the readiness indicator carries when the trail is on.
    #[must_use]
    pub fn summary(self) -> String {
        let store = if self.local_store {
            match self.retention_days {
                0 => "local store on, kept forever".to_owned(),
                days => format!("local store on, {days}-day retention"),
            }
        } else {
            "local store off".to_owned()
        };
        let mode = match self.fail_mode {
            FailMode::Open => "open",
            FailMode::Closed => "closed",
        };
        format!("fail_mode={mode}; {store}")
    }
}

/// The minimum number of days a jurisdiction requires an access-log record to
/// be kept, where one is registered (#3242).
///
/// Keyed by ISO 3166-1 alpha-2, the same key the identifier rules carry, so a
/// deployment's jurisdictions are the ones its `[privacy.identifier_scan]`
/// rules name. A jurisdiction with no registered floor returns `None`: an
/// unknown requirement is never guessed at.
///
/// `NL`: five years from the moment the entry is written, Besluit vaststelling
/// bewaartermijn logging (<https://wetten.overheid.nl/BWBR0042391>, Stcrt.
/// 2019, 38007) under Art. 5 of the Besluit elektronische gegevensverwerking
/// door zorgaanbieders (<https://wetten.overheid.nl/BWBR0040238>), which binds
/// the retention to NEN 7513. Five calendar years never exceed 1830 days, so
/// that is the floor in days.
///
/// `CH`: at least one year, and kept apart from the processing system, for the
/// logs of a large-scale automated processing of sensitive personal data —
/// Datenschutzverordnung (DSV, SR 235.11) Art. 4 Abs. 5 as amended on
/// 1 December 2025 (`docs/law/ch/dpo/text-de.html`,
/// <https://www.fedlex.admin.ch/eli/cc/2022/568/de>). One calendar year never
/// exceeds 366 days, so that is the floor in days.
#[must_use]
pub fn retention_floor_days(jurisdiction: &str) -> Option<u32> {
    match jurisdiction {
        "NL" => Some(1830),
        "CH" => Some(366),
        _ => None,
    }
}

/// The maximum number of days a jurisdiction allows an access-log record to be
/// kept, where one is registered (#3346).
///
/// The mirror of [`retention_floor_days`], keyed the same way, and empty for
/// every jurisdiction but one: a rule that caps a log is rare, and an unknown
/// cap is never guessed at.
///
/// `DE`: the access logs of a telematics-infrastructure application under
/// §§ 327 and 334 Abs. 1 SGB V are kept for the three-year limitation period of
/// § 195 BGB and then deleted "unverzüglich" — SGB V § 309 Abs. 1 and Abs. 3
/// (`docs/law/de/sgb-v/BJNR024820988.xml`,
/// <https://www.gesetze-im-internet.de/sgb_5/__309.html>). Three calendar years
/// are never fewer than 1095 days, so that is the ceiling in days: a horizon at
/// or below it cannot outlast the period. The provision binds "die
/// Verantwortlichen nach § 307", so it reaches a deployment only once that
/// deployment declares itself one (`[audit.store] sgb_v_309_controller`), which is
/// why `declared` gates the row rather than the jurisdiction alone.
#[must_use]
pub fn retention_ceiling_days(jurisdiction: &str, sgb_v_309_controller: bool) -> Option<u32> {
    match jurisdiction {
        "DE" if sgb_v_309_controller => Some(1095),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_with_store_only() {
        let c = AuditConfig::default();
        assert!(c.enabled, "auditing is on by default (owner posture)");
        assert!(c.store.enabled, "the local store is the default sink");
        assert_eq!(c.store.retention_days, 0, "keep forever by default");
        assert!(!c.syslog.enabled, "forwarding is opt-in");
        assert!(!c.fhir_feed.enabled, "forwarding is opt-in");
        assert_eq!(c.syslog.port, 514);
        assert_eq!(c.syslog.transport, Transport::Udp);
        assert_eq!(c.value_if_missing, "UNKNOWN");
        assert!(c.suppress_login_events);
        assert!(c.resolve_subject);
        assert_eq!(c.fail_mode, FailMode::Open);
        assert_eq!(c.fhir_feed.batch_size, 64);
    }
}
