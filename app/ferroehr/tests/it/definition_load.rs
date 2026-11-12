// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr definitions load` over a real PostgreSQL 18: a definition is
//! stored through the upload path of its format, a second load of the same
//! text writes nothing, other text under a stored id is refused, and a source
//! the upload refuses is refused here too. No openEHR spec governs loading
//! definitions out of band: our own design.

use ferroehr::error::ServiceError;
use ferroehr::service::definition::load::{Loaded, SourceFormat};
use ferroehr::service::demographic::default_definitions::Seeded;

use crate::adl2_fixture::adl2_source;

/// The OPT 1.4 fixture and its `template_id`.
const OPT: &str = "templates/blood_pressure.opt";
const OPT_ID: &str = "cnf.blood_pressure";
/// A synthetic ADL 2 archetype id.
const HRID: &str = "openEHR-DEMOGRAPHIC-PERSON.load_test.v1.0.0";

/// Reads a vendored corpus file.
fn corpus(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(relative);
    std::fs::read_to_string(path).expect("the vendored fixture is readable")
}

#[tokio::test]
async fn an_opt_is_stored_once_and_then_left_alone() {
    let (_db, svc) = crate::fixtures::service().await;
    let opt = corpus(OPT);
    let first = svc
        .definition_load(SourceFormat::Opt14, &opt)
        .await
        .expect("the OPT loads");
    assert_eq!(first, Loaded::Stored(OPT_ID.to_owned()));
    let again = svc
        .definition_load(SourceFormat::Opt14, &opt)
        .await
        .expect("the same OPT loads again");
    assert_eq!(again, Loaded::Unchanged(OPT_ID.to_owned()));
    let stored = svc
        .template_adl14_get(OPT_ID)
        .await
        .expect("the OPT is served");
    assert_eq!(stored, opt);
}

#[tokio::test]
async fn an_adl2_source_is_stored_once_and_then_left_alone() {
    let (_db, svc) = crate::fixtures::service().await;
    let source = adl2_source("archetype", HRID, None);
    let first = svc
        .definition_load(SourceFormat::Adl2, &source)
        .await
        .expect("the archetype loads");
    assert_eq!(first, Loaded::Stored(HRID.to_owned()));
    let again = svc
        .definition_load(SourceFormat::Adl2, &source)
        .await
        .expect("the same archetype loads again");
    assert_eq!(again, Loaded::Unchanged(HRID.to_owned()));
}

#[tokio::test]
async fn other_content_under_a_stored_id_is_refused() {
    let (_db, svc) = crate::fixtures::service().await;
    let opt = corpus(OPT);
    svc.definition_load(SourceFormat::Opt14, &opt)
        .await
        .expect("the OPT loads");
    let changed = opt.replacen("<concept>", "<concept>Changed ", 1);
    assert_ne!(changed, opt, "the fixture carries a concept to change");
    let refused = svc
        .definition_load(SourceFormat::Opt14, &changed)
        .await
        .expect_err("other content under the stored id is refused");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
    assert_eq!(
        svc.template_adl14_get(OPT_ID).await.expect("still served"),
        opt
    );

    let source = adl2_source("archetype", HRID, None);
    svc.definition_load(SourceFormat::Adl2, &source)
        .await
        .expect("the archetype loads");
    let changed = source.replace("FerroEHR tests", "Someone else");
    let refused = svc
        .definition_load(SourceFormat::Adl2, &changed)
        .await
        .expect_err("other ADL 2 text under the stored HRID is refused");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
}

