// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The committer the server writes for a bound caller, and the judgement of a
//! stated one (#5005), over a real PostgreSQL 18.
//!
//! RM common `org.openehr.rm.common.audit_details.adoc`: `committer` is the
//! "Identity and optional reference into identity management service, of user
//! who committed the item"; RM common master04 §Audit Details names the
//! "system login identifier" and allows `PARTY_SELF` only "when the committer
//! is the record subject". EHDS Art. 13(4) asks the record to identify the
//! professional and the provider. Refusing a stated committer that differs
//! deviates from the ITS-REST merge rule (`Requests_and_responses.md`
//! §openehr-version and openehr-audit-details), reported in #5019.

use std::sync::Arc;

use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::Outcome;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::resource::Caller;
use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, if_match, object, written_vo};
use crate::policy_fixtures::{as_client, capability, granted, identified, scope};
use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::{BindingMapping, IdentifierSelector, NameSelector, PersonSelectors};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::message::import_policy::ImportPolicy;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::bound::BoundDetails;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

const PERSON_TEMPLATE: &str = "FerroEHR professional";
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";
/// The token issuer.
pub(crate) const ISSUER: &str = "https://login.example";
/// The `identifier_issuer` of its `[[auth.party]]` table.
const LOGIN: &str = "https://login.example/tenant";
const NAME_PATH: &str = "/identities[at0002]/details[at0003]/items[at0004]";
const PROVIDER: (&str, &str) = ("urn:oid:2.16.528.1.1007.3.3", "12345678");

fn mapping() -> BindingMapping {
    let key = IdentifierSelector::new(
        format!("/details[at0001]/items[{CLUSTER}]/items[at0001]"),
        "guid",
    )
    .expect("compiles");
    BindingMapping::default()
        .with_person_selectors(
            PERSON_TEMPLATE,
            PersonSelectors::new(key).with_name(NameSelector::new(NAME_PATH).expect("compiles")),
        )
        .with_professional_role(ROLE_TEMPLATE)
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn archetyped(archetype: &str, template: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "template_id": { "_type": "TEMPLATE_ID", "value": template },
            "rm_version": "1.1.0" })
}

fn party_ref(class: &str, id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": class,
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0002", "name": text("name"),
             "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0003", "name": text("s"),
                 "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0004",
                             "name": text("name"), "value": text(name) }] } }])
}

/// A professional `PERSON` named `name` carrying the key `key`.
fn person(name: &str, key: &str) -> Value {
    json!({
        "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", PERSON_TEMPLATE),
        "name": text("professional"), "identities": identities(name),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("d"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": CLUSTER, "name": text("ids"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0001", "name": text("id"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": LOGIN, "id": key,
                               "type": "guid" } }] }] }
    })
}

fn write<T: serde::de::DeserializeOwned>(data: &Value, preceding: Option<&str>) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Commits a professional named `name` with the key `key`; returns its
/// container.
pub(crate) async fn professional(svc: &FerroEhrService, name: &str, key: &str) -> VoId {
    let body = person(name, key);
    let created = Box::pin(svc.party_create(PartyKind::Person, write(&body, None)))
        .await
        .expect("the PERSON");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    let vo = written_vo(&created);
    let role = json!({
        "_type": "ROLE", "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", ROLE_TEMPLATE),
        "name": text("professional"), "identities": identities("role"),
        "performer": party_ref("PERSON", vo)
    });
    let role = Box::pin(svc.party_create(PartyKind::Role, write(&role, None)))
        .await
        .expect("the ROLE");
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", written_vo(&role))]);
    Box::pin(svc.party_update(
        PartyKind::Person,
        object(vo.to_string()),
        if_match(&uid),
        write(&listed, Some(&uid)),
    ))
    .await
    .expect("the binding");
    vo
}

