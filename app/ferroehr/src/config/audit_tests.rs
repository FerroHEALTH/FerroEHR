// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit]` section's boot rules: the retention floors and ceilings of
//! each jurisdiction, and the retired `[audit.syslog]` TLS keys.

use super::assemble;
use super::tests::{env, standalone, toml_file};
use crate::config::retention::RetentionPeriod;

/// A retention below a jurisdiction's floor is a boot error naming both
/// numbers (#3242); `0` keeps forever and passes; a jurisdiction with no
/// registered floor imposes none.
#[test]
fn a_retention_below_the_jurisdiction_floor_is_refused() {
    let mut config = standalone();
    config.audit.store.retention = Some(RetentionPeriod::Days(30));
    config.privacy.identifier_scan.rules = vec!["nl-bsn".to_owned()];
    let errors = config
        .validate()
        .expect_err("30 days is below the NL floor");
    let text = errors.to_string();
    assert!(
        text.contains("retention = \"P30D\"") && text.contains("1830"),
        "{text}"
    );

    config.audit.store.retention = Some(RetentionPeriod::Days(1830));
    config.validate().expect("the floor itself passes");
    config.audit.store.retention = None;
    config.validate().expect("keep forever passes");

    config.audit.store.retention = Some(RetentionPeriod::Days(30));
    config.privacy.identifier_scan.rules = vec!["no-fodselsnummer".to_owned()];
    config
        .validate()
        .expect("a jurisdiction with no registered floor imposes none");
}

/// Every EU Member State carries the EHDS floor of three years and the
/// refusal names EHDS Art. 9(2); the Dutch five years win over it; the Swiss
/// year stands, Switzerland being outside the Union (#3625).
#[test]
fn the_ehds_floor_holds_in_every_member_state_and_a_longer_national_floor_wins() {
    for rule in ["de-kvnr", "fi-hetu", "se-personnummer"] {
        let mut config = standalone();
        config.privacy.identifier_scan.rules = vec![rule.to_owned()];
        config.audit.store.retention = Some(RetentionPeriod::Days(1095));
        let text = config
            .validate()
            .expect_err("1095 days is below the EHDS floor")
            .to_string();
        assert!(
            text.contains("floor of 1096 days") && text.contains("EHDS Art. 9(2)"),
            "{rule}: {text}"
        );
        config.audit.store.retention = Some(RetentionPeriod::Days(1096));
        config.validate().expect("the EHDS floor itself passes");
    }

    let mut nl = standalone();
    nl.privacy.identifier_scan.rules = vec!["nl-bsn".to_owned()];
    nl.audit.store.retention = Some(RetentionPeriod::Days(1096));
    let text = nl
        .validate()
        .expect_err("the Dutch floor outlasts the EHDS floor")
        .to_string();
    assert!(text.contains("floor of 1830 days"), "{text}");

    let mut ch = standalone();
    ch.privacy.identifier_scan.rules = vec!["ch-ahvn13".to_owned()];
    ch.audit.store.retention = Some(RetentionPeriod::Days(366));
    ch.validate()
        .expect("Switzerland keeps its one-year floor; EHDS does not reach it");
}

/// The German ceiling reaches a deployment only once it declares itself one
/// of the SGB V § 307 controllers, and then caps the horizon — including
/// "keep forever", which is the case the floor rule could never produce
/// (#3346).
#[test]
fn the_sgb_v_ceiling_applies_only_to_a_declared_controller() {
    let mut config = standalone();
    config.privacy.identifier_scan.rules = vec!["de-kvnr".to_owned()];
    config.audit.store.retention = None;
    config
        .validate()
        .expect("an undeclared deployment carries no ceiling");

    config.audit.store.sgb_v_309_controller = true;
    let errors = config
        .validate()
        .expect_err("keep forever outlasts the three-year period");
    let text = errors.to_string();
    assert!(
        text.contains("keep forever") && text.contains("1095"),
        "{text}"
    );

    config.audit.store.retention = Some(RetentionPeriod::Days(1096));
    let errors = config
        .validate()
        .expect_err("1096 days is above the ceiling");
    assert!(errors.to_string().contains("1095"), "{errors}");

    // Germany also carries the EHDS floor of three years (#3625): 1095 days
    // can fall short of three calendar years, so no day count passes both,
    // and the refusal names the horizon in years that does.
    config.audit.store.retention = Some(RetentionPeriod::Days(1095));
    let errors = config
        .validate()
        .expect_err("1095 days can fall short of the three-year EHDS floor");
    assert!(
        errors.to_string().contains("set retention = \"P3Y\""),
        "{errors}"
    );
    config.audit.store.retention = None;
    config.audit.store.retention = Some(RetentionPeriod::Years(3));
    config
        .validate()
        .expect("three calendar years meet the floor and the ceiling exactly");
}

