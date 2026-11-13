// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic references a party write carries, on every write lane
//! (direct create, direct update, CONTRIBUTION): an inline relationship's
//! `source` and `target`, `ROLE.performer`, `ACTOR.roles`, and FerroEHR's own
//! standalone `PARTY_RELATIONSHIP` containers.
//!
//! RM demographic `master02-demographic_package.adoc` §Party Relationships:
//! "The actual kind of reference is via the use of `OBJECT_REFs` containing
//! `HIER_OBJECT_IDs` to denote the Version container of a Party, rather than
//! `OBJECT_VERSION_IDs`"; `role.adoc` `performer`: "Reference to Version
//! container of Actor playing the role"; `actor.adoc` `roles`: "Identifiers of
//! the Version container for each Role played by this Party".

use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Invalid;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use serde_json::{Value, json};

use crate::fixtures::{committer, uv};
use crate::typed_body::typed;

/// The `uid.value` of a versioned-object body.
fn ovid(v: &Value) -> String {
    v["uid"]["value"].as_str().expect("uid.value").to_owned()
}

/// The container id of a versioned-object body.
fn container(v: &Value) -> String {
    ovid(v)
        .split("::")
        .next()
        .expect("a container id")
        .to_owned()
}

/// A root-valid party of `rm_type` named `name`.
fn party(rm_type: &str, name: &str) -> Value {
    let archetype = format!(
        "openEHR-DEMOGRAPHIC-{rm_type}.{}.v1",
        rm_type.to_lowercase()
    );
    json!({
        "_type": rm_type,
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
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
                "items": []
            }
        }]
    })
}

/// A `PARTY_REF` of `rm_type` whose id is `id`, a `HIER_OBJECT_ID` unless it
/// carries `::`.
fn party_ref(rm_type: &str, id: &str) -> Value {
    let id_type = if id.contains("::") {
        "OBJECT_VERSION_ID"
    } else {
        "HIER_OBJECT_ID"
    };
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": rm_type,
        "id": { "_type": id_type, "value": id } })
}

/// A ROLE whose performer is `performer`.
fn role(performer: Value) -> Value {
    let mut body = party("ROLE", "Clinician");
    body["performer"] = performer;
    body
}

/// A `PARTY_RELATIONSHIP` from `source` to `target`.
fn relationship(source: &Value, target: &Value) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "name": { "_type": "DV_TEXT", "value": "related" },
        "source": source,
        "target": target
    })
}

/// Creates a party of `kind` from `body` and returns its body.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> Value {
    svc.party_create(kind, WriteRequest::new(typed(body)))
        .await
        .unwrap_or_else(|e| panic!("create {body}: {e:?}"))
        .body
}

