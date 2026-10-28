// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` server wiring — the binary's testable library half.
//!
//! Boots the `ferroehr-rest` ITS-REST server backed by the DB-backed
//! [`FerroEhrService`]: loads the one
//! configuration tree ([`ferroehr::config`]), initialises tracing, connects the
//! `PostgreSQL` pool, runs migrations, boots the ATNA audit sender, and serves.
//! On shutdown the audit queue is drained before exit.
//!
//! `main.rs` is a thin shell over [`run`] so the whole boot path is reachable
//! from tests — "you can't test the `main` function directly"
//! (<https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html>).
//! `anyhow` is fine here: this lib target IS the binary's own logic half,
//! not a consumable library.

use std::io::IsTerminal as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use ferroehr::aql::subject::ehr_subject;
use ferroehr::config::deployment::{DatabaseFacts, DeploymentPosture, DeploymentProfile};
use ferroehr::config::management::EndpointLevels;
use ferroehr::config::management::ManagementConfig;
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::decommission::{Decommission, Erasure};
use ferroehr::ids::EhrId;
use ferroehr::manufacturer::MANUFACTURER;
use ferroehr::report::Report;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::status::SmError;
use ferroehr::storage::ehr_repo::ehr_subject_columns;
use ferroehr::support::{SupportPeriod, today_utc};
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::config::AuditPosture;
use ferroehr::system_log::sender::{
    AuditHandle, AuditSender, ResolvedSubject, SubjectLookup, SubjectResolver,
};
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::usage_report::product::{self as usage_product, EventKind, FerroEhr, NOTICE_URL};
use ferroehr::usage_report::store::PgStore;
use ferroehr::versioning::signature::signer::Signer;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::{
    AuthzHandle, AuthzResolvers, ContentRules, ResolveError, build_engine,
};
use ferroehr_rest::extensions::management::{LogFilter, Observability};
use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_health::registry::{HealthIndicator, Registry};
use ferrohealth_server::cli::{Common, ConfigArgs, ConfigCommand};
use ferrohealth_server::healthcheck::{Outcome, check, check_tls, target};
use ferrohealth_server::section::ServerSection;
use ferrohealth_server::tls::HealthcheckTls;
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::db;
use ferroehr::service::FerroEhrService;
use ferroehr::telemetry::{self, indicators};
use ferrohealth_telemetry::request_log::RequestLog;

/// How long to wait for the audit queue to flush on shutdown.
const AUDIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// What `ferroehr --version` prints after the binary's name: the version, then
/// the manufacturer with its postal address, single point of contact and
/// website (Regulation (EU) 2025/327, `docs/law/eu/ehds/text.html`
/// Art. 30(1)(g)), then the support period (Regulation (EU) 2024/2847,
/// `docs/law/eu/cra/text.html` Art. 13(19)).
static VERSION_TEXT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}\nSupport: {}",
        MANUFACTURER.version_text(env!("CARGO_PKG_VERSION")),
        SupportPeriod::current().describe_on(today_utc())
    )
});

/// `FerroEHR` server command-line interface: `--config` and the repeatable
/// `--set key=value` every FerroHEALTH binary shares, and the subcommands. No
/// subcommand serves, as `serve` does.
#[derive(Debug, Parser)]
#[command(
    name = "ferroehr",
    version = VERSION_TEXT.as_str(),
    about = "openEHR-conformant CDR server"
)]
pub struct Cli {
    /// `--config` (overrides `FERROEHR_CONFIG`; with neither, no file is read)
    /// and the `--set` overrides, applied last.
    #[command(flatten)]
    config: ConfigArgs,
    #[command(subcommand)]
    command: Option<Command>,
}

/// Top-level subcommands of the `ferroehr` binary.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// `serve`, `config check` and `healthcheck`, the family's commands.
    #[command(flatten)]
    Common(Common),
    /// Database schema utilities (apply or verify the migrations, or erase the
    /// instance), then exit.
    Db {
        /// Which schema utility to run.
        #[command(subcommand)]
        cmd: DbCmd,
    },
    /// The usage report to `FerroPULSE`: print the report this instance would
    /// send, without sending it.
    UsageReport {
        /// Print the report as the exact JSON body a send would carry, then
        /// exit. Required: printing is the only action.
        #[arg(long, required = true)]
        print: bool,
        /// Which report to print.
        #[arg(long, value_enum, default_value_t = ReportEvent::Start)]
        event: ReportEvent,
    },
    /// Write a redacted report of what this deployment runs (build, features,
    /// specification pins, migration level, posture, licence, manufacturer and
    /// configuration), the file a complaint or an incident report attaches.
    Report {
        /// Where to write the report; `-` writes it to stdout. Unset, it is
        /// `ferroehr-report-<UTC time>.json` in the working directory.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

/// `ferroehr db …` subcommands.
///
/// These exist so a least-privilege deployment can separate the two database
/// identities: `migrate` runs under the migrator DSN as a one-shot step
/// (a Kubernetes Job, an init container, a CI/CD stage), and the server then
/// boots with `[db].migrate = "verify"` under a DSN with no DDL rights at all.
#[derive(Debug, Subcommand)]
pub enum DbCmd {
    /// Apply the embedded migrations and exit; the DSN must hold DDL rights.
    Migrate,
    /// Verify, without issuing any DDL, that the database carries exactly this
    /// build's migrations; exit 0 when it does, 1 otherwise.
    Verify,
    /// Permanently erase every piece of data and every setting this instance
    /// stores: the multimedia blobs, then every schema the migrations create.
    ///
    /// Without `--confirm` it erases nothing: it prints what it would erase and
    /// the value `--confirm` must carry, and exits 1. Run it with every server
    /// stopped; the DSN must own the schemas.
    Erase {
        /// The instance to erase, as the run without `--confirm` prints it: the
        /// stored instance id, or the clinical database's name when the
        /// database stores no instance id.
        #[arg(long, value_name = "INSTANCE")]
        confirm: Option<String>,
    },
}

/// The report `ferroehr usage-report --print` renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReportEvent {
    /// The report sent at each process start.
    Start,
    /// The report sent once a day, with the metrics window.
    Daily,
}

