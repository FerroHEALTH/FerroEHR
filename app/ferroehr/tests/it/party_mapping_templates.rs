// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot rules of the shared party mapping that read its templates, a
//! mapped write that keeps what it does not map, and the refusal to delete a
//! template the mapping names (#5013), over a real PostgreSQL 18.
//!
//! The definitions are synthetic ADL 2 archetypes the ADL 2 store compiles to
//! their OPT2: a professional `PERSON` with an identifier `ELEMENT` and an
//! unmapped text `ELEMENT` in `details` and a name in `identities` (RM
//! demographic `master02-demographic_package.adoc` §Names and Addresses,
//! §Party Identification), and a `GROUP`. No openEHR spec governs the
//! mapping: our own design.

use std::collections::HashMap;

use assert_fs::prelude::FileWriteStr as _;
use serde_json::{Value, json};

use ferroehr::config::FerroEhrConfig;
use ferroehr::service::FerroEhrService;

/// The professional `PERSON` archetype.
const PERSON: &str = "openEHR-DEMOGRAPHIC-PERSON.professional.v1.0.0";
/// The care-team `GROUP` archetype.
const GROUP: &str = "openEHR-DEMOGRAPHIC-GROUP.team.v1.0.0";
/// The identifier element.
const KEY_PATH: &str = "/details[id2]/items[id3]";
/// The unmapped text element.
const NOTE_PATH: &str = "/details[id2]/items[id7]";
/// The name element.
const NAME_PATH: &str = "/identities[id4]/details[id5]/items[id6]";

