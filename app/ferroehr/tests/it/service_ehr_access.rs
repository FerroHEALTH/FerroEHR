// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `EHR_ACCESS` scheme-settings round trip end-to-end against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! `EHR_ACCESS` is a mandatory, versioned RM object and the access-decision
//! authority (RM `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class); its
//! `settings` are change-controlled like all content (RM ehr
//! `master04-ehr_package.adoc` §EHR Access). This test commits an `EHR_ACCESS`
//! version carrying the `ferroehr.access_control.v1` scheme settings through a
//! CONTRIBUTION and reads them back through the [`EhrAccessAdapter`], proving
//! the settings-subtype passes commit validation (`Scheme_valid`), the adapter
//! parses them, and the commit invalidated the per-EHR cache.
//!
//! The concrete scheme is our own design — no openEHR spec governs it.

use ferroehr::service::ehr::access_types::DefaultAccess;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use serde_json::{Value, json};

fn committer(name: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": name })
}

fn coded(code: &str, value: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code
        }
    })
}

/// An `EHR_ACCESS` object carrying `ferroehr.access_control.v1` scheme settings.
fn ehr_access_with_settings() -> Value {
    json!({
        "_type": "EHR_ACCESS",
        "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1",
        // Roots carry ARCHETYPED (LOCATABLE.Archetyped_valid).
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_ACCESS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Access" },
        "settings": {
            "_type": "FERROEHR_ACCESS_CONTROL_V1",
            "gate_keeper": "user:alice",
            "default_access": "restricted",
            "access_list": [
                { "principal": "user:bob", "access": "full" },
                { "principal": "role:nurse", "access": "restricted_below", "max_level": 2 }
            ],
            "privacy": {
                "default_level": 0,
                "composition_overrides": [
                    { "uid": "8849182c-82ad-4088-a07f-48ead4180515", "level": 3 }
                ]
            }
        }
    })
}

#[tokio::test]
async fn ehr_access_settings_round_trip_through_contribution() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // A fresh EHR gets the default (settings-less) EHR_ACCESS → default-open.
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");
    assert!(
        svc.current_ehr_access_settings(ehr_id)
            .await
            .expect("read settings")
            .is_none(),
        "a fresh EHR has no scheme settings (default-open)"
    );

    // The current EHR_ACCESS version's OBJECT_VERSION_ID (vo :: creating_system_id
    // :: version) — the modify's preceding_version_uid. The versioned-object id
    // comes from EHR.ehr_access; the first version is `1` and the creating system
    // is the service default (no tenant).
    let ehr = svc.ehr_object(ehr_id).await.expect("ehr object");
    let access_vo = ehr["ehr_access"]["id"]["value"]
        .as_str()
        .expect("EHR.ehr_access.id.value");
    let preceding = format!("{access_vo}::{DEFAULT_SYSTEM_ID}::1");

    // Commit a new EHR_ACCESS version carrying the scheme settings via a
    // CONTRIBUTION (the only EHR_ACCESS write path).
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "commit_audit": { "change_type": coded("251", "modification"), "committer": committer("alice") },
            "lifecycle_state": coded("532", "complete"),
            "preceding_version_uid": preceding,
            "data": ehr_access_with_settings()
        } ],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("alice") }
    });
    svc.create_ehr_contribution(ehr_id, contribution)
        .await
        .expect("EHR_ACCESS contribution with scheme settings must commit");

    // Read the settings back through the adapter (the commit invalidated the
    // cache, so this reflects the new version).
    let settings = svc
        .current_ehr_access_settings(ehr_id)
        .await
        .expect("read settings");
    let settings = (*settings)
        .as_ref()
        .expect("scheme settings present after commit");
    assert_eq!(settings.gate_keeper.as_deref(), Some("user:alice"));
    assert_eq!(settings.default_access, DefaultAccess::Restricted);
    assert_eq!(settings.access_list.len(), 2);
    assert_eq!(
        settings
            .privacy
            .level_for("8849182c-82ad-4088-a07f-48ead4180515"),
        3
    );
}
