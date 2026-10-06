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
    /// Calendar years to keep records, in place of `retention_days`
    /// (`FERROEHR__AUDIT__STORE__RETENTION_YEARS`); unset by default.
    ///
    /// The floors and ceilings the horizon answers to are written in years, and
    /// a day count can only approximate them: three calendar years span 1095
    /// or 1096 days, so a three-year floor and a three-year ceiling leave no
    /// day count that satisfies both. A horizon in years is compared exactly
    /// and reaped with calendar arithmetic. Setting it together with a non-zero
    /// `retention_days`, or to `0`, is a boot error.
    pub retention_years: Option<u32>,
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
    /// Seconds between two scheduled verifications of the store's hash chain
    /// (`FERROEHR__AUDIT__STORE__VERIFY_INTERVAL_SECONDS`); `0` turns the
    /// schedule off. Default one day.
    ///
    /// Each run is `audit.verify_audit_chain()`, a full pass over the trail; a
    /// finding is logged at `ERROR`, counted and reported by the `audit_chain`
    /// health indicator (`docs/law/eu/cra/text.html Annex I Part I(2)(f)`:
    /// "report on corruptions").
    pub verify_interval_seconds: u64,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            // One day: a full digest pass over the trail is too heavy for an
            // hourly cadence on a large repository, and a day bounds how long
            // damage goes unreported.
            verify_interval_seconds: 86_400,
        }
    }
}

/// The horizon the local store keeps records for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase", tag = "unit", content = "value")]
pub enum Retention {
    /// Records are kept forever (`retention_days = 0`, no `retention_years`).
    Forever,
    /// Records older than this many days are reaped.
    Days(u32),
    /// Records older than the same calendar date this many years ago are
    /// reaped.
    Years(u32),
}

impl Retention {
    /// The `retention_days` / `retention_years` spelling, for messages.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Retention::Forever => "retention_days = 0 (keep forever)".to_owned(),
            Retention::Days(days) => format!("retention_days = {days}"),
            Retention::Years(years) => format!("retention_years = {years}"),
        }
    }
}

