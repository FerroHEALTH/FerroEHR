// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties (#5434) over a real PostgreSQL 18: the default policy
//! definitions are seeded when absent and never overwritten, no
//! configuration key names them, each worked example on the book's
//! "Operational policy" page fits its definition and carries the codes that
//! make it a policy party, a deployment's own version or specialisation of
//! an example archetype carrying them is one too, and only an administrator
//! writes or reads a policy party (an `AGENT` holding a client identifier, a
//! `ROLE` holding an operation code, an acts-for relationship inline or of
//! its own). No openEHR spec governs who writes a party: our own design.

use std::collections::HashMap;

use assert_fs::prelude::FileWriteStr as _;
use serde_json::{Value, json};

use crate::fixtures::{at, delete_precondition, if_match, object, written_vo};
use ferroehr::config::FerroEhrConfig;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy::{PolicyKind, PolicyRole};
use ferroehr::party_binding::protect;
use ferroehr::service::FerroEhrService;
use ferroehr::service::definition::load::{Loaded, SourceFormat};
use ferroehr::service::demographic::default_definitions::Seeded;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The acts-for archetype, in the interface form a relationship carries.
const ACTS_FOR: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1";
/// The example ids of the book page, each replaced by the id of the party
/// the test commits: the provider, the partner, the `AGENT`, the
/// professional's `PERSON` and the system client's `ROLE`.
pub(crate) const PROVIDER: &str = "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40";
const PARTNER: &str = "7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36";
pub(crate) const AGENT: &str = "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73";
pub(crate) const PERSON: &str = "3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95";
const ROLE: &str = "e41c7b2a-6d3f-4a9e-8b05-2f7d1c9e3a68";
/// The administrator role of the tests.
const ADMIN: &str = "ADMIN";

/// The text of the book's "Operational policy" page.
fn book_page() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/book/src/operational-policy.md");
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
    let body = after.split_once('\n').expect("a fence opens the example").1;
    body.split_once("\n```")
        .expect("a fence closes the example")
        .0
        .to_owned()
}

/// The JSON example `name` with each example id replaced by its party.
pub(crate) fn body(name: &str, ids: &[(&str, VoId)]) -> Value {
    let mut text = example(name);
    for (example_id, id) in ids {
        text = text.replace(example_id, &id.to_string());
    }
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("the example {name} parses: {e}"))
}

/// The configuration of `toml`.
fn config(toml: &str) -> FerroEhrConfig {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(toml).expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[]).expect("assembles")
}

/// A service over a fresh database holding the policy definitions, seeded
/// as a boot seeds them.
async fn loaded() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    svc.seed_default_definitions()
        .await
        .expect("the policy definitions are seeded");
    (db, svc)
}

/// A service holding the policy definitions with the mapping of `toml`
/// installed, as the server installs it, under an `[authz.grants] allowed`
/// that admits every operation, so each worked example grants what it codes.
async fn mapped_by(toml: &str) -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = loaded().await;
    let config = config(toml);
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    let svc = svc
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(GrantCeiling::new(Operation::ALL));
    (db, svc)
}

/// A service holding the policy definitions under an empty configuration:
/// the policy parties need no key.
pub(crate) async fn mapped() -> (testkit::TestDb, FerroEhrService) {
    mapped_by("").await
}

#[tokio::test]
async fn the_default_definitions_are_seeded_once_and_never_overwritten() {
    let (_db, svc) = crate::fixtures::service().await;
    let first = svc
        .seed_default_definitions()
        .await
        .expect("a first boot seeds them");
    assert_eq!(
        first.len(),
        ferroehr::party_binding::policy::DEFINITIONS.len()
            + ferroehr::party_binding::shipped::DEFINITIONS.len()
    );
    assert!(
        first
            .iter()
            .all(|seeded| matches!(seeded, Seeded::Stored(_))),
        "{first:?}"
    );
    let again = svc
        .seed_default_definitions()
        .await
        .expect("a second boot keeps them");
    assert!(
        again.iter().all(|seeded| matches!(seeded, Seeded::Kept(_))),
        "{again:?}"
    );
}

