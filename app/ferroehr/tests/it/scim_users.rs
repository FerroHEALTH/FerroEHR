// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Users of a SCIM client as the demographic service holds them (#5014),
//! over a real PostgreSQL 18: a creation commits the `PERSON` and its
//! professional `ROLE` in one demographic CONTRIBUTION, each naming the other
//! (RM demographic `org.openehr.rm.demographic.actor.adoc`, `…role.adoc`), and
//! a client finds only its own Users, by `id`, `userName` (compared by its
//! fold, RFC 7643 §4.1.1) and `externalId` (exactly, RFC 7643 §3.1).
//!
//! The definitions are synthetic ADL 2 archetypes the ADL 2 store compiles to
//! their OPT2. No openEHR spec governs SCIM: our own design.

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::provisioned::UserFilter;
use ferroehr::party_binding::{BindingMapping, IdentifierSelector, PersonSelectors};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::provision::{NewUser, ProvisionedUser};
use ferroehr::service::demographic::template::MappedPartyTemplates;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The professional `PERSON` archetype.
const PERSON: &str = "openEHR-DEMOGRAPHIC-PERSON.professional.v1.0.0";
/// The professional `ROLE` archetype.
const ROLE: &str = "openEHR-DEMOGRAPHIC-ROLE.professional.v1.0.0";
/// The identifier element every selector shares.
const KEY_PATH: &str = "/details[id2]/items[id3]";
/// The name element.
const NAME_PATH: &str = "/identities[id4]/details[id5]/items[id6]";
/// The `binding_issuer` of the client under test.
const LOGIN: &str = "urn:example:staff-login";
/// The `binding_issuer` of another client.
const OTHER: &str = "urn:example:other-login";
/// The `PARTY_REF.namespace`.
const NAMESPACE: &str = "demographic";

/// A synthetic ADL 2 archetype `hrid` rooted at `root`, with a name in
/// `identities` and repeatable identifiers in `details`.
fn archetype(hrid: &str, root: &str) -> String {
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
            purpose = <\"A party SCIM provisioning writes.\">
        >
    >

definition
    {root}[id1] matches {{
        identities matches {{
            PARTY_IDENTITY[id4] matches {{
                details matches {{
                    ITEM_TREE[id5] matches {{
                        items matches {{
                            ELEMENT[id6] occurrences matches {{1..1}} matches {{
                                value matches {{ DV_TEXT[id8] }}
                            }}
                        }}
                    }}
                }}
            }}
        }}
        details matches {{
            ITEM_TREE[id2] matches {{
                items matches {{
                    ELEMENT[id3] occurrences matches {{0..*}} matches {{
                        value matches {{ DV_IDENTIFIER[id9] }}
                    }}
                }}
            }}
        }}
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
        >
    >
"
    )
}

/// The key, `userName` and `externalId` selectors: one path, told apart by
/// type.
fn selectors() -> PersonSelectors {
    let selector = |kind: &str| IdentifierSelector::new(KEY_PATH, kind).expect("compiles");
    PersonSelectors::new(selector("guid").with_issuers([LOGIN, OTHER]))
        .with_user_name(selector("scim_user_name"))
        .with_external_id(selector("scim_external_id"))
}

/// A service over a fresh database holding the two archetypes, with the
/// mapping installed.
async fn service() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    for (hrid, root) in [(PERSON, "PERSON"), (ROLE, "ROLE")] {
        svc.template_adl2_upload(&archetype(hrid, root))
            .await
            .expect("the archetype uploads");
    }
    let svc = svc
        .with_mapped_party_templates(
            MappedPartyTemplates::default()
                .with_person_template(PERSON)
                .with_role_template(ROLE),
        )
        .with_binding_mapping(
            BindingMapping::default()
                .with_person_selectors(PERSON, selectors())
                .with_professional_role(ROLE),
        );
    (db, svc)
}

/// The `DV_IDENTIFIER` `id` of `kind` under `issuer`.
fn identifier(issuer: &str, kind: &str, id: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind })
}

