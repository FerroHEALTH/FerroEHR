// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The deployment report `ferroehr report` writes: one redacted JSON document
//! naming what a deployment runs, for a complaint, a non-conformity finding or
//! a serious-incident report.
//!
//! Regulation (EU) 2025/327 (`docs/law/eu/ehds/text.html` Art. 44(5)) has a
//! market surveillance authority pass on "the data necessary for the
//! identification of the EHR system concerned"; this document is that data for
//! one deployment. No openEHR spec governs it — our own design.
//!
//! The parts are the build provenance with the manufacturer
//! ([`BuildInfo`]), the platform the binary was built for, the enabled cargo
//! features, the `openehr-*` crate versions linked, the licence summary
//! `GET /management/status` serves, the deployment posture, the migration level of
//! every schema, and the effective configuration. The [`Report::manifest`]
//! names every part with its state, so a part that could not be read carries
//! its reason rather than an empty value.
//!
//! **Nothing in it identifies a person.** It reads the build, the
//! configuration, the cluster identities and the migration bookkeeping, never
//! a clinical, demographic or audit relation. The configuration is the
//! type-redacted tree
//! ([`FerroEhrConfig::to_redacted_table`]), every URL `userinfo` masked by
//! [`RedactedUrl`]; a database error quoted in a reason
//! has every configured DSN's user name and password masked too, since a
//! refused authentication names the role it tried.

use std::time::Duration;

use serde::Serialize;

use crate::config::FerroEhrConfig;
use crate::config::deployment::{DatabaseFacts, DeploymentPosture};
use crate::db::domain::Domain;
use crate::db::{DatabaseLevels, DbError};
use crate::licence::state::{LicenceState, LicenceStatus};
use crate::service::error::ErrorChain;
use crate::system_log::config::AuditPosture;
use crate::telemetry::build_info::BuildInfo;
use ferrohealth_secret::REDACTED;
use ferrohealth_secret::secret_url::RedactedUrl;

/// The document's format identifier, bumped when a part changes shape.
pub const FORMAT: &str = "ferroehr-report/1";

/// How long the report waits for a database read to finish before naming the
/// database unreachable.
const DATABASE_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the report's own pools wait for a connection; a pool retries a
/// refused connection until then.
const POOL_ACQUIRE_SECS: u64 = 5;

/// Why the image part is never filled: a process cannot read the digest of the
/// image it runs from.
const IMAGE_REASON: &str = "a running binary cannot read the digest of the image it was started \
                            from; attach the runtime's record of it (`docker inspect --format \
                            '{{index .RepoDigests 0}}'`, or the pod's `status.containerStatuses[].\
                            imageID`)";

/// The deployment report.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// [`FORMAT`].
    pub format: &'static str,
    /// When the report was assembled, in UTC.
    pub generated_at: jiff::Timestamp,
    /// Every part, with its state.
    pub manifest: Vec<ManifestEntry>,
    /// Version, commit, build date, `rustc`, the active specification pins,
    /// the audit posture and the manufacturer.
    pub build: BuildInfo,
    /// The platform the binary was built for.
    pub target: Target,
    /// The optional-integration cargo features this binary was built with.
    pub features: Vec<&'static str>,
    /// The `openehr-*` crates this binary links, with their versions.
    pub openehr_crates: Vec<CrateVersion>,
    /// Whether those versions are the ones published to crates.io or the
    /// source tree's.
    pub openehr_crates_origin: CrateOrigin,
    /// The licence summary `GET /management/status` serves; `None` when the embedded
    /// licence anchors could not be read.
    pub licence: Option<LicenceStatus>,
    /// The deployment posture, measured over the databases when they answered
    /// and from the configuration alone otherwise.
    pub deployment: DeploymentPosture,
    /// The migration level of every schema; `None` when no database answered.
    pub schema: Option<Vec<DatabaseLevels>>,
    /// The effective configuration, redacted; `None` when it could not be
    /// rendered.
    pub configuration: Option<toml::Table>,
}

/// One part of the report and its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestEntry {
    /// The part's field name in [`Report`].
    pub part: &'static str,
    /// Whether the part holds what it names.
    pub status: PartStatus,
    /// Why the part is partial or unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The state of one part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartStatus {
    /// The part holds everything it names.
    Included,
    /// The part holds less than it names; the reason says what is missing.
    Partial,
    /// The part could not be read; the reason says why.
    Unavailable,
}

/// The platform a binary was built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Target {
    /// The CPU architecture (`std::env::consts::ARCH`).
    pub arch: &'static str,
    /// The operating system (`std::env::consts::OS`).
    pub os: &'static str,
    /// Whether debug assertions are compiled in, which only a debug build does.
    pub debug_assertions: bool,
}