/// How long `ferroehr healthcheck` waits for the readiness answer: shorter than
/// the image's `HEALTHCHECK --timeout=5s`, so the probe reports the reason
/// itself instead of being killed.
pub const HEALTHCHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Dispatch a parsed [`Cli`] — the whole binary behind one testable seam.
///
/// # Errors
///
/// Returns any boot, configuration, probe, or serve failure of the selected
/// subcommand; the process exit code follows from `main` returning it.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let config = cli.config.config.as_deref();
    let set = cli.config.sets.as_slice();
    match cli.command {
        Some(Command::Common(Common::Healthcheck)) => {
            let loaded = ferroehr::config::load(config, set)?;
            let outcome = healthcheck(&loaded.server).await;
            anyhow::ensure!(outcome.exit_code() == 0, "healthcheck: {outcome}");
            Ok(())
        }
        Some(Command::Common(Common::Config {
            command: ConfigCommand::Check,
        })) => run_config_check(config, set),
        Some(Command::Db { cmd }) => run_db(&cmd, config, set).await,
        Some(Command::UsageReport { print: _, event }) => {
            run_usage_report(event, config, set).await
        }
        Some(Command::Report { output }) => run_report(output.as_deref(), config, set).await,
        Some(Command::Common(Common::Serve)) | None => Box::pin(serve(config, set)).await,
    }
}

/// `ferroehr report`: loads the configuration and hands it to
/// [`write_report`].
///
/// # Errors
/// The configuration does not load, or [`write_report`] fails.
async fn run_report(
    output: Option<&Path>,
    config_path: Option<&Path>,
    overrides: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(config_path, overrides)?;
    write_report(&config, output).await
}

/// Assembles the deployment report for `config` and writes it to `output`, to
/// stdout for `-`, or to `ferroehr-report-<UTC time>.json` in the working
/// directory, then names the file on stderr.
///
/// Telemetry is not initialised, so stdout carries the report alone when it is
/// the destination. The databases are read, never written. This is the seam
/// `ferroehr report` runs after loading, so a test drives it with a
/// configuration assembled away from the process environment.
///
/// # Errors
/// The configuration does not validate, or the report cannot be serialized or
/// written.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the report subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn write_report(
    config: &ferroehr::config::FerroEhrConfig,
    output: Option<&Path>,
) -> anyhow::Result<()> {
    config.validate()?;
    let report = Report::gather(config).await;
    let written = serde_json::to_string_pretty(&report).context("serializing the report")?;
    if output == Some(Path::new("-")) {
        println!("{written}");
        return Ok(());
    }
    let path = output.map_or_else(|| PathBuf::from(report.file_name()), Path::to_path_buf);
    tokio::fs::write(&path, format!("{written}\n"))
        .await
        .with_context(|| format!("writing the report to {}", path.display()))?;
    eprintln!("report written to {}", path.display());
    Ok(())
}

/// `ferroehr db migrate` / `verify` / `erase` — the out-of-band schema steps.
async fn run_db(
    cmd: &DbCmd,
    config_path: Option<&Path>,
    overrides: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(config_path, overrides)?;
    config.validate()?;
    let build_info = BuildInfo::for_profile(config.spec_profile);
    let telemetry = telemetry::init(
        &config.telemetry,
        config.management.flame_file.as_deref(),
        &build_info,
    )
    .context("initialising telemetry")?;

    // Every subcommand reads the schema on the migration DSN
    // (`[db].migrate_url`, falling back to `[db].url`), which is the
    // credential that can reach every schema when the runtime ones each hold
    // one pseudonymisation domain.
    let outcome = match cmd {
        DbCmd::Migrate => db::apply_schema(&config.db, &config.storage)
            .await
            .context("applying migrations"),
        DbCmd::Verify => verify_schema_and_isolation(&config).await,
        DbCmd::Erase { confirm } => erase_instance(&config, confirm.as_deref()).await,
    };
    telemetry.shutdown().await;
    outcome
}

/// `ferroehr db erase`: without a confirmation, prints what an erasure would
/// remove and fails; with one, erases and prints what it removed.
///
/// # Errors
/// The dry run (nothing erased), a confirmation that does not name this
/// instance, an unreachable blob store, or a database failure.
#[expect(
    clippy::print_stdout,
    reason = "the db erase subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
async fn erase_instance(
    config: &ferroehr::config::FerroEhrConfig,
    confirm: Option<&str>,
) -> anyhow::Result<()> {
    let decommission = Decommission::from_config(config)?;
    let Some(confirm) = confirm else {
        let inventory = decommission.inventory().await?;
        if inventory.is_empty() {
            println!("nothing to erase: no database holds a FerroEHR schema and no blob is stored");
            return Ok(());
        }
        println!("`ferroehr db erase --confirm` would permanently delete:");
        if let Some(blobs) = &inventory.blobs {
            println!(
                "  {} multimedia blob(s) in bucket `{}`",
                blobs.objects, blobs.bucket
            );
        }
        for database in &inventory.databases.databases {
            println!(
                "  database `{}`: schemas {}",
                database.database,
                schema_list(&database.schemas)
            );
        }
        match inventory.databases.instance_id {
            Some(id) => println!("instance id: {id}"),
            None => println!("instance id: none stored"),
        }
        let token = inventory
            .confirmation()
            .context("no database holds the clinical domain")?;
        println!("to erase, run: ferroehr db erase --confirm {token}");
        anyhow::bail!("dry run: nothing was erased");
    };
    match decommission.erase(confirm).await? {
        Erasure::NothingToErase => {
            println!("nothing to erase: no database holds a FerroEHR schema and no blob is stored");
        }
        Erasure::Erased {
            databases,
            blobs_deleted,
        } => {
            if let Some(count) = blobs_deleted {
                println!("deleted {count} multimedia blob(s)");
            }
            for database in &databases {
                println!(
                    "database `{}`: dropped schemas {}",
                    database.database,
                    schema_list(&database.schemas)
                );
            }
            println!(
                "erased. The database roles, the databases themselves, backups, WAL archives, \
                 replicas and configuration files are not touched by this command"
            );
        }
    }
    Ok(())
}

/// `a, b, c`, or `none`.
fn schema_list(schemas: &[&str]) -> String {
    if schemas.is_empty() {
        "none".to_owned()
    } else {
        schemas.join(", ")
    }
}

