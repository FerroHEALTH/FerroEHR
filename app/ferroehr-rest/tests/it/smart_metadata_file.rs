// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[auth.smart] issuer_metadata_file` beside more than one `[auth]` issuer: the
//! metadata belongs to the issuer it names (RFC 8414 §3.3), wherever that
//! issuer stands in the list, and the SMART discovery document describes it
//! (SMART App Launch 2.2.0, Conformance, §Metadata).
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;

use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferrohealth_smart::config::IssuerEntry;

use crate::common;

const DISCOVERY: &str = "/ferroehr/rest/.well-known/smart-configuration";
/// The issuer the committed CNF metadata document names.
const CNF_ISSUER: &str = "https://as.cnf.test";
/// An issuer whose key set is a file and whose metadata nothing supplies, as
/// the Basic users of the shipped configurations bind under.
const BASIC_ISSUER: &str = "https://basic-users.ferroehr.invalid";

/// The committed CNF metadata document, written once to a file.
fn metadata_file() -> String {
    static FILE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::temp_dir().join(format!(
            "ferroehr-rest-it-issuer-metadata-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            include_str!("../../../../docs/conformance/party/smart/issuer-metadata.json"),
        )
        .expect("write the metadata document");
        path.display().to_string()
    })
    .clone()
}

/// A `persons` issuer at `issuer` for `audience` reading the committed key set.
fn file_issuer(issuer: &str, audience: &str) -> IssuerEntry {
    IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(issuer, audience)
    }
}

/// The configuration trusting `entries`, with the CNF metadata file and the
/// discovery document served.
fn config(entries: Vec<IssuerEntry>) -> AppConfig {
    let mut cfg = AppConfig {
        auth: common::trusting(entries),
        ..common::api_config(false)
    };
    cfg.auth.smart.discovery = true;
    cfg.auth.smart.issuer_metadata_file = Some(metadata_file());
    cfg.auth.smart.public_base_url = Some("https://cdr.example.com".to_owned());
    cfg
}

#[tokio::test]
async fn the_metadata_file_describes_the_issuer_it_names_behind_another() {
    let cfg = config(vec![
        file_issuer(BASIC_ISSUER, "ferroehr-basic-users"),
        file_issuer(CNF_ISSUER, "ferroehr"),
    ]);
    let (_db, app) = common::app(cfg).await;
    let req = Request::builder()
        .method("GET")
        .uri(DISCOVERY)
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = common::send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let doc: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        doc["authorization_endpoint"], "https://as.cnf.test/authorize",
        "{doc}"
    );
    assert_eq!(doc["token_endpoint"], "https://as.cnf.test/token", "{doc}");
}

#[tokio::test]
async fn a_metadata_file_naming_no_issuer_refuses_the_boot() {
    let cfg = config(vec![file_issuer(BASIC_ISSUER, "ferroehr-basic-users")]);
    let error = Authenticator::new(
        &cfg,
        None,
        None,
        std::sync::Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await
    .expect_err("metadata naming no [auth] issuer is refused");
    assert!(
        error.contains("names the issuer https://as.cnf.test, which is no [auth] issuer"),
        "{error}"
    );
}
