// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr definitions load` over a real PostgreSQL 18: a definition is
//! stored through the upload path of its format, a second load of the same
//! text writes nothing, other text under a stored id is refused, and a source
//! the upload refuses is refused here too. No openEHR spec governs loading
//! definitions out of band: our own design.

use ferroehr::error::ServiceError;
use ferroehr::service::definition::load::{Loaded, SourceFormat};

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
/// the order the lanes load them, with the name of the compose config that
/// carries its copy.
const DEFAULT_SET: [(&str, &str); 7] = [
    (
        "professional/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls",
        "ferroehr-definition-person",
    ),
    (
        "professional/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls",
        "ferroehr-definition-role",
    ),
    (
        "care-team/openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls",
        "ferroehr-definition-group",
    ),
    (
        "care-team/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls",
        "ferroehr-definition-membership",
    ),
    (
        "representation/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls",
        "ferroehr-definition-representation",
    ),
    (
        "patient/openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls",
        "ferroehr-definition-patient",
    ),
    (
        "provider/openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls",
        "ferroehr-definition-provider",
    ),
];

/// The text of the default definition `file`.
fn default_definition(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/definitions")
        .join(file);
    std::fs::read_to_string(path).expect("the default definition is readable")
}

/// The configuration of each lane that loads the default set: the two
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

/// Every lane maps the whole default set, and the mapping it boots with is
/// judged against the loaded files as boot judges it.
#[tokio::test]
async fn the_default_definitions_load_and_carry_the_lanes_mapping() {
    use assert_fs::prelude::FileWriteStr as _;

    let (_db, svc) = crate::fixtures::service().await;
    for (file, _) in DEFAULT_SET {
        let loaded = svc
            .definition_load(SourceFormat::Adl2, &default_definition(file))
            .await
            .unwrap_or_else(|e| panic!("{file} loads: {e:?}"));
        assert!(matches!(loaded, Loaded::Stored(_)), "{loaded:?}");
    }
    for (lane, text) in lane_configs() {
        let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
        file.write_str(&text).expect("write");
        let config =
            ferroehr::config::assemble(Some(file.path()), &std::collections::HashMap::new(), &[])
                .unwrap_or_else(|e| panic!("{lane} assembles: {e:?}"));
        let binding = &config.authz.party_binding;
        assert!(
            binding.professional_role.is_some()
                && binding.group.is_some()
                && binding.membership.is_some()
                && binding.representation.is_some(),
            "{lane} maps the whole default set"
        );
        let errors = svc
            .party_mapping_errors(&binding.sections())
            .await
            .expect("the store answers");
        assert!(errors.is_empty(), "{lane}: {errors:?}");
    }
}

/// Reads a YAML file of the repository.
fn repo_yaml(relative: &str) -> serde_norway::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    let text = std::fs::read_to_string(path).expect("the YAML file is readable");
    serde_norway::from_str(&text).expect("the YAML file parses")
}

/// The copies of the default set the quickstart, the hosted sandbox and the
/// Helm chart's default `definitions.files` carry are the files of
/// `deploy/definitions/`, in their order, so a lane loads what
/// `the_default_definitions_load_and_carry_the_lanes_mapping` checks.
#[test]
fn the_lanes_carry_the_default_definitions_unchanged() {
    for path in ["docker-compose.yml", "deploy/hosted/docker-compose.yml"] {
        let compose = repo_yaml(path);
        for (file, config) in DEFAULT_SET {
            assert_eq!(
                compose["configs"][config]["content"].as_str(),
                Some(default_definition(file).as_str()),
                "the config {config} of {path} is a copy of {file}"
            );
        }
        let loaded: Vec<&str> = compose["services"]["ferroehr-definitions"]["command"]
            .as_sequence()
            .expect("the one-shot has a command")
            .iter()
            .filter_map(serde_norway::Value::as_str)
            .skip(2)
            .collect();
        let mounted: Vec<(&str, &str)> = compose["services"]["ferroehr-definitions"]["configs"]
            .as_sequence()
            .expect("the one-shot mounts configs")
            .iter()
            .filter_map(|entry| Some((entry["source"].as_str()?, entry["target"].as_str()?)))
            .collect();
        let in_order: Vec<&str> = loaded
            .iter()
            .map(|target| {
                mounted
                    .iter()
                    .find(|(_, mount)| mount == target)
                    .map_or("", |(source, _)| *source)
            })
            .collect();
        assert_eq!(
            in_order,
            DEFAULT_SET.map(|(_, config)| config),
            "{path} loads the default set in its order"
        );
    }
    let helm = repo_yaml("deploy/helm/ferroehr/values.yaml");
    let files = helm["definitions"]["files"]
        .as_sequence()
        .expect("the chart loads definitions by default");
    let carried: Vec<(&str, &str)> = files
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().expect("a name"),
                entry["content"].as_str().expect("a content"),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = DEFAULT_SET
        .iter()
        .map(|(file, _)| {
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
        "the chart's default definitions.files is the default set in its order"
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
    for (file, _) in DEFAULT_SET {
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
