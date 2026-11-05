// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A party of a mapped template is validated against that template, with the
//! `553|incomplete|` relaxation of RM common master06 §Incomplete Content
//! (#5042), over a real PostgreSQL 18 (shared testkit harness).
//!
//! RM common `master06-change_control_package.adoc` §Incomplete Content: data
//! "committed in the `incomplete` state therefore respects the same template
//! and archetype(s), but with all existence and cardinality lower limits set
//! to zero", and "Transitioning from `incomplete` to `complete` states
//! requires any missing fields to be populated". The rule is in RM Release
//! 1.1.0 (SPECRM-97, `master00-amendment_record.adoc`) and unchanged in 1.2.0,
//! so every person case runs under both `spec_profile`s.
//!
//! The `PERSON` template is built by the OPT 1.4 writer from the vendored CKM
//! archetypes `openEHR-DEMOGRAPHIC-PERSON.person.v0` and
//! `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0`, narrowing the four
//! attributes CKM leaves wider than the RM (VCAEX). CKM publishes no `GROUP`
//! and no `PARTY_RELATIONSHIP` archetype, so those two are synthetic ADL 2
//! archetypes uploaded to the ADL 2 store, which compiles each to its OPT2.
//! Which templates are mapped, and that the others keep RM validation only,
//! is our own design (no openEHR spec governs it).

use openehr_its::opt14::types::Intervalofinteger;
use openehr_its::opt14_writer::{Narrowing, SlotFill};
use serde_json::{Value, json};

use crate::fixtures::{committer, if_match, object};
use crate::validation_opt14_writer::{archetype_use, definition, library};
use ferroehr::config::profile::SpecProfile;
use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Invalid;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::template::{MappedPartyTemplates, TemplateFit};
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The OPT 1.4 template id of the professional `PERSON`.
const PERSON_TEMPLATE: &str = "FerroEHR professional";
/// The ADL 2 id of the care-team `GROUP`, which is its own template.
const GROUP_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0";
/// The ADL 2 id of the membership `PARTY_RELATIONSHIP`.
const MEMBERSHIP_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0";
/// The `version_lifecycle_state` codes (RM common master06 §Version Lifecycle).
const COMPLETE: &str = "532";
const INCOMPLETE: &str = "553";
/// The profiles and the `rm_version` each one's bodies declare.
const PROFILES: [(SpecProfile, &str); 2] = [
    (SpecProfile::Development, "1.2.0"),
    (SpecProfile::Stable, "1.1.0"),
];

/// The `{1..1}` existence the template gives an RM-mandatory attribute.
const ONE: Intervalofinteger = Intervalofinteger {
    lower_included: Some(true),
    upper_included: Some(true),
    lower_unbounded: false,
    upper_unbounded: false,
    lower: Some(1),
    upper: Some(1),
};

/// The `{0}` existence that removes an attribute.
const NONE: Intervalofinteger = Intervalofinteger {
    lower: Some(0),
    upper: Some(0),
    ..ONE
};

fn existence(path: &str, existence: Intervalofinteger) -> Narrowing {
    Narrowing::Existence {
        path: path.to_owned(),
        existence,
    }
}

fn mandatory(path: &str) -> Narrowing {
    existence(path, ONE)
}

/// The professional `PERSON` template, written by the OPT 1.4 writer from the
/// CKM archetypes: the name archetype fills and closes the identity slot, and
/// the birth-data `details` slot is removed.
fn person_template_xml() -> String {
    let library = library();
    let mut name = archetype_use("openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0");
    name.narrowings.push(mandatory("/name"));
    name.narrowings.push(mandatory("/details"));
    let mut root = archetype_use("openEHR-DEMOGRAPHIC-PERSON.person.v0");
    root.narrowings.push(mandatory("/identities"));
    root.narrowings
        .push(mandatory("/contacts[at0003]/addresses"));
    root.narrowings.push(existence("/details", NONE));
    root.narrowings.push(Narrowing::Exclude {
        path: "/identities[at0002]".to_owned(),
    });
    root.fills.push(SlotFill {
        path: "/identities[at0002]".to_owned(),
        occurrences: None,
        filler: name,
    });
    openehr_its::opt14_writer::write(&definition(PERSON_TEMPLATE, root, &library), &library)
        .expect("the person template builds")
}