/// The User of the client of `issuer` with the key `key`, the `userName`
/// `user_name`, the `externalId` `external_id` and the name `name`.
fn user(issuer: &str, key: &str, user_name: &str, external_id: &str, name: &str) -> NewUser {
    NewUser {
        template_id: PERSON.to_owned(),
        namespace: NAMESPACE.to_owned(),
        values: vec![
            (KEY_PATH.to_owned(), identifier(issuer, "guid", key)),
            (
                KEY_PATH.to_owned(),
                identifier(issuer, "scim_user_name", user_name),
            ),
            (
                KEY_PATH.to_owned(),
                identifier(issuer, "scim_external_id", external_id),
            ),
            (
                NAME_PATH.to_owned(),
                json!({ "_type": "DV_TEXT", "value": name }),
            ),
        ],
    }
}

/// Runs `fut` as a request of a SCIM client, a caller acting for nobody.
async fn as_client<F: Future>(fut: F) -> F::Output {
    let identity = CommitterIdentity {
        subject: "provisioning".to_owned(),
        id_type: "oauth2",
        issuer: Some("https://as.example".to_owned()),
        client: Some("provisioning".to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: true,
        system_client: None,
        external: None,
    };
    with_committer(Some(identity), fut).await
}

/// Creates `user` for the client of `issuer`.
async fn provision(
    svc: &FerroEhrService,
    issuer: &str,
    user: NewUser,
) -> Result<ProvisionedUser, ServiceError> {
    as_client(Box::pin(svc.provision_user(issuer, ROLE, user))).await
}

/// The ids of the Users of the client of `issuer` matching `filter`.
async fn found(svc: &FerroEhrService, issuer: &str, filter: UserFilter) -> Vec<VoId> {
    svc.provisioned_users(issuer, &filter, 0, 10)
        .await
        .expect("the store answers")
        .users
        .iter()
        .map(|user| user.row.party)
        .collect()
}

#[tokio::test]
async fn a_creation_commits_the_person_and_its_role_in_one_contribution() {
    let (_db, svc) = service().await;
    let key = "1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e";
    let created = provision(
        &svc,
        LOGIN,
        user(LOGIN, key, "jane@example.org", "ext-1", "Jane Roe"),
    )
    .await
    .expect("the User is created");
    let id = created.row.party;
    assert_eq!(created.row.login_identifier, key);
    assert_eq!(created.row.user_name.as_deref(), Some("jane@example.org"));
    assert_eq!(created.row.external_id.as_deref(), Some("ext-1"));
    assert_eq!(created.row.lifecycle_state, "532");
    assert!(created.missing.is_empty(), "{:?}", created.missing);
    assert_eq!(created.created, created.last_modified);
    assert_eq!(
        created
            .body
            .pointer("/identities/0/details/items/0/value/value"),
        Some(&Value::from("Jane Roe"))
    );

    // The PERSON lists the ROLE, the ROLE names the PERSON, and one
    // CONTRIBUTION created both.
    let role_ref = created
        .body
        .pointer("/roles/0/id/value")
        .and_then(Value::as_str)
        .expect("the PERSON lists its ROLE");
    let role = VoId::from_uid_head(role_ref).expect("a container id");
    let (person_version, role_version) = (current(&svc, id).await, current(&svc, role).await);
    assert_eq!(
        role_version.pointer("/data/performer/id/value"),
        Some(&Value::from(id.to_string()))
    );
    assert_eq!(
        person_version.pointer("/contribution/id/value"),
        role_version.pointer("/contribution/id/value"),
        "{person_version} {role_version}"
    );
    // The ROLE's identity holds no name, which its template requires.
    assert_eq!(
        role_version["lifecycle_state"]["defining_code"]["code_string"],
        "553"
    );
}

/// The current `ORIGINAL_VERSION` of the party `vo`.
async fn current(svc: &FerroEhrService, vo: VoId) -> Value {
    svc.versioned_party_version_get_at_time(vo, None)
        .await
        .expect("the version reads")
        .body
}

#[tokio::test]
async fn a_client_finds_only_its_own_users_by_id_user_name_and_external_id() {
    let (_db, svc) = service().await;
    let jane = provision(
        &svc,
        LOGIN,
        user(
            LOGIN,
            "2c3d4e5f-6a7b-4c8d-9e0f-1a2b3c4d5e6f",
            "Jane@Example.org",
            "ext-1",
            "Jane Roe",
        ),
    )
    .await
    .expect("created")
    .row
    .party;
    let john = provision(
        &svc,
        LOGIN,
        user(
            LOGIN,
            "3d4e5f6a-7b8c-4d9e-8f0a-2b3c4d5e6f7a",
            "john@example.org",
            "ext-2",
            "John Roe",
        ),
    )
    .await
    .expect("created")
    .row
    .party;
    let theirs = provision(
        &svc,
        OTHER,
        user(
            OTHER,
            "4e5f6a7b-8c9d-4e0f-9a1b-3c4d5e6f7a8b",
            "ann@example.org",
            "ext-1",
            "Ann Roe",
        ),
    )
    .await
    .expect("another client's User, with the same externalId")
    .row
    .party;

    let filter = |ids: &[VoId], user_names: &[&str], external_ids: &[&str]| UserFilter {
        ids: ids.iter().map(|id| id.0).collect(),
        user_names: user_names.iter().map(|n| (*n).to_owned()).collect(),
        external_ids: external_ids.iter().map(|e| (*e).to_owned()).collect(),
    };
    let mut both = vec![jane, john];
    both.sort();
    assert_eq!(found(&svc, LOGIN, UserFilter::default()).await, both);
    assert_eq!(found(&svc, LOGIN, filter(&[jane], &[], &[])).await, [jane]);
    assert_eq!(
        found(&svc, LOGIN, filter(&[], &["jane@EXAMPLE.org"], &[])).await,
        [jane]
    );
    assert_eq!(
        found(&svc, LOGIN, filter(&[], &[], &["ext-1"])).await,
        [jane]
    );
    assert_eq!(
        found(&svc, OTHER, filter(&[], &[], &["ext-1"])).await,
        [theirs]
    );
    assert_eq!(
        found(&svc, LOGIN, filter(&[], &[], &["EXT-1"])).await,
        Vec::<VoId>::new()
    );
    assert_eq!(
        found(
            &svc,
            LOGIN,
            filter(&[jane], &["jane@example.org"], &["ext-1"])
        )
        .await,
        [jane]
    );
    assert_eq!(
        found(&svc, LOGIN, filter(&[jane], &["john@example.org"], &[])).await,
        Vec::<VoId>::new()
    );
    assert_eq!(
        found(&svc, LOGIN, filter(&[theirs], &[], &[])).await,
        Vec::<VoId>::new()
    );
    assert!(
        svc.provisioned_user(LOGIN, theirs)
            .await
            .expect("answers")
            .is_none()
    );

    // A page and the total.
    let page = svc
        .provisioned_users(LOGIN, &UserFilter::default(), 1, 1)
        .await
        .expect("the store answers");
    assert_eq!(page.total, 2);
    assert_eq!(
        page.users.iter().map(|u| u.row.party).collect::<Vec<_>>(),
        [both[1]]
    );
}

#[tokio::test]
async fn a_taken_user_name_or_a_key_of_another_issuer_is_refused() {
    let (_db, svc) = service().await;
    provision(
        &svc,
        LOGIN,
        user(
            LOGIN,
            "5f6a7b8c-9d0e-4f1a-8b2c-4d5e6f7a8b9c",
            "jane@example.org",
            "ext-1",
            "Jane Roe",
        ),
    )
    .await
    .expect("created");

    let refused = provision(
        &svc,
        OTHER,
        user(
            OTHER,
            "6a7b8c9d-0e1f-4a2b-9c3d-5e6f7a8b9c0d",
            "JANE@example.org",
            "ext-9",
            "Jane Doe",
        ),
    )
    .await
    .expect_err("a userName is unique across clients");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");

    let refused = provision(
        &svc,
        LOGIN,
        user(
            OTHER,
            "7b8c9d0e-1f2a-4b3c-8d4e-6f7a8b9c0d1e",
            "joan@example.org",
            "ext-3",
            "Joan Roe",
        ),
    )
    .await
    .expect_err("a key the client's issuer does not issue");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
}

#[tokio::test]
async fn a_user_whose_person_lacks_a_required_value_is_incomplete() {
    let (_db, svc) = service().await;
    let mut new = user(
        LOGIN,
        "8c9d0e1f-2a3b-4c4d-9e5f-7a8b9c0d1e2f",
        "kim@example.org",
        "ext-4",
        "Kim Roe",
    );
    new.values.retain(|(path, _)| path != NAME_PATH);
    let created = provision(&svc, LOGIN, new).await.expect("created");
    assert_eq!(created.row.lifecycle_state, "553");
    assert!(!created.missing.is_empty(), "the missing name is named");
    // RM common master06 §Incomplete Content: an incomplete User is still a User.
    assert!(
        svc.provisioned_user(LOGIN, created.row.party)
            .await
            .expect("answers")
            .is_some()
    );
}
