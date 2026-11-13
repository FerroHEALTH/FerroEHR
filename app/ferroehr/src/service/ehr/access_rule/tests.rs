// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use serde_json::{Value, json};

use super::*;
use crate::party_binding::policy_mark::TERMINOLOGY;

const GUARDIAN: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f80";
const DOCTOR: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f81";
const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f82";

/// A `PARTY_PROXY` of `class` referencing the `PERSON` `id`.
fn named(class: &str, id: &str) -> Value {
    let mut proxy = json!({ "_type": class,
        "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                          "id": { "_type": "HIER_OBJECT_ID", "value": id } } });
    if class == "PARTY_RELATED" {
        proxy["relationship"] = json!({ "_type": "DV_CODED_TEXT", "value": "guardian",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "10" } });
    }
    proxy
}

/// Restricted settings with `gate_keepers`.
fn settings(gate_keepers: &[Value]) -> EhrAccessSettings {
    let mut s = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": { "_type": "DV_CODED_TEXT", "value": "restricted",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": "restricted" } } });
    if !gate_keepers.is_empty() {
        s["gate_keepers"] = json!(gate_keepers);
    }
    EhrAccessSettings::from_ehr_access(&json!({ "_type": "EHR_ACCESS", "settings": s }))
        .expect("the settings read")
        .expect("settings")
}

fn administrator() -> SettingsCaller {
    SettingsCaller {
        administrator: true,
        ..SettingsCaller::default()
    }
}

fn subject() -> SettingsCaller {
    SettingsCaller {
        subject: true,
        ..SettingsCaller::default()
    }
}

fn party(id: &str) -> SettingsCaller {
    SettingsCaller {
        party: Some(id.to_owned()),
        ..SettingsCaller::default()
    }
}

/// The subject as their own gate-keeper, a guardian and a professional
/// named by reference.
fn kept() -> EhrAccessSettings {
    settings(&[
        json!({ "_type": "PARTY_SELF" }),
        named("PARTY_RELATED", GUARDIAN),
        named("PARTY_IDENTIFIED", DOCTOR),
    ])
}

#[test]
fn an_administrator_writes_any_settings() {
    let caller = administrator();
    assert!(caller.may_write(None).is_ok());
    assert!(caller.may_write(Some(&settings(&[]))).is_ok());
    assert!(
        caller
            .may_write(Some(&EhrAccessSettings::HeldClosed))
            .is_ok()
    );
}

#[test]
fn the_subject_writes_only_as_a_named_gate_keeper() {
    assert!(subject().may_write(Some(&kept())).is_ok());
    let refused = subject().may_write(Some(&settings(&[named("PARTY_IDENTIFIED", DOCTOR)])));
    assert!(
        matches!(refused, Err(ServiceError::Forbidden(_))),
        "{refused:?}"
    );
    assert!(subject().may_write(None).is_err());
}

#[test]
fn a_representative_writes_only_as_a_named_gate_keeper() {
    assert!(party(GUARDIAN).may_write(Some(&kept())).is_ok());
    assert!(
        party(GUARDIAN)
            .may_write(Some(&settings(&[json!({ "_type": "PARTY_SELF" })])))
            .is_err()
    );
}

#[test]
fn a_professional_writes_only_as_a_named_gate_keeper() {
    assert!(party(DOCTOR).may_write(Some(&kept())).is_ok());
    assert!(party(NURSE).may_write(Some(&kept())).is_err());
}

/// With no settings, or settings held closed, only an administrator writes.
#[test]
fn with_no_gate_keeper_only_an_administrator_writes() {
    for caller in [subject(), party(DOCTOR), SettingsCaller::default()] {
        assert!(caller.may_write(None).is_err());
        assert!(
            caller
                .may_write(Some(&EhrAccessSettings::HeldClosed))
                .is_err()
        );
    }
}

/// A caller with no party matches no `PARTY_SELF` either, unless it is the
/// subject.
#[test]
fn an_unbound_caller_writes_nothing() {
    assert!(SettingsCaller::default().may_write(Some(&kept())).is_err());
}

#[test]
fn the_readers_are_the_administrator_the_subject_and_the_gate_keepers() {
    let restricted = settings(&[named("PARTY_IDENTIFIED", DOCTOR)]);
    assert!(administrator().may_read(Some(&restricted)));
    assert!(subject().may_read(Some(&restricted)));
    assert!(party(DOCTOR).may_read(Some(&restricted)));
    assert!(!party(NURSE).may_read(Some(&restricted)));
    assert!(!SettingsCaller::default().may_read(Some(&restricted)));
    assert!(administrator().reads_every());
    assert!(subject().reads_every());
    assert!(!party(DOCTOR).reads_every());
}
