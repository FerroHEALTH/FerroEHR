// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit]` section's boot rules: the retention floors and ceilings of
//! each jurisdiction, the flat `[audit.syslog]` TLS keys refused as unknown,
//! and the removed `[audit.access_log]` switches refused as unknown (#5562).

use std::path::PathBuf;

use super::assemble;
use super::tests::{assemble_ok, env, file_refusal, standalone, toml_file};
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

/// The horizon is one key: a day or year key is unknown and refused at load,
/// and a period of zero is no period.
#[test]
fn a_day_or_year_retention_key_is_refused_as_unknown() {
    for (key, name) in [
        ("retention_days = 1096", "retention_days"),
        ("retention_years = 3", "retention_years"),
    ] {
        let text = file_refusal(&format!("[audit.store]\n{key}\n"));
        assert!(
            text.contains(&format!("unknown field `{name}`")) && text.contains("`retention`"),
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

/// The `[audit.syslog]` TLS keys outside the family's `[audit.syslog.tls]`
/// table are unknown and refused at load by name.
#[test]
fn a_flat_syslog_tls_key_is_refused_as_unknown() {
    for key in [
        "tls_ca_file",
        "tls_identity_cert_file",
        "tls_identity_key_file",
    ] {
        let text = file_refusal(&format!("[audit.syslog]\n{key} = \"/etc/a.pem\"\n"));
        assert!(
            text.contains(&format!("unknown field `{key}`")),
            "{key}: {text}"
        );
    }
}

/// `[audit.access_log] key = value` refused by the file loader and by its
/// environment variable, each naming `key` (#5562).
fn assert_removed_access_log_key(key: &str, value: &str) {
    let file = toml_file(&format!("[audit.access_log]\n{key} = {value}\n"));
    let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err("the removed key");
    assert!(err.to_string().contains(key), "{key}: {err}");
    let variable = format!("FERROEHR__AUDIT__ACCESS_LOG__{}", key.to_uppercase());
    let err = assemble(
        None,
        &env(&[(variable.as_str(), value.trim_matches('"'))]),
        &[],
    )
    .expect_err("the removed variable");
    assert!(err.to_string().contains(&variable), "{variable}: {err}");
}

/// Checkpoints follow from an `OpenPGP` key in the keyring, so the switch
/// that turned them on is an unknown key (#5562).
#[test]
fn the_removed_checkpoints_switch_is_an_unknown_key() {
    assert_removed_access_log_key("checkpoints", "true");
}

/// Checkpoints run on a built-in schedule, so the interval is an unknown
/// key (#5562).
#[test]
fn the_removed_checkpoint_interval_is_an_unknown_key() {
    assert_removed_access_log_key("checkpoint_interval", "\"1h\"");
}

/// Links are not signed one by one, so the switch that signed them is an
/// unknown key (#5562).
#[test]
fn the_removed_sign_links_switch_is_an_unknown_key() {
    assert_removed_access_log_key("sign_links", "true");
}

/// `trusted_key_files` is trust and stays a key of the section.
#[test]
fn the_trusted_key_files_stay_a_key() {
    let file = toml_file("[audit.access_log]\ntrusted_key_files = [\"/etc/ferroehr/peer.asc\"]\n");
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    assert_eq!(
        c.audit.access_log.trusted_key_files,
        vec![PathBuf::from("/etc/ferroehr/peer.asc")]
    );
}
