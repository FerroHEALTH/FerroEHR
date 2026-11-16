// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SMART discovery for the hospital deployment (FerroHEALTH-backlog #34):
//! with the hospital's `machines` issuer and its Keycloak `persons` issuer
//! both trusted, the machines issuer listed first,
//! `/.well-known/smart-configuration` describes the persons issuer and never
//! the machines one, since a person's app is launched through the persons
//! issuer (SMART App Launch 2.2.0, Conformance, §Metadata). Which issuer the
//! document describes is our own design.

use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::kind::IssuerKind;
use http::StatusCode;

use crate::common;
use crate::hospital::world::AUDIENCE;
use crate::smart_http::issuer;

/// The discovery document's path.
const DISCOVERY: &str = "/ferroehr/rest/.well-known/smart-configuration";

#[tokio::test]
async fn discovery_describes_the_keycloak_persons_issuer_and_never_the_machines_one() {
    let machines = issuer().await;
    let keycloak = issuer().await;
    let mut config = common::api_config(false);
    config.auth = common::trusting(vec![
        IssuerEntry {
            kind: Some(IssuerKind::Machines),
            ..common::entry(&machines.uri(), AUDIENCE)
        },
        common::entry(&keycloak.uri(), AUDIENCE),
    ]);
    config.auth.smart.discovery = true;
    config.public_root = Some(
        ferroehr::config::server::PublicRoot::new("https://cdr.sint-example.example", "")
            .expect("a public root"),
    );
    let (_db, app) = common::app(config).await;
    let (status, doc) = common::send_json(
        &app,
        common::req("GET", DISCOVERY)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let persons = keycloak.uri();
    assert_eq!(doc["issuer"], persons.as_str(), "{doc}");
    assert_eq!(
        doc["authorization_endpoint"],
        format!("{persons}/authorize"),
        "{doc}"
    );
    assert_eq!(doc["token_endpoint"], format!("{persons}/token"), "{doc}");
    assert!(
        !doc.to_string().contains(&machines.uri()),
        "the machines issuer: {doc}"
    );
}
