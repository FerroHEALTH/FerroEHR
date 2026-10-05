// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests for ATTESTATION support in the CONTRIBUTION path
//! (RM common `master06-change_control_package.adoc` §Change Control /
//! §Attestation; ITS-REST `UpdateVersion.yaml` + `UpdateAttestation.yaml`)
//! against a real `PostgreSQL` 18 (shared testkit harness).
//!
//! Covers: attestations committed with a NEW version
//! (`UPDATE_VERSION.attestations`, "Signing content at committal"); a later
//! `666|attestation|`-only contribution attaching an `ATTESTATION` to an
//! existing `ORIGINAL_VERSION` (no new version); their exposure on the served
//! `ORIGINAL_VERSION` and in `REVISION_HISTORY`; and `CONTRIBUTION.versions` +
//! the aggregate change-type semantics. Plus the error surface (400/422/404).

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]
#![expect(
    clippy::too_many_lines,
    reason = "an end-to-end suite drives one long lifecycle per test on purpose: \
              splitting a case would hide the order its assertions depend on"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;
use ferroehr::service::status::{CallStatusType, SmError};

use ferroehr::service::list::Page;
use ferroehr::service::version_update::lifecycle_state_coded;
use serde_json::{Value, json};

use crate::fixtures::{change_type, committer, composition, uv, vo_of};

/// A wire `UPDATE_ATTESTATION` partial (the server completes it).
fn attestation(reason: &str, is_pending: bool) -> Value {
    json!({
        "_type": "UPDATE_ATTESTATION",
        "change_type": change_type("666", "attestation"),
        "committer": committer("attesting clinician"),
        "reason": { "_type": "DV_TEXT", "value": reason },
        "is_pending": is_pending,
        "proof": "proof-bytes"
    })
}

async fn create_ehr(svc: &FerroEhrService) -> String {
    svc.create_ehr(None).await.expect("create_ehr").to_string()
}

/// Read the served `ORIGINAL_VERSION` of a composition version.
async fn read_version(svc: &FerroEhrService, ehr_id: &str, _vo: &str, ovid: &str) -> Value {
    svc.composition_version_envelope(
        ehr_id.parse().expect("ehr uuid"),
        ovid.parse().expect("ovid"),
    )
    .await
    .expect("versioned composition version")
}

/// The `OBJECT_VERSION_ID` of the first version listed in a `CONTRIBUTION`.
fn first_version_uid(contribution: &Value) -> String {
    contribution["versions"][0]["id"]["value"]
        .as_str()
        .expect("versions[0].id.value")
        .to_owned()
}

#[tokio::test]
async fn accompanying_attestation_then_standalone_666_attestation() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;

    // (1) A CONTRIBUTION that creates a COMPOSITION carrying an attestation
    // committed with the version (UPDATE_VERSION.attestations).
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer("author")
            },
            "lifecycle_state": change_type("532", "complete"),
            "data": composition("v1"),
            "attestations": [ attestation("witnessed", false) ]
        }],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
    });
    let created = svc
        .create_ehr_contribution(ehr_id.parse().expect("ehr uuid"), contribution)
        .await
        .expect("contribution_create with accompanying attestation → 201");
    let ovid_v1 = first_version_uid(&created.body);
    let vo_id = vo_of(&ovid_v1);

    // Reading the ORIGINAL_VERSION exposes the completed ATTESTATION.
    let ov = read_version(&svc, &ehr_id, vo_id, &ovid_v1).await;
    let atts = ov["attestations"].as_array().expect("attestations array");
    assert_eq!(atts.len(), 1, "one attestation after step 1");
    let att = &atts[0];
    assert_eq!(att["_type"], "ATTESTATION");
    assert!(att["system_id"].is_string(), "server-completed system_id");
    assert_eq!(att["time_committed"]["_type"], "DV_DATE_TIME");
    assert_eq!(att["reason"]["value"], "witnessed");
    assert_eq!(att["is_pending"], json!(false));
    // The inherited change_type is the 666|attestation| code.
    assert_eq!(att["change_type"]["defining_code"]["code_string"], "666");

    // (2) A later 666-only CONTRIBUTION attesting that same version. Its
    // member states a lifecycle_state like every other member (SM master03
    // §Version Update Semantics: "must be supplied in all cases").
    let attest_contribution = json!({
        "versions": [{
            "preceding_version_uid": { "value": ovid_v1.clone() },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("senior reviewer"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            }
        }],
        // The change set's own change type is the CLIENT's account of it (the
        // released commit schema requires it), never a server-derived
        // aggregate.
        "audit": {
            "change_type": change_type("666", "attestation"),
            "committer": committer("senior reviewer")
        }
    });
    let attest_created = svc
        .create_ehr_contribution(ehr_id.parse().expect("ehr uuid"), attest_contribution)
        .await
        .expect("666-only contribution → 201");

    // The 666 contribution's aggregate change_type is 666, and its versions
    // list the attested (existing) version.
    assert_eq!(
        attest_created.body["audit"]["change_type"]["defining_code"]["code_string"],
        "666"
    );
    let attest_versions = attest_created.body["versions"]
        .as_array()
        .expect("versions");
    assert_eq!(attest_versions.len(), 1, "attested version is listed");
    assert_eq!(
        attest_versions[0]["id"]["value"].as_str(),
        Some(ovid_v1.as_str())
    );

    // The ORIGINAL_VERSION now lists both attestations.
    let ov = read_version(&svc, &ehr_id, vo_id, &ovid_v1).await;
    assert_eq!(
        ov["attestations"].as_array().map(Vec::len),
        Some(2),
        "two attestations after the standalone 666"
    );

    // REVISION_HISTORY: the single version's audits = commit audit + both
    // attestations (revision_history_item.adoc "there may also be further
    // attestations").
    let rh = svc
        .composition_revision_history(
            ehr_id.parse().expect("ehr uuid"),
            vo_id.parse().expect("vo uuid"),
        )
        .await
        .expect("revision history");
    let items = rh["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "one version");
    let audits = items[0]["audits"].as_array().expect("audits");
    assert_eq!(audits.len(), 3, "commit audit + 2 attestations");
    assert_eq!(audits[0]["_type"], "AUDIT_DETAILS");
    assert_eq!(audits[1]["_type"], "ATTESTATION");
    assert_eq!(audits[2]["_type"], "ATTESTATION");
}

#[tokio::test]
async fn attestation_error_cases() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;

    // A real composition to attest.
    let v1 = svc
        .create_composition(
            ehr_id.parse().expect("ehr uuid"),
            uv(&composition("v1"), "249", None),
        )
        .await
        .expect("composition_create")
        .version_uid();
    let ovid_v1 = v1;

    let attempt = |body: Value| {
        let svc = svc.clone();
        let ehr = ehr_id.clone();
        async move {
            svc.create_ehr_contribution(ehr.parse().expect("ehr uuid"), body)
                .await
        }
    };

    // 666 without preceding_version_uid → 400 (cannot name its target).
    let err = attempt(json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("x"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            }
        }]
    }))
    .await
    .expect_err("666 without preceding → error");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "got {err:?}"
    );

    // 666 carrying data → 422 (attestation adds no content).
    let err = attempt(json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "preceding_version_uid": { "value": ovid_v1.clone() },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("x"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            },
            "data": composition("nope")
        }]
    }))
    .await
    .expect_err("666 with data → error");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ),
        "got {err:?}"
    );

    // Attestation missing reason → 422 (ATTESTATION.reason 1..1).
    let err = attempt(json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "preceding_version_uid": { "value": ovid_v1.clone() },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("x"),
                "is_pending": false
            }
        }]
    }))
    .await
    .expect_err("missing reason → error");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ),
        "got {err:?}"
    );

    // Attestation missing is_pending → 422 (ATTESTATION.is_pending 1..1).
    let err = attempt(json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "preceding_version_uid": { "value": ovid_v1.clone() },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("x"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" }
            }
        }]
    }))
    .await
    .expect_err("missing is_pending → error");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ),
        "got {err:?}"
    );

    // Attestation of a non-existent version → precondition violation, not an
    // existence error: SM I_EHR_CONTRIBUTION.commit_contribution declares only
    // `ehr_id_does_not_exist` (SM `i_ehr_contribution.adoc`) — a missing
    // body-referenced target is invalid committed content (ITS-REST
    // `400_CONTRIBUTION`: the modification does not match a stored object).
    let ghost = "00000000-0000-7000-8000-000000000000::ferroehr.local::1";
    let err = attempt(json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "preceding_version_uid": { "value": ghost },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("x"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            }
        }]
    }))
    .await
    .expect_err("non-existent version → error");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "got {err:?}"
    );
}

/// A `666|attestation|` member states its `lifecycle_state` like every other
/// member (#3590). SM `master03-common_package.adoc` §Version Update Semantics:
/// "The `lifecycle_state` must be supplied in all cases", and ITS-REST
/// `schemas/ehr/UpdateVersion.yaml` lists it under `required`. A member without
/// one is the 400; a stated code outside the openEHR `version_lifecycle_state`
/// group is the 422 every member's out-of-group state gets; the twin stating
/// `532|complete|` commits the attestation.
#[tokio::test]
async fn attestation_member_without_lifecycle_state_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");
    let ovid = svc
        .create_composition(ehr_uuid, uv(&composition("attested"), "249", None))
        .await
        .expect("composition_create")
        .version_uid();

    let attest = |lifecycle_state: Option<Value>| {
        let mut member = json!({
            "preceding_version_uid": { "value": ovid.clone() },
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("senior reviewer"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            }
        });
        if let Some(state) = lifecycle_state {
            member["lifecycle_state"] = state;
        }
        json!({
            "versions": [member],
            "audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("senior reviewer")
            }
        })
    };

    let err = svc
        .create_ehr_contribution(ehr_uuid, attest(None))
        .await
        .expect_err("an attestation member without lifecycle_state is refused");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "the refusal is the 400, got {err:?}"
    );
    assert!(
        err.message
            .starts_with("versions[0]: lifecycle_state is required"),
        "the refusal names the member and the attribute, got {err:?}"
    );

    let err = svc
        .create_ehr_contribution(ehr_uuid, attest(Some(change_type("999", "nonsense"))))
        .await
        .expect_err("an out-of-group lifecycle_state is refused");
    assert_eq!(err.status, CallStatusType::ContentInvalid, "got {err:?}");
    assert!(
        err.message.contains("versions[0]/lifecycle_state")
            && err.message.contains("version_lifecycle_state"),
        "got {err:?}"
    );

    let created = svc
        .create_ehr_contribution(ehr_uuid, attest(Some(change_type("532", "complete"))))
        .await
        .expect("the attestation member stating its lifecycle_state commits");
    assert_eq!(
        created.body["versions"][0]["id"]["value"].as_str(),
        Some(ovid.as_str()),
        "the attested version is listed"
    );
}