/// A synthetic ADL 2 archetype rooted at `root`, whose `attribute` holds
/// `holder`; the terminology defines the `id1` to `id4` codes the holders use.
fn adl2_archetype(hrid: &str, root: &str, attribute: &str, holder: &str) -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)
    {hrid}

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <[\"name\"] = <\"FerroEHR tests\">>
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A demographic node the shared mapping writes.\">
        >
    >

definition
    {root}[id1] matches {{
        {attribute} matches {{
            {holder}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"The root.\">>
            [\"id2\"] = <text = <\"Holder\"> description = <\"The holder.\">>
            [\"id3\"] = <text = <\"Items\"> description = <\"The items.\">>
            [\"id4\"] = <text = <\"Label\"> description = <\"The mandatory label.\">>
        >
    >
"
    )
}

/// The mandatory `id4` label inside an `ITEM_TREE[id3]`.
const LABEL_TREE: &str = "ITEM_TREE[id3] matches {
                items matches {
                    ELEMENT[id4] occurrences matches {1} matches {
                        value matches {
                            DV_TEXT[id5]
                        }
                    }
                }
            }";

fn group_archetype() -> String {
    adl2_archetype(
        GROUP_ARCHETYPE,
        "GROUP",
        "identities",
        &format!("PARTY_IDENTITY[id2] matches {{ details matches {{ {LABEL_TREE} }} }}"),
    )
}

fn membership_archetype() -> String {
    adl2_archetype(
        MEMBERSHIP_ARCHETYPE,
        "PARTY_RELATIONSHIP",
        "details",
        LABEL_TREE,
    )
}

/// The mapping the tests run under: the person template, the group template
/// and the membership archetype.
fn mapping() -> MappedPartyTemplates {
    MappedPartyTemplates::default()
        .with_person_template(PERSON_TEMPLATE)
        .with_group_template(GROUP_ARCHETYPE)
        .with_membership_archetype(MEMBERSHIP_ARCHETYPE)
}

/// A service with the three definitions stored, mapped or not, under
/// `profile`.
async fn service(mapped: bool, profile: SpecProfile) -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    svc.template_adl14_upload(&person_template_xml())
        .await
        .expect("the person template uploads");
    svc.template_adl2_upload(&group_archetype())
        .await
        .expect("the group archetype uploads");
    svc.template_adl2_upload(&membership_archetype())
        .await
        .expect("the membership archetype uploads");
    let svc = svc.with_spec_profile(profile);
    let svc = if mapped {
        svc.with_mapped_party_templates(mapping())
    } else {
        svc
    };
    (db, svc)
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn archetyped(archetype_id: &str, template_id: Option<&str>, rm_version: &str) -> Value {
    let mut details = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype_id },
        "rm_version": rm_version
    });
    if let Some(template_id) = template_id {
        details["template_id"] = json!({ "_type": "TEMPLATE_ID", "value": template_id });
    }
    details
}

/// A professional `PERSON` of `template_id` whose name carries the template's
/// mandatory `Preferred name` (`at0008`, `DV_BOOLEAN`).
fn person(template_id: &str, rm_version: &str) -> Value {
    json!({
        "_type": "PERSON",
        "name": text("Jane Roe"),
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v0",
        "archetype_details": archetyped(
            "openEHR-DEMOGRAPHIC-PERSON.person.v0", Some(template_id), rm_version
        ),
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": {
                "_type": "DV_CODED_TEXT",
                "value": "Reporting name",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                    "code_string": "at0023"
                }
            },
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0",
            "archetype_details": archetyped(
                "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0", None, rm_version
            ),
            "details": {
                "_type": "ITEM_TREE",
                "name": text("Name"),
                "archetype_node_id": "at0001",
                "items": [
                    {
                        "_type": "ELEMENT",
                        "name": text("Family Name"),
                        "archetype_node_id": "at0003",
                        "value": text("Roe")
                    },
                    {
                        "_type": "ELEMENT",
                        "name": text("Preferred name"),
                        "archetype_node_id": "at0008",
                        "value": { "_type": "DV_BOOLEAN", "value": true }
                    }
                ]
            }
        }]
    })
}