/// The identity of the login `sub` whose key `key` the binding binds.
pub(crate) async fn bound(svc: &FerroEhrService, sub: &str, key: &str) -> CommitterIdentity {
    let table = PartySection {
        issuer: SecretUrl::new(ISSUER),
        identifier_issuer: LOGIN.to_owned(),
        identifier_type: "guid".to_owned(),
        ..PartySection::default()
    };
    let source = svc.party_source("demographic");
    let binding = PartyBinding::new(Arc::new(source.clone()))
        .with_issuer(
            table,
            ferrohealth_smart::kind::IssuerKind::Persons,
            &ferrohealth_smart::profile::Profile::default(),
        )
        .expect("the table binds");
    let login = Caller::new(
        sub.to_owned(),
        Some("app".to_owned()),
        "user/*.cruds",
        None,
        false,
        CallerKind::Person,
    )
    .with_issuer(ISSUER.to_owned())
    .with_person_key(Some(key.to_owned()), None);
    let outcome = binding.resolve(&login).await.expect("answers");
    let party = outcome.as_ref().and_then(Outcome::bound).cloned();
    let party_id = party
        .as_ref()
        .map(|p| p.party.id().parse().expect("a uuid"))
        .expect("bound");
    let name = source.name(party_id).await.expect("the name");
    CommitterIdentity {
        subject: sub.to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        client: Some("app".to_owned()),
        person: false,
        party,
        representative: false,
        bound: Some(BoundDetails {
            name,
            provider: Some((PROVIDER.0.to_owned(), PROVIDER.1.to_owned())),
            namespace: "demographic".to_owned(),
        }),
        system: false,
        system_client: None,
        external: None,
    }
}

/// A composition commit naming `committer`, or with no stated audit.
fn version(committer: Option<&Value>) -> WriteRequest<openehr_rm::prelude::Composition> {
    let mut request = write(&composition("c"), None);
    if let Some(committer) = committer {
        request.audit = Some(
            openehr_its::json::from_canonical_value(&json!({
                "change_type": change_type("249", "creation"), "committer": committer
            }))
            .expect("an UPDATE_AUDIT"),
        );
    }
    request
}

/// Commits a composition into `ehr` as `identity`, naming `committer`, and
/// returns the stored committer.
async fn commit(
    svc: &FerroEhrService,
    ehr: EhrId,
    identity: CommitterIdentity,
    committer: Option<&Value>,
) -> Result<Value, ServiceError> {
    let written = with_committer(
        Some(identity),
        Box::pin(svc.create_composition(ehr, version(committer))),
    )
    .await?;
    let landed = &written;
    let ovid = format!(
        "{}::{}::{}",
        landed.vo_id, landed.creating_system_id, landed.tree
    );
    Ok(svc
        .composition_version_envelope(ehr, ovid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the stored version")
        .body["commit_audit"]["committer"]
        .clone())
}

/// The placeholder naming `issuer` and `id`.
pub(crate) fn placeholder(issuer: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id }] })
}

pub(crate) async fn service() -> (testkit::TestDb, FerroEhrService, EhrId) {
    let (db, svc, ehr) = crate::fixtures::setup().await;
    (db, svc.with_binding_mapping(mapping()), ehr)
}

fn forbidden(result: Result<Value, ServiceError>) {
    match result.map_err(ServiceError::flattened) {
        Err(ServiceError::Forbidden(_)) => {}
        other => panic!("expected 403, got {other:?}"),
    }
}

pub(crate) const KEY: &str = "1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e";

#[tokio::test]
async fn a_bound_professional_commits_under_the_server_committer() {
    let (_db, svc, ehr) = service().await;
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let identity = bound(&svc, "sub-a", KEY).await;
    let stored = commit(&svc, ehr, identity.clone(), None)
        .await
        .expect("commits");
    assert_eq!(
        stored,
        json!({
            "_type": "PARTY_IDENTIFIED",
            "external_ref": { "_type": "PARTY_REF",
                              "id": { "_type": "HIER_OBJECT_ID", "value": vo.to_string() },
                              "namespace": "demographic", "type": "PERSON" },
            "name": "Dr Jane Roe",
            "identifiers": [
                { "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": KEY, "type": "oauth2" },
                { "_type": "DV_IDENTIFIER", "issuer": PROVIDER.0, "id": PROVIDER.1,
                  "type": "healthcare-provider" }
            ]
        })
    );
    // The placeholder naming the caller is replaced by the server's committer.
    let replaced = commit(
        &svc,
        ehr,
        identity.clone(),
        Some(&placeholder(ISSUER, "sub-a")),
    )
    .await
    .expect("the placeholder");
    assert_eq!(replaced, stored);
    // A second application's pairwise sub binds one login identifier.
    let other_app = commit(&svc, ehr, bound(&svc, "sub-b", KEY).await, None)
        .await
        .expect("commits");
    assert_eq!(other_app["identifiers"][0], stored["identifiers"][0]);
    // A committer stating nothing the server's does not is accepted.
    let same = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": KEY, "type": "oauth2" }] });
    assert_eq!(
        commit(&svc, ehr, identity, Some(&same))
            .await
            .expect("same"),
        stored
    );
}

