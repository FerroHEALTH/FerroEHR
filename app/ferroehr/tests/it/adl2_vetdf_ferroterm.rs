// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL2 VETDF against a REAL terminology server: `FerroTERM`, the one the
//! product ships beside the CDR (`docker-compose.terminology.yml`), serving the
//! same licence-free shaped seed the conformance lane binds to.
//!
//! `adl2_vetdf` pins every answer shape with a mock; this module proves the
//! contract on the server people actually run. Three bindings, three answers
//! (AM ADL2 `master03-archetype_package.adoc` §Validity Rules, VETDF):
//!
//! - a SNOMED CT concept URI, on a server that serves no SNOMED CT: the code
//!   system is not served, so "no verification was possible" and the archetype
//!   is accepted with a warning, never refused;
//! - a code of the served shaped system that exists: accepted;
//! - a code of the served shaped system that does not exist: VETDF, `422`.
//!
//! The container is `ghcr.io/ferrohealth/ferroterm` at the release the compose
//! overlay pins, started through `testcontainers` with `docker/terminology/seed`
//! mounted as its `FERROTERM_CODESYSTEMS`; the harness's `PostgreSQL` 18 backs
//! the service. Serialized with the other container suites by the nextest
//! `containers` group.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ferroehr::service::FerroEhrService;
use ferroehr::service::status::CallStatusType;
use ferroehr::service::terminology::config::{FhirOperation, FhirProviderConfig, ProviderKind};
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
use testcontainers::core::{ContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// The `FerroTERM` release the compose overlay pins (`docker-compose.terminology.yml`).
const FERROTERM_IMAGE: &str = "ghcr.io/ferrohealth/ferroterm";
/// Tag AND index digest, the reference the overlay carries: the digest is what
/// is pulled, so this suite measures the exact bytes a deployment runs.
const FERROTERM_TAG: &str =
    "0.1.6@sha256:7ec4c9622e88e3028faf7c74a2384fb02e57dc3163c282b21274121a26aa3d5b";
const FERROTERM_PORT: ContainerPort = ContainerPort::Tcp(8080);
/// The shaped code system the seed serves (`docker/terminology/seed/`).
const SHAPED_SYSTEM: &str = "http://cnf.example.test/fhir/CodeSystem/sct-shaped";
/// A concept the shaped system defines, and one it does not.
const SHAPED_MEMBER: &str = "1000001";
const SHAPED_ABSENT: &str = "9999999";

struct FerroTerm {
    _container: ContainerAsync<GenericImage>,
    /// The R4B base URL the CDR's provider points at.
    base: String,
}

impl FerroTerm {
    /// Start `FerroTERM` over the shaped seed and wait until its own health route
    /// answers (the image carries a `HEALTHCHECK`, but readiness is asserted
    /// from here so a slow pull never races the first lookup).
    async fn start() -> Self {
        let seed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docker/terminology/seed")
            .canonicalize()
            .expect("the shaped terminology seed directory exists");
        let container = GenericImage::new(FERROTERM_IMAGE, FERROTERM_TAG)
            .with_exposed_port(FERROTERM_PORT)
            .with_env_var("FERROTERM_CODESYSTEMS", "/data/codesystems")
            .with_env_var("FERROTERM_UI", "off")
            .with_env_var("FERROTERM_LOG_FORMAT", "json")
            .with_mount(Mount::bind_mount(
                seed.to_string_lossy().into_owned(),
                "/data/codesystems",
            ))
            .with_startup_timeout(Duration::from_secs(120))
            .start()
            .await
            .expect("start FerroTERM (is Docker running?)");
        let host = container.get_host().await.expect("host").to_string();
        let port = container
            .get_host_port_ipv4(FERROTERM_PORT)
            .await
            .expect("mapped port");
        let root = format!("http://{host}:{port}");
        let client = reqwest::Client::new();
        let mut healthy = false;
        for _ in 0..60 {
            if let Ok(r) = client.get(format!("{root}/health")).send().await
                && r.status().is_success()
            {
                healthy = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(healthy, "FerroTERM never answered GET /health");
        Self {
            _container: container,
            base: format!("{root}/r4b"),
        }
    }
}

/// A provider at `FerroTERM`'s R4B base, cache off so every test asks the server.
fn provider(base: &str) -> FhirTerminologyProvider {
    let cfg = FhirProviderConfig {
        kind: ProviderKind::Fhir,
        url: base.to_owned(),
        operation: FhirOperation::ValidateCode,
        connect_timeout_ms: 2000,
        request_timeout_ms: 5000,
        oauth2_client: None,
        client_cert_path: None,
        client_key_path: None,
        ca_bundle_path: None,
        cache_ttl_secs: 0,
        cache_capacity: 0,
    };
    FhirTerminologyProvider::new("ferroterm", &cfg).expect("build provider")
}

/// A minimal spec-valid ADL2 archetype named `concept` whose root node binds
/// `target` under the outer terminology key `terminology`.
fn archetype(concept: &str, terminology: &str, target: &str) -> String {
    format!(
        "\
archetype (adl_version=2.0.6; rm_release=1.1.0)
    openEHR-EHR-OBSERVATION.{concept}.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
        >
    >

definition
    OBSERVATION[id1] matches {{ *}}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>
        >
    >
    term_bindings = <
        [\"{terminology}\"] = <
            [\"id1\"] = <{target}>
        >
    >
"
    )
}

#[tokio::test]
async fn ferroterm_decides_vetdf_for_the_three_binding_outcomes() {
    let ferroterm = FerroTerm::start().await;
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_external_terminology(Arc::new(provider(&ferroterm.base)));

    // 1. A SNOMED CT concept URI on a server that serves no SNOMED CT: the
    //    binding is unverifiable, so the archetype is accepted (the sandbox
    //    seed's five refused archetypes, #3311).
    svc.upload_artefact(archetype(
        "snomed_unserved",
        "SNOMED-CT",
        "http://snomed.info/id/50121007",
    ))
    .await
    .expect("a code system the server does not serve leaves the binding unverified");
    assert!(
        svc.has_artefact("openEHR-EHR-OBSERVATION.snomed_unserved.v1.0.0".to_owned())
            .await
            .expect("has_artefact answers"),
        "the unverified archetype is stored"
    );

    // 2. A concept the served shaped system defines: accepted.
    svc.upload_artefact(archetype(
        "shaped_member",
        "CNF-SHAPED",
        &format!("{SHAPED_SYSTEM}/id/{SHAPED_MEMBER}"),
    ))
    .await
    .expect("a concept the served system defines resolves");

    // 3. A concept the served shaped system does not define: VETDF.
    let err = svc
        .upload_artefact(archetype(
            "shaped_absent",
            "CNF-SHAPED",
            &format!("{SHAPED_SYSTEM}/id/{SHAPED_ABSENT}"),
        ))
        .await
        .expect_err("a concept absent from a served system is refused");
    assert_eq!(err.status, CallStatusType::ContentInvalid);
    assert!(
        err.message.contains("VETDF") && err.message.contains(SHAPED_ABSENT),
        "the rejection names the rule and the code: {}",
        err.message
    );
    assert!(
        !svc.has_artefact("openEHR-EHR-OBSERVATION.shaped_absent.v1.0.0".to_owned())
            .await
            .expect("has_artefact answers"),
        "a refused archetype is not stored"
    );
}

/// The archetype the sandbox seed lost first (#3311): a CKM archetype with
/// real SNOMED CT bindings under the key `SNOMED-CT`, uploaded to a CDR whose
/// `FerroTERM` serves no SNOMED CT. Before the fix every binding was asked as
/// `system=SNOMED-CT&code=<uri>` and refused; now the bindings are unverifiable
/// and the archetype is stored.
#[tokio::test]
async fn a_ckm_archetype_with_snomed_bindings_uploads_when_snomed_is_not_served() {
    let ferroterm = FerroTerm::start().await;
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_external_terminology(Arc::new(provider(&ferroterm.base)));

    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../corpus/archetypes/adl2/ckm-2013-12-09/entry/observation/openEHR-EHR-OBSERVATION.refraction.v1.0.0.adls",
        ),
    )
    .expect("the vendored CKM archetype");
    svc.upload_artefact(source)
        .await
        .expect("SNOMED CT bindings on a server without SNOMED CT are unverifiable, not invalid");
    assert!(
        svc.has_artefact("openEHR-EHR-OBSERVATION.refraction.v1.0.0".to_owned())
            .await
            .expect("has_artefact answers"),
        "the CKM archetype is stored"
    );
}