/// `ferroehr usage-report --print`: assembles the report this instance would
/// send and prints its exact JSON body to stdout, sending nothing.
///
/// Telemetry is not initialised, so stdout carries the JSON alone; the notes
/// an operator needs go to stderr. The database is read, never written: a
/// start report on a database with no stored instance id shows a sample id.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the usage-report --print subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
async fn run_usage_report(
    event: ReportEvent,
    config_path: Option<&Path>,
    overrides: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(config_path, overrides)?;
    config.validate()?;
    let licence = load_licence(&config)?;
    let identity = usage_product::identity(&BuildInfo::for_profile(config.spec_profile), &licence);
    let pools = db::connect_domains(&config.db, &config.storage)
        .await
        .context("connecting to PostgreSQL")?;
    let kind = match event {
        ReportEvent::Start => EventKind::Start,
        ReportEvent::Daily => EventKind::Daily,
    };
    let outcome = usage_product::preview(
        &config.usage_report,
        identity,
        &pools.clinical,
        &usage_product::window(&usage_product::api_mounts(&config)),
        kind,
    )
    .await
    .context("assembling the usage report");
    pools.close().await;
    let preview = outcome?;
    println!("{}", serde_json::to_string(&preview.report)?);
    if !preview.instance_stored {
        eprintln!(
            "note: the database holds no instance id yet; the id above is a sample, and the \
             first start creates the real one"
        );
    }
    if !config.usage_report.enabled {
        eprintln!(
            "note: the usage report is OFF (usage_report.enabled = false); this instance sends \
             nothing"
        );
    } else if let Some(variable) = ferroehr::config::loader::usage_report_ci() {
        eprintln!(
            "note: the usage report is OFF: CI detected ({variable}); this instance sends nothing"
        );
    }
    eprintln!("details: {NOTICE_URL}");
    Ok(())
}

/// The licence in force: the configured token, else the one the build embeds.
///
/// # Errors
/// The embedded licence anchors do not parse.
fn load_licence(
    config: &ferroehr::config::FerroEhrConfig,
) -> anyhow::Result<ferroehr::licence::state::LicenceState> {
    let anchors = ferroehr::licence::anchors().context("parsing the embedded licence anchors")?;
    Ok(ferroehr::licence::state::LicenceState::load_now(
        &config.licence,
        ferroehr::licence::EMBEDDED_TOKEN,
        &anchors,
    ))
}

/// `ferroehr db verify`: the recorded schema state, then the pseudonymisation
/// boundary.
///
/// The two checks deliberately authenticate as different credentials, exactly
/// as [`ferroehr::db::prepare`] does at boot. The schema state is read on the
/// migration DSN of each database the domains reach; the isolation check runs
/// on the RUNTIME pools, because what it measures is what the serving
/// credentials can reach — asked of the migration credential it would report
/// on a role that holds every domain by design.
///
/// # Errors
/// A schema divergence, an unreadable migration set, a breached domain
/// boundary, or a connection failure.
async fn verify_schema_and_isolation(
    config: &ferroehr::config::FerroEhrConfig,
) -> anyhow::Result<()> {
    db::verify_schema(&config.db, &config.storage)
        .await
        .map_err(|error| anyhow::Error::new(error).context("verifying the schema"))?;
    let pools = db::connect_domains(&config.db, &config.storage)
        .await
        .context("connecting to PostgreSQL")?;
    let outcome = db::verify_domain_isolation(
        &pools,
        &config.storage.layout(&config.db),
        config.deployment_profile,
    )
    .await
    .context("verifying the pseudonymisation domain isolation");
    pools.close().await;
    outcome
}

/// `ferroehr config check`: loads and validates the configuration, then prints
/// it with every secret redacted and where each key came from.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the config check subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
fn run_config_check(config: Option<&Path>, set: &[(String, String)]) -> anyhow::Result<()> {
    let (loaded, cfg) = ferroehr::config::load_with_provenance(config, set)?;
    cfg.validate()?;
    ferroehr::config::auth::require_mechanism(&cfg.auth)?;
    let rendered = ferrohealth_config::report::check_report(&loaded)?;
    print!("{rendered}");
    // The resolved placement of the four storage domains, so an
    // operator can see whether the deployment they configured is the
    // one the server would run. It names no DSN.
    println!("{}", cfg.storage.layout(&cfg.db).describe());
    if cfg.db.is_dev_default() {
        eprintln!(
            "note: [db].url is the built-in DEVELOPMENT DEFAULT; set it for any \
             non-dev deployment."
        );
    }
    eprintln!("configuration OK");
    Ok(())
}

/// Asks the server `section` configures for its readiness, over the TLS it
/// configures, on the loopback when it listens on every interface: the
/// container's `HEALTHCHECK`.
pub async fn healthcheck(section: &ServerSection) -> Outcome {
    let address = target(section.listen);
    let base = section.base_path.as_str();
    match &section.tls {
        None => check(address, base, HEALTHCHECK_TIMEOUT).await,
        Some(tls) => match HealthcheckTls::read(tls) {
            Ok(pinned) => check_tls(address, base, HEALTHCHECK_TIMEOUT, &pinned).await,
            Err(error) => Outcome::Failed(format!("the TLS files do not read: {error}")),
        },
    }
}

/// Summarizes which management endpoints are mounted, and at which access
/// level, as `info=admin_only, prometheus=public`.
///
/// Worth a boot line of its own: every endpoint defaults to `off`, so what an
/// operator needs to see is exactly which ones a configuration turned on and how
/// exposed each one is — `env` renders the effective configuration and
/// `flamegraph` starts a profiler on request, so neither should ever be a
/// surprise. `none` when the surface is mounted with nothing enabled.
fn mounted_management_endpoints(levels: EndpointLevels) -> String {
    let described = [
        ("info", levels.info),
        ("metrics", levels.metrics),
        ("prometheus", levels.prometheus),
        ("env", levels.env),
        ("loggers", levels.loggers),
        ("flamegraph", levels.flamegraph),
        ("health", levels.health),
        ("status", levels.status),
    ];
    let mounted: Vec<String> = described
        .iter()
        .filter(|(_, level)| level.is_mounted())
        .map(|(name, level)| format!("{name}={}", level.as_str()))
        .collect();
    if mounted.is_empty() {
        "none".to_owned()
    } else {
        mounted.join(", ")
    }
}

