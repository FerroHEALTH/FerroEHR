// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Trace-shape test: drive a request through the root-span
//! middleware with a capturing subscriber and assert the span is named/attributed
//! by **route template** and carries **no PHI** (the `ehr_id` never appears in
//! any span field name or value).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::routing::get;
use axum::{Router, middleware};
use http::Request;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

/// One captured span: its metadata name + recorded field values.
#[derive(Debug, Default, Clone)]
struct Captured {
    name: String,
    fields: BTreeMap<String, String>,
}

/// A tracing layer that records every span's name + fields into a shared store.
#[derive(Clone, Default)]
struct CaptureLayer {
    spans: Arc<Mutex<BTreeMap<u64, Captured>>>,
}

impl<S> Layer<S> for CaptureLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        self.spans.lock().expect("lock").insert(
            id.into_u64(),
            Captured {
                name: attrs.metadata().name().to_owned(),
                fields: visitor.0,
            },
        );
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);
        if let Some(captured) = self.spans.lock().expect("lock").get_mut(&id.into_u64()) {
            captured.fields.extend(visitor.0);
        }
    }
}

/// Collects field name → rendered value.
#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

#[tokio::test]
async fn root_span_is_route_template_and_phi_free() {
    let spans = Arc::new(Mutex::new(BTreeMap::<u64, Captured>::new()));
    let layer = CaptureLayer {
        spans: Arc::clone(&spans),
    };
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));

    // A templated route carrying an id, wrapped in the root-span middleware.
    let app = Router::new()
        .route("/ehr/{ehr_id}/composition", get(|| async { "ok" }))
        .layer(middleware::from_fn(
            ferroehr_rest::extensions::management::http_metrics::root_span,
        ));

    let ehr_id = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let req = Request::builder()
        .method("POST")
        .uri(format!("/ehr/{ehr_id}/composition"))
        .body(Body::empty())
        .expect("request");
    // (POST has no handler on this route → 405, but the root span is still made.)
    let _response = app.oneshot(req).await.expect("response");

    let captured = spans.lock().expect("lock");
    let root = captured
        .values()
        .find(|c| c.name == "http_request")
        .expect("root span captured");

    // Named/attributed by route TEMPLATE, not the raw path.
    assert_eq!(
        root.fields.get("http.route").map(String::as_str),
        Some("/ehr/{ehr_id}/composition"),
        "http.route must be the template"
    );
    assert_eq!(
        root.fields.get("otel.name").map(String::as_str),
        Some("POST /ehr/{ehr_id}/composition"),
        "otel.name must be method + template"
    );

    // PHI denylist: the ehr id must not appear in ANY field name or value, and
    // no id-bearing field (url.path / ehr_id / subject_id) may be present.
    for (name, value) in &root.fields {
        assert!(
            !name.contains("url.path") && name != "ehr_id" && name != "subject_id",
            "PHI-bearing field {name:?} present in span"
        );
        assert!(
            !value.contains(ehr_id),
            "the ehr_id leaked into span field {name:?} = {value:?}"
        );
    }
}