/// Where the recorded `openehr-*` crate versions come from.
///
/// The `openehr-*` manifests in the source tree hold a floor below the line
/// published on crates.io; the release lane writes the version it publishes
/// into its checkout before it builds. No openEHR spec governs this: our own
/// design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrateOrigin {
    /// A release build: the versions are the ones the release published to
    /// crates.io.
    Published,
    /// Any other build, a pre-release included: the versions are the source
    /// tree's manifests, which may name an older published version than the
    /// content this binary links.
    SourceTree,
}

impl CrateOrigin {
    /// The origin the build script recorded for this binary.
    #[must_use]
    pub fn of_this_build() -> Self {
        match env!("FERROEHR_OPENEHR_CRATES_ORIGIN") {
            "published" => Self::Published,
            _ => Self::SourceTree,
        }
    }
}

/// One linked crate and its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrateVersion {
    /// The crate name.
    pub name: &'static str,
    /// The crate version.
    pub version: &'static str,
}

impl ManifestEntry {
    fn included(part: &'static str) -> Self {
        Self {
            part,
            status: PartStatus::Included,
            reason: None,
        }
    }

    fn partial(part: &'static str, reason: String) -> Self {
        Self {
            part,
            status: PartStatus::Partial,
            reason: Some(reason),
        }
    }

    fn unavailable(part: &'static str, reason: String) -> Self {
        Self {
            part,
            status: PartStatus::Unavailable,
            reason: Some(reason),
        }
    }
}

impl Report {
    /// Assembles the report for a loaded, validated configuration.
    ///
    /// Never fails: a part that cannot be read is named in the manifest with
    /// its reason. The databases are read through the configured DSNs, the two
    /// reads side by side and each bounded by 15 seconds, and nothing is
    /// written.
    pub async fn gather(config: &FerroEhrConfig) -> Self {
        let scrubber = Scrubber::from_config(config);
        let mut manifest = vec![
            ManifestEntry::included("build"),
            ManifestEntry::unavailable("image", IMAGE_REASON.to_owned()),
            ManifestEntry::included("target"),
            ManifestEntry::included("features"),
        ];

        let openehr_crates = openehr_crates();
        manifest.push(if openehr_crates.is_empty() {
            ManifestEntry::unavailable(
                "openehr_crates",
                "the build read no Cargo.lock, so it recorded no crate versions".to_owned(),
            )
        } else {
            ManifestEntry::included("openehr_crates")
        });
        let licence = licence_part(config, &mut manifest);
        // Boxed: the two joined database reads make a wide future (clippy `large_futures`).
        let (deployment, schema) = Box::pin(database_parts(config, &scrubber, &mut manifest)).await;
        let configuration = configuration_part(config, &mut manifest);

        Self {
            format: FORMAT,
            generated_at: jiff::Timestamp::now(),
            manifest,
            build: BuildInfo::for_profile(config.spec_profile)
                .with_audit(AuditPosture::of(&config.audit)),
            target: Target {
                arch: std::env::consts::ARCH,
                os: std::env::consts::OS,
                debug_assertions: cfg!(debug_assertions),
            },
            features: enabled_features(),
            openehr_crates,
            openehr_crates_origin: CrateOrigin::of_this_build(),
            licence,
            deployment,
            schema,
            configuration,
        }
    }

    /// Returns the file name the report is written under by default:
    /// `ferroehr-report-<UTC time>.json`.
    #[must_use]
    pub fn file_name(&self) -> String {
        format!(
            "ferroehr-report-{}.json",
            self.generated_at.strftime("%Y%m%dT%H%M%SZ")
        )
    }
}

/// The licence summary, recorded in the manifest.
fn licence_part(
    config: &FerroEhrConfig,
    manifest: &mut Vec<ManifestEntry>,
) -> Option<LicenceStatus> {
    match crate::licence::anchors() {
        Ok(anchors) => {
            manifest.push(ManifestEntry::included("licence"));
            Some(
                LicenceState::load_now(&config.licence, crate::licence::EMBEDDED_TOKEN, &anchors)
                    .status(),
            )
        }
        Err(error) => {
            manifest.push(ManifestEntry::unavailable(
                "licence",
                format!(
                    "the embedded licence anchors do not parse: {}",
                    ErrorChain::new(&error)
                ),
            ));
            None
        }
    }
}

