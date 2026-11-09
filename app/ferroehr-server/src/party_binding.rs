// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared party mapping on the server.
//!
//! It is installed on the service and judged against its templates at boot;
//! the binding side table is filled or checked under its digest, with this
//! replica's lease renewed while it runs; and `ferroehr party-binding refill`
//! is the one path by which the mapping of a deployment changes.
//!
//! A mapping change is a stop-the-world step: every replica stops, the
//! refill runs, and replicas with the new mapping start. No openEHR spec
//! governs the binding: our own design.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::config::FerroEhrConfig;
use ferroehr::db::domain::DomainPools;
use ferroehr::party_binding::check::{Administrator, BasicUser, CheckInput, Expected, Report};
use ferroehr::party_binding::fill::{LEASE_RENEWAL, LEASE_TTL, Lease};
use ferroehr::service::FerroEhrService;
use ferroehr::service::message::import_policy::ImportPolicy;

/// `ferroehr party-binding …` subcommands.
#[derive(Debug, Subcommand)]
pub enum PartyBindingCmd {
    /// Rebuild the binding side table from the party versions under the
    /// configured mapping and store its digest. Run it with every replica
    /// stopped: it refuses while a replica's lease is live.
    Refill,
    /// Report, from the party versions and writing nothing, what binding
    /// will make of them: the keys per issuer, duplicates, inactive,
    /// abandoned and provisional persons, key-bearing persons with no
    /// professional `ROLE`, the mapped Basic users, and the committer that
    /// introduced each key. It exits non-zero on a duplicate, a missing
    /// person or a key whose committer fails the provenance rule.
    Check(CheckArgs),
}

/// The operator input of `ferroehr party-binding check`.
#[derive(Debug, Clone, clap::Args)]
pub struct CheckArgs {
    /// An administrator, named by the container id of its `PERSON`: a key
    /// its commit introduced passes the provenance rule.
    #[arg(long = "admin", value_name = "PARTY_ID")]
    pub admins: Vec<uuid::Uuid>,
    /// An administrator, named by the `identifier_issuer` and value of a
    /// binding key its `PERSON` carries.
    #[arg(long = "admin-key", num_args = 2, value_names = ["ISSUER", "KEY"])]
    pub admin_keys: Vec<String>,
    /// The committer name of a migration the operator declares: a key its
    /// commit introduced passes the provenance rule.
    #[arg(long = "migration", value_name = "COMMITTER_NAME")]
    pub migrations: Vec<String>,
    /// The identity provider's assigned users, one login identifier per
    /// line: every one with no `PERSON` is listed.
    #[arg(long, value_name = "FILE")]
    pub expected: Option<PathBuf>,
    /// The `identifier_issuer` the `--expected` logins are keys of; needed
    /// when more than one `[[auth.party]]` table is configured.
    #[arg(long, value_name = "ISSUER", requires = "expected")]
    pub expected_issuer: Option<String>,
    /// How many days a `553|incomplete|` `PERSON` may stay provisional before
    /// it is reported.
    #[arg(long, default_value_t = 30)]
    pub horizon_days: u32,
}

/// Returns `service` with the import rules and the configured party mapping
/// installed: the templates a mapped party is validated against and the
/// mapping the side table is derived under.
///
/// # Errors
/// A selector path that does not compile, which the configuration check
/// reports first.
pub fn install(
    service: FerroEhrService,
    config: &FerroEhrConfig,
) -> anyhow::Result<FerroEhrService> {
    let resolved = ferroehr::config::auth::resolve(&config.auth.shared)
        .context("resolving [auth] for the party mapping")?;
    let key_issuers = ferroehr::config::binding::key_issuers(&resolved);
    let logins = resolved
        .issuers
        .iter()
        .map(|settings| settings.issuer.clone())
        .chain(key_issuers.iter().cloned());
    let service = service.with_import_policy(ImportPolicy::new(
        config.authz.rbac.import_role.clone(),
        config.authz.import.trusted_systems.clone(),
        logins,
    ));
    let sections = config.authz.party_binding.sections();
    if sections.is_empty() {
        return Ok(service);
    }
    let mapping = sections
        .binding_mapping(&key_issuers, &config.authz.rbac.admin_role)
        .context("compiling [authz.party_binding]")?;
    Ok(service
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping))
}

/// Judges the mapping against its templates, prepares the side table and
/// takes this replica's lease, renewed in the background until it is
/// released.
///
/// # Errors
/// A template the mapping cannot use, a table filled under another mapping,
/// a value two parties carry, or a party store failure.
pub async fn prepare(
    service: &Arc<FerroEhrService>,
    config: &FerroEhrConfig,
) -> anyhow::Result<Option<Held>> {
    let errors = service
        .party_mapping_errors(&config.authz.party_binding.sections())
        .await?;
    anyhow::ensure!(
        errors.is_empty(),
        "[authz.party_binding] is refused: {}",
        errors.join("; ")
    );
    let Some(lease) = service
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .context("preparing the party binding side table")?
    else {
        return Ok(None);
    };
    let lease = Arc::new(lease);
    let renewing = Arc::clone(&lease);
    let renewal = tokio::spawn(async move {
        let mut tick = tokio::time::interval(LEASE_RENEWAL);
        loop {
            tick.tick().await;
            if let Err(e) = renewing.renew().await {
                tracing::warn!(
                    error = %ferroehr::error::ErrorChain::new(&e),
                    "the party binding lease could not be renewed"
                );
            }
        }
    });
    Ok(Some(Held { lease, renewal }))
}

