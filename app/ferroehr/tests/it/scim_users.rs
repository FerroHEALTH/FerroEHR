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
use ferroehr::service::demographic::user_write::{Deactivations, UserUpdate, UserWriteError};
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use ferrohealth_smart::party::source::Lookup;

/// The professional `PERSON` archetype.
pub(crate) const PERSON: &str = "openEHR-DEMOGRAPHIC-PERSON.professional.v1.0.0";
/// The professional `ROLE` archetype.
pub(crate) const ROLE: &str = "openEHR-DEMOGRAPHIC-ROLE.professional.v1.0.0";
/// The identifier element every selector shares.
pub(crate) const KEY_PATH: &str = "/details[id2]/items[id3]";
/// The name element.
pub(crate) const NAME_PATH: &str = "/identities[id4]/details[id5]/items[id6]";
/// The e-mail element under `contacts`.
const EMAIL_PATH: &str = "/contacts[id10]/addresses[id11]/details[id12]/items[id13]";
/// The `binding_issuer` of the client under test.
pub(crate) const LOGIN: &str = "urn:example:staff-login";
/// The `binding_issuer` of another client.
const OTHER: &str = "urn:example:other-login";
/// The `PARTY_REF.namespace`.
pub(crate) const NAMESPACE: &str = "demographic";

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
        contacts matches {{
            CONTACT[id10] matches {{
                addresses matches {{
                    ADDRESS[id11] matches {{
                        details matches {{
                            ITEM_TREE[id12] matches {{
                                items matches {{
                                    ELEMENT[id13] occurrences matches {{0..1}} matches {{
                                        value matches {{ DV_TEXT[id14] }}
                                    }}
                                }}
                            }}
                        }}
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
            [\"id10\"] = <text = <\"Contact\"> description = <\"A contact.\">>
            [\"id11\"] = <text = <\"Address\"> description = <\"An address.\">>
            [\"id12\"] = <text = <\"Address details\"> description = <\"The address.\">>
            [\"id13\"] = <text = <\"Email\"> description = <\"An e-mail address.\">>
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
pub(crate) async fn service() -> (testkit::TestDb, FerroEhrService) {
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
pub(crate) fn identifier(issuer: &str, kind: &str, id: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind })
}

/// The User of the client of `issuer` with the key `key`, the `userName`
/// `user_name`, the `externalId` `external_id` and the name `name`.
pub(crate) fn user(
    issuer: &str,
    key: &str,
    user_name: &str,
    external_id: &str,
    name: &str,
) -> NewUser {
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
pub(crate) async fn provision(
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

/// The bound of one hundred deactivations an hour, the `[scim]` default.
const BOUND: Deactivations = Deactivations {
    max: 100,
    window_minutes: 60,
};

/// Writes `update` to the User `id` of the client of `LOGIN` with `limit`.
async fn update(
    svc: &FerroEhrService,
    id: VoId,
    update: UserUpdate,
    limit: Deactivations,
) -> Result<Option<ProvisionedUser>, UserWriteError> {
    as_client(Box::pin(svc.update_user(LOGIN, id, update, limit))).await
}

/// The `active` write of `active`.
fn active(active: bool) -> UserUpdate {
    UserUpdate {
        active: Some(active),
        ..UserUpdate::default()
    }
}

/// RM common master06 §Version Lifecycle: an abandoned User completed in one
/// request is retrieved to `incomplete` and completed, two commits.
#[tokio::test]
async fn an_abandoned_user_completes_in_two_commits() {
    let (_db, svc) = service().await;
    let mut new = user(
        LOGIN,
        "9d0e1f2a-3b4c-4d5e-8f6a-8b9c0d1e2f3a",
        "lee@example.org",
        "ext-5",
        "Lee Roe",
    );
    new.values.retain(|(path, _)| path != NAME_PATH);
    let id = provision(&svc, LOGIN, new)
        .await
        .expect("created")
        .row
        .party;
    let abandoned = update(&svc, id, active(false), BOUND)
        .await
        .expect("abandoned")
        .expect("found");
    assert_eq!(abandoned.row.lifecycle_state, "801");
    let versions = |svc: &FerroEhrService| {
        let svc = svc.clone();
        async move {
            svc.versioned_party_revision_history(id)
                .await
                .expect("the history reads")
                .body["items"]
                .as_array()
                .map_or(0, Vec::len)
        }
    };
    let before = versions(&svc).await;
    let completed = update(
        &svc,
        id,
        UserUpdate {
            set: vec![(
                NAME_PATH.to_owned(),
                json!({ "_type": "DV_TEXT", "value": "Lee Roe" }),
            )],
            active: Some(true),
            ..UserUpdate::default()
        },
        BOUND,
    )
    .await
    .expect("completed")
    .expect("found");
    assert_eq!(completed.row.lifecycle_state, "532");
    assert_eq!(versions(&svc).await, before + 2, "two commits");
}

/// The test trigger that refuses every `complete` version the party store
/// writes, so the second commit of an abandoned User's completion fails.
const REFUSE_COMPLETE: &str = "
    CREATE FUNCTION party.refuse_complete() RETURNS trigger LANGUAGE plpgsql AS $$
    BEGIN
        IF NEW.lifecycle_state = '532' THEN
            RAISE EXCEPTION 'the second commit fails';
        END IF;
        RETURN NEW;
    END $$;
    CREATE TRIGGER refuse_complete BEFORE INSERT ON party.version
        FOR EACH ROW EXECUTE FUNCTION party.refuse_complete();
";

/// RM common master06 §Version Lifecycle: when the second commit of an
/// abandoned User's completion fails, the first stands, the User is left
/// `incomplete`, and the same request completes it on retry.
#[tokio::test]
async fn a_failed_second_commit_leaves_the_user_incomplete_until_a_retry() {
    let (db, svc) = service().await;
    let mut new = user(
        LOGIN,
        "4a5b6c7d-8e9f-4a0b-9c1d-2e3f4a5b6c7d",
        "kim@example.org",
        "ext-9",
        "Kim Doe",
    );
    new.values.retain(|(path, _)| path != NAME_PATH);
    let id = provision(&svc, LOGIN, new)
        .await
        .expect("created")
        .row
        .party;
    let abandoned = update(&svc, id, active(false), BOUND)
        .await
        .expect("abandoned")
        .expect("found");
    assert_eq!(abandoned.row.lifecycle_state, "801");
    let complete = || UserUpdate {
        set: vec![(
            NAME_PATH.to_owned(),
            json!({ "_type": "DV_TEXT", "value": "Kim Doe" }),
        )],
        active: Some(true),
        ..UserUpdate::default()
    };
    sqlx::raw_sql(REFUSE_COMPLETE)
        .execute(&db.pool())
        .await
        .expect("the trigger installs");
    let failed = update(&svc, id, complete(), BOUND).await;
    assert!(failed.is_err(), "the second commit fails: {failed:?}");
    let left = svc
        .provisioned_user(LOGIN, id)
        .await
        .expect("reads")
        .expect("found");
    assert_eq!(left.row.lifecycle_state, "553", "the retrieval stands");
    sqlx::raw_sql("DROP TRIGGER refuse_complete ON party.version")
        .execute(&db.pool())
        .await
        .expect("the trigger drops");
    let completed = update(&svc, id, complete(), BOUND)
        .await
        .expect("completed on retry")
        .expect("found");
    assert_eq!(completed.row.lifecycle_state, "532");
}

/// A deleted User's key stays behind as a tombstone: a later login on it is
/// refused as deprovisioned.
#[tokio::test]
async fn a_deleted_user_leaves_a_tombstone() {
    let (_db, svc) = service().await;
    let key = "0e1f2a3b-4c5d-4e6f-9a7b-9c0d1e2f3a4b";
    let id = provision(
        &svc,
        LOGIN,
        user(LOGIN, key, "max@example.org", "ext-6", "Max Roe"),
    )
    .await
    .expect("created")
    .row
    .party;
    let deleted = as_client(Box::pin(svc.delete_user(LOGIN, id, None, BOUND, 0)))
        .await
        .expect("deleted");
    assert!(deleted);
    assert!(
        svc.provisioned_user(LOGIN, id)
            .await
            .expect("answers")
            .is_none()
    );
    let lookup = svc
        .party_source(NAMESPACE)
        .read(LOGIN, "guid", key, None, "2026-06-01")
        .await
        .expect("the source answers");
    assert_eq!(lookup, Lookup::Deprovisioned);
}

/// Two replicas on one database share one bound on deactivations.
#[tokio::test]
async fn the_deactivation_bound_holds_across_two_replicas() {
    let (db, svc) = service().await;
    let replica = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
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
    let mut ids = Vec::new();
    for (index, key) in [
        "1f2a3b4c-5d6e-4f7a-8b8c-0d1e2f3a4b5c",
        "2a3b4c5d-6e7f-4a8b-9c9d-1e2f3a4b5c6d",
        "3b4c5d6e-7f8a-4b9c-8d0e-2f3a4b5c6d7e",
    ]
    .into_iter()
    .enumerate()
    {
        let new = user(
            LOGIN,
            key,
            &format!("u{index}@example.org"),
            &format!("e{index}"),
            "R",
        );
        ids.push(
            provision(&svc, LOGIN, new)
                .await
                .expect("created")
                .row
                .party,
        );
    }
    let bound = Deactivations {
        max: 2,
        window_minutes: 60,
    };
    update(&svc, ids[0], active(false), bound)
        .await
        .expect("the first deactivation");
    update(&replica, ids[1], active(false), bound)
        .await
        .expect("the second, on the other replica");
    let refused = update(&svc, ids[2], active(false), bound)
        .await
        .expect_err("the third is past the bound");
    assert!(
        matches!(refused, UserWriteError::Limited { max: 2, .. }),
        "{refused:?}"
    );
    let refused = as_client(Box::pin(replica.delete_user(LOGIN, ids[2], None, bound, 0)))
        .await
        .expect_err("a deletion counts against the same bound");
    assert!(
        matches!(refused, UserWriteError::Limited { .. }),
        "{refused:?}"
    );
}

/// A User of `LOGIN` with the key `key` and the e-mail contact `email`,
/// deactivated once so its second version shares the first one's content.
async fn user_with_contact(svc: &FerroEhrService, key: &str, email: &str) -> VoId {
    let mut new = user(LOGIN, key, email, &format!("ext-{key}"), "Ada Roe");
    new.values.push((
        EMAIL_PATH.to_owned(),
        json!({ "_type": "DV_TEXT", "value": email }),
    ));
    let id = provision(svc, LOGIN, new).await.expect("created").row.party;
    update(svc, id, active(false), BOUND)
        .await
        .expect("deactivated")
        .expect("found");
    id
}

/// The stored body of every version of `id` with content, in order.
async fn stored_bodies(db: &testkit::TestDb, id: VoId) -> Vec<Value> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT body FROM party.version WHERE vo_id = $1 AND body IS NOT NULL \
         ORDER BY sys_version",
    )
    .bind(id.0)
    .fetch_all(&db.pool())
    .await
    .expect("the versions read");
    rows.iter()
        .map(|body| serde_json::from_str(body).expect("a JSON body"))
        .collect()
}

/// How many node rows of `id` belong to a `CONTACT` or below one.
async fn contact_rows(db: &testkit::TestDb, id: VoId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM party.node \
         WHERE vo_id = $1 AND rm_type IN ('CONTACT', 'ADDRESS')",
    )
    .bind(id.0)
    .fetch_one(&db.pool())
    .await
    .expect("the nodes count")
}

/// Asserts that every stored version of `id` holds the name and the three
/// identifiers and no contact, and that the node rows agree with the bodies.
async fn assert_attribution_minimum(db: &testkit::TestDb, svc: &FerroEhrService, id: VoId) {
    let bodies = stored_bodies(db, id).await;
    assert_eq!(bodies.len(), 2, "both versions with content are kept");
    for body in &bodies {
        assert!(body.get("contacts").is_none(), "no contact is left: {body}");
        assert_eq!(
            body.pointer("/identities/0/details/items/0/value/value"),
            Some(&Value::from("Ada Roe")),
            "the name stays"
        );
        let identifiers = body
            .pointer("/details/items")
            .and_then(Value::as_array)
            .expect("the identifiers stay");
        assert_eq!(identifiers.len(), 3, "{body}");
    }
    assert_eq!(contact_rows(db, id).await, 0, "no node row holds a contact");
    let parity = svc
        .rebuild_version_nodes(ferroehr::storage::integrity::StorageParityScope {
            vo_id: Some(id.0),
            ..Default::default()
        })
        .await
        .expect("the sweep runs");
    assert_eq!(parity.versions_damaged, 0, "{parity:?}");
}

/// #5178: with no period, the deletion erases the contacts of every version
/// of the `PERSON` and keeps its name, key and identifiers; the tombstone
/// still refuses the login, and the erasure is an access record.
#[tokio::test]
async fn a_deletion_erases_the_contacts_and_keeps_the_attribution_minimum() {
    let (db, svc) = service().await;
    let audit = ferroehr::system_log::config::AuditConfig {
        enabled: true,
        store: ferroehr::system_log::config::StoreConfig {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let (sender, handle) = ferroehr::system_log::sender::start(audit, None, Some(db.pool()))
        .await
        .expect("the audit sender");
    let svc = svc.with_audit(sender);
    let key = "5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c8d";
    let id = user_with_contact(&svc, key, "ada@example.org").await;
    assert_eq!(stored_bodies(&db, id).await.len(), 2);
    assert!(contact_rows(&db, id).await > 0, "the contact is stored");

    let deleted = as_client(Box::pin(svc.delete_user(LOGIN, id, None, BOUND, 0)))
        .await
        .expect("deleted");
    assert!(deleted);
    assert_attribution_minimum(&db, &svc, id).await;
    let lookup = svc
        .party_source(NAMESPACE)
        .read(LOGIN, "guid", key, None, "2026-06-01")
        .await
        .expect("the source answers");
    assert_eq!(lookup, Lookup::Deprovisioned);
    assert_eq!(
        svc.erase_due_contacts(&[LOGIN.to_owned()], 0)
            .await
            .expect("the sweep runs"),
        0,
        "the deletion erased them already"
    );

    drop(svc);
    handle.shutdown(std::time::Duration::from_secs(5)).await;
    let recorded: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT action, principal FROM audit.audit_event WHERE resource_id = $1")
            .bind(format!("party:{id}#contacts"))
            .fetch_all(&db.pool())
            .await
            .expect("the audit store reads");
    assert_eq!(
        recorded,
        vec![("D".to_owned(), Some("provisioning".to_owned()))],
        "one deletion record naming the client"
    );
}

/// #5178: with a period, the contacts stay until it has passed, and the
/// sweep erases them once, for the deleted Users of the issuers it is given.
#[tokio::test]
async fn the_sweep_erases_the_contacts_once_the_period_has_passed() {
    let (db, svc) = service().await;
    let id = user_with_contact(
        &svc,
        "6b7c8d9e-0f1a-4b2c-9d3e-4f5a6b7c8d9e",
        "bo@example.org",
    )
    .await;
    as_client(Box::pin(svc.delete_user(LOGIN, id, None, BOUND, 60)))
        .await
        .expect("deleted");
    let kept = stored_bodies(&db, id).await;
    assert!(
        kept.iter().all(|body| body.get("contacts").is_some()),
        "the period has not passed"
    );
    assert_eq!(
        svc.erase_due_contacts(&[LOGIN.to_owned()], 60)
            .await
            .expect("the sweep runs"),
        0
    );

    sqlx::query(
        "UPDATE party.binding_identifier SET deprovisioned_at = now() - interval '61 minutes' \
         WHERE party_id = $1 AND deprovisioned_at IS NOT NULL",
    )
    .bind(id.0)
    .execute(&db.pool())
    .await
    .expect("the deletion is backdated");
    assert_eq!(
        svc.erase_due_contacts(&[OTHER.to_owned()], 60)
            .await
            .expect("the sweep runs"),
        0,
        "another client's sweep does not reach the User"
    );
    assert_eq!(
        svc.erase_due_contacts(&[LOGIN.to_owned()], 60)
            .await
            .expect("the sweep runs"),
        1
    );
    assert_attribution_minimum(&db, &svc, id).await;
    assert_eq!(
        svc.erase_due_contacts(&[LOGIN.to_owned()], 60)
            .await
            .expect("the sweep runs"),
        0,
        "each User is erased once"
    );
}
