// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scale test's bulk seeder (#5090): a store of millions of compositions,
//! in the shape the commit path writes, without committing each one.
//!
//! `prototypes` commits a set of compositions and their updates through the
//! service, the product's own commit path. `seed` clones their rows with
//! `COPY` into EHRs of its own, under fresh identifiers, in chunks a stopped
//! run resumes from. `check` compares a sample of clones with their
//! prototypes, in the stored rows and as the service serves them. `measure`
//! runs the scale test's queries against one seeded EHR and writes the report.
//! `lookup` seeds the login binding's side table with bound persons and groups and
//! times the lookup every request of a person makes against its budget (#5087).
//! No openEHR specification governs load generation: our own design.
//!
//! ```text
//! scaleseed --database-url postgres://… prototypes --variants 2000
//! scaleseed --database-url postgres://… seed --ehrs 100000 --compositions 1000000
//! scaleseed --database-url postgres://… check --sample 200
//! scaleseed --database-url postgres://… measure --out target/scale/1m --host "…"
//! scaleseed --database-url postgres://… lookup --out target/perf/lookup
//! ```

#![expect(
    clippy::print_stderr,
    reason = "a command-line tool reports its progress and its verdict on stderr"
)]

mod check;
mod clone;
mod lookup;
mod measure;
mod proto;
mod seed;

use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Parser, Subcommand};

/// The scale test's bulk seeder.
#[derive(Debug, Parser)]
struct Cli {
    /// The database to seed, migrated by the server or by `migrate`.
    #[arg(long, env = "SCALESEED_DATABASE_URL")]
    database_url: String,
    /// What to do.
    #[command(subcommand)]
    command: Command,
}

/// The steps, run in this order.
#[derive(Debug, Subcommand)]
enum Command {
    /// Applies the server's migrations to an empty database.
    Migrate,
    /// Commits the prototype compositions through the service.
    Prototypes {
        /// How many content variants to commit.
        #[arg(long, default_value_t = 2000)]
        variants: usize,
    },
    /// Clones the prototypes into the store, resuming a stopped run.
    Seed {
        /// The EHRs to write.
        #[arg(long)]
        ehrs: u64,
        /// The compositions to write over them.
        #[arg(long)]
        compositions: u64,
        /// The EHRs one transaction writes.
        #[arg(long, default_value_t = 200)]
        chunk: u64,
        /// The chunks written at once.
        #[arg(long, default_value_t = 6)]
        jobs: usize,
    },
    /// Compares sampled clones with their prototypes.
    Check {
        /// How many sampled clones to compare.
        #[arg(long, default_value_t = 200)]
        sample: i64,
    },
    /// Measures the queries and writes the report.
    Measure {
        /// The directory the report and the plans are written to.
        #[arg(long)]
        out: PathBuf,
        /// The host the store runs on, for the report.
        #[arg(long)]
        host: String,
    },
    /// Seeds the login binding's side table and times the lookup.
    Lookup {
        /// The bound persons to seed.
        #[arg(long, default_value_t = 50_000)]
        persons: i64,
        /// The groups to seed.
        #[arg(long, default_value_t = 5_000)]
        groups: i64,
        /// The lookups to time.
        #[arg(long, default_value_t = 5_000)]
        samples: usize,
        /// The p99 budget in milliseconds (#5004).
        #[arg(long, default_value_t = 2.0)]
        budget_ms: f64,
        /// Seed only, and time nothing.
        #[arg(long)]
        seed_only: bool,
        /// The directory the report is written to.
        #[arg(long, default_value = "target/perf/lookup")]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = ferroehr::db::DbConfig::new(cli.database_url.clone());
    config.max_connections = 16;
    config.statement_timeout_ms = 0;
    let pool = ferroehr::db::connect(&config)
        .await
        .context("connect to the database")?;
    let svc = ferroehr::service::FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    );
    match cli.command {
        Command::Migrate => ferroehr::db::run_migrations(&pool)
            .await
            .context("apply the migrations")?,
        Command::Prototypes { variants } => proto::commit(&svc, &pool, variants).await?,
        Command::Seed {
            ehrs,
            compositions,
            chunk,
            jobs,
        } => {
            seed::run(
                svc,
                pool,
                seed::Target {
                    ehrs,
                    compositions,
                    chunk,
                    jobs,
                },
            )
            .await?;
        }
        Command::Check { sample } => {
            let checked = check::run(&svc, &pool, sample).await?;
            eprintln!("scaleseed: {checked} sampled clones match their prototypes");
        }
        Command::Lookup {
            persons,
            groups,
            samples,
            budget_ms,
            seed_only,
            out,
        } => {
            let target = lookup::Target {
                persons,
                groups,
                samples,
                budget_ms,
            };
            lookup::seed(&pool, target).await?;
            if !seed_only {
                let source = svc.party_source("scaleseed");
                let figures = lookup::measure(&pool, &source, target, &out).await?;
                eprintln!(
                    "scaleseed: the lookup p99 is {:.3} ms against a budget of {} ms ({}); \
                     report written to {}",
                    figures.p99_ms,
                    figures.budget_ms,
                    if figures.within_budget() {
                        "within"
                    } else {
                        "over"
                    },
                    out.join("report.md").display()
                );
            }
        }
        Command::Measure { out, host } => {
            measure::run(&svc, &pool, &cli.database_url, &host, &out).await?;
            eprintln!(
                "scaleseed: report written to {}",
                out.join("report.md").display()
            );
        }
    }
    Ok(())
}
