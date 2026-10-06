// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The deployment report `ferroehr report` writes: every part named in the
//! manifest, no credential, no URL `userinfo` and no patient identifier in the
//! document, and an unreachable database named with its reason.
//!
//! No openEHR spec governs the report — our own design; Regulation (EU)
//! 2025/327 (`docs/law/eu/ehds/text.html` Art. 44(5)) is the duty it serves.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              helpers; a failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use std::collections::BTreeMap;

use assert_fs::prelude::*;
use ferroehr::config::FerroEhrConfig;
use ferroehr::config::auth::OidcConfig;
use ferroehr::config::secret::{Secret, SecretUrl};
use ferroehr::db::domain::DomainPools;
use ferroehr::manufacturer::MANUFACTURER;
use ferroehr::report::{ManifestEntry, PartStatus, Report};
use ferroehr::service::FerroEhrService;
use ferroehr::service::terminology::config::TerminologyOauth2Config;

/// Each sentinel is unique, so a leak names the setting it came from.
const DB_PW: &str = "DB_PW_SENTINEL_5c1e";
const MIGRATE_PW: &str = "MIGRATE_PW_SENTINEL_9d2a";
const PARTY_PW: &str = "PARTY_PW_SENTINEL_3b7f";
const HMAC: &str = "HMAC_SENTINEL_8e4c";
const CLIENT_SECRET: &str = "CLIENT_SECRET_SENTINEL_2a6d";
const PASSPHRASE: &str = "PASSPHRASE_SENTINEL_7f3b";
const KEY_MATERIAL: &str = "SIGNING_KEY_MATERIAL_SENTINEL_1c9e";
const LICENCE_MATERIAL: &str = "LICENCE_MATERIAL_SENTINEL_6d4a";
const IDENTIFIER_KEY: &str = "IDENTIFIER_KEY_SENTINEL_4e8b";
const S3_SECRET: &str = "S3_SECRET_SENTINEL_0b5c";
const EVENTS_PW: &str = "EVENTS_PW_SENTINEL_5a2f";
const ISSUER_PW: &str = "ISSUER_PW_SENTINEL_9c1d";

/// A configuration carrying a secret in every slot the tree has, with the
/// databases on a port nothing listens on.
fn configuration_with_secrets(files: &assert_fs::TempDir) -> FerroEhrConfig {
    let key = files.child("signing-key.asc");
    key.write_str(KEY_MATERIAL).expect("write the signing key");
    let token = files.child("licence.token");
    token
        .write_str(LICENCE_MATERIAL)
        .expect("write the licence token");

    let mut config = FerroEhrConfig::default();
    config.db.url = SecretUrl::new(format!(
        "postgres://reportuser:{DB_PW}@127.0.0.1:1/ferroehr"
    ));
    config.db.migrate_url = Some(SecretUrl::new(format!(
        "postgres://migrator:{MIGRATE_PW}@127.0.0.1:1/ferroehr"
    )));
    config.storage.party.url = Some(SecretUrl::new(format!(
        "postgres://partyuser:{PARTY_PW}@127.0.0.1:1/ferroehr"
    )));
    config.auth.oidc = Some(OidcConfig {
        issuer: format!("https://idpuser:{ISSUER_PW}@idp.example/realms/ferroehr"),
        hmac_secret: Some(Secret::new(HMAC)),
        ..OidcConfig::default()
    });
    config.terminology.external.oauth2_clients = BTreeMap::from([(
        "terminology".to_owned(),
        TerminologyOauth2Config {
            token_url: "https://idp.example/token".to_owned(),
            client_id: "ferroehr".to_owned(),
            client_secret: Some(Secret::new(CLIENT_SECRET)),
            ..TerminologyOauth2Config::default()
        },
    )]);
    config.signing.key_path = Some(key.path().to_path_buf());
    config.signing.key_passphrase = Some(Secret::new(PASSPHRASE));
    config.licence.file = Some(token.path().to_path_buf());
    config.demographic.identifier_protection.key = Some(Secret::new(IDENTIFIER_KEY));
    config.multimedia.secret_access_key = Some(Secret::new(S3_SECRET));
    config.events.url = SecretUrl::new(format!("amqp://mq:{EVENTS_PW}@broker:5672/vh"));
    config
}

/// The manifest entry for one part.
fn entry<'a>(report: &'a Report, part: &str) -> &'a ManifestEntry {
    report
        .manifest
        .iter()
        .find(|entry| entry.part == part)
        .unwrap_or_else(|| panic!("the manifest names no `{part}` part: {:?}", report.manifest))
}

