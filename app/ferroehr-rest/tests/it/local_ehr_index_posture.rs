// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The subject-id posture, switched on by `[ehr_index] provider = "local"` from
//! the environment over a configuration written for an IHE patient identity
//! source, end to end over the router (#4379).
//!
//! `OBJECT_REF.namespace` takes any name matching its pattern (BASE
//! `base_types` `OBJECT_REF`), `GENERIC_ID.scheme` may be a local scheme (BASE
//! `GENERIC_ID`), and ITS-REST 1.1.0 `ehr_get_by_subject` matches
//! `external_ref.id.value` and `external_ref.namespace` exactly
//! (`docs/specs/openehr/ITS-REST/docs/ehr/ehr.md`). Which posture the server
//! runs is no openEHR spec's: our own design.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's async bodies; panicking assertions are the intended shape here"
)]

use std::collections::HashMap;
use std::sync::Arc;

use http::StatusCode;
use serde_json::json;

use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr_rest::config::AppConfig;

use crate::common::{self, BASE};

/// A configuration written for an IHE patient identity source: the source and
/// its client credentials grant, with no pseudonym namespace declared.
const IHE_CONFIG: &str = r#"
[ehr_index]
provider = "ihe"
pixm_url = "http://localhost:8090/fhir"
assigning_authority = "urn:oid:2.999.7.1"

[ehr_index.oauth]
token_endpoint = "http://localhost:8180/quickstart/token"
client_id = "ferroehr"
client_secret = "quickstart"
scope = "system/Patient.rs"
"#;

/// On the local posture, an EHR whose subject names an ordinary namespace and
/// an identifier that is not a UUID is created `201`, and the subject lookup
/// finds it.
#[tokio::test]
async fn the_local_posture_creates_and_finds_an_ehr_by_its_subject() {
    let file = tempfile_with(IHE_CONFIG);
    let env = HashMap::from([(
        "FERROEHR__EHR_INDEX__PROVIDER".to_owned(),
        "local".to_owned(),
    )]);
    let config = ferroehr::config::assemble(Some(file.path()), &env, &[])
        .expect("the switched configuration assembles");
    config
        .validate()
        .expect("the switched configuration validates");
    let privacy = config.privacy.clone();
    assert!(
        privacy.subject_namespaces.is_empty(),
        "no pseudonym namespace is declared: {:?}",
        privacy.subject_namespaces
    );

    let (db, pool) = common::migrated_pool().await;
    ferroehr::db::stamp_subject_posture(&pool, !privacy.subject_namespaces.is_empty())
        .await
        .expect("stamp the subject posture");
    let policy = ferroehr::privacy::PrivacyPolicy::compile(&privacy).expect("the policy compiles");
    let ehr_index =
        EhrIndexProvider::from_config(&config.ehr_index).expect("the local provider builds");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_privacy(Arc::new(policy))
        .with_ehr_index(ehr_index);
    let mut app_config = AppConfig {
        server: common::server(),
        ..AppConfig::default()
    };
    app_config.auth.enabled = false;
    let app = common::router_with(app_config, Arc::new(service)).await;
    let _keep = db;

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
                "namespace": "benchmark",
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": "1793131381", "scheme": "benchmark" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    });
    let (created, headers, body) = common::send(
        &app,
        common::post_json(&format!("{BASE}/ehr"), &status.to_string()),
    )
    .await;
    assert_eq!(created, StatusCode::CREATED, "the EHR is created: {body}");
    let ehr_id = common::etag_uid(&headers);

    let (found, body) = common::send_body(
        &app,
        common::get(&format!(
            "{BASE}/ehr?subject_id=1793131381&subject_namespace=benchmark"
        )),
    )
    .await;
    assert_eq!(found, StatusCode::OK, "the subject's EHR is found: {body}");
    let found: serde_json::Value = serde_json::from_str(&body).expect("the EHR body");
    assert_eq!(found["ehr_id"]["value"], json!(ehr_id), "{body}");
}

/// A temporary `.toml` file holding `text`.
fn tempfile_with(text: &str) -> assert_fs::NamedTempFile {
    use assert_fs::prelude::FileWriteStr as _;
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("a temporary file");
    file.write_str(text).expect("write the configuration");
    file
}
