// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The issuer the SMART discovery document describes, chosen by
//! `ferrohealth-smart`'s `describe`: `[auth] described_issuer`, else the one
//! issuer that serves a user launch and whose
//! metadata FerroEHR has, never a `machines` issuer and never by the order of
//! the `[[auth.issuer]]` tables. SMART App Launch 2.2.0, Conformance,
//! §Metadata requires the `authorization_endpoint` a user launch starts at;
//! which of several issuers is described is our own design.

use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::kind::IssuerKind;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

/// An authorization server publishing its RFC 8414 metadata and the committed
/// key set.
async fn issuer_server() -> MockServer {
    let server = MockServer::start().await;
    let issuer = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
            "../../../../../../../docs/conformance/party/smart/jwks.json"
        )))
        .mount(&server)
        .await;
    server
}

/// The `[[auth.issuer]]` table of a `kind` issuer at `issuer`, its metadata
/// read from it.
fn read(issuer: &str, kind: IssuerKind) -> IssuerEntry {
    IssuerEntry {
        kind: Some(kind),
        ..test_support::entry(issuer)
    }
}

/// The `[[auth.issuer]]` table of a `persons` issuer whose key set is a file,
/// so FerroEHR has none of its metadata.
fn file_only(issuer: &str) -> IssuerEntry {
    IssuerEntry {
        jwks_file: test_support::jwks_file().display().to_string(),
        ..test_support::entry(issuer)
    }
}

/// The authenticator trusting `entries`, the document served when `served`,
/// describing `described` when set.
async fn authenticator(
    entries: Vec<IssuerEntry>,
    served: bool,
    described: Option<&str>,
) -> Result<Arc<Authenticator>, String> {
    let mut config = AppConfig {
        auth: ferroehr::config::auth_section::AuthConfig {
            shared: Box::new(AuthSection {
                issuer: entries,
                described_issuer: described.map(|issuer| {
                    ferrohealth_secret::secret_url::SecretUrl::new(issuer.to_owned())
                }),
                ..AuthSection::default()
            }),
            ..ferroehr::config::auth_section::AuthConfig::default()
        },
        ..AppConfig::default()
    };
    config.auth.smart.discovery = served;
    Authenticator::new(&config, None, memory(), test_support::standing()).await
}

/// The `token_endpoint` and `authorization_endpoint` of the document.
fn endpoints(authenticator: &Authenticator) -> (Value, Value) {
    let document = authenticator
        .smart_configuration(&SmartOptions::default())
        .expect("a document");
    (
        document["authorization_endpoint"].clone(),
        document["token_endpoint"].clone(),
    )
}

/// The quickstart's shape: a `persons` issuer read from a key-set file and a
/// `machines` issuer whose metadata is read. Only the `machines` issuer has
/// metadata, and it signs no user token, so no issuer can be described.
#[tokio::test]
async fn a_machines_issuer_is_never_described() {
    let machines = issuer_server().await;
    let Err(message) = authenticator(
        vec![
            file_only("https://basic-users.ferroehr.invalid"),
            read(&machines.uri(), IssuerKind::Machines),
        ],
        true,
        None,
    )
    .await
    else {
        panic!("a document only a machines issuer could describe is refused");
    };
    assert!(message.contains("can describe no issuer"), "{message}");
    assert!(message.contains("machines"), "{message}");
}

/// One eligible issuer is described whichever table comes first.
#[tokio::test]
async fn the_one_eligible_issuer_is_described_in_any_table_order() {
    let machines = issuer_server().await;
    let persons = issuer_server().await;
    let expected = (
        json!(format!("{}/authorize", persons.uri())),
        json!(format!("{}/token", persons.uri())),
    );
    let tables = [
        read(&machines.uri(), IssuerKind::Machines),
        read(&persons.uri(), IssuerKind::Persons),
        file_only("https://basic-users.ferroehr.invalid"),
    ];
    let mut reversed = tables.to_vec();
    reversed.reverse();
    for entries in [tables.to_vec(), reversed] {
        let authenticator = authenticator(entries, true, None)
            .await
            .expect("one eligible issuer boots");
        assert_eq!(endpoints(&authenticator), expected);
    }
}