/// Announces the deployment postures that are legal but easy to leave on by
/// accident, once logging is up.
///
/// The dev-default DSN and permissive CORS are announced rather than silently
/// accepted (OWASP REST Security Cheat Sheet). The HTTPS posture is stated
/// rather than enforced: this server cannot tell plaintext-because-misconfigured
/// from plaintext-behind-a-TLS-terminating-ingress, so authentication over
/// plaintext on a routable bind warns loudly and proceeds.
/// Assembles the platform service from the resolved config tree.
///
/// Every optional collaborator is attached here: the audit sender, the local
/// Audit Record Repository read side (wired only when auditing and the store are
/// both on), terminology, the subject proxy, the multimedia store, and the
/// EHR index provider the caller built and checked.
///
/// # Errors
/// Any collaborator whose configuration is enabled but unbuildable — a slim
/// build missing its cargo feature, or an unusable external dependency.
fn assemble_service(
    config: &ferroehr::config::FerroEhrConfig,
    pools: &DomainPools,
    audit_sender: Option<AuditSender>,
    outbox_enabled: bool,
    signer: Arc<Signer>,
    deployment: DeploymentPosture,
    ehr_index: EhrIndexProvider,
) -> anyhow::Result<FerroEhrService> {
    let audit_enabled = audit_sender.is_some();
    // The MRN patterns compiled here; `FerroEhrConfig::validate` already
    // refused an uncompilable one at boot, so this cannot be the first place a
    // bad pattern is noticed.
    let privacy = ferroehr::privacy::PrivacyPolicy::compile(&config.privacy)
        .context("compiling the [privacy] policy")?;
    // The identifier-protection engine, when the deployment configured one. A
    // key that cannot be parsed is a boot error: a server that believes it
    // seals national identifiers and does not is the failure this refuses to
    // start into.
    let identifiers =
        ferroehr::service::demographic::identifier::engine::IdentifierProtection::from_config(
            &config.demographic.identifier_protection,
            config.demographic.identifier_protection.key.as_ref(),
            pools.party.clone(),
        )
        .context("building the [demographic.identifier_protection] engine")?;

    // The licence in force: the configured token, else the one the build
    // embeds. Never a boot failure; the outcome is logged once, served on
    // GET /management/status, and selects the identifier stamp key.
    let licence = load_licence(config)?;
    tracing::info!(licence = %licence, "licence");

    let mut service = FerroEhrService::new(pools)
        .with_spec_profile(config.spec_profile)
        .with_system_id(config.rest.system_id.clone())
        .with_signer(signer)
        .with_licence(licence)
        .with_deployment(deployment)
        .with_outbox_enabled(outbox_enabled)
        .with_privacy(Arc::new(privacy))
        .with_cohort(config.cohort.clone())
        .with_origins(config.origins.clone())
        .with_identifier_protection_opt(identifiers.map(Arc::new))
        .with_query_config(&config.query)
        .with_ehr_index(ehr_index);
    if let Some(sender) = audit_sender {
        service = service.with_audit(sender);
    }
    if audit_enabled && config.audit.store.enabled {
        service = service.with_audit_store(ferroehr::system_log::store::AuditStore::new(
            pools.audit.clone(),
        ));
    }

    service = attach_terminology(service, config)?;
    attach_multimedia(service, config)
}

/// Wires authorization, active only when authentication is enabled: the RBAC
/// gate, the ABAC engine over the DB-backed attribute resolvers, and the
/// per-EHR access default.
///
/// `None` when authentication is off, or when both gates are off and the
/// per-EHR default is `open`. The boot line names which of the three caused
/// the handle, so a `restricted` default alone reads as the reason.
///
/// # Errors
/// An ABAC block that is enabled but unbuildable aborts boot: configuration
/// promising fine-grained authorization must never degrade to authz-off.
fn wire_authz(
    config: &ferroehr::config::FerroEhrConfig,
    pool: &PgPool,
    service: &Arc<FerroEhrService>,
) -> anyhow::Result<Option<Arc<AuthzHandle>>> {
    if !config.auth.enabled {
        return Ok(None);
    }
    // `validate` has already refused a category map that does not compile.
    let content = ContentRules {
        categories: config
            .audit
            .categories
            .compile()
            .map_err(ferroehr::system_log::AuditError::CategoryMap)
            .context("compiling the [audit.categories] map for the ABAC category attribute")?,
        origins: config.origins.clone(),
        system_id: config.rest.system_id.clone(),
    };
    let authz = build_authz(
        &config.authz,
        &config.rest.base_path,
        authz_resolvers(pool.clone(), Arc::clone(service), content),
    )?;
    if let Some(handle) = &authz {
        let ehr_access_default = handle.ehr_access_default();
        if handle.rbac_active() || handle.abac_active() {
            tracing::info!(
                rbac = handle.rbac_active(),
                abac = handle.abac_active(),
                ?ehr_access_default,
                "authorization enabled"
            );
        } else {
            tracing::info!(
                rbac = false,
                abac = false,
                ?ehr_access_default,
                "authorization enabled for the restricted per-EHR access default only"
            );
        }
    }
    Ok(authz)
}

/// Logs the resolved runtime posture, one line per subsystem, immediately
/// before the listener starts.
///
/// Each line carries that subsystem's own facts, so the listener line stays
/// about the listener: authorization already logged its own line when it was
/// built, and auditing and the management surface get theirs here.
fn log_resolved_posture(
    app_config: &AppConfig,
    audit_enabled: bool,
    audit: &AuditConfig,
    management: &ManagementConfig,
) {
    tracing::info!(
        mechanisms = %ferroehr::config::auth::advertised_mechanisms(&app_config.auth),
        enabled = app_config.auth.enabled,
        "authentication configured"
    );
    if audit_enabled {
        tracing::info!(
            local_repository = audit.store.enabled,
            syslog = audit.syslog.enabled,
            fhir_feed = audit.fhir_feed.enabled,
            queue_capacity = audit.queue_capacity,
            fail_mode = ?audit.fail_mode,
            resolve_subject = audit.resolve_subject,
            retention_days = audit.store.retention_days,
            retention_years = ?audit.store.retention_years,
            category_map = AuditPosture::of(audit)
                .category_map
                .map_or_else(|| "invalid".to_owned(), |digest| digest.to_string()),
            "IHE ATNA audit enabled"
        );
    }
    // A legitimate posture that is wrong to run silently is said at `warn`
    // with a structured field, the same sentence the readiness indicator and
    // `/management/info` carry, so a collector alerts on it without parsing
    // (#3238).
    for caution in AuditPosture::of(audit).cautions() {
        tracing::warn!(posture = "audit", caution, "audit posture");
    }
    if management.enabled {
        tracing::info!(
            base_path = %management.base_path,
            listener = match management.port {
                Some(port) => format!("own port {port}"),
                None => "shared with the API".to_owned(),
            },
            mounted = %mounted_management_endpoints(management.endpoints),
            "management surface enabled"
        );
    }
    tracing::info!(
        enabled = app_config.server.caller_rate.is_some(),
        requests_per_second = ?app_config.server.caller_rate.map(|rate| rate.requests_per_second),
        burst = ?app_config.server.caller_rate.map(|rate| rate.burst),
        "caller rate limiting configured"
    );
    tracing::info!(
        listen = %app_config.server.listen,
        base_path = %app_config.rest.base_path,
        tls = app_config.server.tls.is_some(),
        "starting ferroehr"
    );
}

