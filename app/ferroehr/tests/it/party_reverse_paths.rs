// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Each `reverse_relationships` reference names exactly one relationship in
//! its source party's data (#5474). BASE `architecture_overview/master11-paths.adoc`
//! §Data Paths and Uniqueness: an archetype path can match several nodes,
//! and a `uid`, a unique `name/value` or a position makes it unique. No
//! specification fixes which path a reference carries: our own design.

use std::str::FromStr as _;

use openehr_rm::v1_2::paths::{RmPath, items_at_path};
use serde_json::{Value, json};

use crate::typed_body::typed;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

const RELATIONSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1";

fn person(name: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }]
    })
}

/// A relationship from `source` to `target` named `name`, with `uid` when
/// given.
fn relationship(name: &str, uid: Option<&str>, source: &str, target: &str) -> Value {
    let mut relationship = json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": RELATIONSHIP,
        "name": { "_type": "DV_TEXT", "value": name },
        "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                    "id": { "_type": "HIER_OBJECT_ID", "value": source } },
        "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                    "id": { "_type": "HIER_OBJECT_ID", "value": target } }
    });
    if let Some(uid) = uid {
        relationship["uid"] = json!({ "_type": "HIER_OBJECT_ID", "value": uid });
    }
    relationship
}

/// Two relationships with one node id and one name to one target, one more
/// with a `uid`, one whose name a predicate cannot quote and one with a
/// unique name: each reference resolves to its own relationship and no
/// other.
#[tokio::test]
async fn each_reverse_reference_names_one_relationship() {
    let (_db, svc) = crate::fixtures::service().await;
    let create = async |name: &str| {
        let created = svc
            .party_create(PartyKind::Person, WriteRequest::new(typed(&person(name))))
            .await
            .unwrap_or_else(|e| panic!("create {name}: {e:?}"));
        created.body["uid"]["value"]
            .as_str()
            .expect("uid.value")
            .to_owned()
    };
    let source_uid = create("Source").await;
    let target_uid = create("Target").await;
    let source = source_uid.split("::").next().expect("a container id");
    let target = target_uid.split("::").next().expect("a container id");

    let mut body = person("Source");
    body["relationships"] = json!([
        relationship("parent-of", None, source, target),
        relationship("parent-of", None, source, target),
        relationship(
            "carer",
            Some("6a1c2f4e-8b3d-4e7a-9c05-1d2e3f4a5b6c"),
            source,
            target
        ),
        relationship("o'clock", None, source, target),
        relationship("guardian", None, source, target),
    ]);
    svc.party_update(
        PartyKind::Person,
        crate::fixtures::object(source),
        crate::fixtures::if_match(&source_uid),
        WriteRequest::new(typed(&body)),
    )
    .await
    .expect("the source names the target five times");

    let reverse = svc
        .party_reverse_relationships(crate::fixtures::object(target))
        .await
        .expect("the reverse relationships");
    let paths: Vec<&str> = reverse
        .references
        .iter()
        .map(|reference| reference["path"].as_str().expect("a path"))
        .collect();
    assert_eq!(
        paths,
        [
            "/relationships[1]",
            "/relationships[2]",
            &format!(
                "/relationships[{RELATIONSHIP} and uid='6a1c2f4e-8b3d-4e7a-9c05-1d2e3f4a5b6c']"
            ),
            "/relationships[4]",
            &format!("/relationships[{RELATIONSHIP} and name/value='guardian']"),
        ],
        "one reference per relationship, in list order"
    );
    let held = body["relationships"].as_array().expect("the relationships");
    for (position, path) in paths.iter().enumerate() {
        let parsed = RmPath::from_str(path).unwrap_or_else(|e| panic!("{path} parses: {e:?}"));
        let found = items_at_path(&body, &parsed);
        assert_eq!(
            found,
            vec![&held[position]],
            "{path} names exactly the relationship at position {}",
            position + 1
        );
    }
}
