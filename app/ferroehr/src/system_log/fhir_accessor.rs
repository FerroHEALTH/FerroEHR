// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The accessor of one access record (Regulation (EU) 2025/327 Annex II
//! 3.2(a), (b)), as `ehds-logging` writes it.

use ehds_logging::record::{
    Accessor, Acting, AssuranceLevel, Coded, Correlation, Professional, Purpose,
};

use crate::system_log::AuditError;
use crate::system_log::access_binding::{Members, members};
use crate::system_log::event::{ActingMode, AuditEvent};
use crate::system_log::fhir::nonempty;

/// Who accessed the data (Annex II 3.2(a), (b)), as the authentication layer
/// judged them: a Basic user has no issuer and no client, and an acting mode
/// the deployment did not establish is none. The correlation id of a
/// `chained` request comes beside it, for the record's request.
///
/// # Errors
/// [`AuditError::Render`] when the binding record has no form in the access
/// record.
pub(super) fn accessor(
    event: &AuditEvent,
    missing: &str,
    purposes: Vec<Purpose>,
) -> Result<(Accessor, Option<Correlation>), AuditError> {
    let members = match &event.binding {
        Some(binding) => members(binding, event.issuer.as_deref())
            .map_err(|e| AuditError::Render(Box::new(e)))?,
        None => Members::default(),
    };
    let accessor = Accessor {
        issuer: event.issuer.clone(),
        subject: nonempty(&event.user_id, missing),
        client_id: event.client_id.clone(),
        audience: None,
        provider: event.organisation.clone(),
        acting: event.actor.mode.map(|mode| match mode {
            ActingMode::Client | ActingMode::Delegated => Acting::Client,
            ActingMode::Person => Acting::Person,
            ActingMode::External => Acting::External,
            ActingMode::System => Acting::System,
        }),
        // NOTE: NEN 7513:2018 §5 (not vendored, NEN copyright) asks for the role or authority
        // under which the record was accessed, which BALP 1.1.4 §3:5.7.5.4 writes at role.
        roles: event
            .role_codings()
            .into_iter()
            .map(|role| Coded {
                system: role.system,
                code: role.code,
            })
            .collect(),
        token_id: event.token_id.clone(),
        assurance: event.actor.assurance_level.map(assurance),
        professional: Professional {
            name: None,
            identifier: event.actor.professional_id.clone(),
        },
        alt_id: None,
        purposes,
        // NOTE: Regulation (EU) 2025/327 Art 68 names the data permit; FHIR R4 AuditEvent.agent.policy carries it.
        policies: event.policies(),
        relayed: members.relayed,
        binding: members.binding,
        actors: members.actors,
        represents: members.represents,
        acts_for: members.acts_for,
        facilities: members.facilities,
    };
    Ok((accessor, members.correlation))
}

/// The `ehds-logging` level of an authentication assurance level.
const fn assurance(level: ferrohealth_smart::assurance::AssuranceLevel) -> AssuranceLevel {
    match level {
        ferrohealth_smart::assurance::AssuranceLevel::Low => AssuranceLevel::Low,
        ferrohealth_smart::assurance::AssuranceLevel::Substantial => AssuranceLevel::Substantial,
        ferrohealth_smart::assurance::AssuranceLevel::High => AssuranceLevel::High,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_log::access_binding::tests::{hostile, organisation, representative, system};
    use crate::system_log::categories::{AccessedContent, Classifier};
    use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};
    use crate::system_log::fhir::to_fhir;
    use crate::system_log::message::AuditContext;

    fn written(binding: crate::system_log::binding::BindingRecord) -> serde_json::Value {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        event.user_id = "user-1".to_owned();
        event.issuer = Some("https://federation.example.org".to_owned());
        event.category =
            Some(Classifier::default().classify(&AccessedContent::Objects(Vec::new()), false));
        event.binding = Some(binding);
        let ctx = AuditContext {
            source_id: "ferroehr".to_owned(),
            server_ip: "10.42.23.77".to_owned(),
            value_if_missing: "UNKNOWN".to_owned(),
            ..AuditContext::default()
        };
        to_fhir(&event, &ctx, Some("patient-42")).expect("render")
    }

    /// The `type` and `valueString` of each detail of the entity `name`.
    fn details(written: &serde_json::Value, name: &str) -> Vec<(String, String)> {
        written["entity"]
            .as_array()
            .expect("entities")
            .iter()
            .filter(|entity| entity["name"] == name)
            .flat_map(|entity| entity["detail"].as_array().cloned().unwrap_or_default())
            .map(|detail| {
                (
                    detail["type"].as_str().expect("a type").to_owned(),
                    detail["valueString"].as_str().expect("a value").to_owned(),
                )
            })
            .collect()
    }

    fn has(details: &[(String, String)], kind: &str, value: &str) -> bool {
        details.contains(&(kind.to_owned(), value.to_owned()))
    }

    /// EHDS Annex II 3.2(a), (b): BALP 1.1.4 §3:5.7.5.4 has no element for the
    /// binding fields, so they ride in the `ehds-accessor` and `ehds-relayed`
    /// entities `ehds-logging` writes.
    #[test]
    fn the_balp_record_carries_the_binding_fields() {
        let accessor = details(&written(system()), "ehds-accessor");
        assert!(has(&accessor, "binding", "system"), "{accessor:?}");
        assert!(has(&accessor, "acts-for", "urn:oid:2.999.7|provider-1"));
        assert!(has(&accessor, "facility", "urn:oid:2.999.8|ward-7"));
        let accessor = details(&written(representative()), "ehds-accessor");
        assert!(
            has(&accessor, "party", "demographic|PERSON|party-1"),
            "{accessor:?}"
        );
        assert!(has(&accessor, "represents-relationship", "10"));
        let organisation = written(organisation());
        let relayed = details(&organisation, "ehds-relayed");
        assert!(
            has(&relayed, "agreement-consent", "consent-1"),
            "{relayed:?}"
        );
        assert!(has(&relayed, "asserted-field", "nl.ura=12345678"));
        assert!(
            organisation.to_string().contains("corr-1"),
            "the correlation id"
        );
    }

    /// The system and external acting modes are written, which 0.4.0 left out.
    #[test]
    fn the_system_mode_is_written() {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        event.actor.mode = Some(ActingMode::System);
        let (system, _) = accessor(&event, "UNKNOWN", Vec::new()).expect("an accessor");
        assert_eq!(system.acting, Some(Acting::System));
        event.actor.mode = Some(ActingMode::External);
        let (external, _) = accessor(&event, "UNKNOWN", Vec::new()).expect("an accessor");
        assert_eq!(external.acting, Some(Acting::External));
    }

    /// A hostile binding value is written escaped in the BALP record.
    #[test]
    fn a_hostile_binding_value_is_written_escaped() {
        let mut binding = system();
        binding.facilities[0].id = hostile();
        let text = written(binding).to_string();
        assert!(!text.contains('\u{202e}'), "a raw bidi override");
        assert!(!text.contains('\u{1b}'), "a raw escape");
        assert!(
            text.contains(r"\\u{202e}"),
            "the override written as its code point"
        );
    }
}
