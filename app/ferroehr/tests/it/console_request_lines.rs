// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The console writes the line of every request, a query before it or not
//! (#5696).

use std::process::Command;

use ferrohealth_telemetry::section::{Filter, Format, TelemetrySection};

/// Names the console filter of the child half, and asks the child to run.
const CHILD: &str = "FERROEHR_CONSOLE_LINES_FILTER";

/// One request line, written from one callsite every time.
fn request_line(at: &str) {
    tracing::info!(target: "ferrohealth_telemetry::request", at, "request");
}

/// The child half: FerroEHR's real process subscriber, a request line, a
/// query, and a request line again; the first comes before any query, the
/// second after the harness prepared its database.
#[test]
#[ignore = "the child half of a_request_line_after_a_query_is_written, run in its own process"]
#[expect(
    clippy::disallowed_methods,
    reason = "the parent hands the child its filter in the environment of the process it \
              spawns; the child reads no FerroEHR configuration"
)]
fn console_child() {
    let Ok(filter) = std::env::var(CHILD) else {
        return;
    };
    let section = TelemetrySection {
        format: Format::Pretty,
        filter: Filter::new(filter).expect("the filter parses"),
        ..TelemetrySection::default()
    };
    let _telemetry = ferroehr::telemetry::init(
        &section,
        None,
        &ferroehr::telemetry::build_info::BuildInfo::current(),
    )
    .expect("the process subscriber installs");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    request_line("at-start");
    runtime.block_on(async {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        request_line("before-the-query");
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .expect("the query runs");
        request_line("after-the-query");
    });
}

/// The request line after a query is written under the default filter and
/// under one that lets a target through at `debug`, as the hospital deploy
/// probe sets for `ferrohealth_smart`.
#[test]
fn a_request_line_after_a_query_is_written() {
    let exe = std::env::current_exe().expect("the test binary");
    for filter in ["info", "info,ferrohealth_smart=debug"] {
        let output = Command::new(&exe)
            .args([
                "console_request_lines::console_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, filter)
            .env("NO_COLOR", "1")
            .output()
            .expect("the child runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "filter {filter:?}: the child failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for at in ["at-start", "before-the-query", "after-the-query"] {
            assert!(
                stdout.contains(&format!("at=\"{at}\"")),
                "filter {filter:?}: no request line {at}:\n{stdout}"
            );
        }
    }
}