/// Evaluate the declared deployment posture over the clusters the pools
/// reached, refuse to start under `production` while a separation is open and
/// not accepted, and say every open gap at `warn` otherwise (#3226).
///
/// # Errors
/// The refusal, listing each open gap with its finding and remedy; or a
/// failure reading a pool's cluster identity.
async fn evaluate_deployment(
    config: &ferroehr::config::FerroEhrConfig,
    pools: &DomainPools,
) -> anyhow::Result<DeploymentPosture> {
    let databases = db::database_facts(&config.db, &config.storage, pools)
        .await
        .context(
            "reading each pool's cluster identity and which databases schema preparation \
             reaches on a runtime credential",
        )?;
    let posture = DeploymentPosture::evaluate(config, &databases);
    if !posture.permits_boot() {
        anyhow::bail!("{}", posture.refusal_message());
    }
    for gap in &posture.gaps {
        let accepted = posture.accepted.contains(gap);
        tracing::warn!(
            posture = "deployment",
            profile = %posture.profile,
            gap = %gap,
            accepted,
            "{}",
            gap.describe()
        );
    }
    if posture.profile == DeploymentProfile::Sandbox {
        tracing::warn!(
            posture = "deployment",
            profile = "sandbox",
            open_gaps = posture.gaps.len(),
            "deployment_profile is sandbox: this deployment has not made the production \
             separations and must not hold real patient data"
        );
    } else {
        tracing::info!(
            posture = "deployment",
            profile = "production",
            accepted_gaps = posture.accepted.len(),
            "deployment_profile is production: every separation holds or is accepted by name"
        );
    }
    Ok(posture)
}

/// Stamp the subject-shape posture the database guard reads (#3241) and report
/// the rows a newly declared namespace finds already stored.
///
/// # Errors
/// A failure writing the stamp or counting the rows.
async fn stamp_subject_posture(
    config: &ferroehr::config::FerroEhrConfig,
    pool: &PgPool,
) -> anyhow::Result<()> {
    let required = !config.privacy.subject_namespaces.is_empty();
    db::stamp_subject_posture(pool, required)
        .await
        .context("stamping the subject pseudonym posture")?;
    if required {
        let stored = db::non_pseudonym_subjects(pool)
            .await
            .context("counting stored subject references that are not pseudonyms")?;
        if stored > 0 {
            tracing::warn!(
                posture = "subject_pseudonyms",
                ehrs = stored,
                "privacy.subject_namespaces is declared, but {stored} stored EHR(s) carry a \
                 subject reference that is not a UUID; the database now refuses new ones, and \
                 these need re-pseudonymising"
            );
        }
    }
    Ok(())
}

fn warn_boot_postures(config: &ferroehr::config::FerroEhrConfig) {
    if config.db.is_dev_default() {
        tracing::warn!(
            url = db::DEFAULT_URL,
            "[db].url is the built-in DEVELOPMENT DEFAULT ({}); no file/env/CLI value was \
             supplied. Set db.url (FERROEHR__DB__URL / DATABASE_URL) for any non-dev deployment — \
             production MUST override it.",
            db::DEFAULT_URL,
        );
    }
    if config.rest.cors_permissive {
        tracing::warn!(
            "[rest].cors_permissive is ON: any origin may read API responses. This is a \
             DEVELOPMENT setting — configure explicit origins for any deployment reachable by \
             a browser."
        );
    }
    if config.privacy.allow_identified_parties_in_ehr {
        tracing::warn!(
            "[privacy].allow_identified_parties_in_ehr is ON: beyond the provider names a \
             PARTY_IDENTIFIED may carry by default, clinical content may now also carry \
             formal identifiers on either party class and a PARTY_RELATED name (composer, \
             participations, health_care_facility, feeder audit). Formal identifiers are \
             the national-identifier slot, and a party named in relation to the subject \
             re-identifies the subject — GDPR Art. 25(2) data minimisation is on this \
             deployment to justify."
        );
    }
    if config.privacy.identifier_scan.mode == ferroehr::privacy::config::ScanMode::Warn {
        tracing::warn!(
            "[privacy.identifier_scan].mode is `warn`: a clinical write carrying a value one \
             of the active rules claims is ACCEPTED and recorded, not refused. Set it to \
             `strict` once the recorded findings are down to none."
        );
    }
    // Which jurisdictions are actually covered is the one fact an operator
    // cannot infer, and believing a jurisdiction is scanned when it is not is
    // the failure this line exists to prevent.
    if config.privacy.identifier_scan.rules.is_empty() {
        tracing::warn!(
            "[privacy.identifier_scan].rules is empty: NO national identifier rule is active, \
             so the scanner checks only the configured patterns. Name the rules this \
             deployment's content could carry."
        );
    } else {
        tracing::info!(
            rules = %config.privacy.identifier_scan.rules.join(", "),
            patterns = config.privacy.identifier_scan.patterns.len(),
            mode = config.privacy.identifier_scan.mode.as_str(),
            "clinical writes are scanned for national identifiers"
        );
    }
    if config.privacy.subject_namespaces.is_empty() {
        tracing::info!(
            "[privacy].subject_namespaces is empty: an EHR_STATUS subject reference is \
             accepted in any namespace with any identifier. Declare this deployment's \
             pseudonym namespaces to bind the subject to an opaque UUID."
        );
    } else {
        tracing::info!(
            namespaces = %config.privacy.subject_namespaces.join(", "),
            "EHR_STATUS subject references are bound to a UUID in a configured pseudonym \
             namespace"
        );
    }
}

