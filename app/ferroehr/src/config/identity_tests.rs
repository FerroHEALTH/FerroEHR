// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identifiers this deployment writes and sends: `[api] system_id` and
//! what derives from it, none of which names the product (#6143).

use super::FerroEhrConfig;
use super::deployment::DeploymentProfile;
use super::tests::{assemble_ok, env, standalone, toml_file};

/// `[api] system_id` — the CDR's own openEHR system identifier (stamped
/// into `EHR.system_id`, the `AUDIT_DETAILS.system_id` server default, and
/// every `OBJECT_VERSION_ID.creating_system_id`). Pins every layer: the
/// sandbox fallback, the host of `[server] public_base_url`, the file
/// value, and the `FERROEHR__` env form.
#[test]
fn rest_system_id_default_file_and_env() {
    // Unset, with no public base URL, a sandbox stamps the local host.
    let c = assemble_ok(None, &env(&[]), &[]);
    assert_eq!(c.api.system_id, None);
    assert_eq!(c.api.system_id(), crate::service::DEFAULT_SYSTEM_ID);
    assert_eq!(crate::service::DEFAULT_SYSTEM_ID, "localhost");

    let public = toml_file("[server]\npublic_base_url = \"https://cdr.public.example/ehr\"\n");
    let c = assemble_ok(Some(public.path()), &env(&[]), &[]);
    assert_eq!(c.api.system_id(), "cdr.public.example");

    let file = toml_file(
        "[server]\npublic_base_url = \"https://cdr.public.example\"\n\
         [api]\nsystem_id = \"cdr.hospital.example\"\n",
    );
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    assert_eq!(c.api.system_id(), "cdr.hospital.example");

    let c = assemble_ok(
        Some(file.path()),
        &env(&[("FERROEHR__API__SYSTEM_ID", "cdr.env.example")]),
        &[],
    );
    assert_eq!(c.api.system_id(), "cdr.env.example");

    // `system_id` and the `[api.options]` display identity are
    // independent knobs — setting one must not disturb the other.
    assert_eq!(c.api.options.solution, "FerroEHR");
}

/// `api.system_id` is judged by the openEHR `uid` grammar itself.
///
/// It occupies the `creating_system_id` position of every
/// `OBJECT_VERSION_ID` this CDR mints — "`creating_system_id` = `uid`" with
/// "`uid` = `iso_oid` | `uuid` | `internet_id`" (BASE
/// `base_types/master05-identification_package.adoc` §Syntaxes) — and is
/// stamped into `AUDIT_DETAILS.system_id`, whose RM invariant
/// `System_id_valid` forbids an empty value. Both the accepted and the
/// refused shapes are pinned, including the ones a non-empty/no-`::` check
/// would have let through into unmintable version ids.
#[test]
fn validate_system_id_is_a_legal_uid() {
    let mut c = standalone();
    assert!(c.validate().is_ok());

    for refused in [
        "",                 // AUDIT_DETAILS.System_id_valid
        "   ",              // blank
        "cdr::hospital",    // the OBJECT_VERSION_ID field separator
        "cdr hospital",     // a space is in no `uid` production
        "cdr/hospital",     // nor is a path separator
        "-leading-hyphen",  // an internet_id label starts with a letter
        "trailing-hyphen-", // and ends with a letter or digit
        "1.2.840.",         // an iso_oid group may not be empty
    ] {
        c.api.system_id = Some(refused.to_owned());
        assert!(
            c.validate().is_err(),
            "system_id {refused:?} is not a legal openEHR uid and must be refused at boot"
        );
    }

    for accepted in [
        "cdr.hospital.example",                 // internet_id
        "localhost",                            // internet_id (the default)
        "1.2.840.113554",                       // iso_oid
        "8849182c-82ad-4088-a07f-48ead4180515", // uuid
    ] {
        c.api.system_id = Some(accepted.to_owned());
        assert!(
            c.validate().is_ok(),
            "system_id {accepted:?} is a legal openEHR uid and must boot"
        );
    }
}

/// A `production` deployment names its system: with neither `[api]
/// system_id` nor `[server] public_base_url` it refuses to start, and the
/// refusal names both keys; a sandbox runs on `localhost`.
#[test]
fn production_names_its_system() {
    let mut c = standalone();
    c.deployment.profile = DeploymentProfile::Production;
    let refused = c.validate().expect_err("no system identifier").to_string();
    assert!(refused.contains("api.system_id"), "{refused}");
    assert!(refused.contains("server.public_base_url"), "{refused}");
    c.api.system_id = Some("cdr.hospital.example".to_owned());
    let refused = c
        .validate()
        .map_err(|e| e.to_string())
        .err()
        .unwrap_or_default();
    assert!(!refused.contains("api.system_id"), "{refused}");
    assert!(standalone().validate().is_ok());
}

/// The audit source, the role code system and the Basic issuer name this
/// system by its `system_id` unless the deployment names them.
#[test]
fn the_derived_identifiers_follow_the_system_id() {
    let mut c = FerroEhrConfig::default();
    c.api.system_id = Some("cdr.hospital.example".to_owned());
    let audit = c.audit_section();
    assert_eq!(audit.source_id(), "cdr.hospital.example");
    assert_eq!(audit.role_code_system(), "cdr.hospital.example");
    assert_eq!(c.api.basic_issuer(), "cdr.hospital.example");

    let file = toml_file(
        "[api]\nsystem_id = \"cdr.hospital.example\"\nbasic_issuer = \"urn:example:basic\"\n\
         [audit]\nsource_id = \"cdr-audit\"\nrole_code_system = \"urn:example:roles\"\n",
    );
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    let audit = c.audit_section();
    assert_eq!(audit.source_id(), "cdr-audit");
    assert_eq!(audit.role_code_system(), "urn:example:roles");
    assert_eq!(c.api.basic_issuer(), "urn:example:basic");

    let defaults = FerroEhrConfig::default().audit_section();
    for derived in [defaults.source_id(), defaults.role_code_system()] {
        assert!(!derived.to_ascii_lowercase().contains("ferro"), "{derived}");
    }
}
