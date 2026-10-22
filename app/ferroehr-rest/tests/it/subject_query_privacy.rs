// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The subject identifier of `GET /ehr?subject_id=…` stays out of everything
//! FerroEHR emits (#3906).
//!
//! ITS-REST 1.1.0 looks an EHR up by subject with the identifier in the query
//! string (ITS-REST `specifications/operations/ehr_get_by_subject.yaml`). The test
//! drives the assembled router under a subscriber that records every span
//! field and every event at every level, and asserts the identifier appears in
//! none of them and in no error body. No openEHR spec governs logging; keeping
//! the identifier out of it is our own design.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::common::{self, BASE};

/// A synthetic BSN (it passes the eleven-test and names no one). The clinical
/// side refuses it as stored content, so only the lookup of an unknown subject
/// carries it.
const BSN: &str = "111222333";
/// The pseudonym the stored EHR names its subject by.
const PSEUDONYM: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
const NAMESPACE: &str = "test-bsn";

/// Every span name, span field and event field the subscriber saw, rendered.
#[derive(Clone, Default)]
struct Everything {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Everything {
    fn push(&self, line: String) {
        self.lines.lock().expect("lock").push(line);
    }
}

impl<S> Layer<S> for Everything
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        let mut visitor = Render(format!("span {}", attrs.metadata().name()));
        attrs.record(&mut visitor);
        self.push(visitor.0);
    }

    fn on_record(&self, _id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut visitor = Render("record".to_owned());
        values.record(&mut visitor);
        self.push(visitor.0);
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = Render(format!("event {}", event.metadata().target()));
        event.record(&mut visitor);
        self.push(visitor.0);
    }
}

/// Appends `name=value` for every field.
struct Render(String);

impl Visit for Render {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        write!(self.0, " {}={value:?}", field.name()).expect("a String takes any write");
    }
}

fn lookup(subject: &str) -> Request<Body> {
    common::get(&format!(
        "{BASE}/ehr?subject_id={subject}&subject_namespace={NAMESPACE}"
    ))
}

/// Neither a lookup that finds the EHR nor one that finds none puts the
/// subject id into a log line, a span attribute or the error body.
#[tokio::test]
async fn a_subject_lookup_never_records_the_subject_id() {
    let everything = Everything::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(everything.clone()));

    let (_db, app) = common::test_router().await;
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": NAMESPACE,
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": PSEUDONYM, "scheme": "pseudonym" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    });
    let (created, body) = common::send_body(
        &app,
        common::post_json(&format!("{BASE}/ehr"), &status.to_string()),
    )
    .await;
    assert_eq!(created, StatusCode::CREATED, "the EHR is created: {body}");
    // The commit carries the identifier in its body by design; only what the
    // lookups emit is under test.
    everything.lines.lock().expect("lock").clear();

    let (found, _) = common::send_body(&app, lookup(PSEUDONYM)).await;
    assert_eq!(found, StatusCode::OK, "the subject's EHR is found");

    let (missing, body) = common::send_body(&app, lookup(BSN)).await;
    assert_eq!(
        missing,
        StatusCode::NOT_FOUND,
        "an unknown subject has no EHR"
    );
    assert!(
        !body.contains(BSN),
        "the 404 body names the subject id: {body}"
    );

    let lines = everything.lines.lock().expect("lock");
    assert!(
        lines.iter().any(|line| line.starts_with("span ")),
        "the subscriber saw the request spans"
    );
    for line in lines.iter() {
        assert!(
            !line.contains(BSN) && !line.contains(PSEUDONYM),
            "a subject id reached the trace output: {line}"
        );
    }
}
