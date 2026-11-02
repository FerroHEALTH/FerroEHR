// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the delete of an ADL2 artefact another stored
//! artefact depends on (`DELETE /definition/artefact/adl2/{artefact_id}`),
//! driven through the assembled router over a real `FerroEhrService` on a
//! real `PostgreSQL`.
//!
//! AOM2 VARXR (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules: `C_ARCHETYPE_ROOT`) requires every archetype reference to
//! "refer to an artefact that can be found in the current repository". No
//! openEHR spec governs a delete of a definition artefact (the ITS-REST
//! Definition API declares none), so the `409` is our own design, the
//! "conflict" of ITS-REST `specifications/docs/overview/Requests_and_responses.md`
//! §HTTP status codes. Each test pins one sentence the book's "Deleting a
//! template" section says of this delete.
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use http::{StatusCode, header};

use crate::common;
use crate::common::send_body;

const OBSERVATION: &str = "openEHR-EHR-OBSERVATION.del_constituent.v1.0.0";
const PARENT: &str = "openEHR-EHR-OBSERVATION.del_parent.v1.0.0";
const CHILD: &str = "openEHR-EHR-OBSERVATION.del_parent-child.v1.0.0";

/// Upload one ADL2 source and assert it is stored.
async fn upload(app: &Router, source: String) {
    let req = common::req("POST", common::full("/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .prefer_representation()
        .body(source)
        .build();
    let (status, body) = send_body(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// DELETE one stored artefact, asking for the error detail.
async fn delete(app: &Router, hrid: &str) -> (StatusCode, String) {
    let req = common::req(
        "DELETE",
        common::full(&format!("/definition/artefact/adl2/{hrid}")),
    )
    .prefer_representation()
    .build();
    send_body(app, req).await
}

/// The stored artefact ids.
async fn stored(app: &Router) -> String {
    let req = common::req("GET", common::full("/definition/artefact/adl2")).build();
    let (status, body) = send_body(app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// An ADL2 source of `keyword` with id `hrid` and root node `root`
/// (`TYPE[code]`) matching anything, specialising `parent` when given.
fn artefact(keyword: &str, hrid: &str, parent: Option<&str>, root: &str) -> String {
    let specialise = parent.map_or_else(String::new, |p| format!("specialise\n    {p}\n\n"));
    format!(
        "{keyword} (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n{specialise}\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    {root} matches {{ *}}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"{code}\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n",
        code = root
            .split_once('[')
            .and_then(|(_, c)| c.strip_suffix(']'))
            .expect("a root of the form TYPE[code]"),
    )
}

/// An ADL2 `template` whose `content` holds one `use_archetype` of `filler`
/// (an `rm_type`).
fn template_using(hrid: &str, rm_type: &str, filler: &str) -> String {
    format!(
        "template (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    COMPOSITION[id1] matches {{\n        \
         content cardinality matches {{0..*; unordered}} matches {{\n            \
         use_archetype {rm_type}[id2, {filler}]\n        }}\n    }}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n            \
         [\"id2\"] = <text = <\"Filler\"> description = <\"Filler.\">>\n        >\n    >\n"
    )
}

/// The book: an archetype a stored template uses is not deleted. The delete
/// of an archetype two templates fill a slot with is a `409` naming both, and
/// the archetype stays stored.
#[tokio::test]
async fn deleting_an_archetype_templates_use_is_409_naming_every_template() {
    let (_pg, app) = common::test_router().await;
    let first = "openEHR-EHR-COMPOSITION.del_uses_a.v1.0.0";
    let second = "openEHR-EHR-COMPOSITION.del_uses_b.v1.0.0";
    upload(
        &app,
        artefact("archetype", OBSERVATION, None, "OBSERVATION[id1]"),
    )
    .await;
    for template in [first, second] {
        upload(&app, template_using(template, "OBSERVATION", OBSERVATION)).await;
    }

    let (status, body) = delete(&app, OBSERVATION).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    for template in [first, second] {
        assert!(body.contains(template), "the 409 names {template}: {body}");
    }
    assert!(
        stored(&app).await.contains(OBSERVATION),
        "a refused delete removes nothing"
    );
}

/// The book: a specialised archetype keeps its parent, and so does every
/// artefact built on it. Deleting the parent is a `409` naming the child and
/// the template that uses the child.
#[tokio::test]
async fn deleting_a_specialisation_parent_is_409_naming_the_child_and_its_users() {
    let (_pg, app) = common::test_router().await;
    let template = "openEHR-EHR-COMPOSITION.del_uses_child.v1.0.0";
    upload(
        &app,
        artefact("archetype", PARENT, None, "OBSERVATION[id1]"),
    )
    .await;
    upload(
        &app,
        artefact("archetype", CHILD, Some(PARENT), "OBSERVATION[id1.1]"),
    )
    .await;
    upload(&app, template_using(template, "OBSERVATION", CHILD)).await;

    let (status, body) = delete(&app, PARENT).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains(CHILD), "the 409 names the child: {body}");
    assert!(
        body.contains(template),
        "the 409 names the template built on the child: {body}"
    );
    assert!(stored(&app).await.contains(PARENT));
}

/// The book: delete the dependents first. An archetype nothing uses deletes
/// with a `204` and leaves a template on another archetype compiling, and
/// once its template is gone the used archetype deletes too.
#[tokio::test]
async fn deleting_an_archetype_nothing_uses_is_204() {
    let (_pg, app) = common::test_router().await;
    let unused = "openEHR-EHR-OBSERVATION.del_unused.v1.0.0";
    let user = "openEHR-EHR-COMPOSITION.del_user.v1.0.0";
    upload(
        &app,
        artefact("archetype", OBSERVATION, None, "OBSERVATION[id1]"),
    )
    .await;
    upload(
        &app,
        artefact("archetype", unused, None, "OBSERVATION[id1]"),
    )
    .await;
    upload(&app, template_using(user, "OBSERVATION", OBSERVATION)).await;

    let (status, body) = delete(&app, unused).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let req = common::req(
        "GET",
        common::full(&format!("/definition/template/adl2/{user}")),
    )
    .accept("application/json")
    .build();
    let (status, body) = send_body(&app, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the template still compiles: {body}"
    );

    let (status, body) = delete(&app, user).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = delete(&app, OBSERVATION).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(!stored(&app).await.contains(OBSERVATION));
}