/// A synthetic ADL 2 archetype `hrid` rooted at `root` with `definition`.
fn archetype(hrid: &str, root: &str, definition: &str) -> String {
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
            purpose = <\"A party the shared mapping writes.\">
        >
    >

definition
    {root}[id1] matches {{
        {definition}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"The root.\">>
            [\"id2\"] = <text = <\"Details\"> description = <\"The details.\">>
            [\"id3\"] = <text = <\"Identifier\"> description = <\"An identifier.\">>
            [\"id4\"] = <text = <\"Identity\"> description = <\"The identity.\">>
            [\"id5\"] = <text = <\"Name\"> description = <\"The name structure.\">>
            [\"id6\"] = <text = <\"Name text\"> description = <\"The name.\">>
            [\"id7\"] = <text = <\"Note\"> description = <\"An unmapped note.\">>
        >
    >
"
    )
}

/// The identities and details every test party carries.
const PARTS: &str = "identities matches {
            PARTY_IDENTITY[id4] matches {
                details matches {
                    ITEM_TREE[id5] matches {
                        items matches {
                            ELEMENT[id6] matches { value matches { DV_TEXT[id8] } }
                        }
                    }
                }
            }
        }
        details matches {
            ITEM_TREE[id2] matches {
                items matches {
                    ELEMENT[id3] occurrences matches {0..*} matches {
                        value matches { DV_IDENTIFIER[id9] }
                    }
                    ELEMENT[id7] occurrences matches {0..1} matches {
                        value matches { DV_TEXT[id10] }
                    }
                }
            }
        }";

/// A service over a fresh database holding the two archetypes.
async fn service() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    svc.template_adl2_upload(&archetype(PERSON, "PERSON", PARTS))
        .await
        .expect("the person archetype uploads");
    svc.template_adl2_upload(&archetype(GROUP, "GROUP", PARTS))
        .await
        .expect("the group archetype uploads");
    (db, svc)
}

/// The configuration of `mapping` over a bound persons issuer.
fn config(mapping: &str) -> FerroEhrConfig {
    let toml = format!(
        r#"
[[auth.issuer]]
issuer = "https://idp.example/realms/staff"
kind = "persons"
audience = ["ferroehr"]
first_party_clients = ["ferroehr-viewer"]

[[auth.party]]
issuer = "https://idp.example/realms/staff"
identifier_issuer = "urn:example:staff-login"
identifier_type = "guid"

[authz.party_binding]
provider = {{ issuer = "urn:oid:2.16.528.1.1007.3.3", id = "12345678" }}
{mapping}
"#
    );
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(&toml).expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[]).expect("assembles")
}

/// A mapping of the person archetype with `key` as its key path and `name`
/// as its name path, and the group archetype as `group`.
fn mapping(person: &str, key: &str, name: &str, group: &str) -> String {
    format!(
        r#"
[[authz.party_binding.person.template]]
template_id = "{person}"
default = true
key = {{ path = "{key}", type = "guid" }}

[[authz.party_binding.person.attribute]]
scim = "name.formatted"
name = true
path = {{ "{person}" = "{name}" }}

[authz.party_binding.group]
template_id = "{group}"
external_id = {{ path = "{KEY_PATH}", type = "scim_external_id" }}
display_name = "{NAME_PATH}"
"#
    )
}

/// The template refusals of `mapping`.
async fn errors(svc: &FerroEhrService, mapping: &str) -> Vec<String> {
    let config = config(mapping);
    svc.party_mapping_errors(&config.authz.party_binding.sections())
        .await
        .expect("the store answers")
}

#[tokio::test]
async fn a_mapping_over_its_stored_templates_is_admitted() {
    let (_db, svc) = service().await;
    assert_eq!(
        errors(&svc, &mapping(PERSON, KEY_PATH, NAME_PATH, GROUP)).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn each_template_refusal_names_its_key() {
    let (_db, svc) = service().await;
    for (mapping, expected) in [
        (
            mapping("Not held.v1", KEY_PATH, NAME_PATH, GROUP),
            "which the definition store does not hold",
        ),
        (
            mapping(GROUP, KEY_PATH, NAME_PATH, GROUP),
            "which is rooted at GROUP, not PERSON",
        ),
        (
            mapping(PERSON, KEY_PATH, NAME_PATH, PERSON),
            "which is rooted at PERSON, not GROUP",
        ),
        (
            mapping(PERSON, "/details[id2]/items[id99]", NAME_PATH, GROUP),
            "has no data value at \"/details[id2]/items[id99]\"",
        ),
        (
            mapping(PERSON, NOTE_PATH, NAME_PATH, GROUP),
            "cannot carry a DV_IDENTIFIER",
        ),
        (
            mapping(PERSON, KEY_PATH, NOTE_PATH, GROUP),
            "goes in PARTY.identities",
        ),
    ] {
        let found = if expected.starts_with("goes in") {
            config(&mapping)
                .validate()
                .expect_err("a name in details is refused")
                .to_string()
        } else {
            errors(&svc, &mapping).await.join("; ")
        };
        assert!(found.contains(expected), "{expected}: {found}");
    }
}

#[tokio::test]
async fn a_mapped_write_keeps_what_it_does_not_map() {
    let (_db, svc) = service().await;
    let text = |value: &str| json!({ "_type": "DV_TEXT", "value": value });
    let head = json!({
        "_type": "PERSON",
        "archetype_node_id": PERSON,
        "name": text("Root"),
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": PERSON },
            "template_id": { "_type": "TEMPLATE_ID", "value": PERSON },
            "rm_version": "1.1.0"
        },
        "identities": [{
            "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": text("Identity"),
            "details": {
                "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": text("Name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "id6",
                            "name": text("Name text"), "value": text("Jane Roe") }]
            }
        }],
        "details": {
            "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": text("Details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "id7",
                        "name": text("Note"), "value": text("kept as written") }]
        }
    });
    let next = svc
        .mapped_party_body(
            Some(&head),
            PERSON,
            &[(NAME_PATH.to_owned(), text("Jane Doe"))],
        )
        .await
        .expect("the mapped write");
    assert_eq!(
        next.pointer("/identities/0/details/items/0/value/value"),
        Some(&Value::from("Jane Doe"))
    );
    assert_eq!(next.pointer("/details"), head.pointer("/details"));

    // A path the head lacks is created from the template.
    let key = json!({ "_type": "DV_IDENTIFIER", "issuer": "urn:example:staff-login",
                      "id": "4a5b6c7d-8e9f-4a0b-9c1d-2e3f4a5b6c7d", "type": "guid" });
    let next = svc
        .mapped_party_body(Some(&head), PERSON, &[(KEY_PATH.to_owned(), key.clone())])
        .await
        .expect("the mapped write");
    let items = next
        .pointer("/details/items")
        .and_then(Value::as_array)
        .expect("the details");
    assert_eq!(items.len(), 2, "{next}");
    assert_eq!(items[0]["value"]["value"], "kept as written");
    assert_eq!(items[1]["archetype_node_id"], "id3");
    assert_eq!(items[1]["value"], key);

    // A path the template lacks is refused.
    assert!(
        svc.mapped_party_body(
            Some(&head),
            PERSON,
            &[("/details[id2]/items[id99]".to_owned(), text("x"))]
        )
        .await
        .is_err()
    );
}

/// Two selectors at one path, the binding key and an `externalId`, each
/// write the identifier of their own `type` and `issuer` and leave the
/// other's as it is.
#[tokio::test]
async fn identifiers_at_one_path_are_told_apart_by_type_and_issuer() {
    let (_db, svc) = service().await;
    let identifier = |issuer: &str, kind: &str, id: &str| json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind });
    let key = identifier(
        "urn:example:staff-login",
        "guid",
        "4a5b6c7d-8e9f-4a0b-9c1d-2e3f4a5b6c7d",
    );
    let external = identifier("urn:example:scim-client", "external_id", "ext-1");
    let written = |body: &Value| -> Vec<Value> {
        body.pointer("/details/items")
            .and_then(Value::as_array)
            .expect("the details")
            .iter()
            .map(|item| item["value"].clone())
            .collect()
    };

    let body = svc
        .mapped_party_body(
            None,
            PERSON,
            &[
                (KEY_PATH.to_owned(), key.clone()),
                (KEY_PATH.to_owned(), external.clone()),
            ],
        )
        .await
        .expect("the mapped write");
    assert_eq!(written(&body), [key.clone(), external], "{body}");

    // A new externalId replaces the old one and keeps the key.
    let renamed = identifier("urn:example:scim-client", "external_id", "ext-2");
    let body = svc
        .mapped_party_body(
            Some(&body),
            PERSON,
            &[(KEY_PATH.to_owned(), renamed.clone())],
        )
        .await
        .expect("the mapped write");
    assert_eq!(written(&body), [key.clone(), renamed.clone()], "{body}");

    // An identifier of the same type under another issuer is a third one.
    let other = identifier("urn:example:other-client", "external_id", "ext-3");
    let body = svc
        .mapped_party_body(Some(&body), PERSON, &[(KEY_PATH.to_owned(), other.clone())])
        .await
        .expect("the mapped write");
    assert_eq!(written(&body), [key, renamed, other], "{body}");
}

#[tokio::test]
async fn a_template_the_mapping_names_is_not_deleted() {
    let (db, svc) = crate::fixtures::service().await;
    let library = crate::validation_opt14_writer::library();
    let mut root =
        crate::validation_opt14_writer::archetype_use("openEHR-DEMOGRAPHIC-PERSON.person.v0");
    // NOTE: RM demographic PERSON.identities and CONTACT.addresses are 1..1.
    for path in ["/identities", "/contacts[at0003]/addresses"] {
        root.narrowings
            .push(openehr_its::opt14_writer::Narrowing::Existence {
                path: path.to_owned(),
                existence: openehr_its::opt14::types::Intervalofinteger {
                    lower_included: Some(true),
                    upper_included: Some(true),
                    lower_unbounded: false,
                    upper_unbounded: false,
                    lower: Some(1),
                    upper: Some(1),
                },
            });
    }
    let opt = openehr_its::opt14_writer::write(
        &crate::validation_opt14_writer::definition("FerroEHR mapped person", root, &library),
        &library,
    )
    .expect("the template builds");
    svc.template_adl14_upload(&opt).await.expect("uploads");
    let key = ferroehr::party_binding::IdentifierSelector::new(KEY_PATH, "guid").expect("compiles");
    let mapped = svc.with_binding_mapping(
        ferroehr::party_binding::BindingMapping::default()
            .with_person("FerroEHR MAPPED person", key),
    );
    let refused = mapped
        .admin_template_delete("FerroEHR mapped person")
        .await
        .expect_err("the mapping names it");
    assert!(
        matches!(refused, ferroehr::error::ServiceError::Conflict(_)),
        "{refused:?}"
    );
    assert!(
        refused.to_string().contains("[authz.party_binding]"),
        "{refused}"
    );
    let unmapped =
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    unmapped
        .admin_template_delete("FerroEHR mapped person")
        .await
        .expect("an unmapped template deletes");
}
