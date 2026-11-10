// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr db …`: the schema steps a least-privilege deployment runs out of
//! band, and the erasure of the instance.

use std::path::Path;

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::db;
use ferroehr::decommission::{Decommission, Erasure};
use ferroehr::telemetry::{self, build_info::BuildInfo};

/// `ferroehr db …` subcommands.
///
/// These exist so a least-privilege deployment can separate the two database
/// identities: `migrate` runs under the migrator DSN as a one-shot step
/// (a Kubernetes Job, an init container, a CI/CD stage), and the server then
/// boots with `[database] migrations = "verify"` under a DSN with no DDL rights at all.
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

/// `ferroehr db migrate` / `verify` / `erase`, the out-of-band schema steps.
///
/// # Errors
/// The configuration does not load or validate, telemetry does not start, or
/// the selected step fails.
pub async fn run(
    cmd: &DbCmd,
    config_path: Option<&Path>,
    overrides: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(config_path, overrides)?;
    config.validate()?;
    let build_info = BuildInfo::for_profile(config.deployment.spec_profile);
    let telemetry = telemetry::init(
        &config.telemetry,
        config.management.flame_file.as_deref(),
        &build_info,
    )
    .context("initialising telemetry")?;

    // Every subcommand reads the schema on the migration DSN
    // (`[database] migrate_url`, falling back to `[database] url`), which is the
    // credential that can reach every schema when the runtime ones each hold
    // one pseudonymisation domain.
    let outcome = match cmd {
        DbCmd::Migrate => db::apply_schema(&config.database)
            .await
            .context("applying migrations"),
        DbCmd::Verify => crate::verify_schema_and_isolation(&config).await,
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
