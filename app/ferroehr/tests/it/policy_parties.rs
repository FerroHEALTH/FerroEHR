// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties (#5434) over a real PostgreSQL 18: the default policy
//! definitions of `deploy/definitions/policy/` load, the mapping the book's
//! "Operational policy" page documents passes the boot rules over them, each
//! worked example on that page fits its definition, and only an
//! administrator writes a policy party (the system `AGENT`, a policy `ROLE`,
//! an acts-for relationship inline or of its own). No openEHR spec governs
//! who writes a party: our own design.

use std::collections::HashMap;

use assert_fs::prelude::FileWriteStr as _;
use serde_json::{Value, json};

use crate::fixtures::{at, delete_precondition, if_match, object, written_vo};
use ferroehr::config::FerroEhrConfig;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::protect;
use ferroehr::service::FerroEhrService;
use ferroehr::service::definition::load::SourceFormat;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The policy definitions, in the order the lanes load them.
const POLICY_SET: [&str; 6] = [
    "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0.adls",
    "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1.0.0.adls",
    "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0.adls",
    "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0.adls",
    "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0.adls",
    "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0.adls",
];
/// The acts-for archetype, in the interface form a relationship carries.
const ACTS_FOR: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1";
/// The example ids of the book page, each replaced by the id of the party
/// the test commits: the provider, the partner, the `AGENT`, the
/// professional's `PERSON` and the system client's `ROLE`.
const PROVIDER: &str = "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40";
const PARTNER: &str = "7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36";
const AGENT: &str = "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73";
const PERSON: &str = "3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95";
const ROLE: &str = "e41c7b2a-6d3f-4a9e-8b05-2f7d1c9e3a68";
/// The administrator role of the tests.
const ADMIN: &str = "ADMIN";

/// The text of the book's "Operational policy" page.
fn book_page() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/book/src/installation/operational-policy.md");
    std::fs::read_to_string(path).expect("the book page is readable")
}

/// The fenced block after the `<!-- policy-example: name -->` marker of the
/// book page.
fn example(name: &str) -> String {
    let page = book_page();
    let marker = format!("<!-- policy-example: {name} -->\n");
    let after = page
        .split_once(&marker)
        .unwrap_or_else(|| panic!("the page marks the example {name}"))
        .1;
    let body = after
        .split_once('\n')
        .expect("a fence opens the example")
        .1;
    body.split_once("\n```")
        .expect("a fence closes the example")
        .0
        .to_owned()
}

/// The JSON example `name` with each example id replaced by its party.
fn body(name: &str, ids: &[(&str, VoId)]) -> Value {
    let mut text = example(name);
    for (example_id, id) in ids {
        text = text.replace(example_id, &id.to_string());
    }
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("the example {name} parses: {e}"))
}

/// The text of the policy definition `file`.
fn definition(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/definitions/policy")
        .join(file);
    std::fs::read_to_string(path).expect("the policy definition is readable")
}

/// The configuration of `toml`.
fn config(toml: &str) -> FerroEhrConfig {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(toml).expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[]).expect("assembles")
}

/// A service over a fresh database holding the policy definitions.
async fn loaded() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    for file in POLICY_SET {
        svc.definition_load(SourceFormat::Adl2, &definition(file))
            .await
            .unwrap_or_else(|e| panic!("{file} loads: {e:?}"));
    }
    (db, svc)
}

/// A service holding the policy definitions with the book's mapping
/// installed, as the server installs it.
async fn mapped() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = loaded().await;
    let config = config(&example("mapping"));
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    let svc = svc
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping);
    (db, svc)
}