/// Every attestation states its own `committer` (#3550): ITS-REST
/// `schemas/common/UpdateAttestation.yaml` inherits `required: [change_type,
/// committer]` from `UpdateAudit.yaml` through `allOf`. Neither the
/// `666|attestation|` member's `commit_audit` nor an attestation committed with
/// a new version (`UPDATE_VERSION.attestations`) takes the CONTRIBUTION audit's
/// committer; each refusal is the 400 naming the member.
#[tokio::test]
async fn attestation_without_committer_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");
    let ovid = svc
        .create_composition(ehr_uuid, uv(&composition("attested"), "249", None))
        .await
        .expect("composition_create")
        .version_uid();
    let audit = json!({
        "change_type": change_type("251", "modification"),
        "committer": committer("contribution committer")
    });

    // The 666 member's own audit without a committer.
    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [{
                    "preceding_version_uid": { "value": ovid.clone() },
                    "lifecycle_state": change_type("532", "complete"),
                    "commit_audit": {
                        "change_type": change_type("666", "attestation"),
                        "reason": { "_type": "DV_TEXT", "value": "authorised" },
                        "is_pending": false
                    }
                }],
                "audit": audit.clone()
            }),
        )
        .await
        .expect_err("a 666 member without a committer is refused");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "got {err:?}"
    );
    assert!(
        err.message
            .starts_with("versions[0]: commit_audit.committer is required"),
        "got {err:?}"
    );

    // An attestation committed with a new version, without a committer.
    let mut unattributed = attestation("witnessed", false);
    unattributed
        .as_object_mut()
        .expect("the fixture is an object")
        .remove("committer");
    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("249", "creation"),
                        "committer": committer("author")
                    },
                    "lifecycle_state": change_type("532", "complete"),
                    "data": composition("with an unattributed attestation"),
                    "attestations": [unattributed]
                }],
                "audit": audit
            }),
        )
        .await
        .expect_err("an accompanying attestation without a committer is refused");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "got {err:?}"
    );
    assert!(
        err.message
            .starts_with("versions[0]: UPDATE_ATTESTATION.committer is required"),
        "got {err:?}"
    );
}

/// A valid `EHR_STATUS` for an update (a fresh version, distinct content).
fn ehr_status(queryable: bool) -> Value {
    json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF" },
        "is_queryable": queryable,
        "is_modifiable": true
    })
}

/// SM `I_EHR_CONTRIBUTION.list_contributions` / `contribution_count` +
/// `I_EHR_SERVICE.get_ehr` → `EHR_SUMMARY`, against a real PG 18. Native-API
/// calls only (no ITS-REST route); see `i_ehr_contribution.adoc` +
/// `ehr_summary.adoc`.
#[tokio::test]
async fn contribution_listing_count_and_ehr_summary() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Unknown EHR → NotFound (SM `ehr_id_does_not_exist`, carried granularly
    // through the ServiceError round-trip) for every native call.
    let ghost = "00000000-0000-7000-8000-0000000000ff";
    assert!(matches!(
        svc.list_contributions(ghost.parse().expect("uuid"), None, Page::all())
            .await,
        Err(SmError {
            status: CallStatusType::EhrIdDoesNotExist,
            ..
        })
    ));
    assert!(matches!(
        svc.contribution_count(ghost.parse().expect("uuid"), None)
            .await,
        Err(SmError {
            status: CallStatusType::EhrIdDoesNotExist,
            ..
        })
    ));
    assert!(matches!(
        svc.get_ehr(ghost.parse().expect("uuid")).await,
        Err(SmError {
            status: CallStatusType::EhrIdDoesNotExist,
            ..
        })
    ));

    // Seed: (1) EHR creation, (2) an EHR_STATUS update, (3) a composition — three
    // CONTRIBUTIONs, one of them a COMPOSITION.
    let ehr_id = create_ehr(&svc).await; // contribution #1

    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    let status_uid = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("get current EHR_STATUS")["uid"]["value"]
        .as_str()
        .expect("status uid")
        .to_owned();
    svc.replace_ehr_status(ehr_uuid, uv(&ehr_status(false), "251", Some(&status_uid)))
        .await
        .expect("EHR_STATUS update"); // contribution #2

    svc.create_composition(ehr_uuid, uv(&composition("obs"), "249", None))
        .await
        .expect("composition_create"); // contribution #3

    // contribution_count matches the seeded three.
    let count = svc
        .contribution_count(ehr_uuid, None)
        .await
        .expect("contribution_count");
    assert_eq!(count, 3, "EHR creation + status update + composition");

    // contribution_list returns all three, oldest-first, distinct.
    let all = svc
        .list_contributions(ehr_uuid, None, Page::all())
        .await
        .expect("contribution_list");
    assert_eq!(all.len(), 3, "three contribution ids");
    let distinct: std::collections::BTreeSet<_> = all.iter().collect();
    assert_eq!(distinct.len(), 3, "ids are distinct");

    // Paging: offset 1, fetch 1 → exactly the second id of the full list.
    let page = svc
        .list_contributions(
            ehr_uuid,
            None,
            Page {
                item_offset: Some(1),
                items_to_fetch: Some(1),
            },
        )
        .await
        .expect("paged contribution_list");
    assert_eq!(
        page,
        vec![all[1].clone()],
        "offset 1 / fetch 1 slices the list"
    );

    // time_range: an upper bound before every commit excludes all.
    let empty = svc
        .list_contributions(
            ehr_uuid,
            Some((None, Some("2000-01-01T00:00:00Z".to_owned()))),
            Page::all(),
        )
        .await
        .expect("bounded contribution_list");
    assert!(
        empty.is_empty(),
        "upper bound in the past → no contributions"
    );
    assert_eq!(
        svc.contribution_count(
            ehr_uuid,
            Some((None, Some("2000-01-01T00:00:00Z".to_owned())))
        )
        .await
        .expect("bounded count"),
        0
    );
    // A malformed bound → 400 BadRequest.
    assert!(matches!(
        svc.contribution_count(ehr_uuid, Some((Some("not-a-time".to_owned()), None)))
            .await,
        Err(SmError {
            status: CallStatusType::PreconditionViolation,
            ..
        })
    ));

    // EHR_SUMMARY: mandatory fields + the counts.
    let summary = svc.get_ehr(ehr_uuid).await.expect("get_ehr_summary");
    assert_eq!(summary.ehr_id, ehr_id);
    assert!(!summary.system_id.is_empty(), "system_id (EHR.system_id)");
    assert_eq!(summary.ehr_status["_type"], "EHR_STATUS", "ehr_status copy");
    assert!(
        summary.time_created.parse::<jiff::Timestamp>().is_ok(),
        "time_created is ISO 8601"
    );
    assert_eq!(summary.contribution_count, 3);
    assert_eq!(
        summary.composition_count, 1,
        "one versioned COMPOSITION (versioned objects, not versions)"
    );
}

/// The EHR contribution-list extension (`GET /ehr/{ehr_id}/contribution`, no
/// uid) — OUR OWN EXTENSION (no openEHR spec governs it). Asserts the
/// newest-first order, the `{rows, total}` shape (uid / `time_committed` /
/// committer / `change_type` / `change_type_rubric`), pagination, and the
/// unknown-EHR 404.
#[tokio::test]
async fn ehr_contribution_list_page_extension() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Unknown EHR → NotFound (404, SM `ehr_id_does_not_exist`), like the
    // sibling EHR reads.
    let ghost = "00000000-0000-7000-8000-0000000000ee";
    assert!(matches!(
        svc.ehr_contribution_list_page(ghost.parse().expect("uuid"), 0, 20)
            .await,
        Err(SmError {
            status: CallStatusType::EhrIdDoesNotExist,
            ..
        })
    ));

    // Seed three contributions: EHR create, an EHR_STATUS update, a composition.
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    let status_uid = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("get current EHR_STATUS")["uid"]["value"]
        .as_str()
        .expect("status uid")
        .to_owned();
    svc.replace_ehr_status(ehr_uuid, uv(&ehr_status(false), "251", Some(&status_uid)))
        .await
        .expect("EHR_STATUS update");
    svc.create_composition(ehr_uuid, uv(&composition("obs"), "249", None))
        .await
        .expect("composition_create");

    // Full page: total = 3, three rows, newest first (the composition last).
    let page = svc
        .ehr_contribution_list_page(ehr_uuid, 0, 20)
        .await
        .expect("list page");
    assert_eq!(page["total"], 3);
    let rows = page["rows"].as_array().expect("rows array").clone();
    assert_eq!(rows.len(), 3, "three contribution rows");
    // The latest commit (the composition) is first: change_type 249 with its
    // bundle-resolved rubric, its committer name, a UUID uid, and an ISO-8601
    // time_committed.
    assert_eq!(rows[0]["change_type"], "249");
    assert_eq!(rows[0]["change_type_rubric"], "creation");
    assert_eq!(rows[0]["committer"], "conformance tester");
    assert!(
        rows[0]["uid"]
            .as_str()
            .expect("uid")
            .parse::<uuid::Uuid>()
            .is_ok(),
        "uid is a UUID"
    );
    assert!(
        rows[0]["time_committed"]
            .as_str()
            .expect("time")
            .parse::<jiff::Timestamp>()
            .is_ok(),
        "time_committed is ISO 8601"
    );
    // Rows are strictly newest-first by time_committed.
    let times: Vec<&str> = rows
        .iter()
        .map(|r| r["time_committed"].as_str().expect("time"))
        .collect();
    let mut desc = times.clone();
    desc.sort_unstable();
    desc.reverse();
    assert_eq!(times, desc, "rows are newest-first");

    // Pagination: offset 1 / fetch 1 → exactly the second row; total unchanged.
    let paged = svc
        .ehr_contribution_list_page(ehr_uuid, 1, 1)
        .await
        .expect("paged list");
    assert_eq!(paged["total"], 3);
    assert_eq!(paged["rows"].as_array().expect("rows").len(), 1);
    assert_eq!(
        paged["rows"][0]["uid"], rows[1]["uid"],
        "offset 1 yields the second row"
    );
}

