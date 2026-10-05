// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The on-demand CPU flamegraph body: pprof sampling + inferno SVG render.
//!
//! NOTE: no openEHR spec governs this — our own operational surface (the
//! `/management` extension). The profiler is the `pprof` crate's `SIGPROF`
//! sampler (<https://docs.rs/pprof/latest/pprof/>): one guard samples the
//! whole process for the requested window, then the report renders straight
//! to a flamegraph SVG. Exactly one sample window may run at a time — the
//! profiler is process-global state, so a second concurrent request is a
//! `409`, never a queued wait (an operator retries in seconds; a silent queue
//! would stack sample windows end-to-end and hold request slots).

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::Mutex;

use ferroehr::config::management::ProfilingConfig;
use openehr_its::rest::runtime::ApiError;

/// The one process-wide profiling permit.
///
/// Cheap to clone (an `Arc` handle); `try_acquire` either takes the permit
/// or reports the running sample window.
#[derive(Debug, Clone, Default)]
pub struct ProfilerSlot(Arc<Mutex<()>>);

impl ProfilerSlot {
    /// Take the permit, or `Err` when a sample window is already running.
    fn try_acquire(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, ApiError> {
        Arc::clone(&self.0).try_lock_owned().map_err(|_busy| {
            ApiError::Conflict(
                "a profiling sample window is already running; retry when it completes".to_owned(),
            )
        })
    }
}

/// Query parameters of `GET /management/flamegraph`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlamegraphParams {
    /// The sample window in seconds (default 10, capped by
    /// `management.profiling.max_seconds`).
    pub seconds: Option<u16>,
    /// The sampling frequency in Hz (default 99, capped by
    /// `management.profiling.max_frequency`).
    pub frequency: Option<i32>,
}

/// The validated sample request: window + frequency, both inside the
/// configured caps.
#[derive(Debug, Clone, Copy)]
struct SamplePlan {
    seconds: u16,
    frequency: i32,
}

/// Validate `params` against the configured caps. A request beyond a cap is a
/// `400` naming the cap — never silently clamped (the caller must know what
/// was actually sampled).
fn plan(params: &FlamegraphParams, limits: ProfilingConfig) -> Result<SamplePlan, ApiError> {
    let seconds = params.seconds.unwrap_or_else(|| 10.min(limits.max_seconds));
    if seconds == 0 || seconds > limits.max_seconds {
        return Err(ApiError::BadRequest(format!(
            "seconds must be between 1 and {} (management.profiling.max_seconds)",
            limits.max_seconds
        )));
    }
    let frequency = params
        .frequency
        .unwrap_or_else(|| 99.min(limits.max_frequency));
    if frequency < 1 || frequency > limits.max_frequency {
        return Err(ApiError::BadRequest(format!(
            "frequency must be between 1 and {} Hz (management.profiling.max_frequency)",
            limits.max_frequency
        )));
    }
    Ok(SamplePlan { seconds, frequency })
}

/// Sample the process for the requested window and render the flamegraph SVG.
///
/// The whole session — start the guard, sit out the window, build the report,
/// render — runs on one blocking-pool thread (`spawn_blocking`): the pprof
/// guard is not `Send`, and parking a blocking thread for a bounded,
/// operator-triggered window is the intended use of the blocking pool
/// (<https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html>).
///
/// # Errors
///
/// * [`ApiError::BadRequest`] — a parameter outside the configured caps.
/// * [`ApiError::Conflict`] — a sample window is already running.
/// * [`ApiError::Internal`] — the profiler failed to start, sample, or render;
///   the diagnostic goes to the trace record, the body stays curated.
pub async fn sample_flamegraph_svg(
    slot: &ProfilerSlot,
    limits: ProfilingConfig,
    params: &FlamegraphParams,
) -> Result<Vec<u8>, ApiError> {
    let plan = plan(params, limits)?;
    let permit = slot.try_acquire()?;
    tracing::info!(
        seconds = plan.seconds,
        frequency = plan.frequency,
        "profiling: CPU sample window started"
    );
    let rendered = tokio::task::spawn_blocking(move || {
        // The permit lives on the blocking thread for the whole session, so a
        // concurrent request conflicts for exactly as long as sampling runs.
        let held_permit = permit;
        let svg = run_sample(plan);
        drop(held_permit);
        svg
    })
    .await;
    match rendered {
        Ok(Ok(svg)) => Ok(svg),
        Ok(Err(error)) => {
            tracing::warn!(%error, "profiling: sample window failed");
            Err(ApiError::Internal("profiling failed".to_owned()))
        }
        Err(join_error) => {
            tracing::warn!(%join_error, "profiling: sampling task did not complete");
            Err(ApiError::Internal("profiling failed".to_owned()))
        }
    }
}