#[tokio::test]
async fn a_source_the_upload_refuses_is_refused_and_not_stored() {
    let (_db, svc) = crate::fixtures::service().await;
    let opt = corpus(OPT).replacen("<concept>", "<concept>  </concept><x>", 1);
    let refused = svc
        .definition_load(SourceFormat::Opt14, &opt)
        .await
        .expect_err("a malformed OPT is refused");
    assert!(!matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
    let refused = svc
        .definition_load(SourceFormat::Adl2, "archetype (adl_version=2.0.6")
        .await
        .expect_err("an unparseable ADL 2 source is refused");
    assert!(
        matches!(refused, ServiceError::BadRequest(_)),
        "{refused:?}"
    );
    svc.template_adl14_get(OPT_ID)
        .await
        .expect_err("nothing was stored");
}

/// The default demographic definitions, each under `deploy/definitions/` in
/// the order the binary seeds them.
const DEFAULT_SET: [&str; 7] = [
    "professional/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls",
    "professional/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls",
    "care-team/openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls",
    "care-team/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls",
    "representation/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls",
    "patient/openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls",
    "provider/openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls",
];

/// The text of the default definition `file`.
fn default_definition(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/definitions")
        .join(file);
    std::fs::read_to_string(path).expect("the default definition is readable")
}

/// The configuration of each lane that boots on the default set: the two
/// quickstart configurations, the hosted sandbox and the dev lanes.
fn lane_configs() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |relative: &str| {
        std::fs::read_to_string(root.join(relative)).expect("the lane configuration is readable")
    };
    let compose = repo_yaml("docker-compose.yml");
    let mut lanes: Vec<(String, String)> = ["ferroehr-config", "ferroehr-config-local"]
        .iter()
        .map(|config| {
            let text = compose["configs"][*config]["content"]
                .as_str()
                .expect("the quickstart carries its configuration inline");
            // NOTE: Compose interpolation, `$$` is a literal `$` in a config's content.
            (
                format!("docker-compose.yml {config}"),
                text.replace("$$", "$"),
            )
        })
        .collect();
    for path in [
        "deploy/hosted/ferroehr.sandbox.toml",
        "docker/ferroehr.dev.toml",
    ] {
        lanes.push((path.to_owned(), read(path)));
    }
    lanes
}

/// Every lane boots on the default mapping, which maps the whole default
/// set, and that mapping is judged against the set as boot seeds it.
#[tokio::test]
async fn the_lanes_boot_on_the_seeded_default_definitions() {
    use assert_fs::prelude::FileWriteStr as _;

    let (_db, svc) = crate::fixtures::service().await;
    svc.seed_default_definitions()
        .await
        .expect("a boot seeds the default definitions");
    let default = ferroehr::config::binding::PartyBindingConfig::default();
    for (lane, text) in lane_configs() {
        let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
        file.write_str(&text).expect("write");
        let config =
            ferroehr::config::assemble(Some(file.path()), &std::collections::HashMap::new(), &[])
                .unwrap_or_else(|e| panic!("{lane} assembles: {e:?}"));
        let binding = &config.authz.party_binding;
        assert_eq!(
            (
                &binding.person,
                &binding.professional_role,
                &binding.group,
                &binding.membership,
                &binding.representation,
                &binding.patient,
            ),
            (
                &default.person,
                &default.professional_role,
                &default.group,
                &default.membership,
                &default.representation,
                &default.patient,
            ),
            "{lane} sets no mapping key, so it maps the whole default set"
        );
        let errors = svc
            .party_mapping_errors(&binding.sections())
            .await
            .expect("the store answers");
        assert!(errors.is_empty(), "{lane}: {errors:?}");
    }
}