/// The path step of the person name's mandatory `Preferred name` element.
const PREFERRED_NAME: &str = "items[at0008]";

/// The person without its mandatory `Preferred name`: data missing.
fn person_missing(rm_version: &str) -> Value {
    let mut body = person(PERSON_TEMPLATE, rm_version);
    body.pointer_mut("/identities/0/details/items")
        .and_then(Value::as_array_mut)
        .expect("the name items")
        .retain(|item| item["archetype_node_id"] != "at0008");
    body
}

/// The person whose `Preferred name` is a `DV_TEXT` where the template allows
/// only `DV_BOOLEAN`: data wrong.
fn person_wrong(rm_version: &str) -> Value {
    let mut body = person(PERSON_TEMPLATE, rm_version);
    *body
        .pointer_mut("/identities/0/details/items/1/value")
        .expect("the preferred-name value") = text("yes");
    body
}

/// An `ITEM_TREE[id3]` holding the `id4` label `value`, or no label.
fn label_tree(label: Option<Value>) -> Value {
    let items: Vec<Value> = label
        .into_iter()
        .map(|value| {
            json!({
                "_type": "ELEMENT",
                "name": text("Label"),
                "archetype_node_id": "id4",
                "value": value
            })
        })
        .collect();
    json!({
        "_type": "ITEM_TREE",
        "name": text("Items"),
        "archetype_node_id": "id3",
        "items": items
    })
}

/// A care-team `GROUP` with its label `value`, absent for `None`.
fn group(label: Option<Value>) -> Value {
    json!({
        "_type": "GROUP",
        "name": text("Ward 3 team"),
        "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.care_team.v1",
        "archetype_details": archetyped(
            "openEHR-DEMOGRAPHIC-GROUP.care_team.v1", Some(GROUP_ARCHETYPE), "1.2.0"
        ),
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": text("Team name"),
            "archetype_node_id": "id2",
            "details": label_tree(label)
        }]
    })
}

fn party_ref(class: &str, id: &str) -> Value {
    json!({
        "_type": "PARTY_REF",
        "namespace": "demographic",
        "type": class,
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

/// A membership of the person `member` in the group `team`, with its role
/// label `value`, absent for `None`.
fn membership(team: &str, member: &str, label: Option<Value>) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "name": text("Membership"),
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1",
        "archetype_details": archetyped(
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1", None, "1.2.0"
        ),
        "details": label_tree(label),
        "source": party_ref("GROUP", team),
        "target": party_ref("PERSON", member)
    })
}

/// The `UPDATE_VERSION` of a direct write of `data` in `lifecycle`.
fn request<T: serde::de::DeserializeOwned>(
    data: &Value,
    lifecycle: &str,
    preceding: Option<&str>,
) -> WriteRequest<T> {
    let mut version = json!({
        "lifecycle_state": {
            "value": if lifecycle == COMPLETE { "complete" } else { "incomplete" },
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": lifecycle }
        },
        "data": data,
        "commit_audit": {
            "change_type": {
                "value": "commit",
                "defining_code": {
                    "terminology_id": { "value": "openehr" },
                    "code_string": if preceding.is_some() { "251" } else { "249" }
                }
            },
            "committer": committer("party template tester")
        }
    });
    if let Some(preceding) = preceding {
        version["preceding_version_uid"] = json!({ "value": preceding });
    }
    crate::fixtures::Envelope::from_version(
        openehr_its::json::from_canonical_value(&version).expect("UpdateVersion"),
    )
}

/// Create a party of `kind` from `body` in `lifecycle`, answering its version
/// uid.
async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
    lifecycle: &str,
) -> Result<String, ServiceError> {
    let created = Box::pin(svc.party_create(kind, request(body, lifecycle, None))).await?;
    Ok(created.meta.expect("a create answers its version").uid)
}

