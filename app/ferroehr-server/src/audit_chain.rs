// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr audit-chain …`: the seal of the system chain and its
//! verification (#5573).
//!
//! The system chain holds the records outside the access log. The keyring
//! and the trusted certificates are the access log's (`[record.signing]`,
//! `[audit.access_log] trusted_key_files`). No openEHR spec governs audit
//! signing: our own design.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::system_log::seal::{ChainSeal, Seals};
use ferroehr::versioning::signature::signer::Signer;

/// `ferroehr audit-chain …` subcommands.
#[derive(Debug, Subcommand)]
pub enum AuditChainCmd {
    /// Seal the head of the system chain with the keyring's key, and print
    /// the seal.
    Seal,
    /// Verify the system chain against its newest seal, or the one in
    /// `--seal`, and print every finding. Exits non-zero when the chain does
    /// not verify.
    Verify {
        /// A seal kept outside this system, as the JSON
        /// `ferroehr audit-chain seal` printed.
        #[arg(long, value_name = "FILE")]
        seal: Option<PathBuf>,
    },
}

/// Run one `ferroehr audit-chain …` subcommand.
///
/// # Errors
/// The configuration does not load, the database or the keyring fails, the
/// chain does not verify, or a file does not read.
#[expect(
    clippy::print_stdout,
    reason = "the audit-chain subcommands' PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn run(
    cmd: &AuditChainCmd,
    file: Option<&Path>,
    sets: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(file, sets)?;
    config.validate()?;
    let signer =
        Arc::new(Signer::from_config(&config.record.signing).context("initialising the keyring")?);
    let pools = crate::connect_pool(&config).await?;
    let seals = crate::access_log::build(&config, pools.audit.clone(), signer)?.system_seals();
    let outcome = match cmd {
        AuditChainCmd::Seal => match seals.seal().await? {
            Some(seal) => {
                println!("{}", serde_json::to_string_pretty(&seal)?);
                Ok(())
            }
            None => Err(anyhow::anyhow!(
                "the system chain is empty: nothing to seal"
            )),
        },
        AuditChainCmd::Verify { seal } => verify(&seals, seal.as_deref()).await,
    };
    pools.close().await;
    outcome
}

/// `ferroehr audit-chain verify`: print the verification and fail when the
/// chain does not verify.
#[expect(
    clippy::print_stdout,
    reason = "the verify subcommand's PURPOSE is console output (.claude/rules/reliability.md \
              §tools)"
)]
async fn verify(seals: &Seals, file: Option<&Path>) -> anyhow::Result<()> {
    let given: Option<ChainSeal> = match file {
        Some(path) => {
            let text = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("reading the seal {}", path.display()))?;
            Some(serde_json::from_str(&text).context("the seal does not read")?)
        }
        None => None,
    };
    let verification = seals.verify(given).await?;
    match &verification.seal {
        Some(seal) => println!("seal: sequence {} digest {}", seal.sequence, seal.digest),
        None => println!("no seal to verify against"),
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
    anyhow::ensure!(verification.intact(), "the system chain does not verify");
    println!("the system chain verifies");
    Ok(())
}
