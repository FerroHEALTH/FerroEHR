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

/// The professional fixture set the lanes load, in its load order.
const PROFESSIONAL: [&str; 2] = [
    "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls",
    "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls",
];

/// The mapping the lanes configure over the professional fixture set.
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
async fn the_professional_fixtures_load_and_carry_the_lanes_mapping() {
    use assert_fs::prelude::FileWriteStr as _;

    let (_db, svc) = crate::fixtures::service().await;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/definitions/professional");
    for file in PROFESSIONAL {
        let source = std::fs::read_to_string(dir.join(file)).expect("the fixture is readable");
        let loaded = svc
            .definition_load(SourceFormat::Adl2, &source)
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