/// This replica's lease, renewed in the background.
#[derive(Debug)]
pub struct Held {
    /// The lease.
    lease: Arc<Lease>,
    /// The renewal task.
    renewal: tokio::task::JoinHandle<()>,
}

impl Held {
    /// Stops renewing and releases the lease, so a refill need not wait for
    /// it to lapse.
    pub async fn release(self) {
        let Self { lease, renewal } = self;
        renewal.abort();
        // NOTE: tokio JoinHandle::abort, the aborted task drops its clone of the
        // lease, so this one is the last; the join error of the abort is expected.
        let lease = match renewal.await {
            Ok(()) | Err(_) => Arc::try_unwrap(lease),
        };
        if let Ok(lease) = lease
            && let Err(e) = lease.release().await
        {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&e),
                "the party binding lease could not be released; it lapses unrenewed"
            );
        }
    }
}

/// `ferroehr party-binding refill`.
///
/// # Errors
/// The configuration does not load or validate, no mapping is configured, a
/// replica's lease is live, a value two parties carry, or the party store
/// fails.
#[expect(
    clippy::print_stdout,
    reason = "the party-binding subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn run(
    cmd: &PartyBindingCmd,
    file: Option<&Path>,
    sets: &[(String, String)],
) -> anyhow::Result<()> {
    match cmd {
        PartyBindingCmd::Refill => {
            let config = ferroehr::config::load(file, sets)?;
            config.validate()?;
            let pools = crate::connect_pool(&config).await?;
            judged(&pools, &config)
                .await?
                .refill_binding(LEASE_TTL)
                .await
                .context("refilling the party binding side table")?;
            println!("the party binding side table is refilled under the configured mapping");
            Ok(())
        }
        PartyBindingCmd::Check(args) => {
            let config = ferroehr::config::load(file, sets)?;
            config.validate()?;
            let report = check(args, &config).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            let failures = report.failures();
            anyhow::ensure!(
                failures.is_empty(),
                "the party binding pre-flight fails: {}",
                failures.join("; ")
            );
            Ok(())
        }
    }
}

/// Runs `ferroehr party-binding check` over the configured database, with
/// no migration and no write.
///
/// # Errors
/// No mapping is configured, the mapping or the operator input is refused,
/// the `--expected` file does not read, or the party store fails.
pub async fn check(args: &CheckArgs, config: &FerroEhrConfig) -> anyhow::Result<Report> {
    let resolved = ferroehr::config::auth::resolve(&config.auth.shared)
        .context("resolving [auth] for the party mapping")?;
    let key_types: BTreeMap<String, String> = resolved
        .party
        .iter()
        .map(|table| {
            (
                table.identifier_issuer.clone(),
                table.identifier_type.clone(),
            )
        })
        .collect();
    let expected = match &args.expected {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading the expected logins {}", path.display()))?;
            let issuers: Vec<&String> = key_types.keys().collect();
            let issuer = match (&args.expected_issuer, issuers.as_slice()) {
                (Some(issuer), _) => issuer.clone(),
                (None, [only]) => (*only).clone(),
                (None, _) => anyhow::bail!(
                    "--expected-issuer names the issuer of the expected logins, one of {issuers:?}"
                ),
            };
            Some(Expected {
                issuer,
                logins: text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        }
        None => None,
    };
    let administrators = args
        .admins
        .iter()
        .map(|party| Administrator::Party(*party))
        .chain(args.admin_keys.chunks(2).filter_map(|pair| match pair {
            [issuer, key] => Some(Administrator::Key {
                issuer: issuer.clone(),
                key: key.clone(),
            }),
            _ => None,
        }))
        .collect();
    let input = CheckInput {
        key_types,
        basic_users: config
            .authz
            .party_binding
            .basic_user
            .iter()
            .map(|user| BasicUser {
                user: user.user.clone(),
                key_issuer: user.key_issuer.clone(),
                key: user.key.clone(),
            })
            .collect(),
        scim_clients: config
            .scim
            .client
            .iter()
            .map(|client| (client.issuer.clone(), client.client_id.clone()))
            .collect(),
        administrators,
        migrations: args.migrations.clone(),
        expected,
        horizon_days: args.horizon_days,
        today: ferroehr::support::today_utc().to_string(),
    };
    let pools = ferroehr::db::connect_domains(&config.db, &config.storage)
        .await
        .context("connecting to PostgreSQL")?;
    judged(&pools, config)
        .await?
        .check_binding(&input)
        .await
        .context("running the party binding pre-flight")
}

/// The service over `pools` with the configured mapping installed, once the
/// mapping is judged against its templates.
///
/// # Errors
/// A selector path that does not compile, a template the mapping cannot
/// use, or a definition store failure.
async fn judged(pools: &DomainPools, config: &FerroEhrConfig) -> anyhow::Result<FerroEhrService> {
    let service = install(FerroEhrService::new(pools), config)?;
    let errors = service
        .party_mapping_errors(&config.authz.party_binding.sections())
        .await?;
    anyhow::ensure!(
        errors.is_empty(),
        "[authz.party_binding] is refused: {}",
        errors.join("; ")
    );
    Ok(service)
}