/// A deployment that stored its own text under a default policy id before
/// the first boot keeps it: the seed neither overwrites nor refuses it.
#[tokio::test]
async fn a_deployments_text_under_a_policy_id_is_kept() {
    let (_db, svc) = crate::fixtures::service().await;
    let (_, agent) = ferroehr::party_binding::policy::DEFINITIONS[0];
    let changed = agent.replace("System agent", "Integration engine");
    assert_ne!(changed, agent, "the definition names its root");
    svc.definition_load(SourceFormat::Adl2, &changed)
        .await
        .expect("the deployment's AGENT");
    let seeded = svc
        .seed_default_definitions()
        .await
        .expect("the boot seeds the rest");
    assert!(matches!(&seeded[0], Seeded::Kept(_)), "{seeded:?}");
    assert_eq!(
        svc.definition_load(SourceFormat::Adl2, &changed)
            .await
            .expect("the deployment's text is stored"),
        Loaded::Unchanged("openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0".to_owned())
    );
}

/// The policy parties have no configuration key: each section a policy
/// mapping would take is refused as an unknown key.
#[test]
fn no_configuration_key_names_a_policy_party() {
    for section in [
        "agent",
        "acts_for",
        "system_client_role",
        "data_migration_role",
        "federation_partner_role",
        "privileged_role",
    ] {
        let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
        file.write_str(&format!(
            "[authz.party_binding.{section}]\ntemplate_id = \"x\"\n"
        ))
        .expect("write");
        ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[])
            .expect_err("no policy key exists");
    }
    assert!(
        !ferroehr::config::DEFAULT_TEMPLATE.contains("acts_for"),
        "the reference names no policy key"
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
        with_committer(Some(caller(subject)), Box::pin(fut)),
    )
    .await
}

/// Runs `fut` as a clinician: a caller who is no administrator.
pub(crate) async fn as_clinician<F: Future>(fut: F) -> F::Output {
    as_caller("clinician", &["CLINICIAN"], fut).await
}

/// Runs `fut` as an administrator.
pub(crate) async fn as_admin<F: Future>(fut: F) -> F::Output {
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
pub(crate) async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
) -> Result<(VoId, String), ServiceError> {
    let created = Box::pin(svc.party_create(kind, version(body, None))).await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((written_vo(&created), uid))
}

