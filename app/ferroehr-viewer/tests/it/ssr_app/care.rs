// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care relationships through the viewer's real service over the replayed
//! CDR: an EHR's Care tab, a person's and a carer's Care page, a
//! relationship's versions, an add and an end.

use super::{Viewer, id};

#[tokio::test]
async fn an_ehr_lists_its_subjects_live_care_and_the_ended_on_request() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let live = viewer.page(&cookie, &format!("/ehrs/{ehr}/care")).await;
    assert_eq!(live.status, http::StatusCode::OK, "{}", live.body);
    for words in [
        "Care relationships",
        "responsible party",
        "HL7_ParticipationType",
        "Direct",
    ] {
        assert!(
            live.main().contains(words),
            "says {words:?}:\n{}",
            live.main()
        );
    }
    assert!(
        !live.main().contains("2025-06-30"),
        "an ended relationship is left out by default:\n{}",
        live.main()
    );
    let all = viewer
        .page(&cookie, &format!("/ehrs/{ehr}/care?all=true"))
        .await;
    assert!(all.main().contains("2025-06-30"), "{}", all.main());
}

#[tokio::test]
async fn a_person_shows_its_care_and_a_carer_the_subjects_in_its_care() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let patient = viewer
        .page(&cookie, &format!("/parties/person/{}/care", id("patient")))
        .await;
    assert_eq!(patient.status, http::StatusCode::OK, "{}", patient.body);
    for words in [
        "Care relationships of this person",
        "Add a care relationship",
        "responsible party",
    ] {
        assert!(
            patient.main().contains(words),
            "says {words:?}:\n{}",
            patient.main()
        );
    }
    let carer = viewer
        .page(&cookie, &format!("/parties/person/{}/care", id("party")))
        .await;
    assert!(carer.main().contains("In their care"), "{}", carer.main());
    assert!(
        carer.main().contains(&format!("/care/{}", id("care"))),
        "the carer's list opens the relationship:\n{}",
        carer.main()
    );
}

#[tokio::test]
async fn a_relationship_shows_its_versions_with_their_audit_details() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, &format!("/care/{}", id("care"))).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "This care relationship",
        "responsible party",
        "Versions",
        "End today",
        "Delete",
    ] {
        assert!(
            page.main().contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
    assert!(
        page.main().contains(&format!("/care/{}%3A%3A", id("care"))),
        "the older version opens by its uid:\n{}",
        page.main()
    );
}

#[tokio::test]
async fn an_added_relationship_opens_its_page() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let patient = id("patient");
    let party = id("party");
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/parties/person/{patient}/care"),
            Some(&cookie),
            Some(&[
                ("intent", "add"),
                ("carer_kind", "PERSON"),
                ("carer_id", party.as_str()),
                ("role", "SPRF"),
                ("issuer", "urn:oid:2.999.7.9"),
                ("provider", "dev"),
                ("from", "2026-03-01"),
                ("to", "2026-12-31"),
            ]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::FOUND, "{}", answer.body);
    assert!(
        answer.location().starts_with("/care/"),
        "{}",
        answer.location()
    );
}

#[tokio::test]
async fn an_ended_relationship_returns_to_its_page() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ending = id("care_end");
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/care/{ending}"),
            Some(&cookie),
            Some(&[("intent", "end")]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::FOUND, "{}", answer.body);
    assert_eq!(format!("/care/{ending}"), answer.location());
}