#[tokio::test]
async fn a_committer_that_is_not_the_callers_is_refused() {
    let (_db, svc, ehr) = service().await;
    professional(&svc, "Dr Jane Roe", KEY).await;
    let identity = bound(&svc, "sub-a", KEY).await;
    for stated in [
        placeholder(ISSUER, "sub-other"),
        placeholder("https://other.example", "sub-a"),
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Fake" }),
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "urn:x", "id": "9", "type": "x" }] }),
        json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe",
            "external_ref": party_ref("PERSON", VoId(uuid::Uuid::now_v7())) }),
        json!({ "_type": "PARTY_SELF" }),
        json!({ "_type": "PARTY_RELATED", "name": "Dr Jane Roe",
            "relationship": { "_type": "DV_CODED_TEXT", "value": "unknown",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "253" } } }),
    ] {
        forbidden(commit(&svc, ehr, identity.clone(), Some(&stated)).await);
    }
    // An empty committer breaks PARTY_IDENTIFIED.Basic_validity: 422.
    match commit(
        &svc,
        ehr,
        identity,
        Some(&json!({ "_type": "PARTY_IDENTIFIED" })),
    )
    .await
    .map_err(ServiceError::flattened)
    {
        Err(ServiceError::Unprocessable { .. }) => {}
        other => panic!("expected 422, got {other:?}"),
    }
}

#[tokio::test]
async fn a_person_with_no_name_commits_a_void_name() {
    let (_db, svc, ehr) = service().await;
    let mut body = person("x", KEY);
    body["identities"] = json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0009",
        "name": text("other"), "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0003",
        "name": text("s"), "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0005",
        "name": text("alias"), "value": text("x") }] } }]);
    let created = Box::pin(svc.party_create(PartyKind::Person, write(&body, None)))
        .await
        .expect("the PERSON");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    let vo = written_vo(&created);
    let role = json!({
        "_type": "ROLE", "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", ROLE_TEMPLATE),
        "name": text("professional"), "identities": identities("role"),
        "performer": party_ref("PERSON", vo)
    });
    let role = Box::pin(svc.party_create(PartyKind::Role, write(&role, None)))
        .await
        .expect("ROLE");
    body["roles"] = json!([party_ref("ROLE", written_vo(&role))]);
    Box::pin(svc.party_update(
        PartyKind::Person,
        object(vo.to_string()),
        if_match(&uid),
        write(&body, Some(&uid)),
    ))
    .await
    .expect("the binding");
    let stored = commit(&svc, ehr, bound(&svc, "sub-a", KEY).await, None)
        .await
        .expect("commits");
    assert!(stored.get("name").is_none(), "{stored}");
    assert_eq!(stored["identifiers"][0]["id"], KEY);
}

#[tokio::test]
async fn a_patient_commits_no_demographic_version() {
    let (_db, svc, _) = service().await;
    let patient = CommitterIdentity {
        subject: "jane".to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        client: None,
        person: true,
        party: None,
        representative: false,
        bound: None,
        system: false,
        system_client: None,
        external: None,
    };
    let body = json!({ "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", "Any person"),
        "name": text("p"), "identities": identities("p") });
    let refused = with_committer(
        Some(patient),
        Box::pin(svc.party_create(PartyKind::Person, write(&body, None))),
    )
    .await
    .expect_err("a patients token commits no demographic version")
    .flattened();
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
}