/// The SM `Terminology_code` spelling of `UPDATE_VERSION.lifecycle_state`
/// (`UML/classes/update_version.adoc`: `{terminology_id, code_string}`), which
/// the raw-body CONTRIBUTION lane accepts beside the `DV_CODED_TEXT` the
/// released ITS-REST `UpdateVersion.yaml` declares.
fn lifecycle(code: &str) -> Value {
    json!({ "terminology_id": "openehr", "code_string": code })
}

/// Read the `lifecycle_state.defining_code.code_string` of a served
/// `ORIGINAL_VERSION`.
async fn lifecycle_code_of(svc: &FerroEhrService, ehr_id: &str, vo: &str, ovid: &str) -> String {
    let ov = read_version(svc, ehr_id, vo, ovid).await;
    ov["lifecycle_state"]["defining_code"]["code_string"]
        .as_str()
        .expect("lifecycle_state code_string")
        .to_owned()
}

#[tokio::test]
async fn contribution_honors_the_five_lifecycle_states() {
    // M1 (RM common master06 §"Version Lifecycle"): the client-supplied
    // lifecycle_state on create/modify is stored + served faithfully for every
    // normative code; only the delete path is forced to 523. 553/800/801 are
    // NOT deletions — the version is readable with its data.
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;

    // (1) Create v1 as incomplete (553).
    let created = svc
        .create_ehr_contribution(
            ehr_id.parse().expect("ehr uuid"),
            json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("249", "creation"),
                        "committer": committer("author")
                    },
                    "lifecycle_state": lifecycle("553"),
                    "data": composition("incomplete v1")
                }],
                "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
            }),
        )
        .await
        .expect("create incomplete (553) → 201");
    let ovid_v1 = first_version_uid(&created.body);
    let vo_id = vo_of(&ovid_v1).to_owned();

    let ov1 = read_version(&svc, &ehr_id, &vo_id, &ovid_v1).await;
    assert_eq!(
        ov1["lifecycle_state"]["defining_code"]["code_string"],
        "553"
    );
    assert_eq!(ov1["lifecycle_state"]["value"], "incomplete");
    // A 553 version is not a deletion: its data is served.
    assert_eq!(ov1["data"]["_type"], "COMPOSITION");
    // The latest read returns 200 with data (not the deleted 204 path).
    let latest = svc
        .get_composition_latest(
            ehr_id.parse().expect("ehr uuid"),
            vo_id.parse().expect("vo uuid"),
        )
        .await
        .expect("composition_get incomplete");
    assert_eq!(latest["name"]["value"], "incomplete v1");

    // (2) The master06 §Version Lifecycle state machine only permits listed
    // transitions: `inactive` is entered by `deactivate` FROM `complete`, so an
    // incomplete → inactive modify is rejected 422 naming the state machine.
    let mut current = ovid_v1;
    let illegal = svc
        .create_ehr_contribution(
            ehr_id.parse().expect("ehr uuid"),
            json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("251", "modification"),
                        "committer": committer("author")
                    },
                    "preceding_version_uid": { "value": current },
                    "lifecycle_state": lifecycle("800"),
                    "data": composition("edited")
                }],
                "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
            }),
        )
        .await
        .expect_err("incomplete → inactive is not a listed transition");
    assert!(
        matches!(
            illegal,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ) && illegal.message.contains("state machine"),
        "expected the 422 naming the state machine, got {illegal:?}"
    );

    // Walk the LEGAL transitions (master06 §Version Lifecycle + §Abandoned and
    // Inactive States): 553 →complete→ 532 →deactivate→ 800 →retrieve→ 553
    // →abandon→ 801 — each a new version carrying the client lifecycle_state.
    for (n, code) in [(2, "532"), (3, "800"), (4, "553"), (5, "801")] {
        let modified = svc
            .create_ehr_contribution(
                ehr_id.parse().expect("ehr uuid"),
                json!({
                    "versions": [{
                        "commit_audit": {
                            "change_type": change_type("251", "modification"),
                            "committer": committer("author")
                        },
                        "preceding_version_uid": { "value": current },
                        "lifecycle_state": lifecycle(code),
                        "data": composition("edited")
                    }],
                    "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
                }),
            )
            .await
            .unwrap_or_else(|e| panic!("modify to {code} → 201, got {e:?}"));
        current = first_version_uid(&modified.body);
        assert!(current.ends_with(&format!("::{n}")), "version {n}");
        assert_eq!(
            lifecycle_code_of(&svc, &ehr_id, &vo_id, &current).await,
            code,
            "served lifecycle_state must be {code}"
        );
    }

    // (3) An out-of-group lifecycle code is a 422 naming the group.
    let err = svc
        .create_ehr_contribution(
            ehr_id.parse().expect("ehr uuid"),
            json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("251", "modification"),
                        "committer": committer("author")
                    },
                    "preceding_version_uid": { "value": current },
                    "lifecycle_state": lifecycle("999"),
                    "data": composition("bad state")
                }],
                "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
            }),
        )
        .await
        .expect_err("invalid lifecycle_state → error");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ) && err.message.contains("version_lifecycle_state"),
        "expected content_invalid naming version_lifecycle_state, got {err:?}"
    );

    // (4) The delete path commits 523 — the state the member itself declares,
    // the only one it may — and the latest read is then the deleted (204/null)
    // path.
    svc.create_ehr_contribution(
        ehr_id.parse().expect("ehr uuid"),
        json!({
            "versions": [{
                "commit_audit": {
                    "change_type": change_type("523", "deleted"),
                    "committer": committer("author")
                },
                "lifecycle_state": lifecycle("523"),
                "preceding_version_uid": { "value": current }
            }],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        }),
    )
    .await
    .expect("delete (523) → 201");
    // The now-current version is a deletion: lifecycle 523.
    let del = svc
        .get_composition_latest(
            ehr_id.parse().expect("ehr uuid"),
            vo_id.parse().expect("vo uuid"),
        )
        .await
        .expect("composition_get deleted");
    assert!(
        del.is_null(),
        "a deleted composition reads as an empty body (204), got {del:?}"
    );
}

/// Each member's `commit_audit` is the member's own (#3550): it keeps the
/// `committer` it states, takes the CONTRIBUTION audit's `system_id` when it
/// states none, and carries the one `time_committed` the server sets for the
/// whole set.
///
/// RM common `master06-change_control_package.adoc` §Committal and Audits: the
/// CONTRIBUTION audit's `system_id`/`committer`/`time_committed` "should be
/// copied" into each version's `commit_audit`, and `time_committed` "should
/// therefore be computed on the server". SM `UML/classes/update_version.adoc`
/// gives every member its own mandatory audit, whose `committer` is 1..1
/// (`update_audit.adoc`), so a stated member committer is kept. ITS-REST
/// `schemas/common/UpdateAudit.yaml`: an omitted `system_id` is the server's,
/// which is also what the CONTRIBUTION audit carries when it states none.
#[tokio::test]
async fn member_commit_audit_keeps_its_committer_and_shares_the_commit_instant() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    // The served commit_audits of every version a CONTRIBUTION lists, as
    // (system_id, committer name, time_committed).
    let audits_of = |contribution: Value| {
        let svc = svc.clone();
        let ehr_id = ehr_id.clone();
        async move {
            let mut seen: Vec<(String, String, String)> = Vec::new();
            for v in contribution["versions"].as_array().expect("versions") {
                let ovid = v["id"]["value"].as_str().expect("ovid").to_owned();
                let ov = read_version(&svc, &ehr_id, vo_of(&ovid), &ovid).await;
                let audit = &ov["commit_audit"];
                seen.push((
                    audit["system_id"].as_str().expect("system_id").to_owned(),
                    audit["committer"]["name"]
                        .as_str()
                        .expect("committer name")
                        .to_owned(),
                    audit["time_committed"]["value"]
                        .as_str()
                        .expect("time_committed")
                        .to_owned(),
                ));
            }
            seen.sort();
            seen
        }
    };

    let created = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [
                    // (A) states its committer, omits system_id.
                    {
                        "lifecycle_state": change_type("532", "complete"),
                        "commit_audit": {
                            "change_type": change_type("249", "creation"),
                            "committer": committer("version-A author")
                        },
                        "data": composition("takes the contribution system_id")
                    },
                    // (B) states a committer and a system_id of its own.
                    {
                        "lifecycle_state": change_type("532", "complete"),
                        "commit_audit": {
                            "change_type": change_type("249", "creation"),
                            "committer": committer("version-B author"),
                            "system_id": "version-b.system"
                        },
                        "data": composition("keeps its own audit")
                    }
                ],
                "audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("contribution committer"),
                    "system_id": "contribution.system"
                }
            }),
        )
        .await
        .expect("two-creation contribution → 201");
    let instant = created.body["audit"]["time_committed"]["value"]
        .as_str()
        .expect("the CONTRIBUTION audit's time_committed")
        .to_owned();
    assert_eq!(
        audits_of(created.body).await,
        vec![
            (
                "contribution.system".to_owned(),
                "version-A author".to_owned(),
                instant.clone()
            ),
            (
                "version-b.system".to_owned(),
                "version-B author".to_owned(),
                instant
            ),
        ],
        "each member keeps its committer, A takes the contribution system_id, \
         and every audit carries the one commit instant"
    );

    // No system_id anywhere: the CONTRIBUTION audit and the member both carry
    // the server's own.
    let created = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [{
                    "lifecycle_state": change_type("532", "complete"),
                    "commit_audit": {
                        "change_type": change_type("249", "creation"),
                        "committer": committer("version author")
                    },
                    "data": composition("server system_id")
                }],
                "audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("contribution committer")
                }
            }),
        )
        .await
        .expect("a contribution without any system_id → 201");
    assert_eq!(
        created.body["audit"]["system_id"].as_str(),
        Some(ferroehr::service::DEFAULT_SYSTEM_ID)
    );
    let instant = created.body["audit"]["time_committed"]["value"]
        .as_str()
        .expect("the CONTRIBUTION audit's time_committed")
        .to_owned();
    assert_eq!(
        audits_of(created.body).await,
        vec![(
            ferroehr::service::DEFAULT_SYSTEM_ID.to_owned(),
            "version author".to_owned(),
            instant
        )]
    );
}

