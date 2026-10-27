// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's telemetry: the family's `ferrohealth-telemetry` provider, its
//! console, its panic line and its request log, as the CDR installs them.
//!
//! No specification governs the console, the request log or the panic line:
//! our own design.

use std::sync::Arc;

use ferrohealth_telemetry::request_log::RequestLog;
use ferrohealth_telemetry::section::TelemetrySection;
use ferrohealth_telemetry::telemetry::{Console, InitError, Service, Telemetry};

use crate::config::ViewerConfig;

/// The `service.name` of every span and metric.
pub const SERVICE: &str = "ferroehr-viewer";

/// The crate whose spans are exported, and the scope of the request log's
/// meter.
pub const SCOPE: &str = "ferroehr_viewer";

/// Returns the service the telemetry describes: `ferroehr-viewer` at the
/// viewer's version, exporting the viewer crate's spans.
#[must_use]
pub fn service() -> Service {
    Service::new(SERVICE, env!("CARGO_PKG_VERSION")).export_crate(SCOPE)
}

/// Builds the provider `section` asks for, installs the process subscriber on
/// stdout and the family's panic hook.
///
/// # Errors
///
/// Returns the [`InitError`] of a provider that cannot be built (an OTLP
/// table this build cannot honour, an exporter that cannot start) or of a
/// process that already has a subscriber.
pub fn init(section: &TelemetrySection) -> Result<Arc<Telemetry>, InitError> {
    let telemetry = Arc::new(Telemetry::new(section, service())?);
    // The viewer serves no logger route, so the filter handle is not kept.
    telemetry.install(section, &Console::detect())?;
    ferrohealth_telemetry::panic::install_hook();
    Ok(telemetry)
}

/// Installs the console of `section` alone, exporting nothing: the short-lived
/// jobs (`healthcheck`) start no exporter.
///
/// # Errors
///
/// Returns every error of [`init`].
pub fn init_console(section: &TelemetrySection) -> Result<Arc<Telemetry>, InitError> {
    init(&TelemetrySection {
        otlp: None,
        ..section.clone()
    })
}

/// Returns the request log the family stack writes: the query parameters
/// `[telemetry]` lets through, the HTTP server metrics on `telemetry`'s meter
/// and the scheme the listener serves.
#[must_use]
pub fn request_log(config: &ViewerConfig, telemetry: &Telemetry) -> RequestLog {
    RequestLog::new(&config.telemetry.logged_query_parameters)
        .with_metrics(&telemetry.meter(SCOPE))
        .serving_tls(config.server.tls.is_some())
}

/// Flushes and stops the exporters of `telemetry`, logging a failure: the job
/// has already ended, so a lost flush changes nothing it answers.
pub fn shutdown(telemetry: &Telemetry) {
    if let Err(error) = telemetry.shutdown() {
        tracing::warn!(error = %error, "the telemetry did not flush");
    }
}

#[cfg(test)]
mod tests {
    use super::{SCOPE, SERVICE, service};

    #[test]
    fn the_service_is_the_viewer_and_exports_its_own_spans() {
        let service = service();
        assert_eq!(SERVICE, service.name());
        assert_eq!(SCOPE, env!("CARGO_CRATE_NAME"));
    }
}
