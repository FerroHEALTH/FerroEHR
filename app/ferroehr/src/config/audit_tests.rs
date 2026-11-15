// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit]` section's boot rules: the removed retention keys, the flat `[audit.syslog]` TLS keys refused as unknown,
//! and the removed `[audit.access_log]` switches refused as unknown (#5562).

use std::path::PathBuf;

use super::assemble;
use super::tests::{assemble_ok, env, file_refusal, toml_file};

/// The access log's retention is the controller's policy, held in its
/// `ACCESS_POLICY` (`log_retention`), so the horizon and the declared German
/// controller are unknown keys of `[audit.store]`, refused by the file loader
/// and by their environment variables (#5660).
#[test]
fn the_removed_store_retention_keys_are_unknown() {
    for (key, value) in [("retention", "\"P5Y\""), ("sgb_v_309_controller", "true")] {
        let file = toml_file(&format!("[audit.store]\n{key} = {value}\n"));
        let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err("the removed key");
        assert!(
            err.to_string().contains(&format!("unknown field `{key}`")),
            "{key}: {err}"
        );
        let variable = format!("FERROEHR__AUDIT__STORE__{}", key.to_uppercase());
        let err = assemble(
            None,
            &env(&[(variable.as_str(), value.trim_matches('"'))]),
            &[],
        )
        .expect_err("the removed variable");
        assert!(err.to_string().contains(&variable), "{variable}: {err}");
    }
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

/// `publish_directory` names where the checkpoints are published, and is
/// unset by default (#5704).
#[test]
fn the_publish_directory_is_a_key_unset_by_default() {
    let unset = assemble_ok(None, &env(&[]), &[]);
    assert_eq!(unset.audit.access_log.publish_directory, None);
    let file =
        toml_file("[audit.access_log]\npublish_directory = \"/var/lib/ferroehr/checkpoints\"\n");
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    assert_eq!(
        c.audit.access_log.publish_directory,
        Some(PathBuf::from("/var/lib/ferroehr/checkpoints"))
    );
}