/// A member `commit_audit` without `committer` is refused with the 400 (#3550),
/// never filled from the CONTRIBUTION audit: ITS-REST
/// `schemas/common/UpdateAudit.yaml` `required: [change_type, committer]` types
/// every `versions[i].commit_audit` (`schemas/ehr/UpdateVersion.yaml`), SM
/// `openehr_platform/master03-common_package.adoc` §Version Update Semantics
/// names only `time_committed` and `system_id` as server-generated, and
/// `operations/contribution_create.yaml` declares no 422. A `null` committer is
/// the same absence; the twin stating one commits.
#[tokio::test]
async fn member_commit_audit_without_committer_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    let body = |commit_audit: Value| {
        json!({
            "versions": [
                {
                    "lifecycle_state": change_type("532", "complete"),
                    "commit_audit": {
                        "change_type": change_type("249", "creation"),
                        "committer": committer("first author")
                    },
                    "data": composition("first member")
                },
                {
                    "lifecycle_state": change_type("532", "complete"),
                    "commit_audit": commit_audit,
                    "data": composition("second member")
                }
            ],
            "audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer("contribution committer")
            }
        })
    };

    for commit_audit in [
        json!({ "change_type": change_type("249", "creation") }),
        json!({ "change_type": change_type("249", "creation"), "committer": null }),
    ] {
        let err = svc
            .create_ehr_contribution(ehr_uuid, body(commit_audit.clone()))
            .await
            .expect_err("a member commit_audit without a committer is refused");
        assert_eq!(
            err.status,
            CallStatusType::PreconditionViolation,
            "{commit_audit}: the refusal is the 400, got {err:?}"
        );
        assert!(
            err.message
                .starts_with("versions[1]: commit_audit.committer is required"),
            "{commit_audit}: the refusal names the member, got {err:?}"
        );
    }

    svc.create_ehr_contribution(
        ehr_uuid,
        body(json!({
            "change_type": change_type("249", "creation"),
            "committer": committer("second author")
        })),
    )
    .await
    .expect("the twin stating its committer commits");
}

/// `get_contribution_resolved` (ITS-REST `Prefer: resolve_refs`): the
/// CONTRIBUTION's `versions` carry the full `ORIGINAL_VERSION` objects instead
/// of `OBJECT_REF`s; the unresolved form is unchanged.
#[tokio::test]
async fn contribution_resolve_refs() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    let comp_uid = svc
        .create_composition(ehr_uuid, uv(&composition("obs"), "249", None))
        .await
        .expect("composition_create")
        .version_uid();

    let all = svc
        .list_contributions(ehr_uuid, None, Page::all())
        .await
        .expect("list");
    let cid = all
        .last()
        .expect("a contribution")
        .parse::<uuid::Uuid>()
        .expect("contribution uuid");

    let plain = svc.get_contribution(ehr_uuid, cid).await.expect("plain");
    assert_eq!(
        plain["versions"][0]["_type"], "OBJECT_REF",
        "unresolved versions are OBJECT_REFs: {plain}"
    );

    let resolved = svc
        .get_contribution_resolved(ehr_uuid, cid)
        .await
        .expect("resolved");
    let v = &resolved["versions"][0];
    assert_eq!(
        v["_type"], "ORIGINAL_VERSION",
        "resolve_refs returns the full VERSION: {resolved}"
    );
    assert_eq!(
        v["uid"]["value"].as_str().expect("version uid"),
        comp_uid,
        "the resolved version is the committed composition version"
    );
    assert!(
        v["data"]["_type"] == "COMPOSITION",
        "resolved version carries its data"
    );
}

/// A client-supplied CONTRIBUTION uid is honoured when unused, rejected as a
/// conflict when already in use, and rejected as unprocessable when malformed
/// (ITS-REST `contribution_create`; RM common master06 §Contributions, the CONTRIBUTION `uid`).
#[tokio::test]
async fn contribution_supplied_uid() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let wanted = uuid::Uuid::now_v7();
    let mut body = serde_json::json!({
        "uid": { "_type": "HIER_OBJECT_ID", "value": wanted.to_string() },
        "versions": [{
            "data": composition("obs"),
            "lifecycle_state": { "code_string": "532", "terminology_id": { "value": "openehr" } },
            "commit_audit": { "change_type": { "value": "creation",
                "defining_code": { "code_string": "249", "terminology_id": { "value": "openehr" } } },
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "T" } }
        }],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": { "_type": "PARTY_IDENTIFIED", "name": "T" } }
    });
    let resp = svc
        .create_ehr_contribution(ehr_uuid, body.clone())
        .await
        .expect("supplied uid accepted");
    assert_eq!(
        resp.body["uid"]["value"].as_str(),
        Some(wanted.to_string().as_str()),
        "the supplied uid is the stored uid"
    );

    // Re-using the uid conflicts.
    body["versions"][0]["data"] = composition("obs2");
    let dup = svc
        .create_ehr_contribution(ehr_uuid, body)
        .await
        .expect_err("duplicate uid rejected");
    assert!(dup.message.contains("already in use"), "got {dup:?}");
}

/// The combined EHR-existence + content-writability create gate
/// (`ensure_ehr_content_writable`) preserves the pre-fold error surface after
/// the two separate pool reads were collapsed into one `ehr_writability` round
/// trip: an unknown EHR still maps to a `NotFound` (404, `ehr_id_does_not_exist`
/// — never a DB error or a conflict), and a deactivated EHR (`EHR_STATUS.is_modifiable =
/// false`) still maps to a conflict (409) — RM ehr master04 §EHR Creation /
/// §EHR Active Status.
#[tokio::test]
async fn create_composition_gate_error_surface_survives_the_writability_fold() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // (1) Unknown EHR → 404 `ehr_id_does_not_exist` (the existence signal of
    // the folded query), never a conflict and never a driver error.
    let ghost = "00000000-0000-7000-8000-0000000000fe"
        .parse::<ferroehr::ids::EhrId>()
        .expect("uuid");
    let missing = svc
        .create_composition(ghost, uv(&composition("obs"), "249", None))
        .await
        .expect_err("unknown EHR rejected");
    assert!(
        matches!(missing, ServiceError::NotFound(_)),
        "unknown ehr_id → 404, got {missing:?}"
    );

    // A live, modifiable EHR accepts a composition (the fold does not falsely
    // block — is_modifiable = None/true → writable).
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    svc.create_composition(ehr_uuid, uv(&composition("obs"), "249", None))
        .await
        .expect("modifiable EHR accepts a composition");

    // (2) Deactivate the EHR (EHR_STATUS.is_modifiable = false) and retry: the
    // content write is refused with a conflict (the modifiability signal of the
    // folded query, checked after existence).
    let status_uid = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("current EHR_STATUS")["uid"]["value"]
        .as_str()
        .expect("status uid")
        .to_owned();
    let deactivated = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF" },
        "is_queryable": true,
        "is_modifiable": false
    });
    svc.replace_ehr_status(ehr_uuid, uv(&deactivated, "251", Some(&status_uid)))
        .await
        .expect("EHR_STATUS deactivation");

    let blocked = svc
        .create_composition(ehr_uuid, uv(&composition("obs2"), "249", None))
        .await
        .expect_err("non-modifiable EHR blocks content writes");
    let ServiceError::Conflict(sm) = &blocked else {
        panic!("is_modifiable = false → 409 conflict, got {blocked:?}");
    };
    assert!(sm.message.contains("not modifiable"), "got {}", sm.message);
}

