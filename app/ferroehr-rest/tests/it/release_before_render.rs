// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A composition read gives its connections back before it renders the
//! response (#6255): the render of FLAT, STRUCTURED or canonical XML parses the
//! stored body and serialises it, work that holds no connection.
//!
//! The order is read from the `tracing` events of the request: the release
//! (`ferroehr::db::acquire::RELEASED`) and the representation changes the
//! render records ([`openehr_its::conversion::TARGET`]). No openEHR spec
//! governs connection handling: our own design.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use http::{Request, StatusCode, header};
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use crate::access_evaluator_http::{app, credential, ehr_with_composition};
use crate::common::{self, BASE};

/// The events a request recorded, in order: `released` or `conversion`.
#[derive(Clone, Default)]
struct Order(Arc<Mutex<Vec<&'static str>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Order {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();
        let kind = if target == ferroehr::db::acquire::RELEASED {
            "released"
        } else if target == openehr_its::conversion::TARGET {
            "conversion"
        } else {
            return;
        };
        self.0.lock().unwrap().push(kind);
    }
}

#[tokio::test]
async fn a_composition_read_gives_its_connections_back_before_the_render() {
    let order = Order::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(order.clone()));
    let (_db, _pool, _svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    for accept in [
        "application/openehr.wt.flat+json",
        "application/openehr.wt.structured+json",
        "application/xml",
    ] {
        order.0.lock().unwrap().clear();
        let (status, _, body) = common::send(
            &app,
            Request::get(format!("{BASE}/ehr/{ehr}/composition/{vo}"))
                .header(header::AUTHORIZATION, credential("nurse"))
                .header(header::ACCEPT, accept)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{accept}: {body}");
        let seen = order.0.lock().unwrap().clone();
        let released = seen.iter().position(|kind| *kind == "released");
        let rendered = seen.iter().position(|kind| *kind == "conversion");
        assert!(
            rendered.is_some(),
            "{accept}: the render records its conversions: {seen:?}"
        );
        assert!(
            released < rendered && released.is_some(),
            "{accept}: the connections go back before the render: {seen:?}"
        );
    }
}
