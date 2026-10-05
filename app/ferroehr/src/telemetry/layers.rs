// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Subscriber assembly: a reloadable `EnvFilter` + a stdout
//! `fmt` layer (json/pretty/auto) + an optional `OTel` export layer + an
//! optional span-flamegraph capture layer.
//!
//! `tracing` is the single instrumentation API; this module wires the
//! consumers. The `EnvFilter` sits behind a `reload::Layer` so
//! `/management/loggers` can swap it live; the handle is captured behind
//! type-erased closures into a [`LogReload`]. The `OTel` layer is added **only**
//! when a tracer is supplied (endpoint configured), and the `tracing-flame`
//! layer only when `telemetry.flame_file` is set — absent, neither is in the
//! stack at all (zero overhead).

use std::fs::File;
use std::io::BufWriter;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

use crate::telemetry::log_reload::{ApplyFilter, FilterReloadError, LogReload, ReadFilter};
use opentelemetry_sdk::trace::SdkTracer;
use tracing_subscriber::layer::{Layered, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, reload};

use super::TelemetryError;

/// The subscriber `S` seen by the data layers: the [`Registry`] with the
/// reloadable global `EnvFilter` layer composed onto it. Naming it lets the
/// `fmt`/`OTel` layers be type-erased into a homogeneous `Vec` and added in one
/// `.with()` (sequential `.with()` on boxed `dyn` layers cannot compose).
type Base = Layered<reload::Layer<EnvFilter, Registry>, Registry>;

/// A type-erased data layer over [`Base`].
type BoxedLayer = Box<dyn Layer<Base> + Send + Sync>;

/// The flush handle of the optional span-flamegraph layer (folded stacks to a
/// buffered file); flushing on shutdown is what makes a truncated diagnostic
/// session still render.
pub(super) type FlameFlush = tracing_flame::FlushGuard<BufWriter<File>>;

/// Build the global subscriber and install it. Returns the [`LogReload`] handle
/// onto the reloadable filter plus the span-flamegraph flush guard when
/// `flame_file` is set. Adds the `OTel` layer iff `otel_tracer` is `Some`.
///
/// # Errors
/// Returns [`TelemetryError::Subscriber`] if a global subscriber is already
/// set, or [`TelemetryError::Flame`] if the `flame_file` cannot be created.
pub(super) fn init_subscriber(
    format: super::config::LogFormat,
    boot_filter: &str,
    otel_tracer: Option<SdkTracer>,
    flame_file: Option<&Path>,
) -> Result<(LogReload, Option<FlameFlush>), TelemetryError> {
    // Reloadable global filter. A bad boot directive falls back to `info` so the
    // process still starts (and logs the problem).
    let env_filter = EnvFilter::try_new(boot_filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);

    // Data layers, type-erased over `Base` and added in a single `.with(Vec)`.
    let mut layers: Vec<BoxedLayer> = vec![fmt_layer(format)];
    if let Some(tracer) = otel_tracer {
        layers.push(tracing_opentelemetry::layer().with_tracer(tracer).boxed());
    }
    let mut flame_flush = None;
    if let Some(path) = flame_file {
        let (flame_layer, flush) = tracing_flame::FlameLayer::with_file(path)?;
        layers.push(flame_layer.boxed());
        flame_flush = Some(flush);
    }

    Registry::default()
        .with(filter_layer)
        .with(layers)
        .try_init()?;

    // Type-erase the reload handle (its `S` is `Registry`) behind read/apply
    // closures for `/management/loggers`.
    let read_handle = reload::Handle::clone(&reload_handle);
    let read: ReadFilter = Arc::new(move || {
        read_handle
            .with_current(ToString::to_string)
            .unwrap_or_default()
    });

    let apply: ApplyFilter = Arc::new(move |directives: &str| -> Result<(), FilterReloadError> {
        let new_filter = EnvFilter::try_new(directives)?;
        reload_handle.reload(new_filter)?;
        Ok(())
    });

    Ok((LogReload::new(boot_filter, read, apply), flame_flush))
}

/// The stdout `fmt` layer for the chosen profile, boxed so the subscriber type
/// is uniform. JSON carries `trace_id`/`span_id`/`request_id` via the current
/// span's fields for trace↔log correlation.
fn fmt_layer(format: super::config::LogFormat) -> BoxedLayer {
    use super::config::LogFormat;
    use super::config::ResolvedLogFormat;
    let is_terminal = std::io::stdout().is_terminal();
    if format.resolve(is_terminal) == ResolvedLogFormat::Json {
        fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .boxed()
    } else {
        // An explicit `pretty` is a human asking for the colored dev output
        // (e.g. `docker compose` logs, where stdout is a pipe, not a TTY) —
        // force ANSI on. Only `auto`-selected pretty keeps TTY detection.
        let ansi = matches!(format, LogFormat::Pretty) || is_terminal;
        // The text format renders `Display` fields and interpolated messages
        // verbatim, so its writer neutralises CR/LF: a value cannot forge a
        // record (OWASP Logging Cheat Sheet §Log Injection —
        // `crate::telemetry::log_sanitize`). JSON escapes them itself.
        fmt::layer()
            .with_target(true)
            .with_ansi(ansi)
            .with_writer(super::log_sanitize::LineSafe::new(std::io::stdout))
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flame layer records entered spans as folded stack lines the
    /// flush guard writes out — exercised on a SCOPED subscriber (never the
    /// global one; tests share the process).
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
        let subscriber = Registry::default().with(flame_layer);
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
}
