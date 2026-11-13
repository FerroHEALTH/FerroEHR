// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log's checkpoints, verification and transfer on the server
//! (#5129): built at boot from `[audit.access_log]` and the keyring, the
//! scheduled checkpoint, and `ferroehr access-log …`.
//!
//! The keyring is the one `[record.signing]` configures, so checkpoints and
//! signed links need it in `pgp` mode; a server asked for either without it
//! refuses to start. No openEHR spec governs log signing: our own design.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::transfer::{ImportRecord, LogExport};
use ferroehr::config::FerroEhrConfig;
use ferroehr::support::today_utc;
use ferroehr::versioning::signature::signer::Signer;
use ferroehr::versioning::signature::trust::TrustedCertificates;
use sqlx::PgPool;

/// `ferroehr access-log …` subcommands.
#[derive(Debug, Subcommand)]
pub enum AccessLogCmd {
    /// Seal the head of the access log in a checkpoint signed with the
    /// keyring's key, and print it.
    Checkpoint,
    /// Verify the access log against its newest checkpoint, or the one in
    /// `--checkpoint`, and print every finding. Exits non-zero when the log
    /// does not verify.
    Verify {
        /// A checkpoint kept outside this system, as the JSON
        /// `ferroehr access-log checkpoint` printed.
        #[arg(long, value_name = "FILE")]
        checkpoint: Option<PathBuf>,
    },
    /// Export one EHR's access log, with the checkpoint it verifies against,
    /// for the EHR's transfer to another system.
    Export {
        /// The EHR whose log is exported.
        #[arg(long, value_name = "EHR_ID")]
        ehr_id: String,
        /// Where to write the export; `-` or unset writes it to stdout.
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Import an EHR's access log another system exported, verified against
    /// its checkpoint and `[audit.access_log] trusted_key_files`, and record
    /// the import as an entry of this system's log.
    Import {
        /// The export to import.
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
    },
}

/// The access log over `pool`, signing with `signer` and trusting the
/// configured certificates.
///
/// # Errors
/// Checkpoints or signed links asked for without an `OpenPGP` key in the
/// keyring, or a trusted certificate that does not read.
pub fn build(
    config: &FerroEhrConfig,
    pool: PgPool,
    signer: Arc<Signer>,
) -> anyhow::Result<Checkpoints> {
    let section = &config.audit.access_log;
    anyhow::ensure!(
        !section.needs_key() || signer.holds_key(),
        "[audit.access_log] checkpoints and sign_links sign with the keyring's OpenPGP key: set \
         record.signing.mode = \"pgp\" and record.signing.key_file"
    );
    let trusted = TrustedCertificates::load(&section.trusted_key_files)
        .context("reading [audit.access_log] trusted_key_files")?;
    Ok(Checkpoints::new(
        pool,
        config.api.system_id.clone(),
        signer,
        Arc::new(trusted),
        section.sign_links,
    ))
}

/// Start the scheduled checkpoint when `[audit.access_log]` asks for one.
pub fn spawn_schedule(config: &FerroEhrConfig, checkpoints: &Checkpoints) {
    let section = &config.audit.access_log;
    let interval = section.checkpoint_interval.get();
    if section.checkpoints && !interval.is_zero() {
        tracing::info!(?interval, "access-log checkpoints scheduled");
        drop(tokio::spawn(ferroehr::access_log::schedule::scheduled(
            checkpoints.clone(),
            interval,
        )));
    }
}

/// Run one `ferroehr access-log …` subcommand.
///
/// # Errors
/// The configuration does not load, the database or the keyring fails, the
/// log does not verify, or a file does not read or write.
#[expect(
    clippy::print_stdout,
    reason = "the access-log subcommands' PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn run(
    cmd: &AccessLogCmd,
    file: Option<&Path>,
    sets: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(file, sets)?;
    config.validate()?;
    let signer =
        Arc::new(Signer::from_config(&config.record.signing).context("initialising the keyring")?);
    let pools = crate::connect_pool(&config).await?;
    let checkpoints = build(&config, pools.audit.clone(), signer)?;
    let outcome = match cmd {
        AccessLogCmd::Checkpoint => match checkpoints.seal().await? {
            Some(checkpoint) => {
                println!("{}", serde_json::to_string_pretty(&checkpoint)?);
                Ok(())
            }
            None => Err(anyhow::anyhow!("the access log is empty: nothing to seal")),
        },
        AccessLogCmd::Verify { checkpoint } => verify(&checkpoints, checkpoint.as_deref()).await,
        AccessLogCmd::Export { ehr_id, output } => {
            let export = checkpoints.export(ehr_id).await?;
            let written = serde_json::to_string_pretty(&export)?;
            match output.as_deref() {
                None => println!("{written}"),
                Some(path) if path == Path::new("-") => println!("{written}"),
                Some(path) => tokio::fs::write(path, format!("{written}\n"))
                    .await
                    .with_context(|| format!("writing the export to {}", path.display()))?,
            }
            Ok(())
        }
        AccessLogCmd::Import { input } => {
            let text = tokio::fs::read_to_string(input)
                .await
                .with_context(|| format!("reading the export {}", input.display()))?;
            let export: LogExport =
                serde_json::from_str(&text).context("the export does not read")?;
            let record = ImportRecord::by_operator(checkpoints.system_id(), today_utc());
            let imported = checkpoints.import(&export, record).await?;
            println!(
                "imported {} records of {} for the EHR {}, recorded as the entry {}",
                imported.records,
                imported.system_id,
                export.ehr_id,
                imported.entry.uid.value()
            );
            Ok(())
        }
    };
    pools.close().await;
    outcome
}

/// `ferroehr access-log verify`: print the verification and fail when the
/// log does not verify.
#[expect(
    clippy::print_stdout,
    reason = "the verify subcommand's PURPOSE is console output (.claude/rules/reliability.md \
              §tools)"
)]
async fn verify(checkpoints: &Checkpoints, file: Option<&Path>) -> anyhow::Result<()> {
    let given = match file {
        Some(path) => {
            let text = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("reading the checkpoint {}", path.display()))?;
            Some(serde_json::from_str(&text).context("the checkpoint does not read")?)
        }
        None => None,
    };
    let verification = checkpoints.verify(given).await?;
    match &verification.checkpoint {
        Some(checkpoint) => println!(
            "checkpoint: sequence {} digest {}",
            checkpoint.sequence, checkpoint.digest
        ),
        None => println!("no checkpoint to verify against"),
    }
    for finding in &verification.findings {
        println!("finding: {finding}");
    }
    for finding in &verification.chain {
        println!(
            "chain finding at {}: {}",
            finding
                .chain_seq
                .map_or_else(|| "-".to_owned(), |sequence| sequence.to_string()),
            finding.finding
        );
    }
    anyhow::ensure!(verification.intact(), "the access log does not verify");
    println!("the access log verifies");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checkpoints and signed links sign with the keyring's key, so a server
    /// asked for either with a `digest` keyring refuses to start.
    #[tokio::test]
    async fn signing_the_log_without_a_key_refuses_to_start() {
        for (checkpoints, sign_links) in [(true, false), (false, true)] {
            let mut config = FerroEhrConfig::default();
            config.audit.access_log.checkpoints = checkpoints;
            config.audit.access_log.sign_links = sign_links;
            let pool = PgPool::connect_lazy("postgres://localhost/unused").expect("a lazy pool");
            let refused = build(&config, pool, Arc::new(Signer::digest_default()))
                .expect_err("no OpenPGP key in the keyring");
            assert!(
                refused.to_string().contains("record.signing.mode"),
                "{refused}"
            );
        }
    }
}