#[tokio::test]
async fn the_book_mapping_passes_the_boot_rules_over_the_policy_definitions() {
    let (_db, svc) = loaded().await;
    let config = config(&example("mapping"));
    let sections = config.authz.party_binding.sections();
    assert!(sections.errors(&[], &[]).is_empty());
    let errors = svc
        .party_mapping_errors(&sections)
        .await
        .expect("the store answers");
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn each_policy_refusal_names_its_key() {
    let (_db, svc) = loaded().await;
    let wrong = example("mapping")
        .replace(
            "template_id = \"openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0\"",
            "template_id = \"openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0\"",
        )
        .replace(
            "archetype_id = \"openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1\"",
            "archetype_id = \"openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.unknown.v1\"",
        )
        .replace(
            "template_id = \"openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0\"",
            "template_id = \"openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0\"",
        );
    let refused = config(&wrong);
    let sections = refused.authz.party_binding.sections();
    let static_errors = sections.errors(&[], &[]);
    assert!(
        static_errors.iter().any(|e| e.contains(
            "authz.party_binding.data_migration_role and \
             authz.party_binding.system_client_role name one template"
        )),
        "{static_errors:?}"
    );
    let errors = svc
        .party_mapping_errors(&sections)
        .await
        .expect("the store answers");
    let joined = errors.join("\n");
    assert!(
        joined.contains("authz.party_binding.agent names")
            && joined.contains("rooted at ROLE, not AGENT"),
        "{joined}"
    );
    assert!(
        joined.contains("authz.party_binding.acts_for names"),
        "{joined}"
    );

    let moved = example("mapping").replace(
        "client = { path = \"/details[id2]/items[id3]\"",
        "client = { path = \"/details[id2]/items[id9]\"",
    );
    let moved = config(&moved);
    let errors = svc
        .party_mapping_errors(&moved.authz.party_binding.sections())
        .await
        .expect("the store answers");
    assert!(
        errors
            .iter()
            .any(|e| e.starts_with("authz.party_binding.agent client:")),
        "{errors:?}"
    );
}

/// The identity of a request by `subject`.
fn caller(subject: &str) -> CommitterIdentity {
    CommitterIdentity {
        subject: subject.to_owned(),
        id_type: "oauth2",
        issuer: Some("https://login.example".to_owned()),
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        system_client: None,
        external: None,
    }
}

/// Runs `fut` as a request of `subject` holding `roles`.
async fn as_caller<F: Future>(subject: &str, roles: &[&str], fut: F) -> F::Output {
    ferroehr::system_log::access_context::with_roles(
        roles.iter().map(|r| (*r).to_owned()).collect(),
        with_committer(Some(caller(subject)), fut),
    )
    .await
}

/// Runs `fut` as a clinician: a caller who is no administrator.
async fn as_clinician<F: Future>(fut: F) -> F::Output {
    as_caller("clinician", &["CLINICIAN"], fut).await
}

/// Runs `fut` as an administrator.
async fn as_admin<F: Future>(fut: F) -> F::Output {
    as_caller("admin", &[ADMIN], fut).await
}

/// The write of `data` after `preceding`, committed `complete`.
fn version<T: serde::de::DeserializeOwned>(
    data: &Value,
    preceding: Option<&str>,
) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Creates `body` as a party of `kind`.
async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
) -> Result<(VoId, String), ServiceError> {
    let created = Box::pin(svc.party_create(kind, version(body, None))).await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((written_vo(&created), uid))
}

/// Commits `body` as the next version of `vo` after `uid`.
async fn update(
    svc: &FerroEhrService,
    kind: PartyKind,
    vo: VoId,
    uid: &str,
    body: &Value,
) -> Result<String, ServiceError> {
    let updated = Box::pin(svc.party_update(
        kind,
        object(vo.to_string()),
        if_match(uid),
        version(body, Some(uid)),
    ))
    .await?;
    Ok(updated.meta.expect("a version").uid)
}

/// Deletes the party at `uid`.
async fn delete(svc: &FerroEhrService, kind: PartyKind, uid: &str) -> Result<(), ServiceError> {
    Box::pin(svc.party_delete(
        kind,
        object(uid),
        delete_precondition(uid, None),
        WriteRequest::new(()),
    ))
    .await
    .map(|_| ())
}

