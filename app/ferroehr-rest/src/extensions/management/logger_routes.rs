// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Axum handlers for the logger-reload surface, over the console filter of
//! `ferrohealth-telemetry`.

use axum::Json;
use axum::response::{IntoResponse, Response};
use ferrohealth_telemetry::telemetry::InitError;
use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::LogFilter;

#[derive(Debug, Serialize)]
pub(super) struct LoggersView {
    filter: String,
    boot_filter: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetFilter {
    filter: String,
}

fn view(filter: &LogFilter) -> LoggersView {
    LoggersView {
        filter: filter.handle.current().unwrap_or_default(),
        boot_filter: filter.boot.as_str().to_owned(),
    }
}

pub(super) fn get(filter: &LogFilter) -> Json<LoggersView> {
    Json(view(filter))
}

pub(super) fn set(filter: &LogFilter, body: &SetFilter) -> Response {
    answer(
        filter,
        filter.handle.replace(&body.filter),
        "invalid filter",
    )
}

pub(super) fn reset(filter: &LogFilter) -> Response {
    answer(
        filter,
        filter.handle.replace(filter.boot.as_str()),
        "reset failed",
    )
}

fn answer(filter: &LogFilter, outcome: Result<(), InitError>, prefix: &str) -> Response {
    match outcome {
        Ok(()) => (StatusCode::OK, Json(view(filter))).into_response(),
        Err(e) => crate::overview::problem::Refused::blank(
            refusal_status(&e),
            refusal_message(prefix, &e),
        )
        .into_response(),
    }
}

/// The refusal text: `prefix`, the error and each cause below it.
fn refusal_message(prefix: &str, e: &InitError) -> String {
    let mut message = format!("{prefix}: {e}");
    let mut cause = std::error::Error::source(e);
    while let Some(source) = cause {
        message.push_str(": ");
        message.push_str(&source.to_string());
        cause = source.source();
    }
    message
}

fn refusal_status(e: &InitError) -> StatusCode {
    match e {
        InitError::Filter(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_telemetry::section::{Filter, TelemetrySection};
    use ferrohealth_telemetry::telemetry::{Console, Layers, Service, Telemetry};
    use tracing_subscriber::Registry;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    /// Runs `check` over a live console filter booted at `boot`; the
    /// subscriber lives as long as the check, as the process's does.
    fn with_filter(boot: &str, check: impl FnOnce(&LogFilter)) {
        let section = TelemetrySection {
            filter: Filter::new(boot).expect("boot filter"),
            ..TelemetrySection::default()
        };
        let telemetry =
            Telemetry::new(&section, Service::new("ferroehr", "test")).expect("telemetry");
        let Layers { layers, filter } = telemetry
            .layers::<Registry, _>(&section, &Console::new(false, None), std::io::sink)
            .expect("layers");
        let _subscriber = tracing_subscriber::registry().with(layers);
        check(&LogFilter {
            handle: filter,
            boot: section.filter,
        });
    }

    /// The directives as the running filter renders them back, which orders
    /// them its own way.
    fn rendered(directives: &str) -> String {
        tracing_subscriber::EnvFilter::try_new(directives)
            .expect("the directives parse")
            .to_string()
    }

    #[test]
    fn set_then_reset_round_trip() {
        with_filter("info,ferroehr=info", |filter| {
            assert_eq!(view(filter).filter, rendered("info,ferroehr=info"));
            assert_eq!(view(filter).boot_filter, "info,ferroehr=info");
            let set = set(
                filter,
                &SetFilter {
                    filter: "ferroehr=debug,sqlx=warn".to_owned(),
                },
            );
            assert_eq!(set.status(), StatusCode::OK);
            assert_eq!(view(filter).filter, rendered("ferroehr=debug,sqlx=warn"));
            assert_eq!(reset(filter).status(), StatusCode::OK);
            assert_eq!(view(filter).filter, rendered("info,ferroehr=info"));
        });
    }

    #[test]
    fn bad_filter_is_rejected_and_leaves_current_unchanged() {
        with_filter("info", |filter| {
            let refused = set(
                filter,
                &SetFilter {
                    filter: "ferroehr=!!bad".to_owned(),
                },
            );
            assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
            assert_eq!(view(filter).filter, rendered("info"));
        });
    }

    #[test]
    fn a_refused_filter_body_names_the_parse_diagnosis() {
        let parse = tracing_subscriber::EnvFilter::try_new("ferroehr=!!bad")
            .expect_err("the directive must not parse");
        let diagnosis = parse.to_string();
        let refused = InitError::from(Filter::new("ferroehr=!!bad").expect_err("refused"));
        let message = refusal_message("invalid filter", &refused);
        assert!(message.starts_with("invalid filter: "), "{message}");
        assert!(message.ends_with(&diagnosis), "{message}");
        assert_eq!(refusal_status(&refused), StatusCode::BAD_REQUEST);
    }
}