/// One valid version per lineage at any instant (RM common master06 §The
/// 'Virtual Version Tree') holds by construction in an append-only store: each
/// commit takes the transaction timestamp, so a burst of sequential updates
/// leaves one head row per object and ZERO pairs of one lineage sharing a
/// commit instant — asserted with the same lineage-pair query the admin
/// archive load audits with.
#[tokio::test]
async fn one_version_of_a_lineage_is_in_force_at_any_instant() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    let created = svc
        .create_composition(ehr_uuid, uv(&composition("obs"), "249", None))
        .await
        .expect("create")
        .version_uid();
    let mut preceding = created;
    for i in 0..8 {
        preceding = svc
            .update_composition(
                ehr_uuid,
                preceding
                    .split("::")
                    .next()
                    .expect("vo id part")
                    .parse()
                    .expect("vo uuid"),
                uv(&composition(&format!("obs-v{i}")), "251", Some(&preceding)),
            )
            .await
            .expect("update")
            .version_uid();
    }

    let heads: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM vo_head WHERE ehr_id = $1 AND kind = 'COMPOSITION'",
    )
    .bind(ehr_uuid)
    .fetch_one(&pool)
    .await
    .expect("head-row count");
    assert_eq!(heads, 1, "exactly one head row per composition");

    // Validity is derived from `committed_at`, so what would make "the version
    // in force at an instant" unanswerable is two versions of one lineage
    // sharing an instant. The write path takes the transaction timestamp per
    // commit, so no two can — and this is the audit that says so rather than
    // assuming it.
    let ambiguous: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM version a \
             JOIN version b ON a.vo_id = b.vo_id \
                 AND a.branch_number = b.branch_number \
                 AND (a.branch_number = 0 \
                      OR (a.creating_system_id = b.creating_system_id \
                          AND a.trunk_version = b.trunk_version)) \
                 AND a.sys_version < b.sys_version \
                 AND a.committed_at = b.committed_at \
             WHERE a.ehr_id = $1)",
    )
    .bind(ehr_uuid)
    .fetch_one(&pool)
    .await
    .expect("ambiguity audit");
    assert!(
        !ambiguous,
        "no lineage carries two versions committed at one instant"
    );
}

/// A contribution delete member targeting the `EHR_STATUS` is refused: the
/// status is mandatory on the EHR (RM ehr, EHR class: `ehr_status` 1..1), so
/// deleting the only one would break the EHR's own invariant.
#[tokio::test]
async fn contribution_cannot_delete_the_ehr_status() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let status_uid = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("status read")["uid"]["value"]
        .as_str()
        .expect("status version uid")
        .to_owned();

    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            serde_json::json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("523", "deleted"),
                        "committer": committer("author")
                    },
                    "lifecycle_state": lifecycle("523"),
                    "preceding_version_uid": { "value": status_uid }
                }],
                "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
            }),
        )
        .await
        .expect_err("EHR_STATUS delete member refused");
    assert!(
        err.message.contains("EHR_STATUS cannot be deleted"),
        "message names the mandatory-status ground, got {err:?}"
    );

    // the status is still there
    let still = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("status still present");
    assert_eq!(still["_type"].as_str(), Some("EHR_STATUS"));
}

/// A `523|deleted|` **lifecycle_state** on a version that carries data is
/// refused on every content-carrying route.
///
/// RM common master06 §Logical Deletion states deletion as one indivisible
/// procedure — "create a new Version in the normal way; delete its `_data_`
/// …; set the `_lifecycle_state_` value to the code for `deleted`; commit in
/// the normal way" — so a data-carrying version in the `deleted` state is not
/// producible by the spec's own procedure. Left unchecked, the read side
/// answers `204` for the resource while its node rows stay stored and
/// AQL-queryable.
///
/// Both twins are asserted: the refusal (CONTRIBUTION member AND direct
/// route), and the acceptance of the same content under `532|complete|`.
#[tokio::test]
async fn data_carrying_deleted_lifecycle_is_refused_on_both_routes() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let v1 = svc
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create")
        .version_uid();
    let vo_uuid: ferroehr::ids::VoId = v1
        .split("::")
        .next()
        .expect("vo id part")
        .parse()
        .expect("vo uuid");

    // (1) CONTRIBUTION member: a content-carrying modification claiming the
    // deleted state.
    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("251", "modification"),
                        "committer": committer("author")
                    },
                    "preceding_version_uid": { "value": v1 },
                    "lifecycle_state": lifecycle("523"),
                    "data": composition("deleted but full")
                }],
                "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
            }),
        )
        .await
        .expect_err("a data-carrying 523 member is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ) && err.message.contains("Logical Deletion"),
        "expected the 422 naming §Logical Deletion, got {err:?}"
    );

    // (2) The direct route reaches the same refusal through the
    // `openehr-version: lifecycle_state.code_string="523"` header.
    let mut direct = uv(&composition("deleted but full"), "251", Some(&v1));
    direct.lifecycle_state = lifecycle_state_coded("523");
    let err = svc
        .update_composition(ehr_uuid, vo_uuid, direct)
        .await
        .expect_err("a data-carrying 523 header is refused");
    let err = SmError::from(err);
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ) && err.message.contains("Logical Deletion"),
        "expected the 422 naming §Logical Deletion, got {err:?}"
    );

    // Neither refusal wrote anything: the composition is still at version 1
    // and still readable (no 523 row with node rows behind it).
    let live = svc
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("still live");
    assert_eq!(live["name"]["value"], "v1");

    // (3) The accepting twin: the identical content under 532|complete|.
    let v2 = svc
        .update_composition(
            ehr_uuid,
            vo_uuid,
            uv(&composition("deleted but full"), "251", Some(&v1)),
        )
        .await
        .expect("the same content commits as 532|complete|")
        .version_uid();
    assert!(v2.ends_with("::2"), "the accepted twin is trunk version 2");
}

/// A delete member declares the `523|deleted|` state its own change type
/// commits; a contradictory state is refused rather than silently dropped.
///
/// RM common master06 §Logical Deletion states deletion as ONE procedure —
/// "create a new Version in the normal way; delete its `_data_` …; set the
/// `_lifecycle_state_` value to the code for `deleted`; commit in the normal
/// way" — so a `523|deleted|` member declaring `532|complete|` asks for two
/// contradictory things at once, and committing it would tell the client its
/// instruction was followed when it was discarded. The code is the released
/// `400_CONTRIBUTION` change-control trigger ("the modification type does not
/// match the operation"), the same refusal family the direct DELETE wire's
/// committal header carries.
///
/// Three outcomes are asserted: the contradictory state refused (nothing
/// committed), an out-of-group state refused as the invariant violation it is,
/// and the `523`-declaring twin committing the deletion.
#[tokio::test]
async fn a_delete_member_with_a_contradictory_lifecycle_state_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let v1 = svc
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create")
        .version_uid();
    let vo_uuid: ferroehr::ids::VoId = v1
        .split("::")
        .next()
        .expect("vo id part")
        .parse()
        .expect("vo uuid");

    // One data-less 523 delete member, differing only in the state it declares.
    let delete_member = |state: Value| {
        json!({
            "versions": [{
                "commit_audit": {
                    "change_type": change_type("523", "deleted"),
                    "committer": committer("author")
                },
                "preceding_version_uid": { "value": v1 },
                "lifecycle_state": state
            }],
            "audit": { "change_type": change_type("523", "deleted"), "committer": committer("author") }
        })
    };

    // (1) The contradiction: a deletion claiming the complete state.
    let err = svc
        .create_ehr_contribution(ehr_uuid, delete_member(lifecycle("532")))
        .await
        .expect_err("a delete member declaring 532|complete| is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ) && err.message.contains("Logical Deletion")
            && err.message.contains("contradicts change_type"),
        "expected the 400 naming §Logical Deletion, got {err:?}"
    );

    // (2) An out-of-group state on the same member: the invariant 422.
    let err = svc
        .create_ehr_contribution(ehr_uuid, delete_member(lifecycle("999")))
        .await
        .expect_err("a delete member declaring an out-of-group state is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ) && err.message.contains("version_lifecycle_state"),
        "expected the 422 naming version_lifecycle_state, got {err:?}"
    );

    // Neither refusal committed anything: the composition is still live at v1.
    let live = svc
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("still live");
    assert_eq!(live["name"]["value"], "v1");

    // (3) The accepting twin: the same member declaring 523|deleted| commits,
    // and the latest read is then the deleted (204/null) path.
    svc.create_ehr_contribution(ehr_uuid, delete_member(lifecycle("523")))
        .await
        .expect("the 523-declaring delete member commits");
    let del = svc
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("composition_get deleted");
    assert!(
        del.is_null(),
        "a deleted composition reads as an empty body (204), got {del:?}"
    );
}