/// The deployment posture and the schema levels, read side by side and
/// recorded in the manifest.
async fn database_parts(
    config: &FerroEhrConfig,
    scrubber: &Scrubber,
    manifest: &mut Vec<ManifestEntry>,
) -> (DeploymentPosture, Option<Vec<DatabaseLevels>>) {
    let (facts, levels) = tokio::join!(
        bounded(read_facts(config)),
        bounded(crate::db::schema_levels(&config.db, &config.storage)),
    );
    let deployment = match facts {
        Ok(facts) => {
            manifest.push(ManifestEntry::included("deployment"));
            DeploymentPosture::evaluate(config, &facts)
        }
        Err(failure) => {
            manifest.push(ManifestEntry::partial(
                "deployment",
                format!(
                    "evaluated from the configuration alone, without the cluster identities: {}",
                    failure.describe(scrubber)
                ),
            ));
            DeploymentPosture::evaluate(config, &DatabaseFacts::default())
        }
    };
    let schema = match levels {
        Ok(levels) => {
            manifest.push(ManifestEntry::included("schema"));
            Some(levels)
        }
        Err(failure) => {
            manifest.push(ManifestEntry::unavailable(
                "schema",
                failure.describe(scrubber),
            ));
            None
        }
    };
    (deployment, schema)
}

/// The redacted configuration, recorded in the manifest.
fn configuration_part(
    config: &FerroEhrConfig,
    manifest: &mut Vec<ManifestEntry>,
) -> Option<toml::Table> {
    match config.to_redacted_table() {
        Ok(table) => {
            manifest.push(ManifestEntry::included("configuration"));
            Some(table)
        }
        Err(error) => {
            manifest.push(ManifestEntry::unavailable(
                "configuration",
                format!("the configuration could not be rendered: {error}"),
            ));
            None
        }
    }
}

/// Why a database read produced nothing.
#[derive(Debug)]
enum DatabaseFailure {
    /// The read failed.
    Failed(DbError),
    /// No answer arrived within [`DATABASE_TIMEOUT`].
    TimedOut,
}

impl DatabaseFailure {
    /// The reason as the manifest states it, with every credential masked.
    ///
    /// A read is `unreachable` when no connection was ever made: a failure
    /// only opening a connection produces
    /// ([`crate::storage::error::is_connect_failure`], the storage bridge's own
    /// connect-phase classifier), or a pool that handed out none within its
    /// acquire timeout. Anything else failed on a connection that worked, so
    /// the database is `unreadable`.
    fn describe(&self, scrubber: &Scrubber) -> String {
        match self {
            Self::TimedOut => format!(
                "unreachable: no answer within {} s",
                DATABASE_TIMEOUT.as_secs()
            ),
            Self::Failed(error) => {
                let verdict = match error {
                    DbError::Sqlx(e)
                        if matches!(e, sqlx::Error::PoolTimedOut)
                            || crate::storage::error::is_connect_failure(e) =>
                    {
                        "unreachable"
                    }
                    _ => "unreadable",
                };
                let chain = ErrorChain::new(error).to_string();
                format!("{verdict}: {}", scrubber.scrub(&chain))
            }
        }
    }
}

/// Runs one database read under [`DATABASE_TIMEOUT`].
async fn bounded<T>(read: impl Future<Output = Result<T, DbError>>) -> Result<T, DatabaseFailure> {
    match tokio::time::timeout(DATABASE_TIMEOUT, read).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(DatabaseFailure::Failed(error)),
        Err(_elapsed) => Err(DatabaseFailure::TimedOut),
    }
}

/// Connects the four domain pools, one connection each and waiting at most
/// [`POOL_ACQUIRE_SECS`], reads the deployment facts, and closes them again.
async fn read_facts(config: &FerroEhrConfig) -> Result<DatabaseFacts, DbError> {
    let mut db = config.db.clone();
    db.max_connections = 1;
    db.min_connections = 0;
    db.acquire_timeout_secs = POOL_ACQUIRE_SECS;
    let pools = crate::db::connect_domains(&db, &config.storage).await?;
    let facts = crate::db::database_facts(&db, &config.storage, &pools).await;
    pools.close().await;
    facts
}

/// The optional-integration cargo features compiled into this build.
fn enabled_features() -> Vec<&'static str> {
    [
        ("events", cfg!(feature = "events")),
        ("fhir", cfg!(feature = "fhir")),
        ("multimedia", cfg!(feature = "multimedia")),
    ]
    .into_iter()
    .filter_map(|(name, enabled)| enabled.then_some(name))
    .collect()
}

/// The `openehr-*` crates this crate links, as the build script recorded them
/// from the workspace lock file (`name=version`, comma-separated); empty when
/// it recorded none.
fn openehr_crates() -> Vec<CrateVersion> {
    env!("FERROEHR_OPENEHR_CRATES")
        .split(',')
        .filter_map(|entry| entry.split_once('='))
        .map(|(name, version)| CrateVersion { name, version })
        .collect()
}

/// The user names and passwords of every configured DSN, which a database
/// error may quote.
#[derive(Debug)]
struct Scrubber {
    /// The credential parts, raw and percent-decoded, longest first.
    needles: Vec<String>,
}

