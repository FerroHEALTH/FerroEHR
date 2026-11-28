// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's telemetry: the family's `ferrohealth-telemetry` provider, its
//! console, its panic line and its request log, as the CDR installs them.
//!
//! No specification governs the console, the request log or the panic line:
//! our own design.

use ferrohealth_server::stack::StackPolicy;
use ferrohealth_server::telemetry::{Installed, TelemetryBuildError};
use ferrohealth_telemetry::request_log::RequestLog;
use ferrohealth_telemetry::section::TelemetrySection;
use ferrohealth_telemetry::telemetry::{Service, Telemetry};

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
/// The returned [`Installed`] holds the clearing that keeps an `https`
/// collector cleared for revocation, so the caller keeps it for as long as
/// the job runs.
///
/// # Errors
///
/// Returns the [`TelemetryBuildError`] of a provider that cannot be built (an
/// OTLP table this build cannot honour, a collector TLS table it cannot read,
/// an exporter that cannot start) or of a process that already has a
/// subscriber.
pub fn init(section: &TelemetrySection) -> Result<Installed, TelemetryBuildError> {
    ferrohealth_server::telemetry::install(section, service())
}

/// Installs the console of `section` alone, exporting nothing: the short-lived
/// jobs (`healthcheck`) start no exporter.
///
/// # Errors
///
/// Returns every error of [`init`].
pub fn init_console(section: &TelemetrySection) -> Result<Installed, TelemetryBuildError> {
    init(&TelemetrySection {
        otlp: None,
        ..section.clone()
    })
}

/// Returns what the viewer decides inside the family stack.
#[must_use]
pub fn stack_policy(config: &ViewerConfig, telemetry: &Telemetry) -> StackPolicy {
    let meter = telemetry.meter(SCOPE);
    StackPolicy {
        request_log: RequestLog::new(&config.telemetry.logged_query_parameters)
            .with_metrics(&meter)
            .serving_tls(config.server.tls.is_some()),
        meter: Some(meter),
        ..StackPolicy::default()
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
