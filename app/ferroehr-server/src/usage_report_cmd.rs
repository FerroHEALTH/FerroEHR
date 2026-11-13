// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr usage-report --print`: the report this instance would send to
//! `FerroPULSE`, printed and never sent.

use std::path::Path;

use anyhow::Context as _;
use ferroehr::db;
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::usage_report::product::{self as usage_product, EventKind, NOTICE_URL};

use crate::{ReportEvent, load_licence};

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
pub(crate) async fn run(
    event: ReportEvent,
    config_path: Option<&Path>,
    overrides: &[(String, String)],
) -> anyhow::Result<()> {
    let config = ferroehr::config::load(config_path, overrides)?;
    config.validate()?;
    let licence = load_licence(&config)?;
    let build = BuildInfo::for_profile(config.deployment.spec_profile);
    let identity = usage_product::identity(&build, &licence);
    let pools = db::connect_domains(&config.database)
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