/// A deployment with templates of its own sets the sections that name them:
/// the boot seeds the default set beside them and stores nothing under its
/// ids, so the mapping is refused until the deployment stores them, and a
/// text of its own stored under a default id before the first boot is kept.
#[tokio::test]
async fn a_deployments_own_templates_are_never_seeded_or_overwritten() {
    use assert_fs::prelude::FileWriteStr as _;

    let own_role = "openEHR-DEMOGRAPHIC-ROLE.own_professional.v1.0.0";
    let (_db, svc) = crate::fixtures::service().await;
    let (_, shipped_person) = ferroehr::party_binding::shipped::DEFINITIONS[0];
    let own_person = shipped_person.replace("A healthcare professional's PERSON", "A clinician");
    assert_ne!(
        own_person, shipped_person,
        "the definition names its purpose"
    );
    svc.definition_load(SourceFormat::Adl2, &own_person)
        .await
        .expect("the deployment's text under the default id");

    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(&format!(
        "[authz.party_binding.professional_role]\ntemplate_id = \"{own_role}\"\n"
    ))
    .expect("write");
    let config =
        ferroehr::config::assemble(Some(file.path()), &std::collections::HashMap::new(), &[])
            .expect("assembles");
    let sections = config.authz.party_binding.sections();

    svc.seed_default_definitions()
        .await
        .expect("a boot seeds the default definitions");
    let errors = svc
        .party_mapping_errors(&sections)
        .await
        .expect("the store answers");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains(&format!(
            "authz.party_binding.professional_role names \"{own_role}\", which the definition \
             store does not hold"
        )),
        "nothing is seeded under the deployment's id: {errors:?}"
    );

    svc.definition_load(
        SourceFormat::Adl2,
        &adl2_source("archetype", own_role, None),
    )
    .await
    .expect("the deployment stores its own template");
    let again = svc
        .seed_default_definitions()
        .await
        .expect("the next boot seeds nothing new");
    assert!(
        again.iter().all(|seeded| matches!(seeded, Seeded::Kept(_))),
        "{again:?}"
    );
    assert_eq!(
        svc.party_mapping_errors(&sections)
            .await
            .expect("the store answers"),
        Vec::<String>::new()
    );
    assert_eq!(
        svc.definition_load(SourceFormat::Adl2, &own_person)
            .await
            .expect("the deployment's text is stored"),
        Loaded::Unchanged("openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0".to_owned())
    );
}

/// Reads a YAML file of the repository.
fn repo_yaml(relative: &str) -> serde_norway::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    let text = std::fs::read_to_string(path).expect("the YAML file is readable");
    serde_norway::from_str(&text).expect("the YAML file parses")
}

