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
const DEFAULT_SET: [(&str, &str); 5] = [
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
];

/// The text of the default definition `file`.
fn default_definition(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/definitions")
        .join(file);
    std::fs::read_to_string(path).expect("the default definition is readable")
}

/// The mapping the lanes configure over the professional pair of the set.
const PROFESSIONAL_MAPPING: &str = r#"
[[auth.issuer]]
issuer = "https://basic-users.ferroehr.invalid"
kind = "persons"
audience = ["ferroehr-basic-users"]

[[auth.party]]
issuer = "https://basic-users.ferroehr.invalid"
identifier_issuer = "urn:ferroehr:basic-user"
identifier_type = "login"

[authz.party_binding]
provider = { issuer = "urn:oid:2.999.7.9", id = "dev" }

[[authz.party_binding.person.template]]
template_id = "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0"
default = true
key = { path = "/details[id2]/items[id3]", type = "login" }
user_name = { path = "/details[id2]/items[id3]", type = "scim_user_name" }
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }

[[authz.party_binding.person.attribute]]
scim = "name.formatted"
name = true
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id6]" }

[[authz.party_binding.person.attribute]]
scim = "name.givenName"
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id7]" }

[[authz.party_binding.person.attribute]]
scim = "name.familyName"
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id8]" }

[[authz.party_binding.person.attribute]]
scim = 'emails[type eq "work"].value'
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/contacts[id9]/addresses[id10]/details[id11]/items[id12]" }

[authz.party_binding.professional_role]
template_id = "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0"
"#;

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
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(PROFESSIONAL_MAPPING).expect("write");
    let config =
        ferroehr::config::assemble(Some(file.path()), &std::collections::HashMap::new(), &[])
            .expect("the mapping assembles");
    let errors = svc
        .party_mapping_errors(&config.authz.party_binding.sections())
        .await
        .expect("the store answers");
    assert!(errors.is_empty(), "{errors:?}");
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
