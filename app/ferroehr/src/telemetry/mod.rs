// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Telemetry: the family's `ferrohealth-telemetry` provider, with FerroEHR's
//! own instruments and the span-timing capture beside it.
//!
//! [`init`] builds the one OpenTelemetry provider from `[telemetry]`, installs
//! the console with the optional `[management] flame_file` capture, and
//! registers FerroEHR's instruments ([`metrics`]) on the shared meter. It
//! returns a [`TelemetryGuard`] holding the provider, the console filter
//! handle `/management/loggers` drives, and the samplers; the guard flushes
//! the exporters on [`TelemetryGuard::shutdown`].
//!
//! No openEHR specification governs this: our own design. Operational
//! telemetry is separate from the ATNA audit trail (`crate::system_log`).

pub mod indicators;
pub mod metrics;
pub mod samplers;

pub mod build_info;
pub mod provenance;

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;

use build_info::BuildInfo;
use ferrohealth_telemetry::section::{Filter, TelemetrySection};
use ferrohealth_telemetry::telemetry::{Console, FilterHandle, Layers, Service, Telemetry};
use opentelemetry::metrics::Meter;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{Layer as _, Registry};

/// The `service.name` of every span and metric, and the crate prefix whose
/// spans are exported.
pub const SERVICE: &str = "ferroehr";

/// The flush guard of the span-timing capture.
type FlameFlush = tracing_flame::FlushGuard<BufWriter<File>>;