/// A `553|incomplete|` COMPOSITION missing mandatory data commits; the
/// `532|complete|` twin of the same content is refused; and `553` content that
/// is WRONG rather than merely missing stays refused.
///
/// RM common master06 §Incomplete Content (NOTE): "In the `incomplete` state, a
/// limited form of invalidity is allowed: mandatory attributes may be absent.
/// Concretely, single-valued attributes may have null values and container
/// attributes may be empty, even though they may have minimum existence and
/// cardinality respectively of one. All other validity requirements must be
/// satisfied. In other words, in an `incomplete` commit, data may be missing,
/// but it may not be wrong."
#[tokio::test]
async fn incomplete_admits_missing_composition_data_but_never_wrong_data() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // `COMPOSITION.composer` is 1..1 (absent here); the ADMIN_ENTRY below omits
    // its own mandatory `language`/`encoding`/`subject`; and `CLUSTER.items` is
    // 1..* (empty here). All three are exactly the shapes the NOTE describes —
    // "single-valued attributes may have null values and container attributes
    // may be empty, even though they may have minimum existence and cardinality
    // respectively of one".
    let mut missing = composition("partial notes");
    missing
        .as_object_mut()
        .expect("composition object")
        .remove("composer");
    missing["content"] = json!([{
        "_type": "ADMIN_ENTRY",
        "name": { "_type": "DV_TEXT", "value": "partial entry" },
        "archetype_node_id": "at0002",
        "data": {
            "_type": "ITEM_TREE",
            "name": { "_type": "DV_TEXT", "value": "tree" },
            "archetype_node_id": "at0003",
            "items": [{
                "_type": "CLUSTER",
                "name": { "_type": "DV_TEXT", "value": "empty cluster" },
                "archetype_node_id": "at0004",
                "items": []
            }]
        }
    }]);

    let member = |data: &Value, state: &str| {
        json!({
            "versions": [{
                "commit_audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("author")
                },
                "lifecycle_state": lifecycle(state),
                "data": data
            }],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        })
    };

    // (1) 553 accepts it.
    let created = svc
        .create_ehr_contribution(ehr_uuid, member(&missing, "553"))
        .await
        .expect("a 553 commit accepts absent mandatory data (master06 §Incomplete Content)");
    let ovid = first_version_uid(&created.body);
    assert_eq!(
        lifecycle_code_of(&svc, &ehr_id, vo_of(&ovid), &ovid).await,
        "553"
    );

    // (2) The 532 twin of the SAME content is refused — the relaxation is
    // scoped to the incomplete state and nothing else changed. The class is
    // the 400 row: a complete commit runs the strict typed door, and the
    // released `responses/422.yaml` scopes 422 to content that "could be
    // converted to a resource".
    let err = svc
        .create_ehr_contribution(ehr_uuid, member(&missing, "532"))
        .await
        .expect_err("the complete twin of incomplete content is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "expected 400 for the complete twin, got {err:?}"
    );

    // (3) WRONG data stays refused under 553: a `language` CODE_PHRASE whose
    // `code_string` is a number cannot hold a String, and a `category` code
    // outside the openEHR terminology group is not a missing value at all.
    let mut wrong_type = missing.clone();
    wrong_type["language"]["code_string"] = json!(42);
    let err = svc
        .create_ehr_contribution(ehr_uuid, member(&wrong_type, "553"))
        .await
        .expect_err("a 553 commit still refuses a wrongly-typed value");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ),
        "expected 422 for wrongly-typed 553 content, got {err:?}"
    );

    let mut wrong_code = missing.clone();
    wrong_code["category"]["defining_code"]["code_string"] = json!("999");
    let err = svc
        .create_ehr_contribution(ehr_uuid, member(&wrong_code, "553"))
        .await
        .expect_err("a 553 commit still refuses an out-of-group code");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::ContentInvalid,
                ..
            }
        ),
        "expected 422 for an out-of-group code under 553, got {err:?}"
    );
}

/// The `553|incomplete|` relaxation is not COMPOSITION-only: master06
/// §Incomplete Content names "Unfinished content items (EHR Compositions,
/// Demographic Parties etc)" and states the implementation rule generally
/// ("with all existence and cardinality lower limits set to zero"). A FOLDER
/// committed incomplete without its mandatory `name` is accepted; the
/// `532|complete|` twin is refused.
#[tokio::test]
async fn incomplete_relaxes_the_folder_kind_too() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // `LOCATABLE.name` is 1..1 — absent here.
    let nameless = json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1"
    });
    let body = |lifecycle_code: &str| {
        json!({
            "versions": [{
                "commit_audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("author")
                },
                "lifecycle_state": lifecycle(lifecycle_code),
                "data": nameless
            }],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        })
    };

    svc.create_ehr_contribution(ehr_uuid, body("553"))
        .await
        .expect("a 553 FOLDER may omit its mandatory name");

    // The complete twin's class is the 400 row — the strict typed door, per
    // the released `responses/422.yaml` scope ("could be converted").
    let err = svc
        .create_ehr_contribution(ehr_uuid, body("532"))
        .await
        .expect_err("the complete twin is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "expected 400 for the complete FOLDER twin, got {err:?}"
    );
}

/// `UPDATE_VERSION.lifecycle_state` is REQUIRED on the CONTRIBUTION wire — SM
/// `master03-common_package.adoc` §Version Update Semantics ("The
/// `lifecycle_state` must be supplied in all cases") + the released
/// `UpdateVersion` schema's `required` list. A member that omits it is refused
/// as the shape failure it is; the twin that states it commits.
#[tokio::test]
async fn contribution_member_without_lifecycle_state_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [{
                    "commit_audit": {
                        "change_type": change_type("249", "creation"),
                        "committer": committer("author")
                    },
                    "data": composition("no lifecycle")
                }],
                "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
            }),
        )
        .await
        .expect_err("a member without lifecycle_state is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ) && err.message.contains("lifecycle_state"),
        "expected the 400 naming lifecycle_state, got {err:?}"
    );

    // The accepting twin: the same member stating its lifecycle.
    svc.create_ehr_contribution(
        ehr_uuid,
        json!({
            "versions": [{
                "commit_audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("author")
                },
                "lifecycle_state": lifecycle("532"),
                "data": composition("with lifecycle")
            }],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        }),
    )
    .await
    .expect("the member that states its lifecycle_state commits");
}

/// `other_input_version_uids` is not a member of the released commit wire:
/// ITS-REST `UpdateVersion.yaml` declares no such property and
/// `NewContribution.versions` items are `UpdateVersion`, so the merge commit
/// has no released shape — the same absence the import commit has. Merge
/// provenance is produce-only — `OriginalVersion.yaml`
/// declares it on reads. A member carrying it is refused; the twin without it
/// commits and serves no merge provenance.
#[tokio::test]
async fn merge_provenance_is_refused_on_the_commit_wire() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let v1 = svc
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create")
        .version_uid();

    let member = |extra: Option<Value>| {
        let mut version = json!({
            "commit_audit": {
                "change_type": change_type("251", "modification"),
                "committer": committer("author")
            },
            "preceding_version_uid": { "value": v1 },
            "lifecycle_state": lifecycle("532"),
            "data": composition("v2")
        });
        if let Some(extra) = extra {
            version["other_input_version_uids"] = extra;
        }
        json!({
            "versions": [version],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        })
    };

    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            member(Some(json!([
                { "value": "0198f1f1-0000-7000-8000-000000000001::sysA.example.org::3" }
            ]))),
        )
        .await
        .expect_err("an undeclared merge-provenance member is refused");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ) && err.message.contains("other_input_version_uids"),
        "expected the 400 naming other_input_version_uids, got {err:?}"
    );

    // The accepting twin commits, and the served ORIGINAL_VERSION carries no
    // merge provenance (`Is_merged_validity` — a locally committed version is
    // never a merge).
    let committed = svc
        .create_ehr_contribution(ehr_uuid, member(None))
        .await
        .expect("the same member without the undeclared property commits");
    let v2 = first_version_uid(&committed.body);
    let ov = read_version(&svc, &ehr_id, vo_of(&v2), &v2).await;
    assert!(
        ov.get("other_input_version_uids").is_none(),
        "a locally committed version carries no merge provenance, got {ov:?}"
    );
}

/// None of the three identity-bearing keys of an `IMPORTED_VERSION` shape is a
/// member of the released commit wire: ITS-REST `UpdateVersion.yaml` declares
/// exactly six properties (`preceding_version_uid`, `signature`,
/// `lifecycle_state`, `attestations`, `data`, `commit_audit`) and
/// `NewContribution.versions` items are `UpdateVersion` with no `oneOf` and no
/// discriminator, so the import commit has no released shape at all. RM common
/// master06 §Copying puts it behind
/// `VERSIONED_OBJECT.commit_imported_version` ("Details of version id etc come
/// from the `ORIGINAL_VERSION`"), realized here by the EHR-Extract import route.
///
/// A member carrying `item`, `uid`, or a `_type` naming any other class is
/// refused naming the key — accepting it would commit a DECLARED FOREIGN
/// version as a locally created `ORIGINAL_VERSION` under a freshly minted local
/// uid, discarding the identity and provenance the client declared. The
/// accepting twins: the same member without them, and the same member
/// self-tagged `_type: ORIGINAL_VERSION` (a legal self-tag — ITS-REST
/// `Resources.md`: the value "MUST be the uppercase class name from the RM
/// specification").
#[tokio::test]
async fn foreign_version_identity_is_refused_on_the_commit_wire() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let v1 = svc
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create")
        .version_uid();

    // The foreign version a client would be declaring: an ORIGINAL_VERSION
    // created by another system, in the `::`-separated OBJECT_VERSION_ID form
    // (BASE `object_version_id.adoc`).
    let foreign_uid = "0198f1f1-0000-7000-8000-000000000042::sysA.example.org::3";
    let member = |extra: Option<(&str, Value)>| {
        let mut version = json!({
            "commit_audit": {
                "change_type": change_type("251", "modification"),
                "committer": committer("author")
            },
            "preceding_version_uid": { "value": v1 },
            "lifecycle_state": lifecycle("532"),
            "data": composition("v2")
        });
        if let Some((key, value)) = extra {
            version[key] = value;
        }
        json!({
            "versions": [version],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        })
    };

    for (key, value) in [
        (
            "item",
            json!({
                "_type": "ORIGINAL_VERSION",
                "uid": { "_type": "OBJECT_VERSION_ID", "value": foreign_uid },
                "data": composition("foreign")
            }),
        ),
        (
            "uid",
            json!({ "_type": "OBJECT_VERSION_ID", "value": foreign_uid }),
        ),
        ("_type", json!("IMPORTED_VERSION")),
    ] {
        let err = svc
            .create_ehr_contribution(ehr_uuid, member(Some((key, value))))
            .await
            .expect_err("a member declaring a foreign version identity is refused");
        assert!(
            matches!(
                err,
                SmError {
                    status: CallStatusType::PreconditionViolation,
                    ..
                }
            ) && err.message.contains(key),
            "expected the 400 naming {key}, got {err:?}"
        );
    }

    // Twin 1: the same member with none of the three keys commits.
    let committed = svc
        .create_ehr_contribution(ehr_uuid, member(None))
        .await
        .expect("the same member without the undeclared keys commits");
    let v2 = first_version_uid(&committed.body);
    let ov = read_version(&svc, &ehr_id, vo_of(&v2), &v2).await;
    assert_eq!(
        ov["_type"], "ORIGINAL_VERSION",
        "a locally committed version is an ORIGINAL_VERSION, got {ov:?}"
    );
    assert!(
        ov.get("item").is_none(),
        "a locally committed version wraps nothing, got {ov:?}"
    );
    // The identity is this repository's, never the declared foreign one.
    assert!(
        !ov["uid"]["value"]
            .as_str()
            .expect("uid")
            .contains("sysA.example.org"),
        "the committed version must not carry the foreign creating system, got {ov:?}"
    );

    // Twin 2: the LEGAL self-tag — `_type: ORIGINAL_VERSION` names the class
    // this wire commits, so it is accepted.
    svc.create_ehr_contribution(
        ehr_uuid,
        json!({
            "versions": [{
                "_type": "ORIGINAL_VERSION",
                "commit_audit": {
                    "change_type": change_type("251", "modification"),
                    "committer": committer("author")
                },
                "preceding_version_uid": { "value": v2 },
                "lifecycle_state": lifecycle("532"),
                "data": composition("v3")
            }],
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
        }),
    )
    .await
    .expect("a member self-tagged as the class this wire commits is accepted");
}