/// No secret, no URL `userinfo` and no secret file's contents reaches the
/// report, while the non-secret identity around them stays legible.
#[tokio::test]
async fn no_credential_reaches_the_report() {
    let files = assert_fs::TempDir::new().expect("temp dir");
    let config = configuration_with_secrets(&files);
    let report = Report::gather(&config).await;
    let written = serde_json::to_string_pretty(&report).expect("the report serializes");

    for sentinel in [
        DB_PW,
        MIGRATE_PW,
        PARTY_PW,
        HMAC,
        CLIENT_SECRET,
        PASSPHRASE,
        KEY_MATERIAL,
        LICENCE_MATERIAL,
        IDENTIFIER_KEY,
        S3_SECRET,
        EVENTS_PW,
        ISSUER_PW,
    ] {
        assert!(
            !written.contains(sentinel),
            "{sentinel} leaked into the report:\n{written}"
        );
    }
    for userinfo in ["reportuser", "migrator:", "partyuser", "idpuser", "mq:"] {
        assert!(
            !written.contains(userinfo),
            "the userinfo `{userinfo}` leaked into the report:\n{written}"
        );
    }
    // The identity around the secrets stays legible.
    assert!(
        written.contains("postgres://***@127.0.0.1:1/ferroehr"),
        "{written}"
    );
    assert!(
        written.contains("https://***@idp.example/realms/ferroehr"),
        "{written}"
    );
    assert!(written.contains("\"client_id\": \"ferroehr\""), "{written}");
}

/// A database that does not answer is named in the manifest with its reason,
/// the schema part is left out rather than empty, and the posture falls back
/// to the configuration's own reading.
#[tokio::test]
async fn the_manifest_names_an_unreachable_database() {
    let files = assert_fs::TempDir::new().expect("temp dir");
    let report = Report::gather(&configuration_with_secrets(&files)).await;

    let schema = entry(&report, "schema");
    assert_eq!(schema.status, PartStatus::Unavailable);
    let reason = schema.reason.as_deref().unwrap_or_default();
    assert!(reason.starts_with("unreachable: "), "{reason}");
    assert!(report.schema.is_none());

    let deployment = entry(&report, "deployment");
    assert_eq!(deployment.status, PartStatus::Partial);
    assert!(
        deployment
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("unreachable: ")),
        "{deployment:?}"
    );

    let image = entry(&report, "image");
    assert_eq!(image.status, PartStatus::Unavailable);
    assert!(image.reason.is_some());
    for part in ["build", "target", "features", "licence", "configuration"] {
        assert_eq!(entry(&report, part).status, PartStatus::Included, "{part}");
    }
}

/// The report names the manufacturer, the build and the specification pins it
/// runs.
#[tokio::test]
async fn the_report_names_the_build_and_the_manufacturer() {
    let files = assert_fs::TempDir::new().expect("temp dir");
    let report = Report::gather(&configuration_with_secrets(&files)).await;

    assert_eq!(report.format, ferroehr::report::FORMAT);
    assert_eq!(report.build.manufacturer, MANUFACTURER);
    assert_eq!(report.build.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(report.build.spec.its_rest, openehr_its::SPEC_VERSION);
    assert!(
        report
            .openehr_crates
            .iter()
            .any(|linked| linked.name == "openehr-rm"),
        "{:?}",
        report.openehr_crates
    );
    assert!(
        report.file_name().starts_with("ferroehr-report-")
            && report.file_name().ends_with("Z.json"),
        "{}",
        report.file_name()
    );
}

/// Against a reachable database the report carries every schema's migration
/// level and the measured posture, and nothing from the clinical record: an
/// EHR's id and its subject's identifier stay out of it.
#[tokio::test]
async fn a_reachable_database_gives_levels_and_no_patient_identifier() {
    const SUBJECT: &str = "subject-SENTINEL-7d3e";
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&DomainPools::from_shared(&db.pool()));
    let status = serde_json::json!({
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
                "namespace": "patients",
                "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": SUBJECT }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    });
    let status = openehr_its::json::from_canonical_value::<openehr_rm::prelude::EhrStatus>(&status)
        .expect("a valid EHR_STATUS");
    let ehr_id = svc.create_ehr(Some(status)).await.expect("create_ehr");

    let mut config = FerroEhrConfig::default();
    config.db.url = SecretUrl::new(db.url());
    let report = Report::gather(&config).await;
    let written = serde_json::to_string(&report).expect("the report serializes");

    assert_eq!(entry(&report, "schema").status, PartStatus::Included);
    assert_eq!(entry(&report, "deployment").status, PartStatus::Included);
    let databases = report.schema.as_ref().expect("schema levels");
    assert!(!databases.is_empty());
    for set in databases.iter().flat_map(|database| &database.sets) {
        assert_eq!(set.applied, set.embedded, "{set:?}");
    }

    assert!(!written.contains(SUBJECT), "{written}");
    assert!(!written.contains(&ehr_id.to_string()), "{written}");
    let url = url::Url::parse(db.url()).expect("the testkit DSN is a URL");
    if let Some(password) = url.password() {
        let userinfo = format!("{}:{password}@", url.username());
        assert!(!written.contains(&userinfo), "{written}");
    }
}