/// Asserts `refused` is the `403` of a policy party, naming the template or
/// archetype that makes it one.
fn assert_policy_refusal<T: std::fmt::Debug>(refused: Result<T, ServiceError>) {
    let refused = refused.expect_err("a policy party is written by an administrator only");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let text = refused.to_string();
    assert!(
        text.contains("is a policy party of the template openehr-demographic-")
            || text.contains(
                "is a policy party of the archetype \
                 openehr-demographic-party_relationship.acts_for.v1"
            ),
        "{text}"
    );
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A plain `PARTY` of `class` named `name`, of no mapped template.
fn plain(class: &str, archetype: &str, name: &str) -> Value {
    json!({
        "_type": class, "archetype_node_id": archetype, "name": text(name),
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" },
        "identities": [{
            "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                            "name": text("name"), "value": text(name) }] }
        }]
    })
}

/// The parties the examples refer to, which any caller writes: the
/// provider and the partner `ORGANISATION`s and a professional's `PERSON`.
async fn referred(svc: &FerroEhrService) -> [(VoId, String); 3] {
    let organisation = "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0";
    let provider = plain("ORGANISATION", organisation, "Example hospital");
    let partner = plain("ORGANISATION", organisation, "Regional care network");
    let person = plain("PERSON", "openEHR-DEMOGRAPHIC-PERSON.person.v0", "J. Jansen");
    as_clinician(async {
        [
            create(svc, PartyKind::Organisation, &provider)
                .await
                .expect("an organisation"),
            create(svc, PartyKind::Organisation, &partner)
                .await
                .expect("an organisation"),
            create(svc, PartyKind::Person, &person)
                .await
                .expect("a person"),
        ]
    })
    .await
}

#[tokio::test]
async fn each_worked_example_fits_its_definition() {
    let (_db, svc) = loaded().await;
    let any = VoId::from_uid_head(AGENT).expect("a uuid");
    let ids = [(AGENT, any), (PROVIDER, any), (PARTNER, any), (PERSON, any), (ROLE, any)];
    for (name, template) in [
        ("agent", "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0"),
        ("agent-acts-for", "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0"),
        ("system-client-role", "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0"),
        ("data-migration-role", "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0"),
        (
            "federation-partner-role",
            "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0",
        ),
        ("privileged-role", "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0"),
    ] {
        let fit = svc
            .party_template_fit(&body(name, &ids), template)
            .await
            .unwrap_or_else(|e| panic!("{name} fits {template}: {e:?}"));
        assert_eq!(
            fit,
            ferroehr::service::demographic::template::TemplateFit::Complete,
            "{name}"
        );
    }
    let relationship = &body("agent-acts-for", &ids)["relationships"][0];
    let fit = svc
        .party_template_fit(relationship, "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1.0.0")
        .await
        .expect("the acts-for relationship fits its archetype");
    assert_eq!(
        fit,
        ferroehr::service::demographic::template::TemplateFit::Complete
    );
}

#[tokio::test]
async fn only_an_administrator_writes_a_system_client() {
    let (_db, svc) = mapped().await;
    let [(provider, _), _, _] = referred(&svc).await;

    // Nobody but an administrator creates the system AGENT, and even an
    // administrator's is held to its definition: it carries its identifier.
    let agent = body("agent", &[]);
    let mut anonymous = agent.clone();
    anonymous
        .as_object_mut()
        .expect("an object")
        .remove("details");
    let refused = as_admin(create(&svc, PartyKind::Agent, &anonymous))
        .await
        .expect_err("an AGENT without its identifier");
    assert!(
        matches!(refused, ServiceError::Unprocessable { .. }),
        "{refused:?}"
    );
    assert!(
        format!("{refused:?}").contains("'details' is missing"),
        "{refused}"
    );
    assert_policy_refusal(as_clinician(create(&svc, PartyKind::Agent, &agent)).await);
    let (agent_vo, agent_uid) = as_admin(create(&svc, PartyKind::Agent, &agent))
        .await
        .expect("an administrator creates the AGENT");

    // Nor grants it a ROLE.
    let role = body("system-client-role", &[(AGENT, agent_vo)]);
    assert_policy_refusal(as_clinician(create(&svc, PartyKind::Role, &role)).await);
    let (role_vo, role_uid) = as_admin(create(&svc, PartyKind::Role, &role))
        .await
        .expect("an administrator grants the ROLE");

    // Nor names the provider it acts for.
    let acts_for = body(
        "agent-acts-for",
        &[(AGENT, agent_vo), (PROVIDER, provider), (ROLE, role_vo)],
    );
    assert_policy_refusal(
        as_clinician(update(&svc, PartyKind::Agent, agent_vo, &agent_uid, &acts_for)).await,
    );
    let agent_uid = as_admin(update(&svc, PartyKind::Agent, agent_vo, &agent_uid, &acts_for))
        .await
        .expect("an administrator names the provider");

    // A non-administrator neither changes nor deletes them.
    assert_policy_refusal(
        as_clinician(update(&svc, PartyKind::Agent, agent_vo, &agent_uid, &agent)).await,
    );
    assert_policy_refusal(as_clinician(delete(&svc, PartyKind::Role, &role_uid)).await);
    assert_policy_refusal(as_clinician(delete(&svc, PartyKind::Agent, &agent_uid)).await);

    // The administrator ends the grant.
    as_admin(delete(&svc, PartyKind::Role, &role_uid))
        .await
        .expect("an administrator ends the grant");
}

#[tokio::test]
async fn only_an_administrator_grants_a_policy_role() {
    let (_db, svc) = mapped().await;
    let [_, (partner, _), (person, _)] = referred(&svc).await;
    let (agent, _) = as_admin(create(&svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    for (name, ids) in [
        ("data-migration-role", [(AGENT, agent)]),
        ("federation-partner-role", [(PARTNER, partner)]),
        ("privileged-role", [(PERSON, person)]),
    ] {
        let role = body(name, &ids);
        assert_policy_refusal(as_clinician(create(&svc, PartyKind::Role, &role)).await);
        let (vo, uid) = as_admin(create(&svc, PartyKind::Role, &role))
            .await
            .unwrap_or_else(|e| panic!("an administrator grants {name}: {e:?}"));
        assert_policy_refusal(as_clinician(update(&svc, PartyKind::Role, vo, &uid, &role)).await);
        as_admin(update(&svc, PartyKind::Role, vo, &uid, &role))
            .await
            .unwrap_or_else(|e| panic!("an administrator versions {name}: {e:?}"));
    }
}

/// A SCIM client writes the parties it provisions, never a policy party.
#[tokio::test]
async fn a_scim_client_never_writes_a_policy_party() {
    let (_db, svc) = mapped().await;
    let [_, _, (person, _)] = referred(&svc).await;
    let role = body("privileged-role", &[(PERSON, person)]);
    let mut identity = caller("scim-client");
    identity.system = true;
    let refused = ferroehr::system_log::access_context::with_roles(
        Vec::new(),
        with_committer(
            Some(identity),
            protect::provisioning(
                "https://login.example/tenant".to_owned(),
                create(&svc, PartyKind::Role, &role),
            ),
        ),
    )
    .await;
    assert_policy_refusal(refused);
}

/// An acts-for relationship is a policy party wherever it is held: inline in
/// any party, or as a `PARTY_RELATIONSHIP` of its own.
#[tokio::test]
async fn an_acts_for_relationship_is_written_by_an_administrator_only() {
    let (_db, svc) = mapped().await;
    let [(provider, _), (partner, partner_uid), _] = referred(&svc).await;
    let relationship = |source: VoId, class: &str| {
        let mut relationship = body("agent-acts-for", &[(PROVIDER, provider)])["relationships"]
            [0]
        .clone();
        relationship["source"]["type"] = json!(class);
        relationship["source"]["id"]["value"] = json!(source.to_string());
        relationship
    };

    // Inline, in a party of no policy template.
    let mut organisation = plain(
        "ORGANISATION",
        "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0",
        "Regional care network",
    );
    organisation["relationships"] = json!([relationship(partner, "ORGANISATION")]);
    assert_policy_refusal(
        as_clinician(update(
            &svc,
            PartyKind::Organisation,
            partner,
            &partner_uid,
            &organisation,
        ))
        .await,
    );

    // Of its own, on FerroEHR's relationship routes.
    let (agent, _) = as_admin(create(&svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    let mut standalone = relationship(agent, "AGENT");
    standalone["archetype_details"] = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": ACTS_FOR },
        "rm_version": "1.1.0"
    });
    let refused = as_clinician(svc.party_relationship_create(version(&standalone, None))).await;
    assert_policy_refusal(refused);
    as_admin(svc.party_relationship_create(version(&standalone, None)))
        .await
        .expect("an administrator writes the relationship");
}

/// A demographic `CONTRIBUTION` creating `data` as one `complete` version,
/// committed by `committer`.
fn contribution(data: &Value) -> Value {
    let creation = json!({
        "_type": "DV_CODED_TEXT", "value": "creation",
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": "249" }
    });
    let committer = json!({ "_type": "PARTY_IDENTIFIED", "name": "administrator" });
    json!({
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "lifecycle_state": crate::fixtures::lifecycle("532"),
            "commit_audit": { "change_type": creation, "committer": committer },
            "data": data
        }],
        "audit": { "change_type": creation, "committer": committer }
    })
}

/// A `CONTRIBUTION` holding a policy party takes the same rule as the party's
/// own route: the protection judges each version it commits.
#[tokio::test]
async fn a_contribution_holding_a_policy_party_is_an_administrators() {
    let (_db, svc) = mapped().await;
    let agent = contribution(&body("agent", &[]));
    assert_policy_refusal(as_clinician(svc.demographic_contribution_create(agent.clone())).await);
    as_admin(svc.demographic_contribution_create(agent))
        .await
        .expect("an administrator commits the AGENT in a CONTRIBUTION");
}

/// Only an administrator reads a policy party, through any read of the
/// Demographic API; every other party stays readable.
#[tokio::test]
async fn only_an_administrator_reads_a_policy_party() {
    let (_db, svc) = mapped().await;
    let [(provider, provider_uid), _, _] = referred(&svc).await;
    let (agent, agent_uid) = as_admin(create(&svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    let (role, role_uid) = as_admin(create(
        &svc,
        PartyKind::Role,
        &body("system-client-role", &[(AGENT, agent)]),
    ))
    .await
    .expect("an administrator grants the ROLE");

    for (kind, vo, uid) in [
        (PartyKind::Agent, agent, &agent_uid),
        (PartyKind::Role, role, &role_uid),
    ] {
        assert_policy_refusal(as_clinician(svc.party_get(kind, at(vo.to_string()), None)).await);
        assert_policy_refusal(as_clinician(svc.party_get(kind, at(uid), None)).await);
        assert_policy_refusal(as_clinician(svc.versioned_party_get(vo)).await);
        assert_policy_refusal(as_clinician(svc.versioned_party_revision_history(vo)).await);
        assert_policy_refusal(
            as_clinician(svc.versioned_party_version_get_at_time(vo, None)).await,
        );
        assert_policy_refusal(as_clinician(svc.party_tags_get(kind, at(vo.to_string()))).await);
        as_admin(svc.party_get(kind, at(vo.to_string()), None))
            .await
            .expect("an administrator reads the policy party");
        as_admin(svc.versioned_party_revision_history(vo))
            .await
            .expect("an administrator reads the grant history");
    }

    // A party of no policy definition stays anyone's to read.
    as_clinician(svc.party_get(PartyKind::Organisation, at(&provider_uid), None))
        .await
        .expect("the provider is no policy party");
    as_clinician(svc.versioned_party_get(provider))
        .await
        .expect("the provider is no policy party");
}

/// The professional `ROLE`'s `CAPABILITY` holds a clinician's
/// qualifications (CKM `individual_credentials`), never a grant: its
/// template is no policy role, so a `ROLE` of it is no policy party, and a
/// clinician reads it.
#[tokio::test]
async fn a_professional_credentials_capability_is_never_read_as_a_grant() {
    let (_db, svc) = loaded().await;
    let professional = "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/definitions/professional")
        .join(format!("{professional}.adls"));
    let source = std::fs::read_to_string(path).expect("the professional ROLE is readable");
    svc.definition_load(SourceFormat::Adl2, &source)
        .await
        .expect("the professional ROLE loads");
    let config = config(&format!(
        "{}\n[authz.party_binding.professional_role]\ntemplate_id = \"{professional}\"\n",
        example("mapping")
    ));
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    assert_eq!(mapping.policy_role(professional), None);
    for (kind, _, role) in sections.policy.roles() {
        assert_ne!(
            mapping.policy_role(&role.template_id),
            None,
            "{kind:?} is a grant"
        );
    }
    let svc = svc
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping);
    let [_, _, (person, _)] = referred(&svc).await;
    let credentials = json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                "value": "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1" },
            "template_id": { "_type": "TEMPLATE_ID", "value": professional },
            "rm_version": "1.1.0" },
        "name": text("Healthcare professional role"),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": text("Role name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": text("Name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "id4", "name": text("Full name"),
                            "value": text("General practitioner") }] } }],
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } },
        "capabilities": [{
            "_type": "CAPABILITY", "archetype_node_id": "id6", "name": text("Professional credentials"),
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "id7", "name": text("Details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "id8", "name": text("Field of practice"),
                            "value": text("General practice") }] }
        }]
    });
    let (role, _) = as_admin(create(&svc, PartyKind::Role, &credentials))
        .await
        .expect("an administrator grants the professional ROLE");
    as_clinician(svc.party_get(PartyKind::Role, at(role.to_string()), None))
        .await
        .expect("a professional ROLE is no policy party");
}