/// THE CLOSED MEMBER READ (#1753): an undeclared key on a CONTRIBUTION
/// version member is refused with the member index in the path — the released
/// commit wire declares exactly six member properties (ITS-REST
/// `UpdateVersion.yaml`) plus the adjudicated `_type` self-tag, and the strict
/// reader discipline (post-#1702) admits nothing else silently. The valid
/// twin is every accepted member in this suite.
#[tokio::test]
async fn an_undeclared_contribution_member_key_is_refused_with_its_path() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    let mut member = json!({
        "commit_audit": {
            "change_type": change_type("249", "creation"),
            "committer": committer("author")
        },
        "lifecycle_state": change_type("532", "complete"),
        "data": composition("closed member read")
    });
    member
        .as_object_mut()
        .expect("member object")
        .insert("contribution".to_owned(), json!({"id": "x"}));

    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [member],
                "audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("author")
                }
            }),
        )
        .await
        .expect_err("an undeclared member key must be refused");
    assert!(
        err.message.contains("versions[0]/contribution"),
        "the refusal names the key at its member path, got {err:?}"
    );
}

/// The CONTRIBUTION's own `audit.change_type` is REQUIRED and never derived.
///
/// RM common `UML/classes/org.openehr.rm.common.audit_details.adoc` §Attributes
/// types `change_type` 1..1 on the mandatory `CONTRIBUTION.audit`, and the
/// released commit schema requires it on the wire
/// (`specifications/schemas/ehr/NewContribution.yaml` `required: [versions,
/// audit]` over `specifications/schemas/common/UpdateAudit.yaml` `required:
/// [change_type, committer]`). The ITS-REST docs text is silent on the
/// contribution BODY — its "None of these headers are mandatory" sentence
/// governs the direct routes' header merge — so the released OAS grounds the
/// requirement. master06 §Contributions calls the aggregate value approximate
/// and "not expected to be used as a computable value", which is precisely why
/// the server must not invent one under the client's name.
#[tokio::test]
async fn contribution_audit_change_type_is_required_not_derived() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    let member = || {
        json!({
            "commit_audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer("author")
            },
            "lifecycle_state": change_type("532", "complete"),
            "data": composition("audit twin")
        })
    };

    // The invalid twin: an audit that names a committer but no change type.
    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [member()],
                "audit": { "committer": committer("author") }
            }),
        )
        .await
        .expect_err("a CONTRIBUTION audit without a change type must be refused");
    assert_eq!(
        err.status,
        CallStatusType::ContentInvalid,
        "the refusal is the 422 content-invalid row, got {err:?}"
    );
    assert!(
        err.message.contains("CONTRIBUTION.audit.change_type"),
        "the refusal names the missing attribute, got {err:?}"
    );

    // The committer sibling (#1817 — the same UpdateAudit.yaml `required`
    // member): an audit naming a change type but NO committer is refused; the
    // server never invents the committing identity. The refusal is the 400
    // (#3550): a body missing a schema-required member is syntactically
    // invalid content (`responses/400_CONTRIBUTION.yaml`), and
    // `operations/contribution_create.yaml` declares no 422.
    let err = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [member()],
                "audit": { "change_type": change_type("249", "creation") }
            }),
        )
        .await
        .expect_err("a CONTRIBUTION audit without a committer must be refused");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "the refusal is the 400 row, got {err:?}"
    );
    assert!(
        err.message.contains("CONTRIBUTION.audit.committer"),
        "the refusal names the missing attribute, got {err:?}"
    );

    // …and an entirely absent audit is the same refusal (the change type is
    // absent either way).
    let err = svc
        .create_ehr_contribution(ehr_uuid, json!({ "versions": [member()] }))
        .await
        .expect_err("a CONTRIBUTION with no audit at all must be refused");
    assert_eq!(err.status, CallStatusType::ContentInvalid, "got {err:?}");

    // The valid twin: the client states its own change type and the commit
    // succeeds, storing that code verbatim.
    let created = svc
        .create_ehr_contribution(
            ehr_uuid,
            json!({
                "versions": [member()],
                "audit": {
                    "change_type": change_type("249", "creation"),
                    "committer": committer("author")
                }
            }),
        )
        .await
        .expect("a CONTRIBUTION stating its change type commits");
    assert_eq!(
        created.body["audit"]["change_type"]["defining_code"]["code_string"], "249",
        "the client's own change type is stored verbatim"
    );
}

/// Every member-scoped CONTRIBUTION refusal names `versions[i]` (#2590):
/// measured live, the declared-key check named the member while the data
/// parse and validation refusals did not — on the one route whose purpose is
/// a multi-member change set, a client could not tell which member failed.
#[tokio::test]
async fn member_scoped_refusals_name_the_offending_version_index() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr = ehr_id.parse().expect("ehr uuid");

    let good = json!({
        "data": composition("attributed-good"),
        "lifecycle_state": { "_type": "DV_CODED_TEXT", "value": "complete", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "532" } },
        "commit_audit": { "change_type": change_type("249", "creation"), "committer": committer("conformance tester") }
    });
    let audit = json!({
        "change_type": change_type("249", "creation"),
        "committer": committer("conformance tester")
    });

    // (a) the second member's data fails the strict canonical parse
    // (`language` removed) — the 400 names versions[1].
    let mut broken = composition("attributed-parse");
    broken.as_object_mut().expect("object").remove("language");
    let bad_parse = json!({
        "data": broken,
        "lifecycle_state": good["lifecycle_state"],
        "commit_audit": good["commit_audit"]
    });
    let err = svc
        .create_ehr_contribution(
            ehr,
            json!({ "audit": audit, "versions": [good.clone(), bad_parse] }),
        )
        .await
        .expect_err("a non-decoding member is refused");
    let sm = err;
    assert!(
        sm.message.contains("versions[1]"),
        "the parse refusal names the member: {}",
        sm.message
    );

    // (b) the second member's change_type contradicts its shape — 422 naming
    // versions[1].
    let bad_change = json!({
        "data": composition("attributed-change"),
        "lifecycle_state": good["lifecycle_state"],
        "preceding_version_uid": format!("{}::ferroehr.local::1", uuid::Uuid::nil()),
        "commit_audit": { "change_type": change_type("249", "creation"), "committer": committer("conformance tester") }
    });
    let err = svc
        .create_ehr_contribution(
            ehr,
            json!({ "audit": audit, "versions": [good, bad_change] }),
        )
        .await
        .expect_err("creation with a preceding version is refused");
    let sm = err;
    assert!(
        sm.message.contains("versions[1]"),
        "the change-type refusal names the member: {}",
        sm.message
    );

    // (c) the second member names a template the store does not hold — the
    // validation 422 names versions[1].
    let mut templated = composition("attributed-template");
    templated["archetype_details"]["template_id"] =
        json!({ "_type": "TEMPLATE_ID", "value": "no_such_template.v1" });
    let good = json!({
        "data": composition("attributed-good-2"),
        "lifecycle_state": { "_type": "DV_CODED_TEXT", "value": "complete", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "532" } },
        "commit_audit": { "change_type": change_type("249", "creation"), "committer": committer("conformance tester") }
    });
    let bad_template = json!({
        "data": templated,
        "lifecycle_state": good["lifecycle_state"],
        "commit_audit": good["commit_audit"]
    });
    let err = svc
        .create_ehr_contribution(
            ehr,
            json!({ "audit": audit, "versions": [good, bad_template] }),
        )
        .await
        .expect_err("an unknown template is refused");
    let sm = err;
    assert!(
        sm.message.contains("versions[1]") && sm.message.contains("no_such_template.v1"),
        "the template refusal names the member and the template: {}",
        sm.message
    );
}

/// The [`ehr_status`] shape with an explicit `is_modifiable` (queryable stays
/// `true`) — the mixed-CONTRIBUTION gate fixtures.
fn ehr_status_modifiable(modifiable: bool) -> Value {
    let mut status = ehr_status(true);
    status["is_modifiable"] = json!(modifiable);
    status
}

/// An `ORIGINAL_VERSION` contribution member (raw wire shape).
fn member(data: Value, change: (&str, &str), preceding: Option<&str>) -> Value {
    let mut m = json!({
        "_type": "ORIGINAL_VERSION",
        "commit_audit": {
            "change_type": change_type(change.0, change.1),
            "committer": committer("author")
        },
        "lifecycle_state": change_type("532", "complete")
    });
    m["data"] = data;
    if let Some(p) = preceding {
        m["preceding_version_uid"] = json!({ "value": p });
    }
    m
}