/// Connects the four domain pools and prepares every database they reach.
///
/// Each pool carries its own domain's `search_path`, and a domain a deployment
/// gave `[storage.<domain>].url` carries its own credential — which is what
/// makes the pseudonymisation boundary a role boundary rather than only a
/// schema one. The domain split is our own deployment extension; no openEHR
/// spec governs it.
///
/// Schema preparation is [`ferroehr::db::prepare`], which spans every schema of
/// each database on that database's migration DSN (`[db].migrate_url` for every
/// domain reaching the database it reaches, `[storage.migrate.<domain>].url`
/// for a database of its own, the domain's own DSN when that is unset) and then
/// measures the runtime pools' own reach with
/// [`ferroehr::db::verify_domain_isolation`], refusing to boot a database whose
/// grants let one runtime role read another domain.
///
/// # Errors
/// A connection, migration or domain-isolation failure, contextualized for the
/// operator.
async fn connect_pool(config: &ferroehr::config::FerroEhrConfig) -> anyhow::Result<DomainPools> {
    let pools = db::connect_domains(&config.db, &config.storage)
        .await
        .context("connecting to PostgreSQL")?;
    db::prepare(
        &config.db,
        &config.storage,
        &pools,
        config.deployment_profile,
    )
    .await
    .context("preparing the database schema")?;
    for domain in Domain::ALL {
        if config.storage.is_separated(domain) {
            tracing::info!(
                %domain,
                "the domain connects on its own DSN: its credential is a database role of its own"
            );
        }
    }
    Ok(pools)
}

/// Wires the opt-in FHIR terminology server (`[terminology.server]`), the
/// "terminology query server" BASE
/// `architecture_overview/master12-terminology.adoc` §Binding Terminology
/// Value-sets to Archetypes places the resolver of a constraint binding in.
///
/// # Errors
/// A server that cannot be built (a boot error: configuration promising a
/// terminology binding must never degrade to no binding).
fn attach_terminology(
    service: FerroEhrService,
    config: &ferroehr::config::FerroEhrConfig,
) -> anyhow::Result<FerroEhrService> {
    let server = &config.terminology.server;
    if !server.enabled {
        return Ok(service);
    }
    let client = ferroehr::service::terminology::fhir::FhirTerminologyProvider::new(server)
        .context("initialising the terminology server client")?;
    tracing::info!(
        server = %server.url,
        fail_on_error = server.fail_on_error,
        authenticated = server.oauth.is_some(),
        "FHIR terminology server configured"
    );
    Ok(service.with_terminology_server(Arc::new(client)))
}

/// Wires the opt-in `DV_MULTIMEDIA` externalization behind the `multimedia`
/// cargo feature; a slim build refuses an enabled config loudly.
///
/// A store kept only to read back already-offloaded blobs must never stop the
/// server starting, so an unbuildable store is fatal only when the integration
/// is enabled.
///
/// # Errors
/// An unbuildable object store while the integration is enabled, or an enabled
/// configuration in a build without the feature.
fn attach_multimedia(
    service: FerroEhrService,
    config: &ferroehr::config::FerroEhrConfig,
) -> anyhow::Result<FerroEhrService> {
    #[cfg(feature = "multimedia")]
    {
        let engine = match ferroehr::extensions::multimedia::engine_from_config(&config.multimedia)
        {
            Ok(engine) => engine,
            Err(e) if !config.multimedia.enabled => {
                tracing::warn!(
                    error = %e,
                    "multimedia is disabled and its object store could not be built: \
                     already-externalized content cannot be re-inlined, and a read that \
                     asks for it will be refused rather than answered with the reference"
                );
                None
            }
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context("initialising the multimedia object store")
                );
            }
        };
        let Some(engine) = engine else {
            return Ok(service);
        };
        if engine.offload_enabled() {
            tracing::info!(
                bucket = %config.multimedia.bucket,
                threshold_bytes = config.multimedia.threshold_bytes,
                "DV_MULTIMEDIA externalization enabled"
            );
        } else {
            tracing::info!(
                bucket = %config.multimedia.bucket,
                "DV_MULTIMEDIA externalization disabled; the configured store stays \
                 readable so already-externalized content can still be served"
            );
        }
        Ok(service.with_multimedia(Arc::new(engine)))
    }
    #[cfg(not(feature = "multimedia"))]
    {
        ferroehr::extensions::multimedia::require_disabled(&config.multimedia)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(service)
    }
}