/// The collapsed-stack lines pprof's own `flamegraph` feature folds a report
/// into (`thread;outer;…;leaf count`), reproduced from pprof 0.15's
/// `Report::flamegraph_with_options` over the report's public fields.
///
/// NOTE: #2406 — pprof's `flamegraph` feature pins `inferno ^0.11` (quick-xml
/// 0.26, RUSTSEC-2026-0194/0195); folding here and rendering through the
/// direct `inferno` 0.12 dependency keeps that pair out of the graph.
fn folded_lines(report: &pprof::Report) -> Vec<String> {
    report
        .data
        .iter()
        .map(|(frames, count)| {
            let mut segments = vec![frames.thread_name_or_id()];
            for frame in frames.frames.iter().rev() {
                for symbol in frame.iter().rev() {
                    segments.push(symbol.to_string());
                }
            }
            format!("{} {count}", segments.join(";"))
        })
        .collect()
}

/// The synchronous profiling session: guard → sleep out the window → report →
/// SVG bytes. Runs entirely on one blocking-pool thread.
fn run_sample(plan: SamplePlan) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(plan.frequency)
        // Frames inside these libraries are unwind hazards pprof's own
        // documentation blocklists (signal handlers interrupting non-reentrant
        // code); the application frames we profile are unaffected.
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()?;
    std::thread::sleep(Duration::from_secs(u64::from(plan.seconds)));
    let report = guard.report().build()?;
    let mut svg = Vec::new();
    let lines = folded_lines(&report);
    if !lines.is_empty() {
        inferno::flamegraph::from_lines(
            &mut inferno::flamegraph::Options::default(),
            lines.iter().map(String::as_str),
            &mut svg,
        )?;
    }
    // A window that caught ZERO samples (an idle process) renders nothing —
    // pprof/inferno write no bytes for an empty frame set. Serving 0 bytes as
    // `image/svg+xml` would be a lie; answer with a well-formed SVG that says
    // what happened instead. (Our own operational surface — no openEHR spec
    // governs this.)
    if svg.is_empty() {
        svg.extend_from_slice(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="600" height="40"><text x="10" y="25" font-family="monospace">no samples captured in the window (process idle)</text></svg>"#,
        );
    }
    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ProfilingConfig {
        ProfilingConfig::default()
    }

    #[test]
    fn defaults_fit_inside_caps() {
        let p = plan(&FlamegraphParams::default(), limits()).expect("defaults must plan");
        assert_eq!(p.seconds, 10);
        assert_eq!(p.frequency, 99);
    }

    #[test]
    fn defaults_shrink_to_a_tighter_cap() {
        let tight = ProfilingConfig {
            max_seconds: 5,
            max_frequency: 50,
        };
        let p = plan(&FlamegraphParams::default(), tight).expect("defaults must plan");
        assert_eq!(p.seconds, 5);
        assert_eq!(p.frequency, 50);
    }

    #[test]
    fn over_cap_is_refused_not_clamped() {
        let over_seconds = FlamegraphParams {
            seconds: Some(31),
            frequency: None,
        };
        assert!(matches!(
            plan(&over_seconds, limits()),
            Err(ApiError::BadRequest(_))
        ));
        let over_frequency = FlamegraphParams {
            seconds: None,
            frequency: Some(1000),
        };
        assert!(matches!(
            plan(&over_frequency, limits()),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn zero_and_negative_are_refused() {
        let zero_seconds = FlamegraphParams {
            seconds: Some(0),
            frequency: None,
        };
        assert!(matches!(
            plan(&zero_seconds, limits()),
            Err(ApiError::BadRequest(_))
        ));
        let negative_frequency = FlamegraphParams {
            seconds: None,
            frequency: Some(-1),
        };
        assert!(matches!(
            plan(&negative_frequency, limits()),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn second_acquire_conflicts_while_held() {
        let slot = ProfilerSlot::default();
        let held = slot.try_acquire().expect("first acquire");
        assert!(matches!(slot.try_acquire(), Err(ApiError::Conflict(_))));
        drop(held);
        assert!(slot.try_acquire().is_ok());
    }
}