/// The current `EHR_STATUS` version uid of an EHR.
async fn current_status_uid(svc: &FerroEhrService, ehr: ferroehr::ids::EhrId) -> String {
    svc.get_ehr_status_at_time(ehr, None)
        .await
        .expect("current EHR_STATUS")["uid"]["value"]
        .as_str()
        .expect("status uid")
        .to_owned()
}

/// A deactivating CONTRIBUTION may carry its final content updates: content
/// members beside an `EHR_STATUS` member setting `is_modifiable = false` are
/// accepted against an ACTIVE EHR (RM ehr master04 §EHR Active Status's own
/// first deactivation scenario — the death-related updates land in the act
/// that deactivates; the gate mechanics are spec-silent, adjudicated on
/// #2673), and the deactivation takes effect for every LATER write.
#[tokio::test]
async fn deactivating_contribution_carries_its_final_content() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");
    let status_uid = current_status_uid(&svc, ehr_uuid).await;

    let mixed = json!({
        "versions": [
            member(composition("final note"), ("249", "creation"), None),
            member(ehr_status_modifiable(false), ("251", "modification"), Some(&status_uid)),
        ],
        "audit": { "change_type": change_type("251", "modification"), "committer": committer("author") }
    });
    svc.create_ehr_contribution(ehr_uuid, mixed)
        .await
        .expect("write-then-deactivate in one CONTRIBUTION is accepted");

    // The deactivation holds from the next act on.
    let blocked = svc
        .create_composition(ehr_uuid, uv(&composition("too late"), "249", None))
        .await
        .expect_err("the EHR is deactivated after the mixed commit");
    assert!(
        matches!(blocked, ServiceError::Conflict(_)),
        "post-deactivation content write → 409, got {blocked:?}"
    );
}

/// The gate over a deactivated EHR, order-independently: content beside a
/// REACTIVATING `EHR_STATUS` member is accepted (content listed FIRST, so list
/// order proves nothing); content beside a still-`false` status member — and
/// content alone — stay refused with the existing 409 (adjudication #2673).
#[tokio::test]
async fn reactivating_contribution_unlocks_its_own_content() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    // Deactivate up front.
    let status_uid = current_status_uid(&svc, ehr_uuid).await;
    svc.replace_ehr_status(
        ehr_uuid,
        uv(&ehr_status_modifiable(false), "251", Some(&status_uid)),
    )
    .await
    .expect("EHR_STATUS deactivation");

    // (1) Content alone → refused.
    let content_only = json!({
        "versions": [ member(composition("blocked"), ("249", "creation"), None) ],
        "audit": { "change_type": change_type("249", "creation"), "committer": committer("author") }
    });
    let blocked = svc
        .create_ehr_contribution(ehr_uuid, content_only)
        .await
        .expect_err("content against a deactivated EHR is refused");
    assert!(
        blocked.message.contains("not modifiable"),
        "the deactivated-EHR refusal names the cause: {blocked:?}"
    );

    // (2) Content beside a status member that KEEPS is_modifiable = false →
    // still refused (no reactivation happens).
    let status_uid = current_status_uid(&svc, ehr_uuid).await;
    let still_off = json!({
        "versions": [
            member(composition("still blocked"), ("249", "creation"), None),
            member(ehr_status_modifiable(false), ("251", "modification"), Some(&status_uid)),
        ],
        "audit": { "change_type": change_type("251", "modification"), "committer": committer("author") }
    });
    svc.create_ehr_contribution(ehr_uuid, still_off)
        .await
        .expect_err("a non-reactivating mixed CONTRIBUTION stays refused");

    // (3) Content FIRST, the reactivating status member SECOND → accepted:
    // the rule reads the atomic change set, not the list order.
    let status_uid = current_status_uid(&svc, ehr_uuid).await;
    let reactivating = json!({
        "versions": [
            member(composition("welcome back"), ("249", "creation"), None),
            member(ehr_status_modifiable(true), ("251", "modification"), Some(&status_uid)),
        ],
        "audit": { "change_type": change_type("251", "modification"), "committer": committer("author") }
    });
    svc.create_ehr_contribution(ehr_uuid, reactivating)
        .await
        .expect("a reactivating mixed CONTRIBUTION is accepted");

    // The EHR is active again: a plain content write goes through.
    svc.create_composition(ehr_uuid, uv(&composition("active again"), "249", None))
        .await
        .expect("content write after the reactivating commit");
}

/// The content-write gate reads `is_modifiable` INSIDE the commit transaction
/// under a row lock: a deactivation already holding the EHR row when the
/// content CONTRIBUTION arrives commits first, and the content commit
/// observes the flipped flag — the pre-transaction read this replaces saw the
/// still-`true` committed value and let the content land after the
/// deactivation. (The flip is a raw column update here: the pin targets the
/// gate's locked read, and the promoted `ehr.is_modifiable` column is exactly
/// what the gate reads.)
#[tokio::test]
async fn concurrent_deactivation_is_seen_by_the_content_commit() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    // An open transaction flips the flag and HOLDS the row lock.
    let mut flip = db.pool().begin().await.expect("begin the flip tx");
    sqlx::query("UPDATE ehr SET is_modifiable = false WHERE id = $1")
        .bind(ehr_uuid)
        .execute(&mut *flip)
        .await
        .expect("uncommitted deactivation");

    // The content CONTRIBUTION must block on the gate's row lock, then see
    // the committed flip and refuse.
    let pool = db.pool();
    let racing = tokio::spawn(async move {
        let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
        let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");
        let content = json!({
            "versions": [ member(composition("raced"), ("249", "creation"), None) ],
            "audit": { "change_type": change_type("249", "creation"), "committer": committer("author") }
        });
        svc.create_ehr_contribution(ehr_uuid, content).await
    });
    // Give the racing commit time to reach the in-transaction gate and block.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    flip.commit().await.expect("commit the deactivation");

    let refused = racing
        .await
        .expect("the racing task completes")
        .expect_err("the content commit observes the concurrent deactivation");
    assert!(
        refused.message.contains("not modifiable"),
        "the refusal names the deactivation: {refused:?}"
    );
}

/// A create body carrying its own `uid` succeeds, and the served identity is
/// the SERVER-minted `OBJECT_VERSION_ID` — a version identifier names a
/// version that does not exist until this commit mints it, and no released
/// operation gives a create-body uid any semantics (`composition_create.yaml`
/// states nothing; BASE `architecture_overview` master09 §Levels of
/// Identification defines what the stored uid must carry — the containing
/// VERSION's `OBJECT_VERSION_ID`; the adjudication is #2918, the party twin
/// #1578).
#[tokio::test]
async fn create_body_uid_is_replaced_by_the_minted_identity() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");

    let mut body = composition("client uid");
    body["uid"] = json!({
        "_type": "OBJECT_VERSION_ID",
        "value": "11111111-1111-1111-1111-111111111111::client.example::7"
    });
    let committed = svc
        .create_composition(ehr_uuid, uv(&body, "249", None))
        .await
        .expect("a schema-valid body uid never refuses the create");
    let minted = committed.version_uid();
    assert_ne!(
        minted, "11111111-1111-1111-1111-111111111111::client.example::7",
        "the identity is server-minted"
    );

    let served = svc
        .get_composition_at_version(ehr_uuid, minted.parse().expect("OBJECT_VERSION_ID"))
        .await
        .expect("the committed version exists");
    assert_eq!(
        served["uid"]["value"],
        json!(minted),
        "the served body uid IS the identifier the create returned"
    );
}

/// The EHR-state gate answers before content validation (#3257): a
/// CONTRIBUTION carrying a DEFECTIVE composition into a deactivated EHR is
/// refused as the `409` state conflict, not as the `422` its body would earn,
/// so the caller is told the record cannot be written rather than sent to
/// repair a body that would be refused anyway. RM ehr master04 §EHR Active
/// Status; the gate mechanics are spec-silent, adjudicated on #2673.
#[tokio::test]
async fn a_deactivated_ehr_reports_the_state_conflict_before_a_content_defect() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid: ferroehr::ids::EhrId = ehr_id.parse().expect("ehr uuid");
    let status_uid = current_status_uid(&svc, ehr_uuid).await;
    svc.replace_ehr_status(
        ehr_uuid,
        uv(&ehr_status_modifiable(false), "251", Some(&status_uid)),
    )
    .await
    .expect("EHR_STATUS deactivation");

    // A composition missing its mandatory `language`: a 422 on a writable EHR.
    let mut defective = composition("broken");
    defective
        .as_object_mut()
        .expect("object")
        .remove("language");
    let set = json!({
        "versions": [ member(defective, ("249", "creation"), None) ],
        "audit": { "change_type": change_type("249", "creation"), "committer": committer("author") }
    });
    let refused = svc
        .create_ehr_contribution(ehr_uuid, set)
        .await
        .expect_err("content into a deactivated EHR is refused");
    assert_eq!(
        refused.status,
        CallStatusType::Conflict,
        "the state conflict comes first, got {refused:?}"
    );
    assert!(
        refused.message.contains("not modifiable"),
        "the refusal names the EHR state: {refused:?}"
    );

    // The same defect on a REACTIVATING set reaches content validation.
    let status_uid = current_status_uid(&svc, ehr_uuid).await;
    let mut defective = composition("broken");
    defective
        .as_object_mut()
        .expect("object")
        .remove("language");
    let reactivating = json!({
        "versions": [
            member(defective, ("249", "creation"), None),
            member(ehr_status_modifiable(true), ("251", "modification"), Some(&status_uid)),
        ],
        "audit": { "change_type": change_type("251", "modification"), "committer": committer("author") }
    });
    let refused = svc
        .create_ehr_contribution(ehr_uuid, reactivating)
        .await
        .expect_err("the defective body is refused");
    assert_ne!(
        refused.status,
        CallStatusType::Conflict,
        "a reactivating set is judged on its content, got {refused:?}"
    );
}
