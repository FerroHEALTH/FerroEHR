// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit]` section's boot rules: an unknown key of each table is refused
//! from the file and the environment.

use std::path::PathBuf;

use super::assemble;
use super::tests::{assemble_ok, env, standalone, toml_file};

/// An unknown key of each `[audit]` table is refused by the file loader and
/// by its environment variable, each naming it.
#[test]
fn an_unknown_audit_key_is_refused() {
    for table in ["store", "syslog", "syslog.tls", "access_log"] {
        let file = toml_file(&format!("[audit.{table}]\ncolour = \"blue\"\n"));
        let err = assemble(Some(file.path()), &env(&[]), &[]).expect_err("an unknown key");
        assert!(err.to_string().contains("colour"), "{table}: {err}");
    }
    for table in ["STORE", "SYSLOG", "ACCESS_LOG"] {
        let variable = format!("FERROEHR__AUDIT__{table}__COLOUR");
        let err = assemble(None, &env(&[(variable.as_str(), "blue")]), &[])
            .expect_err("an unknown variable");
        assert!(err.to_string().contains(&variable), "{variable}: {err}");
    }
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

/// `[audit.store] gather_window` is accepted up to one millisecond and
/// refused above it by name, since every request that writes an access-log
/// entry waits up to it (#6242).
#[test]
fn a_gather_window_above_one_millisecond_is_refused() {
    let mut config = standalone();
    config.audit.store.gather_window = ferrohealth_config::units::Duration::from_millis(1);
    assert!(config.validate().is_ok());
    config.audit.store.gather_window = ferrohealth_config::units::Duration::from_millis(2);
    let refused = config
        .validate()
        .expect_err("a window past 1 ms")
        .to_string();
    assert!(refused.contains("audit.store.gather_window"), "{refused}");
}
