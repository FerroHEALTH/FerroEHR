// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ferroehr-viewer` command line: the family's `serve`, `config check`
//! and `healthcheck` (`ferrohealth_server::cli::Common`), with `serve` when
//! no subcommand is named.
//!
//! No specification governs the command line: our own design.

use std::error::Error;
use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_health::registry::Registry;
use ferrohealth_server::cli::{Common, ConfigArgs, ConfigCommand};
use ferrohealth_server::health::Health;
use ferrohealth_server::healthcheck::{Outcome, check, check_tls, target};
use ferrohealth_server::run::{EXIT_CONFIG, Failure, Run};
use ferrohealth_server::section::ServerSection;
use ferrohealth_server::tls::{HealthcheckTls, TlsKeys};
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

/// The viewer's process: `args` (the program name first) parsed by the
/// family's `ferrohealth_server::run::Run`, [`run`] on its runtime, and the
/// exit code, with a failure's whole cause on stderr.
#[must_use]
pub fn main<I: IntoIterator<Item = T>, T: Into<OsString> + Clone>(args: I) -> ExitCode {
    Run::new("ferroehr-viewer").run(args, run)
}

/// Runs the job `cli` names.
///
/// # Errors
/// Returns the [`Failure`] of the job: a configuration the viewer refuses
/// exits [`EXIT_CONFIG`] (78, `EX_CONFIG` of `sysexits(3)`), a failed probe,
/// boot or listener exits 1.
pub async fn run(cli: Cli) -> Result<ExitCode, Failure> {
    let path = cli.config.config.as_deref();
    let sets = cli.config.sets.as_slice();
    let config = crate::config::load(path, sets).map_err(refused)?;
    match cli.command.unwrap_or(Common::Serve) {
        Common::Healthcheck => {
            crate::telemetry::init_console(&config.telemetry)?;
            let outcome = healthcheck(&config.server).await;
            if outcome.exit_code() == 0 {
                Ok(ExitCode::SUCCESS)
            } else {
                Err(Failure::boxed(format!("healthcheck: {outcome}")))
            }
        }
        Common::Config {
            command: ConfigCommand::Check,
        } => {
            let telemetry = crate::telemetry::init(&config.telemetry)?;
            let outcome = check_clients(&config);
            if outcome.is_ok() {
                tracing::info!("the configuration is valid");
            }
            crate::telemetry::shutdown(&telemetry);
            outcome.map(|()| ExitCode::SUCCESS)
        }
        Common::Serve => {
            let telemetry = crate::telemetry::init(&config.telemetry)?;
            let outcome = serve(config, &telemetry).await;
            crate::telemetry::shutdown(&telemetry);
            outcome.map(|()| ExitCode::SUCCESS)
        }
    }
}

/// The failure of a configuration the viewer refuses: exit [`EXIT_CONFIG`].
fn refused(error: impl Error + 'static) -> Failure {
    Failure::from(error).with_code(EXIT_CONFIG)
}

/// Asks the viewer `section` configures for its readiness, over the TLS it
/// configures, on the loopback when it listens on every interface.
pub async fn healthcheck(section: &ServerSection) -> Outcome {
    let address = target(section.listen);
    let base = section.base_path.as_str();
    match &section.tls {
        None => check(address, base, HEALTHCHECK_TIMEOUT).await,
        Some(tls) => match HealthcheckTls::read_of(tls, TlsKeys::SERVER) {
            Ok(pinned) => check_tls(address, base, HEALTHCHECK_TIMEOUT, &pinned).await,
            Err(error) => Outcome::Failed(format!("server.tls does not read: {error}")),
        },
    }
}

/// Builds every client the viewer serves with, short of reaching a remote:
/// the CDR client and the session store. `[oidc]` and `[session]` were
/// checked when the configuration loaded.
fn check_clients(config: &ViewerConfig) -> Result<(), Failure> {
    let cdr = crate::cdr::CdrClient::new(&config.cdr).map_err(refused)?;
    crate::state::AppState::new(config.clone(), cdr, None).map_err(refused)?;
    Ok(())
}

/// Serves the viewer until `SIGTERM` or `SIGINT`, then drains within
/// `server.shutdown_timeout`, with the request log on `telemetry`.
async fn serve(config: ViewerConfig, telemetry: &Telemetry) -> Result<(), Failure> {
    let request_log = crate::telemetry::request_log(&config, telemetry);
    let cdr = crate::cdr::CdrClient::new(&config.cdr).map_err(refused)?;
    // NOTE: ferrohealth-session 0.2.1 `config::OidcSettings::relying_party`: the provider's
    // metadata is discovered at the start, so a broken provider refuses it.
    let party = if config.auth.oidc_enabled {
        let settings = config
            .oidc
            .check_with_tls(&ferrohealth_smart::config::ServerTls)
            .map_err(refused)?;
        Some(settings.relying_party().await?)
    } else {
        None
    };
    let app_state = crate::state::AppState::new(config, cdr, party).map_err(refused)?;
    let config = Arc::clone(&app_state.config);

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
    use std::net::TcpListener;
    use std::process::ExitCode;

    use clap::{CommandFactory, Parser};
    use ferrohealth_server::cli::{Common, ConfigCommand};
    use ferrohealth_server::run::{EXIT_CONFIG, EXIT_FAILURE};

    use super::{Cli, main};

    #[test]
    fn a_refused_configuration_exits_78() {
        let code = main([
            "ferroehr-viewer",
            "--set",
            "nonsense.key=1",
            "config",
            "check",
        ]);
        assert_eq!(ExitCode::from(EXIT_CONFIG.get()), code);
    }

    #[test]
    fn a_failed_probe_exits_1() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("the bound address");
        drop(listener);
        let listen = format!("server.listen={address}");
        let code = main(["ferroehr-viewer", "--set", listen.as_str(), "healthcheck"]);
        assert_eq!(ExitCode::from(EXIT_FAILURE.get()), code);
    }

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