/// The binary carries every file of `deploy/definitions/`, in the order it
/// seeds them, so no lane keeps a copy or a load step of its own: the
/// quickstart and the hosted sandbox run no definitions one-shot, and the
/// Helm chart's `definitions.files` is empty by default.
#[test]
fn the_binary_carries_the_default_definitions_and_no_lane_copies_them() {
    let carried: Vec<(&str, &str)> = ferroehr::party_binding::shipped::DEFINITIONS.to_vec();
    let expected: Vec<(String, String)> = DEFAULT_SET
        .iter()
        .map(|file| {
            let name = file.rsplit('/').next().expect("a file name");
            (name.to_owned(), default_definition(file))
        })
        .collect();
    assert_eq!(
        carried,
        expected
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect::<Vec<_>>(),
        "the binary carries the default set in its order"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/definitions");
    let mut on_disk: Vec<String> = std::fs::read_dir(&root)
        .expect("deploy/definitions is readable")
        .flat_map(|dir| {
            let dir = dir.expect("an entry").path();
            std::fs::read_dir(&dir)
                .map(|entries| {
                    entries
                        .map(|entry| entry.expect("an entry").path())
                        .filter(|path| path.extension().is_some_and(|ext| ext == "adls"))
                        .map(|path| {
                            path.strip_prefix(&root)
                                .expect("under the root")
                                .display()
                                .to_string()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = DEFAULT_SET.iter().map(|file| (*file).to_owned()).collect();
    listed.sort();
    assert_eq!(
        on_disk, listed,
        "every file of deploy/definitions is carried"
    );

    for path in ["docker-compose.yml", "deploy/hosted/docker-compose.yml"] {
        let compose = repo_yaml(path);
        assert!(
            compose["services"]["ferroehr-definitions"].is_null(),
            "{path} runs no definitions one-shot"
        );
        let copies: Vec<&str> = compose["configs"]
            .as_mapping()
            .expect("the compose file has configs")
            .keys()
            .filter_map(serde_norway::Value::as_str)
            .filter(|name| name.starts_with("ferroehr-definition"))
            .collect();
        assert!(copies.is_empty(), "{path} carries copies: {copies:?}");
    }
    let helm = repo_yaml("deploy/helm/ferroehr/values.yaml");
    assert_eq!(
        helm["definitions"]["files"].as_sequence().map(Vec::len),
        Some(0),
        "the chart's default definitions.files is empty"
    );
}

/// A `DV_TEXT` of `value`.
fn dv_text(value: &str) -> serde_json::Value {
    serde_json::json!({ "_type": "DV_TEXT", "value": value })
}

/// An `ELEMENT` `id` named `name` holding `value`.
fn element(id: &str, name: &str, value: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "_type": "ELEMENT", "archetype_node_id": id, "name": dv_text(name), "value": value })
}

/// A `PARTY_REF` to the party `id` of `class`.
fn party_ref(class: &str, id: &str) -> serde_json::Value {
    serde_json::json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": class,
        "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// A `DV_CODED_TEXT` of the local code `code` reading `value`.
fn local(code: &str, value: &str) -> serde_json::Value {
    serde_json::json!({ "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
            "code_string": code } })
}

/// The default patient `PERSON` holds a patient identifier, a date of birth
/// and a sex in `details` (RM demographic §Party Identification), its legal
/// name in `identities` named by its usage, a postal and an electronic
/// address, and its relationships to a payer and a provider with the patient
/// identifiers each issues; the provider `ORGANISATION` holds its identifier
/// and its business name; the professional `ROLE` holds the performer's
/// registration as a `CAPABILITY`, as the CKM credentials archetype records
/// it.
#[tokio::test]
async fn the_patient_the_provider_and_the_professional_credentials_fit_their_definitions() {
    use ferroehr::service::demographic::template::TemplateFit;
    use serde_json::json;

    let (_db, svc) = crate::fixtures::service().await;
    for file in DEFAULT_SET {
        svc.definition_load(SourceFormat::Adl2, &default_definition(file))
            .await
            .unwrap_or_else(|e| panic!("{file} loads: {e:?}"));
    }
    let patient_id = "openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0";
    let me = "3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95";
    let identifier = |issuer: &str, id: &str| json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": "NNNLD" });
    let relationship = |at: &str, tree: &str, item: &str, class: &str, target: &str| {
        json!({
            "_type": "PARTY_RELATIONSHIP", "archetype_node_id": at, "name": dv_text("relationship"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": tree, "name": dv_text("details"),
                "items": [element(item, "identifier", &identifier("urn:oid:2.16.840.1.113883.2.4.6.3", "123456789"))] },
            "source": party_ref("PERSON", me),
            "target": party_ref(class, target)
        })
    };
    let patient = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.patient.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.patient.v1" },
            "template_id": { "_type": "TEMPLATE_ID", "value": patient_id },
            "rm_version": "1.1.0" },
        "name": dv_text("Patient"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": dv_text("Details"),
            "items": [
                element("id3", "Patient identifier", &identifier("urn:oid:2.16.840.1.113883.2.4.6.3", "123456789")),
                element("id4", "Date of birth", &json!({ "_type": "DV_DATE", "value": "2015-03-14" })),
                element("id5", "Sex", &local("at11", "Female"))
            ] },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id6", "name": local("at24", "Legal name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id7", "name": dv_text("Name structure"),
                "items": [element("id8", "Full name", &dv_text("Sam de Vries"))] } }],
        "contacts": [{ "_type": "CONTACT", "archetype_node_id": "id11", "name": dv_text("Contact"),
            "addresses": [
                { "_type": "ADDRESS", "archetype_node_id": "id12", "name": local("at34", "Residential"),
                  "details": { "_type": "ITEM_TREE", "archetype_node_id": "id13", "name": dv_text("Address details"),
                    "items": [
                        element("id14", "Address line", &dv_text("Dorpsstraat 1")),
                        element("id15", "Postal code", &dv_text("1234 AB")),
                        element("id16", "Suburb/town/locality", &dv_text("Utrecht"))
                    ] } },
                { "_type": "ADDRESS", "archetype_node_id": "id19", "name": local("at44", "E-mail"),
                  "details": { "_type": "ITEM_TREE", "archetype_node_id": "id20", "name": dv_text("Components"),
                    "items": [
                        element("id21", "Usage", &local("at50", "Personal")),
                        element("id22", "Details", &dv_text("sam@example.org"))
                    ] } }
            ] }],
        "relationships": [
            relationship("id26", "id27", "id28", "ORGANISATION", "7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36"),
            relationship("id29", "id30", "id31", "ORGANISATION", "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40")
        ]
    });
    let fit = svc
        .party_template_fit(&patient, patient_id)
        .await
        .expect("the patient fits");
    assert_eq!(fit, TemplateFit::Complete);

    let provider_id = "openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0";
    let provider = json!({
        "_type": "ORGANISATION",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1" },
            "template_id": { "_type": "TEMPLATE_ID", "value": provider_id },
            "rm_version": "1.1.0" },
        "name": dv_text("Healthcare provider"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": dv_text("Identifiers"),
            "items": [element("id3", "Provider identifier",
                &json!({ "_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.16.528.1.1007.3.3", "id": "01234567", "type": "URA" }))] },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": local("at12", "Business name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": dv_text("Items"),
                "items": [element("id6", "Name", &dv_text("Example hospital"))] } }]
    });
    let fit = svc
        .party_template_fit(&provider, provider_id)
        .await
        .expect("the provider fits");
    assert_eq!(fit, TemplateFit::Complete);
    let mut anonymous = provider.clone();
    anonymous
        .as_object_mut()
        .expect("an object")
        .remove("details");
    let fit = svc
        .party_template_fit(&anonymous, provider_id)
        .await
        .expect("the provider is of its template");
    assert!(
        matches!(&fit, TemplateFit::Incomplete(missing) if missing.iter().any(|v| v.path == "/details")),
        "a provider carries its identifier: {fit:?}"
    );

    let role_id = "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0";
    let role = json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1" },
            "template_id": { "_type": "TEMPLATE_ID", "value": role_id },
            "rm_version": "1.1.0" },
        "name": dv_text("Healthcare professional role"),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": dv_text("Role name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": dv_text("Name"),
                "items": [element("id4", "Full name", &dv_text("General practitioner"))] } }],
        "performer": party_ref("PERSON", me),
        "capabilities": [{
            "_type": "CAPABILITY", "archetype_node_id": "id6", "name": dv_text("Professional credentials"),
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "id7", "name": dv_text("Details"),
                "items": [
                    element("id8", "Field of practice", &dv_text("General practice")),
                    { "_type": "CLUSTER", "archetype_node_id": "id10", "name": dv_text("Registration"),
                      "items": [
                        element("id11", "Registration ID", &dv_text("19012345601")),
                        element("id12", "Registering body", &dv_text("BIG-register")),
                        element("id13", "Registration status", &json!({ "_type": "DV_CODED_TEXT",
                            "value": "Active/Full registration",
                            "defining_code": { "_type": "CODE_PHRASE",
                                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                                "code_string": "at10" } })),
                        element("id14", "Period", &json!({ "_type": "DV_INTERVAL",
                            "lower": { "_type": "DV_DATE", "value": "2020-01-01" },
                            "lower_unbounded": false, "upper_unbounded": true,
                            "lower_included": true, "upper_included": false }))
                      ] }
                ] }
        }]
    });
    let fit = svc
        .party_template_fit(&role, role_id)
        .await
        .expect("the professional's registration fits");
    assert_eq!(fit, TemplateFit::Complete);
}