/// Update the party at `current` to `body` in `lifecycle`, answering the new
/// version uid.
async fn update(
    svc: &FerroEhrService,
    current: &str,
    body: &Value,
    lifecycle: &str,
) -> Result<String, ServiceError> {
    let updated = Box::pin(svc.party_update(
        PartyKind::Person,
        object(current),
        if_match(current),
        request(body, lifecycle, Some(current)),
    ))
    .await?;
    Ok(updated.meta.expect("an update answers its version").uid)
}

async fn relate(
    svc: &FerroEhrService,
    body: &Value,
    lifecycle: &str,
) -> Result<String, ServiceError> {
    let created = svc
        .party_relationship_create(request(body, lifecycle, None))
        .await?;
    Ok(created.meta.expect("a create answers its version").uid)
}

/// The paths a write's `422` names: the demographic surfaces answer one
/// content-invalid refusal joining each `path: message` pair.
fn refused<T: std::fmt::Debug>(outcome: Result<T, ServiceError>) -> Vec<String> {
    match outcome {
        Err(ServiceError::Unprocessable {
            kind: Invalid::Content,
            violation,
        }) => violation
            .detail()
            .split("; ")
            .map(|pair| {
                pair.split_once(": ")
                    .map_or(pair, |(path, _)| path)
                    .to_owned()
            })
            .collect(),
        other => panic!("expected a 422 naming the failing paths, got {other:?}"),
    }
}

/// The paths a fit refuses as wrong data, failing on any other outcome.
fn wrong_paths(outcome: Result<TemplateFit, ServiceError>) -> Vec<String> {
    match outcome {
        Err(ServiceError::ValidationFailed(violations)) => {
            violations.into_iter().map(|v| v.path).collect()
        }
        other => panic!("expected the fit to refuse wrong data, got {other:?}"),
    }
}

fn names(paths: &[String], step: &str) -> bool {
    paths.iter().any(|path| path.contains(step))
}

/// The paths an incomplete fit reports, failing on a complete one.
fn missing(fit: TemplateFit) -> Vec<String> {
    match fit {
        TemplateFit::Incomplete(missing) => missing.into_iter().map(|v| v.path).collect(),
        TemplateFit::Complete => panic!("expected an incomplete fit"),
    }
}

#[tokio::test]
async fn a_person_meeting_its_mapped_template_commits_complete() {
    for (profile, rm_version) in PROFILES {
        let (_db, svc) = service(true, profile).await;
        let body = person(PERSON_TEMPLATE, rm_version);
        let fit = svc.party_template_fit(&body, PERSON_TEMPLATE).await;
        assert!(
            matches!(fit, Ok(TemplateFit::Complete)),
            "{profile:?}: the person meets its template: {fit:?}"
        );
        let uid = create(&svc, PartyKind::Person, &body, COMPLETE)
            .await
            .unwrap_or_else(|e| panic!("{profile:?}: the complete person commits: {e:?}"));
        assert!(uid.ends_with("::1"), "the create is the first version");
    }
}

#[tokio::test]
async fn a_person_missing_a_mandatory_element_validates_incomplete_with_the_path() {
    for (profile, rm_version) in PROFILES {
        let (_db, svc) = service(true, profile).await;
        let body = person_missing(rm_version);
        let fit = svc
            .party_template_fit(&body, PERSON_TEMPLATE)
            .await
            .unwrap_or_else(|e| panic!("{profile:?}: missing data is not wrong data: {e:?}"));
        let paths = missing(fit);
        assert!(
            names(&paths, PREFERRED_NAME),
            "{profile:?}: the fit names the missing element: {paths:?}"
        );

        let paths = refused(create(&svc, PartyKind::Person, &body, COMPLETE).await);
        assert!(
            names(&paths, PREFERRED_NAME),
            "{profile:?}: a complete commit is refused naming the element: {paths:?}"
        );
        create(&svc, PartyKind::Person, &body, INCOMPLETE)
            .await
            .unwrap_or_else(|e| panic!("{profile:?}: the incomplete commit is accepted: {e:?}"));
    }
}