/// Boot the server: config, telemetry, pool, migrations, audit, health, serve.
#[expect(
    clippy::too_many_lines,
    reason = "the remaining body is one linear boot sequence of subsystem \
              startups, each closed over by the next; splitting it further \
              would obscure the order that makes it correct"
)]
async fn serve(config_path: Option<&Path>, overrides: &[(String, String)]) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    // One load + one aggregated validate (all errors at once), then distribute.
    let config = ferroehr::config::load(config_path, overrides)?;
    config.validate()?;

    // ASCII banner before telemetry/log init, on the same resolved rendering the
    // log layer installs (so JSON output stays pure from the first byte). The
    // posture is the one the configuration alone shows; the cluster check needs
    // the pools and follows in the boot log.
    let declared = DeploymentPosture::evaluate(&config, &DatabaseFacts::default());
    ferroehr::banner::print(
        &telemetry::build_info::build(),
        config.spec_profile,
        &declared,
        config
            .telemetry
            .format
            .resolve(std::io::stdout().is_terminal()),
    );

    let build_info =
        BuildInfo::for_profile(config.spec_profile).with_audit(AuditPosture::of(&config.audit));
    let mut telemetry = telemetry::init(
        &config.telemetry,
        config.management.flame_file.as_deref(),
        &build_info,
    )
    .context("initialising telemetry")?;

    warn_boot_postures(&config);
    // The support statement at boot, and a WARN at boot and daily once the
    // period has ended (CRA Art. 13(19)).
    let support_watch = tokio::spawn(ferroehr::support::watch(SupportPeriod::current()));
    let pools = connect_pool(&config).await?;
    let pool = pools.clinical.clone();

    // The declared deployment posture, over the clusters the pools actually
    // reached (#3226): `production` refuses here, `sandbox` says what is open.
    let deployment = evaluate_deployment(&config, &pools).await?;

    // The database's own subject-shape guard follows the declared namespaces
    // (#3241): stamped by the runtime role on every boot, read by the trigger.
    stamp_subject_posture(&config, &pool).await?;

    // Fail-open at boot, except in a slim build, which cannot render the FHIR
    // `AuditEvent` the store and the ATX:FHIR Feed carry.
    #[cfg(not(feature = "fhir"))]
    ferroehr::system_log::require_fhir_disabled(&config.audit).map_err(|e| anyhow::anyhow!(e))?;
    let audit_config: AuditConfig = config.audit.clone();
    let (audit_sender, audit_handle) = start_audit(&audit_config, &pool, &pools.audit).await;

    // Contribution-outbox eventing (off by default).
    let outbox_enabled = config.events.enabled;
    #[cfg(feature = "events")]
    let events_handle = if config.events.enabled {
        tracing::info!(exchange = %config.events.exchange, "contribution-outbox eventing enabled");
        Some(ferroehr::extensions::events::publisher::start(
            config.events.clone(),
            pool.clone(),
            pools.party.clone(),
        ))
    } else {
        None
    };
    #[cfg(not(feature = "events"))]
    ferroehr::extensions::events::require_disabled(&config.events)
        .map_err(|e| anyhow::anyhow!(e))?;

    // Health indicators.
    let mut indicators: Vec<Arc<dyn HealthIndicator>> = vec![
        Arc::new(indicators::DbHealth::new(pool.clone())),
        Arc::new(indicators::MigrationsHealth::new(pool.clone())),
    ];
    // Always registered: a deployment that writes no access log reads DEGRADED
    // with the consequence stated rather than showing no row (#3238).
    let audit_posture = AuditPosture::of(&config.audit);
    indicators.push(Arc::new(match &audit_sender {
        Some(sender) => indicators::AuditHealth::new(sender.clone(), audit_posture),
        None => indicators::AuditHealth::disabled(audit_posture),
    }));
    if let Some(check) = audit_handle.as_ref().and_then(AuditHandle::chain_check) {
        indicators.push(Arc::new(indicators::AuditChainHealth::new(check)));
    }
    if let Some(delivery) = audit_handle
        .as_ref()
        .map(AuditHandle::delivery)
        .filter(|delivery| !delivery.statuses().is_empty())
    {
        indicators.push(Arc::new(indicators::AuditDeliveryHealth::new(delivery)));
    }
    #[cfg(feature = "events")]
    if let Some(handle) = &events_handle {
        indicators.push(Arc::new(indicators::EventsHealth::new(handle.healthy())));
    }

    telemetry.start_samplers(pool.clone());

    // `/management/env` and `GET {base}/admin/config` serve the whole config
    // tree, redacted by its `Secret`/`SecretUrl` leaf types.
    let env_snapshot = Arc::new(
        config
            .to_redacted_json()
            .context("rendering the redacted configuration snapshot")?,
    );

    // Version signing (fail-closed at boot for `pgp` without a usable key).
    let signer =
        Arc::new(Signer::from_config(&config.signing).context("initialising the version signer")?);
    tracing::info!(
        signing = signer.enabled(),
        verify_on_read = ?signer.verify_on_read(),
        "version signing configured"
    );

    // `[server] system_id` is stamped into `EHR.system_id`,
    // `AUDIT_DETAILS.system_id` and `OBJECT_VERSION_ID.creating_system_id`;
    // logged so an operator can see the key took.
    tracing::info!(system_id = %config.rest.system_id, "openEHR system identifier");

    // An IHE patient identity source answers its capabilities read here, so an
    // unreachable one refuses boot rather than the first request (#3380).
    let ehr_index = EhrIndexProvider::from_config(&config.ehr_index)
        .context("building the [ehr_index] identity source")?;
    ehr_index
        .ready()
        .await
        .context("the [ehr_index] identity source did not answer its capabilities read")?;
    tracing::info!(provider = ehr_index.name(), "EHR index provider");

    let audit_enabled = audit_sender.is_some();
    let service = Arc::new(assemble_service(
        &config,
        &pools,
        audit_sender,
        outbox_enabled,
        signer,
        deployment,
        ehr_index,
    )?);

    // The schemas are prepared by now, so a configured identifier scheme the
    // registry does not hold is refused here, not at its first party commit.
    if let Some(engine) = service.identifier_protection() {
        engine.verify_registered().await.context(
            "[demographic.identifier_protection].schemes names a scheme the identifier_scheme \
             registry does not hold",
        )?;
    }

    // The retention register keys category periods on the map in force, so it
    // is mirrored before the listener takes traffic. `validate` has already
    // refused a map that does not compile.
    let category_map = config
        .audit
        .categories
        .compile()
        .map_err(ferroehr::system_log::AuditError::CategoryMap)
        .context("compiling the [audit.categories] map")?;
    service
        .mirror_category_map(&category_map)
        .await
        .context("mirroring the [audit.categories] map into the retention register")?;

    // Outbound only and detached: neither boot, health nor readiness waits on
    // it, and a slow or unreachable collector reaches none of them.
    let usage_window = usage_product::window(&usage_product::api_mounts(&config));
    let usage_report = ferrohealth_usage_report::reporter::start_with_store(
        &config.usage_report,
        &usage_product::NOTICE,
        usage_product::identity(&build_info, service.licence()),
        Arc::new(PgStore::new(pool.clone())),
        Arc::new(FerroEhr::new(pool.clone())),
        Arc::clone(&usage_window),
        started,
        ferroehr::config::loader::usage_report_ci(),
    )
    .await;

    // The readiness registry closes over every started subsystem, so it is
    // assembled after the last optional one.
    let observability = Observability {
        management: config.management.clone(),
        telemetry: Some(telemetry.telemetry()),
        log_filter: Some(LogFilter {
            handle: telemetry.filter(),
            boot: telemetry.boot_filter(),
        }),
        request_log: RequestLog::new(&config.telemetry.logged_query_parameters)
            .with_metrics(&telemetry.telemetry().meter(telemetry::metrics::SCOPE))
            .with_recorder(usage_window)
            .serving_tls(config.server.tls.is_some()),
        // NOTE: no specification governs this: our own design; each check keeps
        // the one-second bound the probes have always had.
        health: Registry::new(indicators).with_timeout(Duration::from_secs(1)),
        lifecycle: Lifecycle::default(),
        build_info,
        env_snapshot,
    };

    let authz = wire_authz(&config, &pool, &service)?;

    // Assemble the REST adapter's runtime config view from the tree.
    let app_config = AppConfig {
        server: config.server.clone(),
        rest: config.rest.clone(),
        auth: config.auth.clone(),
        admin: config.admin.clone(),
        smart: config.smart.clone(),
        terminology_api_enabled: config.terminology.api_enabled,
        events_admin_api: config.events.admin_api,
        audit_organization_claim: config.authz.abac.organization_claim().map(str::to_owned),
        rbac: config.authz.rbac.clone(),
        assurance: config.authz.assurance.clone(),
        professional: config.authz.professional.clone(),
        spec_profile: config.spec_profile,
    };

    // RFC 9110 §11.6.1: a 401 challenge must name a scheme applicable to the
    // target resource, and a server with no mechanism has none to name.
    ferroehr::config::auth::require_mechanism(&app_config.auth)?;
    log_resolved_posture(
        &app_config,
        audit_enabled,
        &config.audit,
        &observability.management,
    );
    let drained = ferroehr_rest::serve_full(app_config, service, authz, observability)
        .await
        .context("serving ferroehr-rest")?;
    tracing::info!(?drained, "the listeners drained");

    if let Some(handle) = audit_handle {
        handle.shutdown(AUDIT_DRAIN_TIMEOUT).await;
    }
    #[cfg(feature = "events")]
    if let Some(handle) = events_handle {
        handle.shutdown(AUDIT_DRAIN_TIMEOUT).await;
    }
    support_watch.abort();
    if let Some(handle) = usage_report {
        handle.abort();
    }
    telemetry.shutdown().await;
    Ok(())
}