#[tokio::test]
async fn an_import_needs_the_role_and_refuses_a_local_login_from_an_untrusted_system() {
    let (_source_db, source) = crate::fixtures::service().await;
    let source = source.with_system_id("legacy.example");
    let (_target_db, target) = crate::fixtures::service().await;
    let target = crate::policy_fixtures::granting(target).await;
    let ehr = crate::fixtures::create_ehr(&source).await;
    let local_login = json!({ "_type": "PARTY_IDENTIFIED", "name": "bob",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "ferroehr", "id": "bob", "type": "basic" }] });
    let mut stated = WriteRequest::new(crate::typed_body::typed(&composition("from elsewhere")));
    stated.audit = Some(
        openehr_its::json::from_canonical_value(&json!({
            "change_type": change_type("249", "creation"), "committer": local_login
        }))
        .expect("an UPDATE_AUDIT"),
    );
    Box::pin(source.create_composition(ehr, stated))
        .await
        .expect("the source commits");
    let extracts = source.extract_ehrs(ehr).await.expect("export");
    let extract: openehr_rm::prelude::Extract =
        openehr_its::json::from_canonical_value(&extracts[0]).expect("an EXTRACT");

    let strict = target.with_import_policy(ImportPolicy::new(
        Some("IMPORTER".to_owned()),
        vec![ISSUER.to_owned()],
    ));
    // A caller without the import role is refused.
    let clinician = CommitterIdentity {
        subject: "clinician".to_owned(),
        id_type: "basic",
        issuer: None,
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        system_client: None,
        external: None,
    };
    let refused = ferroehr::system_log::access_context::with_roles(
        vec!["USER".to_owned()],
        with_committer(
            Some(clinician.clone()),
            Box::pin(strict.import_ehr(None, extract.clone())),
        ),
    )
    .await
    .expect_err("no import role");
    assert!(matches!(refused.flattened(), ServiceError::Forbidden(_)));
    // With the role, a wrapped committer carrying a login of this deployment
    // from an untrusted system is refused.
    let refused = ferroehr::system_log::access_context::with_roles(
        vec!["IMPORTER".to_owned()],
        with_committer(
            Some(clinician.clone()),
            Box::pin(strict.import_ehr(None, extract.clone())),
        ),
    )
    .await
    .expect_err("a local login from an untrusted system");
    assert!(matches!(refused.flattened(), ServiceError::Forbidden(_)));
    // A grant that vouches for the source system on another operation, or
    // vouches for this node itself, trusts nothing for an EHR import.
    let source_system = source_system_id(&extracts[0]);
    assert_eq!(source_system, "legacy.example");
    let extract_only = capability(
        "import_ehr_extract",
        &[scope("creating_system", &source_system)],
    );
    let own_node = capability(
        "import_ehr",
        &[scope(
            "creating_system",
            ferroehr::service::DEFAULT_SYSTEM_ID,
        )],
    );
    granted(&strict, "extract-importer", &[extract_only, own_node]).await;
    let refused = as_client(
        &strict,
        "extract-importer",
        ferroehr::system_log::access_context::with_roles(
            vec!["IMPORTER".to_owned()],
            with_committer(
                Some(clinician.clone()),
                Box::pin(strict.import_ehr(None, extract.clone())),
            ),
        ),
    )
    .await
    .expect_err("no import_ehr grant vouches for the source system");
    assert!(matches!(refused.flattened(), ServiceError::Forbidden(_)));
    // From a system the caller's import grant vouches for, read from the CDR
    // on the request, it lands, and the wrapped committer is kept.
    granted(
        &strict,
        "migrator",
        &[capability(
            "import_ehr",
            &[identified("creating_system", &source_system)],
        )],
    )
    .await;
    as_client(
        &strict,
        "migrator",
        ferroehr::system_log::access_context::with_roles(
            vec!["IMPORTER".to_owned()],
            with_committer(Some(clinician), Box::pin(strict.import_ehr(None, extract))),
        ),
    )
    .await
    .expect("a vouched-for system's import lands");
}

/// The `creating_system_id` of the first wrapped version of `extract`.
pub(crate) fn source_system_id(extract: &Value) -> String {
    let text = extract.to_string();
    let marker = "::";
    // NOTE: every wrapped version uid of the fixture is `{vo}::{system}::{tree}`.
    let uid = crate::fixtures::find_by_xtype(extract, "X_VERSIONED_COMPOSITION")
        .and_then(|item| {
            item.pointer("/versions/0/uid/value")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| text.clone());
    uid.split(marker).nth(1).unwrap_or_default().to_owned()
}

/// The key of the representative's `PERSON`.
const REPRESENTATIVE_KEY: &str = "2c3d4e5f-6a7b-4c8d-9e0f-1a2b3c4d5e6f";

/// `composition("…")` composed by `composer`.
fn composed_by(name: &str, composer: &Value) -> Value {
    let mut body = composition(name);
    body["composer"] = composer.clone();
    body
}

/// The `PARTY_RELATED` composer a representative names.
fn related() -> Value {
    json!({ "_type": "PARTY_RELATED", "name": "Rep Roe",
        "relationship": { "_type": "DV_CODED_TEXT", "value": "unknown",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "253" } } })
}

