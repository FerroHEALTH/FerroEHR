// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archetypes over stubbed archetype routes: the list by family, an ADL 1.4
//! archetype's source and ADL 2 conversion, an upload, a template's
//! conversion, and a CDR that keeps no archetypes (#6659).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::Value;

use crate::ssr_app::{Viewer, id, stub};

/// The extensions root the stubs answer under.
const EXT: &str = "/ferroehr/rest/extensions/v1";

/// The ADL 1.4 archetype the stubs hold.
const ARCHETYPE: &str = "openEHR-EHR-OBSERVATION.blood_pressure.v1";

/// The ADL 2 archetype the stubs hold.
const ADL2: &str = "openEHR-EHR-OBSERVATION.cnf_count.v1.0.0";

/// A conversion of `root` with one note.
fn conversion(root: &str) -> String {
    serde_json::json!({
        "roots": [{
            "archetype_id": root,
            "adl2": "archetype (adl_version=2.4; rm_release=1.1.0)",
            "log": {"external_at_codes": {}, "value_sets": {}, "notes": ["kept every ADL 1.4 code"]},
            "conversion_details": {"source_model": root}
        }],
        "structure": []
    })
    .to_string()
}

/// The two lists and the ADL 1.4 archetype's source and conversion.
fn kept() -> Vec<Value> {
    vec![
        stub(
            &format!("{EXT}/definition/archetype/adl1.4"),
            "application/json",
            &format!("[\"{ARCHETYPE}\"]"),
        ),
        stub(
            &format!("{EXT}/definition/archetype/adl2"),
            "application/json",
            &format!("[\"{ADL2}\"]"),
        ),
        stub(
            &format!("{EXT}/definition/archetype/adl1.4/{ARCHETYPE}"),
            "text/plain",
            "archetype (adl_version=1.4)",
        ),
        stub(
            &format!("{EXT}/definition/archetype/adl1.4/{ARCHETYPE}/adl2"),
            "application/json",
            &conversion(ARCHETYPE),
        ),
        stub(
            &format!("/ferroehr/rest/openehr/v1/definition/template/adl2/{ADL2}"),
            "text/plain",
            "archetype (adl_version=2.4)",
        ),
    ]
}

/// The list draws each family's ids, each a link to its page, and offers
/// Upload.
#[tokio::test]
async fn the_list_shows_both_families() {
    let viewer = Viewer::start_with(kept()).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/archetypes").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    let main = page.main();
    assert!(
        main.contains(&format!(r#"href="/archetypes/{ARCHETYPE}""#)),
        "{main}"
    );
    assert!(main.contains(r#"href="/archetypes/upload""#), "{main}");
    let adl2 = viewer.page(&cookie, "/archetypes?family=adl2").await;
    assert!(
        adl2.main()
            .contains(&format!(r#"href="/archetypes/adl2/{ADL2}""#)),
        "{}",
        adl2.main()
    );
    assert!(
        page.body.contains(r#"href="/archetypes""#),
        "the nav links it"
    );
}

/// An ADL 1.4 archetype opens on its source with its download, and its ADL 2
/// tab draws the conversion; an ADL 2 archetype opens on its source.
#[tokio::test]
async fn an_archetype_shows_its_source_and_its_conversion() {
    let viewer = Viewer::start_with(kept()).await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .page(&cookie, &format!("/archetypes/{ARCHETYPE}"))
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(
        page.main().contains("archetype (adl_version=1.4)"),
        "{}",
        page.main()
    );
    assert!(
        page.main()
            .contains(&format!("/download/archetype/adl14/{ARCHETYPE}")),
        "{}",
        page.main()
    );
    let converted = viewer
        .page(&cookie, &format!("/archetypes/{ARCHETYPE}?tab=adl2"))
        .await;
    for words in ["kept every ADL 1.4 code", "adl_version=2.4", "source_model"] {
        assert!(
            converted.main().contains(words),
            "{words}:\n{}",
            converted.main()
        );
    }
    let adl2 = viewer
        .page(&cookie, &format!("/archetypes/adl2/{ADL2}"))
        .await;
    assert_eq!(adl2.status, http::StatusCode::OK, "{}", adl2.body);
    assert!(adl2.main().contains("adl_version=2.4"), "{}", adl2.main());
}

/// An upload the CDR refuses stays on the page in the CDR's words; one it
/// stores opens the archetype.
#[tokio::test]
async fn an_upload_is_refused_in_the_cdrs_words_or_opens_the_archetype() {
    let post = |status: u16, response: &str| {
        serde_json::json!({
            "method": "POST",
            "path": format!("{EXT}/definition/archetype/adl1.4"),
            "body": "archetype (adl_version=1.4)",
            "status": status,
            "headers": {"content-type": "application/json"},
            "response": response,
        })
    };
    let refused = Viewer::start_with(vec![post(
        422,
        r#"{"title":"Unprocessable Entity","detail":"the source does not parse as ADL 1.4"}"#,
    )])
    .await;
    let cookie = refused.sign_in().await;
    let form = [
        ("source-text", "archetype (adl_version=1.4)"),
        ("intent", "upload"),
    ];
    let page = refused
        .send(
            http::Method::POST,
            "/archetypes/upload",
            Some(&cookie),
            Some(&form),
        )
        .await;
    assert!(
        page.main().contains("the source does not parse as ADL 1.4"),
        "{}",
        page.main()
    );
    let stored = Viewer::start_with(vec![post(201, &format!("\"{ARCHETYPE}\""))]).await;
    let cookie = stored.sign_in().await;
    let answer = stored
        .send(
            http::Method::POST,
            "/archetypes/upload",
            Some(&cookie),
            Some(&form),
        )
        .await;
    assert_eq!(answer.location(), format!("/archetypes/{ARCHETYPE}"));
}

/// A template offers its ADL 2 conversion as a tab.
#[tokio::test]
async fn a_template_offers_its_adl2_conversion() {
    let template = id("template");
    let viewer = Viewer::start_with(vec![stub(
        &format!(
            "{EXT}/definition/template/adl1.4/{}/adl2",
            urlencoding::encode(&template)
        ),
        "application/json",
        &conversion("openEHR-EHR-EVALUATION.minimal.v1"),
    )])
    .await;
    let cookie = viewer.sign_in().await;
    let base = format!("/templates/{}", urlencoding::encode(&template));
    let overview = viewer.page(&cookie, &base).await;
    assert!(overview.main().contains("?tab=adl2"), "{}", overview.main());
    let page = viewer.page(&cookie, &format!("{base}?tab=adl2")).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(
        page.main().contains("openEHR-EHR-EVALUATION.minimal.v1"),
        "{}",
        page.main()
    );
}

/// A CDR that keeps no archetypes answers `404`, and the page offers nothing.
#[tokio::test]
async fn a_cdr_without_archetypes_offers_none() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/archetypes").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(
        page.main().contains("keeps no archetypes"),
        "{}",
        page.main()
    );
    assert!(
        !page.main().contains(r#"href="/archetypes/upload""#),
        "{}",
        page.main()
    );
}