#[tokio::test]
async fn a_person_breaking_a_value_constraint_is_refused_naming_the_path() {
    for (profile, rm_version) in PROFILES {
        let (_db, svc) = service(true, profile).await;
        let body = person_wrong(rm_version);
        let paths = wrong_paths(svc.party_template_fit(&body, PERSON_TEMPLATE).await);
        assert!(
            names(&paths, PREFERRED_NAME),
            "{profile:?}: wrong data fits no lifecycle state: {paths:?}"
        );
        for lifecycle in [COMPLETE, INCOMPLETE] {
            let paths = refused(create(&svc, PartyKind::Person, &body, lifecycle).await);
            assert!(
                names(&paths, PREFERRED_NAME),
                "{profile:?} {lifecycle}: the refusal names the element: {paths:?}"
            );
        }
    }
}

/// RM common master06 §Incomplete Content: "Transitioning from `incomplete`
/// to `complete` states requires any missing fields to be populated"; a
/// logical delete (§Logical Deletion) carries no data to judge.
#[tokio::test]
async fn an_incomplete_person_completes_only_once_its_missing_element_is_populated() {
    let (_db, svc) = service(true, SpecProfile::Development).await;
    let draft = person_missing("1.2.0");
    let v1 = create(&svc, PartyKind::Person, &draft, INCOMPLETE)
        .await
        .expect("the draft commits incomplete");
    let paths = refused(update(&svc, &v1, &draft, COMPLETE).await);
    assert!(names(&paths, PREFERRED_NAME), "{paths:?}");
    let v2 = update(&svc, &v1, &draft, INCOMPLETE)
        .await
        .expect("the draft stays incomplete");
    let v3 = update(&svc, &v2, &person(PERSON_TEMPLATE, "1.2.0"), COMPLETE)
        .await
        .expect("the populated person completes");
    Box::pin(svc.party_delete(
        PartyKind::Person,
        object(&v3),
        crate::fixtures::delete_precondition(&v3, None),
        WriteRequest::new(()),
    ))
    .await
    .expect("a mapped person deletes");
}

#[tokio::test]
async fn a_group_of_the_mapped_group_template_is_validated() {
    let (_db, svc) = service(true, SpecProfile::Development).await;
    let complete = group(Some(text("Ward 3")));
    let fit = svc.party_template_fit(&complete, GROUP_ARCHETYPE).await;
    assert!(matches!(fit, Ok(TemplateFit::Complete)), "{fit:?}");
    create(&svc, PartyKind::Group, &complete, COMPLETE)
        .await
        .expect("the complete group commits");

    let unlabelled = group(None);
    let fit = svc
        .party_template_fit(&unlabelled, GROUP_ARCHETYPE)
        .await
        .expect("a group without its label is incomplete");
    let paths = missing(fit);
    assert!(names(&paths, "items[id4]"), "{paths:?}");
    let paths = refused(create(&svc, PartyKind::Group, &unlabelled, COMPLETE).await);
    assert!(names(&paths, "items[id4]"), "{paths:?}");
    create(&svc, PartyKind::Group, &unlabelled, INCOMPLETE)
        .await
        .expect("the incomplete group commits");

    let wrong = group(Some(json!({ "_type": "DV_COUNT", "magnitude": 3 })));
    for lifecycle in [COMPLETE, INCOMPLETE] {
        let paths = refused(create(&svc, PartyKind::Group, &wrong, lifecycle).await);
        assert!(names(&paths, "items[id4]"), "{lifecycle}: {paths:?}");
    }
}