/// A demographic CONTRIBUTION of one version of `data`: a creation, or a
/// modification of `preceding`.
fn contribution(data: &Value, preceding: Option<&str>) -> Value {
    let coded = |code: &str, value: &str| {
        json!({ "value": value,
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": code } })
    };
    let change = if preceding.is_some() {
        coded("251", "modification")
    } else {
        coded("249", "creation")
    };
    let mut version = json!({
        "lifecycle_state": coded("532", "complete"),
        "commit_audit": { "change_type": change.clone(), "committer": committer("tester") },
        "data": data
    });
    if let Some(preceding) = preceding {
        version["preceding_version_uid"] = json!({ "value": preceding });
    }
    json!({
        "versions": [version],
        "audit": { "change_type": change, "committer": committer("tester") }
    })
}

/// Asserts `result` is a `422` at a path ending in `path`, naming `invariant`.
fn assert_refused<T: std::fmt::Debug>(
    result: Result<T, ServiceError>,
    path: &str,
    invariant: &str,
) {
    match result {
        Err(ServiceError::Unprocessable {
            kind: Invalid::Content,
            violation,
        }) => {
            assert_eq!(violation.invariant(), Some(invariant), "{violation:?}");
            assert!(
                violation.path().is_some_and(|at| at.ends_with(path)),
                "{violation:?}"
            );
        }
        other => panic!("expected the 422 at {path} ({invariant}), got {other:?}"),
    }
}

/// The label a reference to one version instead of its container carries,
/// the same on the inline and the standalone representation.
const VERSION_CONTAINER: &str = "RM demographic master02";

/// The three write lanes, each refusing `body` the same way: a direct create
/// (`create`), a direct update of `holder` and a CONTRIBUTION modification of
/// it, and a CONTRIBUTION creation.
async fn refused_on_every_lane(
    svc: &FerroEhrService,
    (kind, holder): (PartyKind, &Value),
    body: &Value,
    create_too: bool,
    (path, invariant): (&str, &str),
) {
    if create_too {
        assert_refused(
            svc.party_create(kind, WriteRequest::new(typed(body))).await,
            path,
            invariant,
        );
        assert_refused(
            svc.demographic_contribution_create(contribution(body, None))
                .await,
            path,
            invariant,
        );
    }
    assert_refused(
        svc.party_update(
            kind,
            crate::fixtures::object(container(holder)),
            crate::fixtures::if_match(ovid(holder)),
            WriteRequest::new(typed(body)),
        )
        .await,
        path,
        invariant,
    );
    assert_refused(
        svc.demographic_contribution_create(contribution(body, Some(&ovid(holder))))
            .await,
        path,
        invariant,
    );
}

/// An inline relationship whose `target` (or `source`) names one version of a
/// party is refused like a standalone one (RM demographic master02 §Party
/// Relationships).
#[tokio::test]
async fn an_inline_relationship_naming_a_version_is_refused() {
    let (_db, svc) = crate::fixtures::service().await;
    let holder = create(&svc, PartyKind::Person, &party("PERSON", "Holder")).await;
    let other = create(&svc, PartyKind::Person, &party("PERSON", "Other")).await;

    let mut body = party("PERSON", "Holder");
    body["relationships"] = json!([relationship(
        &party_ref("PERSON", &container(&holder)),
        &party_ref("PERSON", &ovid(&other)),
    )]);
    refused_on_every_lane(
        &svc,
        (PartyKind::Person, &holder),
        &body,
        false,
        ("PERSON.relationships[0].target.id", VERSION_CONTAINER),
    )
    .await;

    body["relationships"] = json!([relationship(
        &party_ref("PERSON", &ovid(&holder)),
        &party_ref("PERSON", &container(&other)),
    )]);
    refused_on_every_lane(
        &svc,
        (PartyKind::Person, &holder),
        &body,
        false,
        ("PERSON.relationships[0].source.id", VERSION_CONTAINER),
    )
    .await;
}

/// `ROLE.performer` names the version container of an Actor (RM demographic
/// `role.adoc` `performer`): a ROLE type, a version id, a container this
/// server does not hold or one of another kind is refused on every lane.
#[tokio::test]
async fn a_role_performer_names_an_actor_container() {
    let (_db, svc) = crate::fixtures::service().await;
    let person = create(&svc, PartyKind::Person, &party("PERSON", "Performer")).await;
    let holder = create(
        &svc,
        PartyKind::Role,
        &role(party_ref("PERSON", &container(&person))),
    )
    .await;

    let label = "RM demographic role.adoc performer";
    for (performer, (path, label)) in [
        (
            party_ref("ROLE", &container(&holder)),
            ("ROLE.performer.type", label),
        ),
        (
            party_ref("PERSON", &ovid(&person)),
            ("ROLE.performer.id", VERSION_CONTAINER),
        ),
        (
            party_ref("PERSON", "0198f1f1-0000-7000-8000-00000000dead"),
            ("ROLE.performer", label),
        ),
        (
            party_ref("ACTOR", &container(&holder)),
            ("ROLE.performer", label),
        ),
    ] {
        refused_on_every_lane(
            &svc,
            (PartyKind::Role, &holder),
            &role(performer),
            true,
            (path, label),
        )
        .await;
    }

    // A performer typed by its supertype names the same actor.
    create(
        &svc,
        PartyKind::Role,
        &role(party_ref("ACTOR", &container(&person))),
    )
    .await;
}

/// `ACTOR.roles` names version containers of ROLEs (RM demographic
/// `actor.adoc` `roles`): another type, a version id or a container this
/// server does not hold is refused on every lane.
#[tokio::test]
async fn actor_roles_name_role_containers() {
    let (_db, svc) = crate::fixtures::service().await;
    let holder = create(&svc, PartyKind::Person, &party("PERSON", "Actor")).await;
    let other = create(&svc, PartyKind::Person, &party("PERSON", "Other")).await;
    let held = create(
        &svc,
        PartyKind::Role,
        &role(party_ref("PERSON", &container(&holder))),
    )
    .await;

    let label = "RM demographic actor.adoc roles";
    for (reference, (path, label)) in [
        (
            party_ref("PERSON", &container(&other)),
            ("PERSON.roles[0].type", label),
        ),
        (
            party_ref("ROLE", &container(&other)),
            ("PERSON.roles[0]", label),
        ),
        (
            party_ref("ROLE", &ovid(&held)),
            ("PERSON.roles[0].id", VERSION_CONTAINER),
        ),
        (
            party_ref("ROLE", "0198f1f1-0000-7000-8000-00000000dead"),
            ("PERSON.roles[0]", label),
        ),
    ] {
        let mut body = party("PERSON", "Actor");
        body["roles"] = json!([reference]);
        refused_on_every_lane(
            &svc,
            (PartyKind::Person, &holder),
            &body,
            true,
            (path, label),
        )
        .await;
    }

    let mut body = party("PERSON", "Actor");
    body["roles"] = json!([party_ref("ROLE", &container(&held))]);
    svc.party_update(
        PartyKind::Person,
        crate::fixtures::object(container(&holder)),
        crate::fixtures::if_match(ovid(&holder)),
        WriteRequest::new(typed(&body)),
    )
    .await
    .expect("a ROLE container this server holds is a legal role");
}

/// A standalone relationship's `source` and `target` name party containers
/// this server holds, and the relationship reaches its target's reverse list.
#[tokio::test]
async fn a_standalone_relationship_names_held_parties_and_is_listed_at_its_target() {
    let (_db, svc) = crate::fixtures::service().await;
    let source = create(&svc, PartyKind::Person, &party("PERSON", "Source")).await;
    let target = create(&svc, PartyKind::Person, &party("PERSON", "Target")).await;
    let dangling = "0198f1f1-0000-7000-8000-00000000dead";
    let label = "RM demographic party_relationship.adoc";

    for (body, path) in [
        (
            relationship(
                &party_ref("PERSON", dangling),
                &party_ref("PERSON", &container(&target)),
            ),
            "PARTY_RELATIONSHIP.source",
        ),
        (
            relationship(
                &party_ref("PERSON", &container(&source)),
                &party_ref("PERSON", dangling),
            ),
            "PARTY_RELATIONSHIP.target",
        ),
    ] {
        assert_refused(
            Box::pin(crate::fixtures::create_relationship(
                &svc,
                uv(&body, "249", None),
            ))
            .await,
            path,
            label,
        );
        assert_refused(
            svc.demographic_contribution_create(contribution(&body, None))
                .await,
            path,
            label,
        );
    }

    let relationship_id = Box::pin(crate::fixtures::create_relationship(
        &svc,
        uv(
            &relationship(
                &party_ref("PERSON", &container(&source)),
                &party_ref("PERSON", &container(&target)),
            ),
            "249",
            None,
        ),
    ))
    .await
    .expect("a relationship between held parties commits");
    let reverse = svc
        .party_reverse_relationships(container(&target).parse().expect("a container id"), None)
        .await
        .expect("read the reverse list");
    assert_eq!(
        reverse.references,
        vec![json!({
            "_type": "LOCATABLE_REF",
            "namespace": "local",
            "type": "PARTY_RELATIONSHIP",
            "id": { "_type": "HIER_OBJECT_ID", "value": relationship_id.to_string() }
        })],
        "the standalone relationship is listed at its target"
    );
}