impl Scrubber {
    fn from_config(config: &FerroEhrConfig) -> Self {
        let dsns = Domain::ALL
            .iter()
            .map(|domain| config.storage.dsn(*domain, &config.db))
            .chain(std::iter::once(config.db.migrate_dsn()));
        let mut needles: Vec<String> = Vec::new();
        for dsn in dsns {
            // NOTE: no openEHR spec governs this — our own design; a DSN that does not
            // parse as a URL carries no `userinfo` to mask.
            let Ok(url) = url::Url::parse(dsn) else {
                continue;
            };
            for part in std::iter::once(url.username()).chain(url.password()) {
                if part.is_empty() {
                    continue;
                }
                needles.push(part.to_owned());
                if let Ok(decoded) = urlencoding::decode(part) {
                    needles.push(decoded.into_owned());
                }
            }
        }
        needles.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        needles.dedup();
        Self { needles }
    }

    /// Masks the `userinfo` of every URL in `text`, then every credential part
    /// that stands as a word of its own.
    fn scrub(&self, text: &str) -> String {
        let mut out = text
            .split(' ')
            .map(|word| {
                if word.contains("://") {
                    RedactedUrl(word).to_string()
                } else {
                    word.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        for needle in &self.needles {
            out = mask_word(&out, needle);
        }
        out
    }
}

/// Replaces every occurrence of `needle` in `text` that no identifier
/// character adjoins with [`REDACTED`].
///
/// The adjacency rule keeps a runtime role such as `ferroehr_clinical` legible
/// when the login user is `ferroehr`.
fn mask_word(text: &str, needle: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, tail)) = rest.find(needle).and_then(|at| rest.split_at_checked(at)) {
        let Some(after) = tail.strip_prefix(needle) else {
            break;
        };
        out.push_str(before);
        if is_word(out.chars().next_back()) || is_word(after.chars().next()) {
            out.push_str(needle);
        } else {
            out.push_str(REDACTED);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrohealth_secret::secret_url::SecretUrl;

    /// A credential part is masked where it stands alone and kept where it is
    /// part of a longer identifier.
    #[test]
    fn a_credential_part_is_masked_only_as_a_word() {
        assert_eq!(
            mask_word(
                "password authentication failed for user \"ferroehr\"",
                "ferroehr"
            ),
            "password authentication failed for user \"***\""
        );
        assert_eq!(
            mask_word("role `ferroehr_clinical` is missing", "ferroehr"),
            "role `ferroehr_clinical` is missing"
        );
        assert_eq!(mask_word("ferroehr", "ferroehr"), "***");
        assert_eq!(mask_word("", "ferroehr"), "");
    }

    /// The scrubber masks the user name and password of every configured DSN,
    /// raw and percent-decoded, and the `userinfo` of a quoted URL.
    #[test]
    fn the_scrubber_masks_every_configured_credential() {
        let mut config = FerroEhrConfig::default();
        config.db.url = SecretUrl::new("postgres://dbuser:p%40ss@db.internal:5432/ferroehr");
        let scrubber = Scrubber::from_config(&config);
        let masked = scrubber.scrub(
            "role dbuser refused (p@ss, p%40ss) at postgres://dbuser:p%40ss@db.internal/ferroehr",
        );
        assert_eq!(
            masked,
            "role *** refused (***, ***) at postgres://***@db.internal/ferroehr"
        );
    }

    /// The `openehr-*` crates the build script recorded are the ones this crate
    /// links, each with a version.
    #[test]
    fn the_build_records_the_linked_openehr_crates() {
        let crates = openehr_crates();
        let names: Vec<&str> = crates.iter().map(|c| c.name).collect();
        for name in ["openehr-am", "openehr-base", "openehr-its", "openehr-rm"] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
        assert!(crates.iter().all(|c| !c.version.is_empty()), "{crates:?}");
        assert!(
            !names.contains(&"openehr-codegen"),
            "the code generator is not linked: {names:?}"
        );
    }

    /// A build no release lane stamped reports its `openehr-*` versions as the
    /// source tree's, at the version the manifests hold.
    #[test]
    fn an_unstamped_build_names_its_crate_versions_the_source_trees() {
        assert_eq!(CrateOrigin::of_this_build(), CrateOrigin::SourceTree);
        assert_eq!(
            serde_json::to_value(CrateOrigin::of_this_build()).ok(),
            Some(serde_json::json!("source_tree"))
        );
        assert_eq!(
            serde_json::to_value(CrateOrigin::Published).ok(),
            Some(serde_json::json!("published"))
        );
        let manifest = include_str!("../../../crates/openehr-rm/Cargo.toml");
        let floor = manifest
            .lines()
            .find_map(|line| line.strip_prefix("version = \""))
            .and_then(|rest| rest.strip_suffix('"'));
        let rm = openehr_crates()
            .into_iter()
            .find(|linked| linked.name == "openehr-rm")
            .map(|linked| linked.version);
        assert_eq!(rm, floor);
    }
}
