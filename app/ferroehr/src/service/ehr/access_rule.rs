// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who may write and read the `EHR_ACCESS` settings of an EHR.
//!
//! BASE Architecture Overview §Security Policy, "Access control of access
//! settings", names a gate-keeper who alone changes the settings, and the
//! `ACCESS_DIRECTIVE_SETTINGS.gate_keepers` of the draft access control hold
//! them: `PARTY_SELF` for the subject, otherwise a party referenced into the
//! demographic service; with none, the change is left to the system's
//! administrators. A new version is therefore committed by a gate-keeper of
//! the current settings or by `admin_role`, acting on the subject's
//! instruction; every other caller is refused `403`. The settings are read by
//! the same callers and by the subject, and answered as absent to anyone
//! else, because EHDS Art. 8 forbids a restriction being visible to a
//! provider (`docs/law/eu/ehds/text.html`). Who reads and writes the
//! settings is our own design (#5508).

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;

use crate::error::ServiceError;
use crate::party_binding::BindingMapping;
use crate::service::ehr::access_settings::EhrAccessSettings;

/// Who the running request's caller is to the `EHR_ACCESS` settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsCaller {
    /// Whether the caller writes and reads every EHR's settings: the server
    /// itself (no request caller) or a holder of `admin_role`.
    administrator: bool,
    /// Whether the caller is the record's person, whose committer is
    /// `PARTY_SELF` (RM common master04 §Audit Details).
    subject: bool,
    /// The id of the demographic party the caller is bound to.
    party: Option<String>,
}

impl SettingsCaller {
    /// The caller of the running request, read from the committer identity
    /// and the roles the protocol adapter publishes, with `mapping` naming
    /// the administrator role.
    #[must_use]
    pub fn current(mapping: &BindingMapping) -> Self {
        let administrator = crate::party_binding::protect::administrator(mapping);
        let committer = crate::versioning::committer::current_committer()
            .and_then(|identity| crate::versioning::committer::bound::server_committer(&identity));
        let subject = matches!(committer, Some(PartyProxy::PartySelf(_)));
        let party = committer.as_ref().and_then(reference).and_then(party_key);
        Self {
            administrator,
            subject,
            party,
        }
    }

    /// Whether the caller writes and reads every EHR's settings.
    #[must_use]
    pub fn administrator(&self) -> bool {
        self.administrator
    }

    /// Whether the caller is the record's person.
    #[must_use]
    pub fn subject(&self) -> bool {
        self.subject
    }

    /// The id of the demographic party the caller is bound to.
    #[must_use]
    pub fn party(&self) -> Option<&str> {
        self.party.as_deref()
    }

    /// Whether a gate-keeper of `settings` names the caller: `PARTY_SELF`
    /// names the subject, and any other `PARTY_PROXY` names the party its
    /// `external_ref` references.
    #[must_use]
    pub fn is_gate_keeper(&self, settings: Option<&EhrAccessSettings>) -> bool {
        settings
            .map(EhrAccessSettings::gate_keepers)
            .unwrap_or_default()
            .iter()
            .any(|keeper| match keeper {
                PartyProxy::PartySelf(_) => self.subject,
                PartyProxy::PartyIdentified(_) => self.party.as_ref().is_some_and(|party| {
                    reference(keeper).and_then(party_key).as_ref() == Some(party)
                }),
            })
    }

    /// Whether the caller may commit the next `EHR_ACCESS` version of an EHR
    /// whose current settings are `current`.
    ///
    /// # Errors
    /// [`ServiceError::not_permitted`] (`403`) for a caller who is neither an
    /// administrator nor a gate-keeper of `current`.
    pub fn may_write(&self, current: Option<&EhrAccessSettings>) -> Result<(), ServiceError> {
        if self.administrator || self.is_gate_keeper(current) {
            return Ok(());
        }
        Err(ServiceError::not_permitted(
            "only a gate-keeper the current EHR_ACCESS settings name, or an administrator acting \
             on the subject's instruction, commits a new EHR_ACCESS version (BASE Architecture \
             Overview, Access control of access settings)",
        ))
    }

    /// Whether the caller may read the settings `current` of an EHR: an
    /// administrator, the subject and a gate-keeper.
    #[must_use]
    pub fn may_read(&self, current: Option<&EhrAccessSettings>) -> bool {
        self.administrator || self.subject || self.is_gate_keeper(current)
    }

    /// Whether the caller reads the settings of every EHR it reaches, with no
    /// look at their gate-keepers: an administrator, and the subject, whom
    /// the patient context confines to their own EHR.
    #[must_use]
    pub fn reads_every(&self) -> bool {
        self.administrator || self.subject
    }
}

/// The `external_ref` of a `PARTY_PROXY`.
fn reference(proxy: &PartyProxy) -> Option<&PartyRef> {
    match proxy {
        PartyProxy::PartySelf(party) => party.external_ref.as_ref(),
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) => {
            party.external_ref.as_ref()
        }
        PartyProxy::PartyIdentified(PartyIdentified::PartyRelated(party)) => {
            party.external_ref.as_ref()
        }
    }
}

/// The id a party reference names, `None` for an id kind that names no
/// party.
// NOTE: our own design, the local party source names each party by its container's
// UUID, unique across namespaces, so the id alone identifies the party.
pub(crate) fn party_key(reference: &PartyRef) -> Option<String> {
    match &reference.id {
        ObjectId::HierObjectId(id) => Some(id.value().to_owned()),
        ObjectId::GenericId(id) => Some(id.value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
}