/// Commits `body` as the next version of `vo` after `uid`.
pub(crate) async fn update(
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
pub(crate) async fn delete(
    svc: &FerroEhrService,
    kind: PartyKind,
    uid: &str,
) -> Result<(), ServiceError> {
    Box::pin(svc.party_delete(
        kind,
        object(uid),
        delete_precondition(uid, None),
        WriteRequest::new(()),
    ))
    .await
    .map(|_| ())
}

/// Asserts `refused` is the `403` of a policy party.
pub(crate) fn assert_policy_refusal<T: std::fmt::Debug>(refused: Result<T, ServiceError>) {
    let refused = refused.expect_err("a policy party is written by an administrator only");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let text = refused.to_string();
    assert!(text.contains(" is a policy party"), "{text}");
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A plain `PARTY` of `class` named `name`, of no mapped template.
pub(crate) fn plain(class: &str, archetype: &str, name: &str) -> Value {
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
pub(crate) async fn referred(svc: &FerroEhrService) -> [(VoId, String); 3] {
    let organisation = "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0";
    let provider = plain("ORGANISATION", organisation, "Example hospital");
    let partner = plain("ORGANISATION", organisation, "Regional care network");
    let person = plain(
        "PERSON",
        "openEHR-DEMOGRAPHIC-PERSON.person.v0",
        "J. Jansen",
    );
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
    let ids = [
        (AGENT, any),
        (PROVIDER, any),
        (PARTNER, any),
        (PERSON, any),
        (ROLE, any),
    ];
    for (name, template) in [
        ("agent", "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0"),
        ("agent-acts-for", "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0"),
        (
            "system-client-role",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0",
        ),
        (
            "data-migration-role",
            "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0",
        ),
        (
            "federation-partner-role",
            "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0",
        ),
        (
            "privileged-role",
            "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0",
        ),
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
        .party_template_fit(
            relationship,
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1.0.0",
        )
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
        as_clinician(update(
            &svc,
            PartyKind::Agent,
            agent_vo,
            &agent_uid,
            &acts_for,
        ))
        .await,
    );
    let agent_uid = as_admin(update(
        &svc,
        PartyKind::Agent,
        agent_vo,
        &agent_uid,
        &acts_for,
    ))
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
        let mut relationship =
            body("agent-acts-for", &[(PROVIDER, provider)])["relationships"][0].clone();
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
    let refused = as_clinician(Box::pin(
        svc.party_relationship_create(version(&standalone, None)),
    ))
    .await;
    assert_policy_refusal(refused);
    as_admin(Box::pin(
        svc.party_relationship_create(version(&standalone, None)),
    ))
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
    assert_policy_refusal(
        as_clinician(Box::pin(svc.demographic_contribution_create(agent.clone()))).await,
    );
    as_admin(Box::pin(svc.demographic_contribution_create(agent)))
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
/// qualifications (CKM `individual_credentials`), never a grant: it holds no
/// code of FerroEHR's operation terminology, so a `ROLE` of it is no policy
/// party, and a clinician reads it.
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
        "[authz.party_binding.professional_role]\ntemplate_id = \"{professional}\"\n"
    ));
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    assert!(mapping.names_policy());
    assert_eq!(
        PolicyKind::of("openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1"),
        None
    );
    assert_eq!(
        PolicyKind::of("openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0"),
        None
    );
    for kind in PolicyRole::ALL {
        assert_eq!(
            PolicyKind::of(kind.archetype_id()),
            Some(PolicyKind::Role(kind))
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

/// The system client `ROLE` example under another template: `template_id`
/// and the root archetype id `archetype`.
pub(crate) fn system_client_under(template_id: &str, archetype: &str, agent: VoId) -> Value {
    let mut role = body("system-client-role", &[(AGENT, agent)]);
    role["archetype_node_id"] = json!(archetype);
    role["archetype_details"]["archetype_id"]["value"] = json!(archetype);
    role["archetype_details"]["template_id"]["value"] = json!(template_id);
    role
}

/// A deployment versions the system client `ROLE` and builds a specialised
/// one under another id, through the definition upload the REST route
/// calls: both carry the operation code of the example, so both are policy
/// parties, and a boot keeps them.
#[tokio::test]
async fn a_deployments_version_and_specialisation_are_policy_parties() {
    let (_db, svc) = mapped().await;
    let (_, shipped) = ferroehr::party_binding::policy::DEFINITIONS[2];
    let version = shipped.replace(
        "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0",
        "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.1.0",
    );
    assert_ne!(version, shipped, "the definition names its id");
    let version_id = svc
        .template_adl2_upload(&version)
        .await
        .expect("the deployment uploads a new version");
    let specialised = crate::adl2_fixture::adl2_source(
        "archetype",
        "openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1.0.0",
        Some("openEHR-DEMOGRAPHIC-ROLE.system_client.v1"),
    );
    let specialised_id = svc
        .template_adl2_upload(&specialised)
        .await
        .expect("the deployment uploads a specialisation");

    let (agent, _) = as_admin(create(&svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    for (template_id, archetype) in [
        (
            version_id.as_str(),
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
        ),
        (
            specialised_id.as_str(),
            "openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1",
        ),
    ] {
        let role = system_client_under(template_id, archetype, agent);
        assert_policy_refusal(as_clinician(create(&svc, PartyKind::Role, &role)).await);
        let (vo, _) = as_admin(create(&svc, PartyKind::Role, &role))
            .await
            .unwrap_or_else(|e| panic!("an administrator grants under {template_id}: {e:?}"));
        assert_policy_refusal(
            as_clinician(svc.party_get(PartyKind::Role, at(vo.to_string()), None)).await,
        );
    }

    let seeded = svc
        .seed_default_definitions()
        .await
        .expect("a boot after the uploads");
    assert!(
        seeded.iter().all(|s| matches!(s, Seeded::Kept(_))),
        "{seeded:?}"
    );
    for text in [&version, &specialised] {
        assert!(matches!(
            svc.definition_load(SourceFormat::Adl2, text).await,
            Ok(Loaded::Unchanged(_))
        ));
    }
}