#[tokio::test]
async fn a_membership_of_the_mapped_archetype_is_validated() {
    let (_db, svc) = service(true, SpecProfile::Development).await;
    let team = create(
        &svc,
        PartyKind::Group,
        &group(Some(text("Ward 3"))),
        COMPLETE,
    )
    .await
    .expect("the team");
    let member = create(
        &svc,
        PartyKind::Person,
        &person(PERSON_TEMPLATE, "1.2.0"),
        COMPLETE,
    )
    .await
    .expect("the member");
    let (team, member) = (object(&team).to_string(), object(&member).to_string());

    let complete = membership(&team, &member, Some(text("member")));
    let fit = svc
        .party_template_fit(&complete, MEMBERSHIP_ARCHETYPE)
        .await;
    assert!(matches!(fit, Ok(TemplateFit::Complete)), "{fit:?}");
    relate(&svc, &complete, COMPLETE)
        .await
        .expect("the complete membership commits");

    let unlabelled = membership(&team, &member, None);
    let paths = refused(relate(&svc, &unlabelled, COMPLETE).await);
    assert!(names(&paths, "items[id4]"), "{paths:?}");
    relate(&svc, &unlabelled, INCOMPLETE)
        .await
        .expect("the incomplete membership commits");

    let wrong = membership(
        &team,
        &member,
        Some(json!({ "_type": "DV_BOOLEAN", "value": true })),
    );
    for lifecycle in [COMPLETE, INCOMPLETE] {
        let paths = refused(relate(&svc, &wrong, lifecycle).await);
        assert!(names(&paths, "items[id4]"), "{lifecycle}: {paths:?}");
    }
}

/// The owner's rule for the demographic API: a party under a template the
/// mapping does not name keeps RM validation only, and so does every party of
/// a deployment with no mapping.
#[tokio::test]
async fn a_party_under_an_unmapped_template_is_accepted_as_today() {
    let (_db, mapped) = service(true, SpecProfile::Development).await;
    let mut other = person_missing("1.2.0");
    other["archetype_details"]["template_id"]["value"] = json!("Some other person template");
    create(&mapped, PartyKind::Person, &other, COMPLETE)
        .await
        .expect("a person of an unmapped template commits complete");
    let mut untemplated = person_missing("1.2.0");
    untemplated["archetype_details"]
        .as_object_mut()
        .expect("archetype_details")
        .remove("template_id");
    create(&mapped, PartyKind::Person, &untemplated, COMPLETE)
        .await
        .expect("a person with no template commits complete");

    let (_db, unmapped) = service(false, SpecProfile::Development).await;
    create(
        &unmapped,
        PartyKind::Person,
        &person_missing("1.2.0"),
        COMPLETE,
    )
    .await
    .expect("with no mapping the person template is not enforced");
    let team = create(&unmapped, PartyKind::Group, &group(None), COMPLETE)
        .await
        .expect("with no mapping the group template is not enforced");
    let team = object(&team).to_string();
    relate(&unmapped, &membership(&team, &team, None), COMPLETE)
        .await
        .expect("with no mapping the membership archetype is not enforced");
}

/// A party naming a mapped template rooted at another class is refused at the
/// root.
#[tokio::test]
async fn a_party_of_another_class_than_its_mapped_template_is_refused() {
    let (_db, svc) = service(true, SpecProfile::Development).await;
    let mut body = group(Some(text("Ward 3")));
    body["archetype_details"]["template_id"]["value"] = json!(PERSON_TEMPLATE);
    let paths = refused(create(&svc, PartyKind::Group, &body, COMPLETE).await);
    assert!(paths.iter().any(|path| path == "/"), "{paths:?}");
}

/// The CONTRIBUTION lane commits the same parties and takes the same check.
#[tokio::test]
async fn a_contribution_of_a_mapped_person_takes_the_same_check() {
    let (_db, svc) = service(true, SpecProfile::Development).await;
    let coded = |code: &str, value: &str| {
        json!({
            "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code
            }
        })
    };
    let contribution = |lifecycle: &str, rubric: &str| {
        json!({
            "versions": [{
                "lifecycle_state": coded(lifecycle, rubric),
                "commit_audit": {
                    "change_type": coded("249", "creation"),
                    "committer": committer("party template tester")
                },
                "data": person_missing("1.2.0")
            }],
            "audit": {
                "change_type": coded("249", "creation"),
                "committer": committer("party template tester")
            }
        })
    };
    let paths = refused(
        svc.demographic_contribution_create(contribution(COMPLETE, "complete"))
            .await,
    );
    assert!(names(&paths, PREFERRED_NAME), "{paths:?}");
    svc.demographic_contribution_create(contribution(INCOMPLETE, "incomplete"))
        .await
        .expect("an incomplete member missing data commits");
}