/// A `patients` issuer serves a patient standalone launch, so it is eligible.
#[tokio::test]
async fn a_patients_issuer_is_eligible() {
    let patients = issuer_server().await;
    let authenticator = authenticator(
        vec![
            file_only("https://basic-users.ferroehr.invalid"),
            read(&patients.uri(), IssuerKind::Patients),
        ],
        true,
        None,
    )
    .await
    .expect("a patients issuer is described");
    assert_eq!(
        endpoints(&authenticator).1,
        json!(format!("{}/token", patients.uri()))
    );
}

/// Several eligible issuers and no `described_issuer` refuse the start,
/// naming each issuer and the key.
#[tokio::test]
async fn several_eligible_issuers_without_the_key_are_refused() {
    let persons = issuer_server().await;
    let patients = issuer_server().await;
    let Err(message) = authenticator(
        vec![
            read(&persons.uri(), IssuerKind::Persons),
            read(&patients.uri(), IssuerKind::Patients),
        ],
        true,
        None,
    )
    .await
    else {
        panic!("several eligible issuers without described_issuer are refused");
    };
    assert!(message.contains("auth.described_issuer"), "{message}");
    assert!(message.contains(&persons.uri()), "{message}");
    assert!(message.contains(&patients.uri()), "{message}");
}

/// `described_issuer` picks one of several eligible issuers.
#[tokio::test]
async fn the_named_issuer_is_described() {
    let persons = issuer_server().await;
    let patients = issuer_server().await;
    let entries = vec![
        read(&persons.uri(), IssuerKind::Persons),
        read(&patients.uri(), IssuerKind::Patients),
    ];
    for named in [persons.uri(), patients.uri()] {
        let authenticator = authenticator(entries.clone(), true, Some(&named))
            .await
            .expect("the named issuer boots");
        assert_eq!(
            endpoints(&authenticator),
            (
                json!(format!("{named}/authorize")),
                json!(format!("{named}/token"))
            )
        );
    }
}

/// `described_issuer` naming a `machines` issuer is refused, though its
/// metadata is read.
#[tokio::test]
async fn a_named_machines_issuer_is_refused() {
    let machines = issuer_server().await;
    let persons = issuer_server().await;
    let Err(message) = authenticator(
        vec![
            read(&machines.uri(), IssuerKind::Machines),
            read(&persons.uri(), IssuerKind::Persons),
        ],
        true,
        Some(&machines.uri()),
    )
    .await
    else {
        panic!("a named machines issuer is refused");
    };
    assert!(message.contains("auth.described_issuer"), "{message}");
    assert!(message.contains("machines issuer"), "{message}");
}

/// `described_issuer` naming no `[auth]` issuer is refused.
#[tokio::test]
async fn a_named_unknown_issuer_is_refused() {
    let persons = issuer_server().await;
    let Err(message) = authenticator(
        vec![read(&persons.uri(), IssuerKind::Persons)],
        true,
        Some("https://elsewhere.example"),
    )
    .await
    else {
        panic!("a named unknown issuer is refused");
    };
    assert!(
        message.contains(
            "described_issuer names https://elsewhere.example, which is no trusted issuer"
        ),
        "{message}"
    );
}

/// `described_issuer` naming an issuer whose metadata FerroEHR does not have
/// is refused.
#[tokio::test]
async fn a_named_issuer_without_metadata_is_refused() {
    let persons = issuer_server().await;
    let Err(message) = authenticator(
        vec![
            read(&persons.uri(), IssuerKind::Persons),
            file_only("https://basic-users.ferroehr.invalid"),
        ],
        true,
        Some("https://basic-users.ferroehr.invalid"),
    )
    .await
    else {
        panic!("a named issuer without metadata is refused");
    };
    assert!(message.contains("auth.described_issuer"), "{message}");
    assert!(message.contains("issuer_metadata_file"), "{message}");
}

/// With the document not served, several eligible issuers refuse nothing and
/// no document is built.
#[tokio::test]
async fn an_unserved_document_refuses_nothing() {
    let persons = issuer_server().await;
    let patients = issuer_server().await;
    let authenticator = authenticator(
        vec![
            read(&persons.uri(), IssuerKind::Persons),
            read(&patients.uri(), IssuerKind::Patients),
        ],
        false,
        None,
    )
    .await
    .expect("discovery off boots");
    assert!(
        authenticator
            .smart_configuration(&SmartOptions::default())
            .is_none()
    );
}
