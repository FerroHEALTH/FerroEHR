// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM EHR-package cross-version + tag-scoping duties, end-to-end against a
//! real `PostgreSQL` 18 (shared testkit harness) — A1 rm-ehr chapter.
//!
//! Spec: RM ehr `org.openehr.rm.ehr.versioned_composition.adoc`
//! (`Archetype_node_id_valid`, `Persistent_validity` — a versioned composition
//! cannot switch archetype or persistence category across versions) and
//! `org.openehr.rm.ehr.ehr.adoc` `EHR.tags` + master04 §Tags ("Tag `_target_`
//! values can only be within the same EHR").

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;
use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_rm::prelude::PartyProxy;
use serde_json::{Value, json};
use uuid::Uuid;

fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: preceding.map(|p| p.parse().expect("OBJECT_VERSION_ID")),
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: None,
            committer: openehr_its::json::from_canonical_value::<PartyProxy>(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "ehr-package tester" }),
            )
            .expect("committer"),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        signature: None,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// A minimal *valid* RM COMPOSITION with a chosen archetype + category.
fn composition(archetype: &str, category_code: &str, category_label: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": archetype,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "ehr-package test" },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "territory": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL"
        },
        "category": {
            "_type": "DV_CODED_TEXT",
            "value": category_label,
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": category_code
            }
        },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "ehr-package tester" }
    })
}

const ENCOUNTER: &str = "openEHR-EHR-COMPOSITION.encounter.v1";
const REPORT: &str = "openEHR-EHR-COMPOSITION.report.v1";

/// `VERSIONED_COMPOSITION.Archetype_node_id_valid`: an update whose root
/// `archetype_node_id` differs from the first version's is a 422; the same
/// archetype updates fine.
#[tokio::test]
async fn versioned_composition_cannot_switch_archetype() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");
    let v1 = svc
        .create_composition(
            ehr,
            uv(&composition(ENCOUNTER, "433", "event"), "249", None),
        )
        .await
        .expect("v1")
        .version_uid();
    let vo: ferroehr::ids::VoId = v1.split("::").next().unwrap().parse().unwrap();

    let err = svc
        .update_composition(
            ehr,
            vo,
            uv(&composition(REPORT, "433", "event"), "251", Some(&v1)),
        )
        .await
        .expect_err("switching archetype across versions must be rejected");
    match err {
        // The invariant is DATA on the violation, not a substring of prose.
        ServiceError::Unprocessable { violation: v, .. } => {
            assert_eq!(v.path(), Some("COMPOSITION.archetype_node_id"));
            assert_eq!(
                v.invariant(),
                Some("VERSIONED_COMPOSITION.Archetype_node_id_valid")
            );
        }
        other => panic!("expected ContentInvalid, got {other:?}"),
    }

    // Same archetype: fine.
    svc.update_composition(
        ehr,
        vo,
        uv(&composition(ENCOUNTER, "433", "event"), "251", Some(&v1)),
    )
    .await
    .expect("same-archetype update");
}

/// `VERSIONED_COMPOSITION.Persistent_validity`: flipping the category between
/// persistent (431) and event (433) across versions is a 422.
#[tokio::test]
async fn versioned_composition_cannot_flip_persistence() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");
    let v1 = svc
        .create_composition(
            ehr,
            uv(&composition(ENCOUNTER, "433", "event"), "249", None),
        )
        .await
        .expect("v1")
        .version_uid();
    let vo: ferroehr::ids::VoId = v1.split("::").next().unwrap().parse().unwrap();

    let err = svc
        .update_composition(
            ehr,
            vo,
            uv(
                &composition(ENCOUNTER, "431", "persistent"),
                "251",
                Some(&v1),
            ),
        )
        .await
        .expect_err("flipping is_persistent across versions must be rejected");
    match err {
        ServiceError::Unprocessable { violation: v, .. } => {
            assert_eq!(v.path(), Some("COMPOSITION.category"));
            assert_eq!(
                v.invariant(),
                Some("VERSIONED_COMPOSITION.Persistent_validity")
            );
        }
        other => panic!("expected ContentInvalid, got {other:?}"),
    }
}

