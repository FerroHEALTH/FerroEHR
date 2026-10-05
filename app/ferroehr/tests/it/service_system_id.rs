// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The configured openEHR **system identifier** reaches every wire value that
//! carries it, against a real `PostgreSQL` 18 (shared testkit harness).
//!
//! `[server] system_id` (`FERROEHR__SERVER__SYSTEM_ID`) is wired into the
//! service by the binary via `with_system_id`; these tests pin the three
//! places the value surfaces:
//!
//! - `EHR.system_id` at creation — RM ehr `master04-ehr_package.adoc` §EHR
//!   Identifier Allocation: "the `EHR._system_id_` value should be set to the
//!   value that would normally be used for locally created EHRs";
//! - `AUDIT_DETAILS.system_id` when the client supplies none — ITS-REST
//!   `specifications/docs/overview/Requests_and_responses.md`
//!   §"openehr-version and openehr-audit-details": "when `system_id` is not
//!   provided by the client, the server MUST set it to its own configured
//!   system identifier";
//! - `OBJECT_VERSION_ID.creating_system_id` on every minted version — RM common
//!   `master06-change_control_package.adoc` §Distributed Versioning.
//!
//! The default (no `with_system_id` call) is pinned too: it must stay
//! `DEFAULT_SYSTEM_ID`, so an unset config key is byte-identical to previous
//! behaviour.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::ids::EhrId;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};

use crate::fixtures::{composition, uv};

/// Create an EHR + one composition on `svc` and return
/// `(EHR.system_id, OBJECT_VERSION_ID, AUDIT_DETAILS.system_id)`.
async fn stamped_identities(svc: &FerroEhrService) -> (String, String, String) {
    let ehr_id: EhrId = svc.create_ehr(None).await.expect("create_ehr");
    let summary = svc.get_ehr(ehr_id).await.expect("get_ehr");

    let ovid = svc
        .create_composition(ehr_id, uv(&composition("system id"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();
    let original = svc
        .composition_version_envelope(ehr_id, ovid.parse().expect("ovid"))
        .await
        .expect("composition_version_envelope");
    let audit_system_id = original["commit_audit"]["system_id"]
        .as_str()
        .expect("commit_audit.system_id")
        .to_owned();

    (summary.system_id, ovid, audit_system_id)
}

#[tokio::test]
async fn configured_system_id_stamps_ehr_audit_and_version_uid() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_system_id("custom.sys");

    let (ehr_system_id, ovid, audit_system_id) = stamped_identities(&svc).await;

    // RM ehr master04 §EHR Identifier Allocation.
    assert_eq!(
        ehr_system_id, "custom.sys",
        "EHR.system_id must carry the configured identifier"
    );
    // RM common master06 §Distributed Versioning: the uid's middle segment is
    // the creating system id.
    assert_eq!(
        ovid.split("::").nth(1),
        Some("custom.sys"),
        "OBJECT_VERSION_ID.creating_system_id must carry the configured identifier ({ovid})"
    );
    // ITS-REST Requests_and_responses.md: the server default for a commit that
    // carried no client `system_id`.
    assert_eq!(
        audit_system_id, "custom.sys",
        "AUDIT_DETAILS.system_id must default to the configured identifier"
    );
}

#[tokio::test]
async fn unset_system_id_keeps_the_compatibility_default() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let (ehr_system_id, ovid, audit_system_id) = stamped_identities(&svc).await;

    assert_eq!(ehr_system_id, DEFAULT_SYSTEM_ID);
    assert_eq!(ovid.split("::").nth(1), Some(DEFAULT_SYSTEM_ID));
    assert_eq!(audit_system_id, DEFAULT_SYSTEM_ID);
}