/// The telemetry could not be started.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// The shared provider, the console or the subscriber could not be built.
    #[error(transparent)]
    Init(#[from] ferrohealth_telemetry::telemetry::InitError),
    /// The span-timing capture file could not be created.
    #[error("the span-timing capture file could not be created")]
    Flame(#[from] tracing_flame::Error),
    /// A subscriber is already installed in this process.
    #[error("a log subscriber is already installed")]
    Subscriber(#[from] tracing_subscriber::util::TryInitError),
}

/// Returns the service the telemetry describes: `ferroehr` at
/// `<version>+<git sha>`.
#[must_use]
pub fn service(build: &BuildInfo) -> Service {
    Service::new(SERVICE, format!("{}+{}", build.version, build.git_sha))
}

/// Builds the provider `section` asks for, installs the process subscriber on
/// stdout (with the span-timing capture when `flame_file` is set) and
/// registers FerroEHR's instruments on the shared meter.
///
/// # Errors
///
/// Returns [`TelemetryError::Init`] when the provider or the console cannot be
/// built, [`TelemetryError::Flame`] when the capture file cannot be created,
/// and [`TelemetryError::Subscriber`] when a subscriber is already installed.
pub fn init(
    section: &TelemetrySection,
    flame_file: Option<&Path>,
    build: &BuildInfo,
) -> Result<TelemetryGuard, TelemetryError> {
    let telemetry = Arc::new(Telemetry::new(section, service(build))?);
    let Layers { mut layers, filter } =
        telemetry.layers::<Registry, _>(section, &Console::detect(), std::io::stdout)?;
    let mut flame = None;
    if let Some(path) = flame_file {
        let (layer, flush) = tracing_flame::FlameLayer::with_file(path)?;
        layers.push(layer.boxed());
        flame = Some(flush);
    }
    tracing_subscriber::registry().with(layers).try_init()?;
    ferrohealth_telemetry::panic::install_hook();

    let meter = telemetry.meter(metrics::SCOPE);
    metrics::init(&meter);
    metrics::register_static_gauges(&meter, build);

    Ok(TelemetryGuard {
        telemetry,
        filter,
        boot_filter: section.filter.clone(),
        meter,
        flame,
        sampler: None,
    })
}

/// The running telemetry: the provider, the console filter and the samplers.
pub struct TelemetryGuard {
    telemetry: Arc<Telemetry>,
    filter: FilterHandle,
    boot_filter: Filter,
    meter: Meter,
    flame: Option<FlameFlush>,
    sampler: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for TelemetryGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryGuard")
            .field("telemetry", &self.telemetry)
            .field("flame_capture", &self.flame.is_some())
            .field("sampler_running", &self.sampler.is_some())
            .finish_non_exhaustive()
    }
}

impl TelemetryGuard {
    /// Returns the provider, which renders the Prometheus exposition and
    /// hands out the meter of the request log.
    #[must_use]
    pub fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }

    /// Returns the handle that replaces the console filter.
    #[must_use]
    pub fn filter(&self) -> FilterHandle {
        self.filter.clone()
    }

    /// Returns the console filter the configuration set at boot.
    #[must_use]
    pub fn boot_filter(&self) -> Filter {
        self.boot_filter.clone()
    }

    /// Starts the pool and runtime gauges over `pool`; a second call is a
    /// no-op.
    pub fn start_samplers(&mut self, pool: PgPool) {
        if self.sampler.is_none() {
            self.sampler = Some(samplers::spawn(pool, self.meter.clone()));
        }
    }

    /// Stops the samplers, flushes the span-timing capture and flushes and
    /// stops the exporters.
    pub async fn shutdown(mut self) {
        if let Some(sampler) = self.sampler.take() {
            sampler.abort();
        }
        if let Some(flame) = self.flame.take()
            && let Err(e) = flame.flush()
        {
            tracing::warn!(error = %e, "span-flame flush at shutdown failed");
        }
        let telemetry = Arc::clone(&self.telemetry);
        let flushed = tokio::task::spawn_blocking(move || telemetry.shutdown()).await;
        match flushed {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "telemetry flush at shutdown failed"),
            Err(e) => tracing::warn!(error = %e, "telemetry shutdown flush task failed"),
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(sampler) = self.sampler.take() {
            sampler.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ferrohealth_telemetry::section::{Format, TelemetrySection};
    use ferrohealth_telemetry::telemetry::{Console, Layers, Service, Telemetry};
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::{Layer as _, Registry};

    const FORGED: &str = "ok\r\n2026-08-06T00:00:00Z ERROR forged: the patient record was deleted";

    #[derive(Clone, Debug, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn contents(&self) -> String {
            let guard = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8_lossy(&guard).into_owned()
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut guard = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Writes one event carrying [`FORGED`] through FerroEHR's console in
    /// `format` and returns what it wrote.
    fn console_line(format: Format) -> String {
        let section = TelemetrySection {
            format,
            ..TelemetrySection::default()
        };
        let telemetry =
            Telemetry::new(&section, Service::new("ferroehr", "test")).expect("telemetry");
        let sink = Capture::default();
        let Layers { layers, filter: _ } = telemetry
            .layers::<Registry, _>(&section, &Console::new(false, None), sink.clone())
            .expect("layers");
        let subscriber = tracing_subscriber::registry().with(layers);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(value = %FORGED, "a client value reached a logged field: {FORGED}");
        });
        sink.contents()
    }

    #[test]
    fn a_crlf_bearing_value_cannot_forge_a_log_line() {
        let captured = console_line(Format::Pretty);
        assert_eq!(
            captured.lines().count(),
            1,
            "the event must occupy exactly one line: {captured:?}"
        );
        assert!(captured.ends_with('\n'), "{captured:?}");
        assert!(!captured.contains("ok\r\n"), "{captured:?}");
        assert!(captured.contains(r"ok\r\n"), "{captured:?}");
    }

    #[test]
    fn the_json_format_escapes_control_characters_itself() {
        let captured = console_line(Format::Json);
        assert_eq!(captured.lines().count(), 1, "{captured:?}");
        assert!(!captured.contains("ok\r\n"), "{captured:?}");
    }

    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "Result-returning test with assertions — the Book ch11 shape \
                  (testing.md adjudication)"
    )]
    fn flame_layer_writes_folded_stacks() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = assert_fs::TempDir::new()?;
        let path = tmp.path().join("tracing.folded");
        let (flame_layer, flush) = tracing_flame::FlameLayer::with_file(&path)?;
        let subscriber = Registry::default().with(flame_layer.boxed());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("flame_probe_span");
            let entered = span.enter();
            std::thread::sleep(std::time::Duration::from_millis(5));
            drop(entered);
        });
        flush.flush()?;
        let folded = std::fs::read_to_string(&path)?;
        assert!(
            folded.contains("flame_probe_span"),
            "the folded stacks must record the entered span: {folded:?}"
        );
        Ok(())
    }

    #[test]
    fn the_default_section_exports_nothing() {
        let c = crate::config::FerroEhrConfig::default();
        assert!(c.telemetry.otlp.is_none());
        assert_eq!(c.telemetry.format, Format::Auto);
        assert_eq!(
            c.telemetry.filter.as_str(),
            ferrohealth_telemetry::section::DEFAULT_FILTER
        );
        assert!(c.management.flame_file.is_none());
    }
}