/// A horizon in calendar years is compared exactly against floors in years:
/// the Dutch five years and the Swiss one year (#3625).
#[test]
fn retention_years_meets_a_floor_in_years_exactly() {
    let mut nl = standalone();
    nl.privacy.identifier_scan.rules = vec!["nl-bsn".to_owned()];
    nl.audit.store.retention = Some(RetentionPeriod::Years(4));
    let text = nl
        .validate()
        .expect_err("four years are below the Dutch floor")
        .to_string();
    assert!(
        text.contains("retention = \"P4Y\"") && text.contains("floor of 5 years"),
        "{text}"
    );
    nl.audit.store.retention = Some(RetentionPeriod::Years(5));
    nl.validate().expect("five years meet the Dutch floor");

    let mut ch = standalone();
    ch.privacy.identifier_scan.rules = vec!["ch-ahvn13".to_owned()];
    ch.audit.store.retention = Some(RetentionPeriod::Years(1));
    ch.validate().expect("one year meets the Swiss floor");

    let mut de = standalone();
    de.privacy.identifier_scan.rules = vec!["de-kvnr".to_owned()];
    de.audit.store.sgb_v_309_controller = true;
    de.audit.store.retention = Some(RetentionPeriod::Years(4));
    let text = de
        .validate()
        .expect_err("four years outlast the SGB V period")
        .to_string();
    assert!(text.contains("ceiling of 3 years"), "{text}");
}

/// The horizon is one key: the retired day and year keys are refused at load
/// naming it, and a period of zero is no period.
#[test]
fn the_retired_retention_keys_are_refused_naming_retention() {
    for key in ["retention_days = 1096", "retention_years = 3"] {
        let file = toml_file(&format!("[audit.store]\n{key}\n"));
        let text = assemble(Some(file.path()), &env(&[]), &[])
            .expect_err(key)
            .to_string();
        assert!(
            text.contains("was replaced by audit.store.retention") && text.contains("P5Y"),
            "{key}: {text}"
        );
    }
    let file = toml_file("[audit.store]\nretention = \"P0Y\"\n");
    assert!(assemble(Some(file.path()), &env(&[]), &[]).is_err());
}

/// A ceiling below a floor is a contradiction no horizon resolves, so it is
/// refused at boot rather than silently resolved in favour of one of them
/// (#3346).
#[test]
fn a_ceiling_below_a_floor_is_refused() {
    let mut config = standalone();
    config.audit.store.sgb_v_309_controller = true;
    config.privacy.identifier_scan.rules = vec!["de-kvnr".to_owned(), "nl-bsn".to_owned()];
    config.audit.store.retention = Some(RetentionPeriod::Days(1095));
    let errors = config
        .validate()
        .expect_err("the DE ceiling is below the NL floor");
    let text = errors.to_string();
    assert!(
        text.contains("ceiling of 1095") && text.contains("floor of 1830"),
        "{text}"
    );
}

/// The `[audit.syslog]` TLS keys FerroEHR read before the table took the
/// family's `[audit.syslog.tls]` are refused by name at load, each refusal
/// naming the key that replaces it (#5217).
#[test]
fn the_retired_syslog_tls_keys_are_refused_by_name() {
    for (key, replacement) in [
        ("tls_ca_file", "trust_roots_file"),
        ("tls_identity_cert_file", "client_identity_file"),
        ("tls_identity_key_file", "client_identity_file"),
    ] {
        let file = toml_file(&format!("[audit.syslog]\n{key} = \"/etc/a.pem\"\n"));
        let text = assemble(Some(file.path()), &env(&[]), &[])
            .expect_err(key)
            .to_string();
        assert!(
            text.contains(&format!("audit.syslog.{key} was replaced by"))
                && text.contains(replacement),
            "{key}: {text}"
        );
    }
}
