// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The JSON passthrough of a COMPOSITION read (#5755): a locally committed
//! COMPOSITION is served as its stored `version.body` text, with no parse and
//! re-serialisation, and that text is the canonical document with the version
//! uid stamped at its BMM position.

use ferroehr::service::FerroEhrService;
use ferroehr::service::response::{RawServiceResponse, ReadBody};
use serde_json::Value;

use crate::fixtures::{composition, uv};

/// The stored `version.body` text of version `tree` of `vo_id`.
async fn stored_body(db: &testkit::TestDb, vo_id: &str, tree: i32) -> String {
    sqlx::query_scalar(
        "SELECT body FROM clinical.version WHERE vo_id = $1::uuid AND sys_version = $2",
    )
    .bind(vo_id)
    .bind(tree)
    .fetch_one(&db.pool())
    .await
    .expect("the committed version row reads")
}

/// The text a read served verbatim; fails the test when the read parsed the
/// body instead.
fn passthrough(read: RawServiceResponse) -> String {
    match read.body {
        ReadBody::RawJson(text) => text.text().expect("the stored text is UTF-8").to_owned(),
        ReadBody::Value(value) => panic!("the read parsed the stored body: {value}"),
    }
}

/// The served text is the canonical document: it decodes as the typed RM
/// COMPOSITION, re-encodes to the same bytes, and carries `version_uid` as its
/// root `uid` right after `archetype_node_id`.
fn assert_canonical_with_uid(text: &str, version_uid: &str) {
    let parsed: Value = serde_json::from_str(text).expect("the served text is JSON");
    let keys: Vec<&str> = parsed
        .as_object()
        .expect("a JSON object")
        .keys()
        .take(4)
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["_type", "name", "archetype_node_id", "uid"],
        "the stamped uid sits at its BMM-declared position"
    );
    assert_eq!(parsed["uid"]["_type"], "OBJECT_VERSION_ID", "the uid type");
    assert_eq!(
        parsed["uid"]["value"], version_uid,
        "the uid is this version's"
    );
    let typed: openehr_rm::v1_2::composition::composition::Composition =
        openehr_its::json::from_canonical_value(&parsed).expect("the text decodes as typed RM");
    assert_eq!(
        openehr_its::json::to_canonical_json(&typed),
        text,
        "the text is the canonical encoding"
    );
}

async fn reads(svc: &FerroEhrService, ehr: ferroehr::ids::EhrId, version_uid: &str) -> [String; 2] {
    let vo = version_uid
        .split("::")
        .next()
        .expect("a version uid")
        .parse::<ferroehr::ids::VoId>()
        .expect("a uuid object id");
    let latest = svc
        .read_composition_raw(ehr, vo, None)
        .await
        .expect("the latest COMPOSITION reads");
    let pinned = svc
        .composition_at_version(ehr, version_uid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the addressed COMPOSITION version reads");
    [passthrough(latest), passthrough(pinned)]
}

#[tokio::test]
async fn a_json_composition_read_serves_the_stored_body_verbatim() {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let mut body = composition("Passthrough");
    body.as_object_mut().expect("a JSON object").remove("uid");
    let v1 = svc
        .create_composition(ehr, uv(&body, "249", None))
        .await
        .expect("the COMPOSITION commits")
        .version_uid();
    let vo = v1.split("::").next().expect("a version uid").to_owned();

    let stored = stored_body(&db, &vo, 1).await;
    for served in reads(&svc, ehr, &v1).await {
        assert_eq!(served, stored, "the read serves the stored bytes");
        assert_canonical_with_uid(&served, &v1);
    }

    // A second version is stamped with its own uid and passes through too.
    let v2 = svc
        .update_composition(
            ehr,
            vo.parse().expect("a uuid object id"),
            uv(&composition("Passthrough v2"), "251", Some(&v1)),
        )
        .await
        .expect("the COMPOSITION update commits")
        .version_uid();
    let stored = stored_body(&db, &vo, 2).await;
    for served in reads(&svc, ehr, &v2).await {
        assert_eq!(served, stored, "the read serves the stored bytes");
        assert_canonical_with_uid(&served, &v2);
    }
}

/// The document a read served by the parse path; fails the test when the read
/// served the stored text verbatim instead.
fn parsed(read: RawServiceResponse) -> Value {
    match read.body {
        ReadBody::Value(value) => value,
        ReadBody::RawJson(text) => panic!("the read served the stored text verbatim: {text:?}"),
    }
}

#[tokio::test]
async fn a_stored_body_without_its_own_root_uid_takes_the_parse_path() {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let v1 = svc
        .create_composition(ehr, uv(&composition("Parse path"), "249", None))
        .await
        .expect("the COMPOSITION commits")
        .version_uid();
    let vo = v1.split("::").next().expect("a version uid").to_owned();
    let mut document: Value =
        serde_json::from_str(&stored_body(&db, &vo, 1).await).expect("the stored body is JSON");
    let other = format!("{vo}::other.system::9");
    let own = serde_json::json!({"_type": "OBJECT_VERSION_ID", "value": v1});
    let foreign = serde_json::json!({"_type": "OBJECT_VERSION_ID", "value": other});

    let mut without = document.clone();
    without
        .as_object_mut()
        .expect("a JSON object")
        .remove("uid");
    document["uid"] = foreign;
    for stored in [without, document] {
        sqlx::query(
            "UPDATE clinical.version SET body = $1 WHERE vo_id = $2::uuid AND sys_version = 1",
        )
        .bind(serde_json::to_string(&stored).expect("the body serializes"))
        .bind(&vo)
        .execute(&db.pool())
        .await
        .expect("the stored body is replaced");
        let read = svc
            .read_composition_raw(ehr, vo.parse().expect("a uuid object id"), None)
            .await
            .expect("the COMPOSITION reads");
        let served = parsed(read);
        assert_eq!(served["uid"], own, "the read injects the version's own uid");
        let mut expected = stored.clone();
        expected["uid"] = own.clone();
        assert_eq!(
            served, expected,
            "the rest of the stored document is served"
        );
    }
}