/// Writes `body` into `ehr` as `identity`: a creation, or with `preceding` an
/// update of that version; the version uid it landed.
async fn write_as(
    svc: &FerroEhrService,
    ehr: EhrId,
    identity: CommitterIdentity,
    body: &Value,
    preceding: Option<(&VoId, &str)>,
) -> Result<String, ServiceError> {
    let written = match preceding {
        None => {
            with_committer(
                Some(identity),
                Box::pin(svc.create_composition(ehr, write(body, None))),
            )
            .await
        }
        Some((vo, uid)) => {
            with_committer(
                Some(identity),
                Box::pin(svc.update_composition(ehr, *vo, write(body, Some(uid)))),
            )
            .await
        }
    }
    .map_err(ServiceError::flattened)?;
    Ok(format!(
        "{}::{}::{}",
        written.vo_id, written.creating_system_id, written.tree
    ))
}

/// The versioned object of the version uid `uid`.
fn vo_of(uid: &str) -> VoId {
    VoId(
        uid.split("::")
            .next()
            .and_then(|id| id.parse().ok())
            .expect("a versioned object id"),
    )
}

/// #5359: a representative's write is the person's under EHDS Art. 5. It names
/// the person or the representative as composer, may not alter a
/// professional's entry, is recorded as `PARTY_RELATED`, and the person may
/// change it and sees it flagged as inserted for them.
#[tokio::test]
async fn a_representatives_write_is_held_to_the_persons_rules() {
    let (_db, svc, ehr) = service().await;
    professional(&svc, "Dr Jane Roe", KEY).await;
    professional(&svc, "Rep Roe", REPRESENTATIVE_KEY).await;
    let clinician = bound(&svc, "sub-a", KEY).await;
    let mut representative = bound(&svc, "sub-r", REPRESENTATIVE_KEY).await;
    representative.representative = true;
    let patient = CommitterIdentity {
        subject: "jane".to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        client: None,
        person: true,
        party: None,
        representative: false,
        bound: None,
        system: false,
        system_client: None,
        external: None,
    };
    let refused = |result: Result<String, ServiceError>| match result {
        Err(ServiceError::Restricted { .. } | ServiceError::Forbidden(_)) => {}
        other => panic!("expected 403, got {other:?}"),
    };

    let by_professional = write_as(&svc, ehr, clinician, &composition("by Dr Roe"), None)
        .await
        .expect("the professional's entry");
    refused(
        write_as(
            &svc,
            ehr,
            representative.clone(),
            &composed_by("altered", &related()),
            Some((&vo_of(&by_professional), &by_professional)),
        )
        .await,
    );
    refused(
        write_as(
            &svc,
            ehr,
            representative.clone(),
            &composition("as a professional"),
            None,
        )
        .await,
    );
    let by_representative = write_as(
        &svc,
        ehr,
        representative,
        &composed_by("by the representative", &related()),
        None,
    )
    .await
    .expect("the representative's entry");
    let stored = svc
        .composition_version_envelope(ehr, by_representative.parse().expect("a version uid"))
        .await
        .expect("the stored version")
        .body["commit_audit"]["committer"]
        .clone();
    assert_eq!(stored["_type"], "PARTY_RELATED", "{stored}");
    write_as(
        &svc,
        ehr,
        patient,
        &composed_by("amended by the person", &json!({ "_type": "PARTY_SELF" })),
        Some((&vo_of(&by_representative), &by_representative)),
    )
    .await
    .expect("the person amends the representative's entry");

    let listed = svc
        .ehr_contribution_list_page(ehr, 0, 10)
        .await
        .expect("the contribution list");
    let flags: Vec<(String, bool)> = listed["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| {
            (
                row["committer"].as_str().unwrap_or_default().to_owned(),
                row["inserted_by_person"].as_bool().expect("a flag"),
            )
        })
        .collect();
    assert!(flags.contains(&("Rep Roe".to_owned(), true)), "{flags:?}");
    assert!(
        flags.contains(&("Dr Jane Roe".to_owned(), false)),
        "{flags:?}"
    );
}