/// Start the audit subsystem from config. `(None, None)` when disabled or on a
/// boot failure (fail-open).
///
/// Two pools, deliberately: the record is written to the `audit` domain, while
/// the optional subject resolution reads the `clinical` one. A single pool
/// would put one of the two schemas outside the connection's `search_path`.
async fn start_audit(
    config: &AuditConfig,
    clinical: &PgPool,
    audit: &PgPool,
) -> (Option<AuditSender>, Option<AuditHandle>) {
    if !config.enabled {
        return (None, None);
    }
    let resolver = config
        .resolve_subject
        .then(|| subject_resolver(clinical.clone()));
    match ferroehr::system_log::sender::start(config.clone(), resolver, Some(audit.clone())).await {
        Ok((sender, handle)) => (Some(sender), Some(handle)),
        Err(e) => {
            tracing::error!("ATNA audit failed to start ({e}); continuing without auditing");
            (None, None)
        }
    }
}

/// Builds the full authorization handle the binary serves with: the RBAC gate
/// plus the ABAC gate over the boot-built policy engine.
///
/// `None` when neither layer is active. Fine-grained authorization is our own
/// extension — no openEHR spec governs it (ITS-REST places authorization out of
/// band).
///
/// # Errors
/// An ABAC block that is enabled but unbuildable: startup must abort rather than
/// silently run without the promised gate.
pub fn build_authz(
    config: &ferroehr::config::authz::AuthzConfig,
    base_path: &str,
    resolvers: AuthzResolvers,
) -> anyhow::Result<Option<Arc<AuthzHandle>>> {
    let engine = build_engine(&config.abac).context("building the ABAC policy engine")?;
    Ok(AuthzHandle::build(config, base_path, engine, resolvers).map(Arc::new))
}

/// The DB-backed ABAC attribute resolvers.
///
/// Builds `ferroehr_rest::extensions::access::authz::AuthzResolvers`: the EHR
/// subject as the patient scope addresses it (`ferroehr::aql::subject::ehr_subject`,
/// the rule the AQL patient scope applies) and the committed template of a
/// COMPOSITION version with its root archetype and origins (via the service
/// read-back), and the category and origin rules in `content`.
/// Failures are typed [`ResolveError`]s — the PEP fails closed on them, never
/// silently permits; a subject lookup that cannot obtain a connection carries
/// the `service_overloaded` call status, which the PEP answers with `503`.
#[must_use]
pub fn authz_resolvers(
    pool: PgPool,
    service: Arc<FerroEhrService>,
    content: ContentRules,
) -> AuthzResolvers {
    AuthzResolvers {
        subject: Arc::new(move |ehr_id: String| {
            let pool = pool.clone();
            Box::pin(async move {
                let id = Uuid::parse_str(&ehr_id)
                    .map_err(|e| ResolveError::new(format!("ehr id {ehr_id}"), e))?;
                ehr_subject(&pool, EhrId(id))
                    .await
                    .map_err(|e| ResolveError::new("ehr subject lookup", SmError::from(e)))
            })
        }),
        content_of_version: Arc::new(move |vo: String, version: Option<String>| {
            let service = Arc::clone(&service);
            Box::pin(async move {
                let vo_id = vo
                    .parse::<ferroehr::ids::VoId>()
                    .map_err(|e| ResolveError::new(format!("vo id {vo}"), e))?;
                service
                    .content_of_version(vo_id, version.as_deref())
                    .await
                    .map_err(|e| ResolveError::new("version content lookup", e))
            })
        }),
        content: Arc::new(content),
    }
}

/// The subject of an EHR for the Patient-Number participant object and the
/// access record's patient, read in the background through
/// `ferroehr::storage::ehr_repo::ehr_subject_columns`.
fn subject_resolver(pool: PgPool) -> SubjectResolver {
    Arc::new(move |ehr_id: String| {
        let pool = pool.clone();
        Box::pin(async move {
            // NOTE: an id that is no UUID names no stored EHR, so the parse failure is the answer.
            let Ok(id) = Uuid::parse_str(&ehr_id) else {
                return SubjectLookup::NoSubject;
            };
            match ehr_subject_columns(&pool, EhrId(id)).await {
                Ok(Some((Some(id), namespace))) => {
                    SubjectLookup::Found(ResolvedSubject { id, namespace })
                }
                Ok(_) => SubjectLookup::NoSubject,
                Err(e) => {
                    tracing::warn!(error = %e, "audit subject lookup failed; the record names the lookup unavailable");
                    SubjectLookup::Unavailable
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::run_report;

    /// A configuration that cannot be read keeps its cause: the chain of the
    /// error a subcommand returns ends at the file system's own error.
    #[tokio::test]
    async fn a_configuration_failure_keeps_its_root_cause() {
        let missing = std::path::Path::new("/nonexistent/ferroehr-test/ferroehr.toml");
        let err = run_report(None, Some(missing), &[])
            .await
            .expect_err("an unreadable configuration file is refused");
        let root = err.root_cause();
        let io = root
            .downcast_ref::<std::io::Error>()
            .unwrap_or_else(|| panic!("the root cause is the I/O error, not {root}: {err:#}"));
        assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
    }
}