/// `EHR.tags`: "Tag `_target_` values can only be within the same EHR" — a
/// tag whose target versioned object lives in ANOTHER EHR is rejected, while
/// tagging one's own composition succeeds.
#[tokio::test]
async fn tag_targets_must_be_within_the_same_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_a = svc.create_ehr(None).await.expect("ehr A");
    let ehr_b = svc.create_ehr(None).await.expect("ehr B");
    let v1 = svc
        .create_composition(
            ehr_a,
            uv(&composition(ENCOUNTER, "433", "event"), "249", None),
        )
        .await
        .expect("composition in A")
        .version_uid();
    let vo_a: Uuid = v1.split("::").next().unwrap().parse().unwrap();

    let tag = vec![crate::item_tag_fixture::ehr_tag(
        "clin-proj-27a",
        None,
        None,
    )];
    // Tagging A's composition from A: accepted. The route family must match
    // the stored kind — the kind guard rejects an EHR_STATUS route for a
    // COMPOSITION target.
    svc.target_tags_replace(ehr_a, vo_a.to_string(), "COMPOSITION", tag.clone())
        .await
        .expect("own-EHR tag accepted");

    // Tagging A's composition from B: rejected — cross-EHR target (the type
    // matches, so the refusal is the same-EHR duty, not the kind guard).
    let err = svc
        .target_tags_replace(ehr_b, vo_a.to_string(), "COMPOSITION", tag.clone())
        .await
        .expect_err("cross-EHR tag target must be rejected");
    assert!(
        err.message.contains("same EHR"),
        "should cite the same-EHR duty, got: {}",
        err.message
    );
}

/// RM ehr `master04-ehr_package.adoc` §Tags, the chapter's one hard MUST:
/// tags "have no direct association with the objects they annotate. This has
/// the following consequences: ... they do not constitute part of the content
/// they annotate; accordingly, where they annotate versioned content, **they
/// do not cause re-versioning of the content**".
///
/// The wire-level twin is the CNF case
/// `I_ITS_REST_ITEM_TAGS.composition_tags_update-no_reversioning`, which can
/// only see the revision history and the VERSION's own `contribution`
/// reference — the released ITS-REST surfaces no contribution-listing
/// operation. This service-level test is where the stronger statement is
/// asserted: the EHR's CONTRIBUTION COUNT is unchanged by a tag write, a tag
/// replace, and a tag delete alike, so no change set was opened at all.
#[tokio::test]
async fn tagging_does_not_re_version_or_contribute() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");
    let v1 = svc
        .create_composition(
            ehr,
            uv(&composition(ENCOUNTER, "433", "event"), "249", None),
        )
        .await
        .expect("v1")
        .version_uid();
    let vo: ferroehr::ids::VoId = v1.split("::").next().unwrap().parse().unwrap();

    // The state a tag write must not disturb: the contribution set of the whole
    // EHR and the versioned object's revision history.
    let contributions_before = svc
        .contribution_count(ehr, None)
        .await
        .expect("contribution count before");
    let history_before = svc
        .composition_revision_history(ehr, vo)
        .await
        .expect("revision history before");
    assert_eq!(
        history_before["items"].as_array().map(Vec::len),
        Some(1),
        "one commit, one REVISION_HISTORY_ITEM"
    );

    // Write, replace and delete tags on both addressable collections — the
    // container's and the version's (RM `item_tag.adoc`: `target` "may be a
    // VERSIONED_OBJECT<T> or a VERSION<T>").
    for target in [vo.to_string(), v1.clone()] {
        svc.target_tags_replace(
            ehr,
            target.clone(),
            "COMPOSITION",
            vec![crate::item_tag_fixture::ehr_tag(
                "clin-proj-27a",
                Some("first"),
                None,
            )],
        )
        .await
        .expect("tag write");
        svc.target_tags_replace(
            ehr,
            target.clone(),
            "COMPOSITION",
            vec![crate::item_tag_fixture::ehr_tag(
                "clin-proj-27a",
                Some("second"),
                None,
            )],
        )
        .await
        .expect("tag replace");
        svc.target_tag_delete(ehr, target, "COMPOSITION", "clin-proj-27a".to_owned())
            .await
            .expect("tag delete");
    }

    // No new VERSION …
    let history_after = svc
        .composition_revision_history(ehr, vo)
        .await
        .expect("revision history after");
    assert_eq!(
        history_after, history_before,
        "tagging must not add, remove or alter a revision"
    );
    // … and no new CONTRIBUTION anywhere in the EHR.
    assert_eq!(
        svc.contribution_count(ehr, None)
            .await
            .expect("contribution count after"),
        contributions_before,
        "tagging must open no change set"
    );
}
