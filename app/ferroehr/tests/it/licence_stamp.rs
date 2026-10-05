// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every identifier the server mints carries the licence stamp
//! (`ferroehr::licence::stamp`), through the real commit path against the
//! shared test database: the EHR id, the versioned-object id and the
//! contribution id of a committed composition, under the fail-safe key for a
//! build with no embedded token and under a licence's key once one is in
//! force.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::ids::EhrId;
use ferroehr::licence::document::{Licence, Use};
use ferroehr::licence::stamp::StampKey;
use ferroehr::licence::state::{LicenceState, Source};
use ferroehr::licence::verify::Verified;
use ferroehr::service::FerroEhrService;
use jiff::civil::date;
use serde_json::json;
use uuid::Uuid;

use crate::fixtures::{change_type, committer, composition, vo_of};

/// A licence in force for the service builder, bypassing the token chain the
/// unit tests already cover: the builder only needs the document to derive
/// the stamp key.
fn licensed(id: Uuid) -> LicenceState {
    LicenceState::Licensed {
        verified: Verified {
            licence: Licence {
                id,
                licensee: "Example Hospital NV".to_owned(),
                issued: date(2026, 9, 11),
                not_before: date(2026, 9, 11),
                not_after: date(2099, 12, 31),
                permitted_use: Use::Commercial,
            },
            primary: pgp::types::Fingerprint::V4([0; 20]),
            signing_subkey: pgp::types::Fingerprint::V4([1; 20]),
            signed_at: jiff::Timestamp::UNIX_EPOCH,
            subkey_expires_at: None,
        },
        source: Source::Configured,
        configured_failure: None,
    }
}

/// Commits one composition and returns `(versioned-object id, contribution id)`.
async fn commit_one(svc: &FerroEhrService, ehr_id: EhrId) -> (Uuid, Uuid) {
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer("author")
            },
            "lifecycle_state": change_type("532", "complete"),
            "data": composition("stamped"),
        }],
        "audit": { "change_type": change_type("249", "creation"), "committer": committer("author") }
    });
    let created = svc
        .create_ehr_contribution(ehr_id, contribution)
        .await
        .expect("contribution commits");
    let contribution_id = created.body["uid"]["value"]
        .as_str()
        .expect("contribution uid")
        .parse::<Uuid>()
        .expect("contribution uid is a uuid");
    let version_uid = created.body["versions"][0]["id"]["value"]
        .as_str()
        .expect("version uid")
        .to_owned();
    let vo = vo_of(&version_uid)
        .parse::<Uuid>()
        .expect("object id is a uuid");
    (vo, contribution_id)
}

#[tokio::test]
async fn every_server_minted_identifier_carries_the_stamp_in_force() {
    let db = testkit::db().await.expect("testkit database");

    // A bare service: no licence in force, the fail-safe key.
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let fail_safe = StampKey::fail_safe();
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");
    assert!(fail_safe.carries(ehr_id.0), "EHR id {ehr_id:?}");
    let (vo, contribution) = commit_one(&svc, ehr_id).await;
    assert!(fail_safe.carries(vo), "versioned-object id {vo}");
    assert!(
        fail_safe.carries(contribution),
        "contribution id {contribution}"
    );

    // With a licence in force: its key, and the fail-safe key no longer matches.
    let licence_id = Uuid::now_v7();
    let key = StampKey::for_licence(licence_id);
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_licence(licensed(licence_id));
    assert!(svc.licence().is_licensed());
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");
    assert!(key.carries(ehr_id.0), "EHR id {ehr_id:?}");
    assert!(!fail_safe.carries(ehr_id.0));
    let (vo, contribution) = commit_one(&svc, ehr_id).await;
    assert!(key.carries(vo), "versioned-object id {vo}");
    assert!(key.carries(contribution), "contribution id {contribution}");
    assert!(!fail_safe.carries(vo) && !fail_safe.carries(contribution));
}