impl StoreConfig {
    /// The horizon this configuration states. A `retention_years` takes the
    /// place of `retention_days`; the boot validation refuses the two together.
    #[must_use]
    pub fn retention(&self) -> Retention {
        match (self.retention_years, self.retention_days) {
            (Some(years), _) => Retention::Years(years),
            (None, 0) => Retention::Forever,
            (None, days) => Retention::Days(days),
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
    /// `[audit.categories]` — the template and archetype map every access is
    /// classified through by EHDS priority category (Annex II 3.2(c)). Empty by
    /// default: FerroEHR ships no map, and an unmapped access is recorded
    /// `unclassified`.
    pub categories: crate::system_log::categories::CategoryMapConfig,
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
            categories: crate::system_log::categories::CategoryMapConfig::default(),
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
    /// Calendar years the local store keeps records, when the horizon is stated
    /// in years (`retention_years`).
    pub retention_years: Option<u32>,
    /// The digest of the `[audit.categories]` map every access record is
    /// classified through and carries (EHDS Annex II 3.2(c)); `None` only for a
    /// map the boot validation refuses.
    pub category_map: Option<crate::system_log::categories::MapDigest>,
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
            retention_years: config.store.retention_years,
            // NOTE: an invalid map has no digest; `FerroEhrConfig::validate` refuses to boot on it.
            category_map: config.categories.compile().ok().map(|map| map.digest()),
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
            match (self.retention_years, self.retention_days) {
                (Some(years), _) => format!("local store on, {years}-year retention"),
                (None, 0) => "local store on, kept forever".to_owned(),
                (None, days) => format!("local store on, {days}-day retention"),
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

/// A bound a jurisdiction sets on how long an access-log record is kept,
/// written in calendar years as the law writes it, with the provision it rests
/// on.
///
/// A horizon in years (`retention_years`) is compared against [`Self::years`]
/// exactly. A horizon in days is compared conservatively against [`Self::days`]:
/// for a floor, the most that many calendar years can span; for a ceiling, the
/// fewest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionBound {
    /// The bound in calendar years.
    pub years: u32,
    /// The bound in days a day-counted horizon is held to.
    pub days: u32,
    /// The provision, as the boot refusal names it.
    pub source: &'static str,
}

impl RetentionBound {
    /// Whether `retention` keeps records at least this long.
    #[must_use]
    pub fn admits_as_floor(self, retention: Retention) -> bool {
        match retention {
            Retention::Forever => true,
            Retention::Days(days) => days >= self.days,
            Retention::Years(years) => years >= self.years,
        }
    }

    /// Whether `retention` keeps records no longer than this.
    #[must_use]
    pub fn admits_as_ceiling(self, retention: Retention) -> bool {
        match retention {
            Retention::Forever => false,
            Retention::Days(days) => days <= self.days,
            Retention::Years(years) => years <= self.years,
        }
    }
}

/// The Member States of the European Union, as ISO 3166-1 alpha-2 codes.
///
/// EHDS is a Union regulation, "binding in its entirety and directly applicable
/// in all Member States" (Regulation (EU) 2025/327 Art. 105,
/// `docs/law/eu/ehds/text.html`). The EEA states outside the Union (Iceland,
/// Liechtenstein, Norway) are deliberately absent: the act is marked "Text with
/// EEA relevance", and it reaches them only through an EEA Joint Committee
/// decision, which the vendored corpus does not carry.
pub const EU_MEMBER_STATES: [&str; 27] = [
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
];

/// The access-log floor EHDS sets in every Member State.
///
/// Art. 9(2): the access information "shall be available for at least three
/// years from each date of access to the data" (`docs/law/eu/ehds/text.html`).
/// Art. 105 applies Art. 9 from 26 March 2029 to the priority categories of
/// Art. 14(1)(a) to (c) and from 26 March 2031 to (d) to (f); the floor is
/// enforced from now on, which only keeps a log longer. Three calendar years
/// can span 1096 days, so that is the floor a day-counted horizon is held to.
const EHDS_FLOOR: RetentionBound = RetentionBound {
    years: 3,
    days: 1096,
    source: "three years from each date of access, EHDS Art. 9(2), Regulation (EU) 2025/327, \
             applying under Art. 105 from 26 March 2029 to Art. 14(1)(a)-(c) data and from \
             26 March 2031 to (d)-(f)",
};

/// The minimum an access-log record must be kept in `jurisdiction`, where one
/// is registered (#3242, #3625).
///
/// Keyed by ISO 3166-1 alpha-2, the same key the identifier rules carry, so a
/// deployment's jurisdictions are the ones its `[privacy.identifier_scan]`
/// rules name. A jurisdiction with no registered floor returns `None`: an
/// unknown requirement is never guessed at. Every EU Member State
/// ([`EU_MEMBER_STATES`]) carries the EHDS floor, and a national floor wins
/// where it is longer.
///
/// `NL`: five years from the moment the entry is written, Besluit vaststelling
/// bewaartermijn logging (<https://wetten.overheid.nl/BWBR0042391>, Stcrt.
/// 2019, 38007) under Art. 5 of the Besluit elektronische gegevensverwerking
/// door zorgaanbieders (<https://wetten.overheid.nl/BWBR0040238>), which binds
/// the retention to NEN 7513. Five calendar years never exceed 1830 days, so
/// that is the floor a day-counted horizon is held to.
///
/// `CH`: at least one year, and kept apart from the processing system, for the
/// logs of a large-scale automated processing of sensitive personal data —
/// Datenschutzverordnung (DSV, SR 235.11) Art. 4 Abs. 5 as amended on
/// 1 December 2025 (`docs/law/ch/dpo/text-de.html`,
/// <https://www.fedlex.admin.ch/eli/cc/2022/568/de>). One calendar year never
/// exceeds 366 days. Switzerland is outside the Union, so the EHDS floor does
/// not reach it.
#[must_use]
pub fn retention_floor(jurisdiction: &str) -> Option<RetentionBound> {
    let national = match jurisdiction {
        "NL" => Some(RetentionBound {
            years: 5,
            days: 1830,
            source: "five years, Besluit vaststelling bewaartermijn logging, \
                     https://wetten.overheid.nl/BWBR0042391",
        }),
        "CH" => Some(RetentionBound {
            years: 1,
            days: 366,
            source: "one year, DSV Art. 4 Abs. 5, \
                     https://www.fedlex.admin.ch/eli/cc/2022/568/de",
        }),
        _ => None,
    };
    let union = EU_MEMBER_STATES
        .contains(&jurisdiction)
        .then_some(EHDS_FLOOR);
    match (national, union) {
        (Some(national), Some(union)) if union.years > national.years => Some(union),
        (Some(national), _) => Some(national),
        (None, union) => union,
    }
}

/// The maximum an access-log record may be kept in `jurisdiction`, where one
/// is registered (#3346).
///
/// The mirror of [`retention_floor`], keyed the same way, and empty for
/// every jurisdiction but one: a rule that caps a log is rare, and an unknown
/// cap is never guessed at.
///
/// `DE`: the access logs of a telematics-infrastructure application under
/// §§ 327 and 334 Abs. 1 SGB V are kept for the three-year limitation period of
/// § 195 BGB and then deleted "unverzüglich" — SGB V § 309 Abs. 1 and Abs. 3
/// (`docs/law/de/sgb-v/BJNR024820988.xml`,
/// <https://www.gesetze-im-internet.de/sgb_5/__309.html>). Three calendar years
/// are never fewer than 1095 days, so that is the ceiling a day-counted horizon
/// is held to. The provision binds "die Verantwortlichen nach § 307", so it
/// reaches a deployment only once that deployment declares itself one
/// (`[audit.store] sgb_v_309_controller`), which is why `declared` gates the row
/// rather than the jurisdiction alone.
#[must_use]
pub fn retention_ceiling(jurisdiction: &str, sgb_v_309_controller: bool) -> Option<RetentionBound> {
    match jurisdiction {
        "DE" if sgb_v_309_controller => Some(RetentionBound {
            years: 3,
            days: 1095,
            source: "the three-year limitation period of SGB V § 309 Abs. 1, after which Abs. 3 \
                     requires deletion unverzüglich, \
                     https://www.gesetze-im-internet.de/sgb_5/__309.html",
        }),
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
        assert!(c.categories.templates.is_empty() && c.categories.archetypes.is_empty());
    }

    /// Every EU Member State carries the EHDS floor of three years, a longer
    /// national floor wins, and a state outside the Union keeps its own.
    #[test]
    fn the_ehds_floor_reaches_every_member_state_and_a_longer_national_floor_wins() {
        for state in ["DE", "FI", "SE", "FR", "AT"] {
            let floor = retention_floor(state).expect(state);
            assert_eq!((floor.years, floor.days), (3, 1096), "{state}");
            assert!(floor.source.contains("EHDS Art. 9(2)"), "{state}");
        }
        let nl = retention_floor("NL").expect("NL");
        assert_eq!(
            (nl.years, nl.days),
            (5, 1830),
            "the five Dutch years outlast the three EHDS years"
        );
        let ch = retention_floor("CH").expect("CH");
        assert_eq!(
            (ch.years, ch.days),
            (1, 366),
            "Switzerland is not a Member State"
        );
        for outside in ["NO", "GB", "IS", "LI"] {
            assert_eq!(retention_floor(outside), None, "{outside}");
        }
        assert_eq!(EU_MEMBER_STATES.len(), 27);
    }

    /// A horizon in years is compared exactly against a bound in years; a
    /// horizon in days conservatively, so three years pass a three-year floor
    /// and a three-year ceiling while no day count passes both.
    #[test]
    fn a_horizon_in_years_meets_bounds_in_years_exactly() {
        let floor = retention_floor("DE").expect("the EHDS floor");
        let ceiling = retention_ceiling("DE", true).expect("the SGB V ceiling");
        let three = Retention::Years(3);
        assert!(floor.admits_as_floor(three) && ceiling.admits_as_ceiling(three));
        assert!(!floor.admits_as_floor(Retention::Years(2)));
        assert!(!ceiling.admits_as_ceiling(Retention::Years(4)));
        for days in 1090..1100 {
            let horizon = Retention::Days(days);
            assert!(
                !(floor.admits_as_floor(horizon) && ceiling.admits_as_ceiling(horizon)),
                "{days} days cannot satisfy a three-year floor and ceiling at once"
            );
        }
        assert!(floor.admits_as_floor(Retention::Forever));
        assert!(!ceiling.admits_as_ceiling(Retention::Forever));
        assert_eq!(retention_ceiling("DE", false), None);
    }

    /// `retention_years` takes the place of `retention_days`.
    #[test]
    fn the_store_states_one_horizon() {
        let mut store = StoreConfig::default();
        assert_eq!(store.retention(), Retention::Forever);
        store.retention_days = 30;
        assert_eq!(store.retention(), Retention::Days(30));
        store.retention_days = 0;
        store.retention_years = Some(3);
        assert_eq!(store.retention(), Retention::Years(3));
    }
}
