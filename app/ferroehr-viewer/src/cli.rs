// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ferroehr-viewer` command line: the family's `serve`, `config check`
//! and `healthcheck` (`ferrohealth_server::cli::Common`), with `serve` when
//! no subcommand is named.
//!
//! No specification governs the command line: our own design.

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_health::registry::Registry;
use ferrohealth_server::cli::{Common, ConfigArgs, ConfigCommand};
use ferrohealth_server::health::Health;
use ferrohealth_server::healthcheck::{Outcome, check, check_tls, target};
use ferrohealth_server::section::ServerSection;
use ferrohealth_server::tls::HealthcheckTls;
use ferrohealth_telemetry::telemetry::Telemetry;

use crate::config::ViewerConfig;

/// How long `healthcheck` waits for the readiness answer: shorter than the
/// image's `HEALTHCHECK --timeout=5s`, so the probe reports the reason itself.
pub const HEALTHCHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// The `ferroehr-viewer` command line.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "ferroehr-viewer", version, about = "The FerroEHR viewer")]
pub struct Cli {
    /// `--config`, overriding `FERROEHR_VIEWER_CONFIG`, and `--set`.
    #[command(flatten)]
    pub config: ConfigArgs,
    /// The job to run; `serve` when none is named.
    #[command(subcommand)]
    pub command: Option<Common>,
}

/// Runs the job `cli` names.
///
/// # Errors
/// Returns the configuration, boot, probe or listener failure of the job.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let path = cli.config.config.as_deref();
    let sets = cli.config.sets.as_slice();
    match cli.command.unwrap_or(Common::Serve) {
        Common::Healthcheck => {
            let config = crate::config::load(path, sets)?;
            crate::telemetry::init_console(&config.telemetry)?;
            let outcome = healthcheck(&config.server).await;
            anyhow::ensure!(outcome.exit_code() == 0, "healthcheck: {outcome}");
            Ok(())
        }
        Common::Config {
            command: ConfigCommand::Check,
        } => {
            let config = crate::config::load(path, sets)?;
            let telemetry = crate::telemetry::init(&config.telemetry)?;
            let outcome = check_clients(&config);
            if outcome.is_ok() {
                tracing::info!("the configuration is valid");
            }
            crate::telemetry::shutdown(&telemetry);
            outcome
        }
        Common::Serve => {
            let config = crate::config::load(path, sets)?;
            let telemetry = crate::telemetry::init(&config.telemetry)?;
            let outcome = serve(config, &telemetry).await;
            crate::telemetry::shutdown(&telemetry);
            outcome
        }
    }
}

/// Asks the viewer `section` configures for its readiness, over the TLS it
/// configures, on the loopback when it listens on every interface.
pub async fn healthcheck(section: &ServerSection) -> Outcome {
    let address = target(section.listen);
    let base = section.base_path.as_str();
    match &section.tls {
        None => check(address, base, HEALTHCHECK_TIMEOUT).await,
        Some(tls) => match HealthcheckTls::read(tls) {
            Ok(pinned) => check_tls(address, base, HEALTHCHECK_TIMEOUT, &pinned).await,
            Err(error) => Outcome::Failed(format!("server.tls does not read: {error}")),
        },
    }
}

/// Builds every client the viewer serves with, short of reaching a remote:
/// the CDR client, the issuer client and the session key.
fn check_clients(config: &ViewerConfig) -> anyhow::Result<()> {
    crate::cdr::CdrClient::new(&config.cdr)?;
    if config.auth.oidc.enabled {
        crate::oidc::issuer_client(&config.auth.oidc)?;
    }
    crate::session::SessionKeys::from_secret(&config.session.secret)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}

/// Serves the viewer until `SIGTERM` or `SIGINT`, then drains within
/// `server.shutdown_timeout_ms`, with the request log on `telemetry`.
async fn serve(config: ViewerConfig, telemetry: &Telemetry) -> anyhow::Result<()> {
    let request_log = crate::telemetry::request_log(&config, telemetry);
    let config = Arc::new(config);
    let cdr = crate::cdr::CdrClient::new(&config.cdr)?;
    let oidc = if config.auth.oidc.enabled {
        Some(Arc::new(crate::oidc::discover(&config.auth.oidc).await?))
    } else {
        None
    };
    let session_keys = crate::session::SessionKeys::from_secret(&config.session.secret)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let app_state = crate::state::AppState {
        config: Arc::clone(&config),
        cdr,
        oidc,
        session_keys,
    };

    let lifecycle = Lifecycle::default();
    let health = Health::new(Registry::default(), lifecycle.clone());
    let service = crate::server::app(app_state, health, request_log);

    let section = &config.server;
    let listener = tokio::net::TcpListener::bind(section.listen).await?;
    tracing::info!(listen = %section.listen, "ferroehr-viewer listening");
    let drained = ferrohealth_server::serve::run(
        listener,
        service,
        section,
        lifecycle,
        ferrohealth_server::serve::shutdown_signal(),
    )
    .await?;
    tracing::info!(?drained, "ferroehr-viewer stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use ferrohealth_server::cli::{Common, ConfigCommand};

    use super::Cli;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_job_parses_and_no_job_serves() {
        for (args, command) in [
            (vec!["ferroehr-viewer"], None),
            (vec!["ferroehr-viewer", "serve"], Some(Common::Serve)),
            (
                vec!["ferroehr-viewer", "config", "check"],
                Some(Common::Config {
                    command: ConfigCommand::Check,
                }),
            ),
            (
                vec!["ferroehr-viewer", "healthcheck"],
                Some(Common::Healthcheck),
            ),
        ] {
            let parsed = Cli::try_parse_from(&args).expect("the job parses");
            assert_eq!(command, parsed.command, "{args:?}");
        }
        let parsed = Cli::try_parse_from([
            "ferroehr-viewer",
            "--config",
            "viewer.toml",
            "healthcheck",
            "--set",
            "server.listen=0.0.0.0:3000",
        ])
        .expect("the job parses");
        assert_eq!(Some("viewer.toml".into()), parsed.config.config);
        assert_eq!(1, parsed.config.sets.len());
    }
}
