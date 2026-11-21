// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The writer of a care relationship judged at the confidentiality of its
//! subject's record (#6141; draft access control decision procedure,
//! Writing Relationships, item 2): at `N` any authorised writer writes, at
//! `R` and `V` only a writer an `ACCESS_POLICY.relationship_writers` entry
//! names for that level, the administrator included, and every other write
//! is refused `403` naming the level. Our own design over the draft.

use serde_json::{Value, json};

use crate::care_bounds::{care_of, coded, interval, party, policy, relate};
use crate::policy_fixtures::{CARE_PROVIDER, care_provider, create, granting};
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::service::demographic::types::PartyKind;

/// Open settings whose marking is `level`, or none.
fn settings(level: Option<&str>) -> Value {
    let mut settings = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": coded(TERMINOLOGY, "open") });
    if let Some(level) = level {
        settings["marking"] = json!({ "_type": "CONFIDENTIALITY_MARKING",
            "confidentiality": coded("HL7_Confidentiality", level),
            "set_by": { "_type": "PARTY_IDENTIFIED", "name": "privacy officer" },
            "set_at": { "_type": "DV_DATE_TIME", "value": "2026-11-19T10:00:00Z" } });
    }
    settings
}

/// Commits `level` as the marking of `ehr` as the controller.
async fn mark(svc: &FerroEhrService, ehr: EhrId, level: Option<&str>) {
    let versions = svc
        .ehr_access_versions(ehr)
        .await
        .expect("the EHR_ACCESS versions read");
    let (current, _) = versions.first().expect("a current EHR_ACCESS version");
    svc.commit_ehr_access_settings(ehr, current, settings(level))
        .await
        .expect("the controller marks the record");
}

/// Asserts `result` is a `403` naming the level `level`.
fn refused<T: std::fmt::Debug>(result: Result<T, ServiceError>, level: &str) {
    let error = result.expect_err("a 403");
    assert!(
        matches!(&error, ServiceError::Forbidden(_)),
        "a 403, got {error:?}"
    );
    let text = format!("{error:?}");
    assert!(
        text.contains(&format!("marked {level}")) && text.contains("relationship_writers"),
        "it names the level and the missing rule: {text}"
    );
}

/// At `N` a writer no entry names writes; at `R` the writer the policy names
/// for `R` writes and the one it does not is refused; at `V`, which the
/// entry does not name, every writer is refused, the administrator included.
#[tokio::test]
async fn a_relationship_to_a_marked_record_needs_a_writer_named_for_its_level() {
    let (_db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    care_provider(&svc, CARE_PROVIDER).await;
    let named_for_r = json!({ "relationship_writers": [{
        "_type": "ACCESS_RELATIONSHIP_WRITER",
        "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                         "caller_kind": coded(TERMINOLOGY, "professional") }],
        "confidentialities": [coded("HL7_Confidentiality", "R")] }] });
    let first = svc
        .put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("a policy naming no writer")
        .version_uid;
    let (patient, _) = create(&svc, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    let (team, _) = create(&svc, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    let team = team.to_string();
    let ehr = crate::fixtures::create_ehr_for_party(&svc, patient).await;
    let stay = |from: &str| {
        care_of(
            patient,
            ("GROUP", &team),
            "ATND",
            &interval(from, Some("2026-12-31")),
        )
    };

    relate(&svc, &stay("2026-11-01"))
        .await
        .expect("at N any authorised writer writes");

    mark(&svc, ehr, Some("R")).await;
    refused(relate(&svc, &stay("2026-11-02")).await, "R");
    svc.put_access_policy(
        &PolicyPrecondition::Replace(first),
        &policy("P30D", &named_for_r),
    )
    .await
    .expect("a policy naming professionals for R");
    relate(&svc, &stay("2026-11-03"))
        .await
        .expect("a writer the policy names for R writes");

    mark(&svc, ehr, Some("V")).await;
    refused(relate(&svc, &stay("2026-11-04")).await, "V");

    mark(&svc, ehr, None).await;
    relate(&svc, &stay("2026-11-05"))
        .await
        .expect("a cleared marking is N again");
}
