// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The grounds of each decision as the access record names them (#5514): a
//! test per ground kind, a refusal per code with its step, the absent
//! answers of steps 1 and 2 included, and the entry the record writes.
//!
//! EHDS Annex II 3.2 asks the log for each access, Art. 9 gives it to the
//! person, and Art. 11(5) requires an access in the person's vital interest
//! to be logged (`docs/law/eu/ehds/text.html`); which grounds are named is
//! our own design.

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, Refusal, Term};
use openehr_rm::draft::access_log::access_log::access_authorisation::AccessAuthorisation;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use serde_json::json;

use super::*;
use ferroehr::access_eval::condition::{Asking, EhrEntry, InForce, RecordFacts, RequestAccess};
use ferroehr::access_eval::record::{Decision, Step};
use ferroehr::access_log::entry::{entry_of, horizon_retention};
use ferroehr::ids::EhrId;
use ferroehr::system_log::event::{
    AccessDecision, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

/// The EHR of the tests.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";
/// Another patient's EHR, the one a subject caller of [`EHR`] is not.
const OTHER_EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01399";
/// The uid of the `EHR_ACCESS` version that carries its settings.
const EHR_ACCESS_VERSION: &str = "5a1c0b8e-55a6-4d0b-9a63-3f0f6c3c2a11::cdr.example.org::2";
/// The uid of the `ACCESS_POLICY` version in force.
const POLICY_VERSION: &str = "1f2e3d4c-5b6a-4978-8a9b-0c1d2e3f4a5b::cdr.example.org::3";
/// The `ROLE` whose `CAPABILITY` grants the caller an operation.
const ROLE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa0";

/// The request of `caller` asking `asked` under `policy`, naming the grant
/// of `access_service` by [`ROLE`] and the policy version in its record.
fn request(caller: Caller, asked: Asked<'_>, policy: Option<AccessPolicy>) -> RequestAccess {
    RequestAccess::new(
        caller,
        Asking {
            operation: asked.operation,
            purpose: asked.purpose.map(str::to_owned),
            emergency: asked.emergency,
        },
        InForce {
            policy: policy.map(Arc::new),
            categories: categories(),
            server_default: AccessDefault::Open,
            now: now(),
            grants: None,
        },
    )
    .with_record_facts(RecordFacts {
        policy_version: Some(POLICY_VERSION.to_owned()),
        capabilities: vec![("access_service", uuid(ROLE))],
    })
}

/// The subject of [`EHR`], through its patient context.
fn subject_of_ehr() -> Caller {
    Caller {
        own_ehr: Some(EhrId(uuid(EHR))),
        ..caller(CallerKind::Subject, GUARDIAN)
    }
}

/// The subject of another patient's EHR, through a patient context naming
/// that EHR.
fn subject_of_other_ehr() -> Caller {
    Caller {
        own_ehr: Some(EhrId(uuid(OTHER_EHR))),
        ..caller(CallerKind::Subject, GUARDIAN)
    }
}

/// The decision `access` records for the EHR with `settings`, after it
/// answered `item`.
fn decide_on(
    access: &RequestAccess,
    settings: Option<EhrAccessSettings>,
    item: &Item<'_>,
) -> (Outcome, Decision) {
    let ehr_id = EhrId(uuid(EHR));
    let version = settings.as_ref().map(|_| EHR_ACCESS_VERSION.to_owned());
    let verdict = access.judge_ehr(
        ehr_id,
        &EhrEntry {
            settings: Arc::new(settings),
            subject: None,
            version,
            revision: ferroehr::storage::access_projection::Revision::default(),
        },
    );
    let outcome = access.note(ehr_id, &verdict, item);
    (outcome, access.decision(Some(ehr_id)))
}

/// The grounds of `decision` of `kind`.
fn of_kind<'a>(decision: &'a Decision, kind: &str) -> Vec<&'a AccessAuthorisation> {
    decision
        .grounds
        .iter()
        .filter(|ground| ground.kind.value == kind)
        .collect()
}

/// The capabilities and roles the caller held are `role` grounds: each
/// grant by its operation code and `ROLE`, each functional role by its code.
#[test]
fn a_role_ground_names_each_capability_and_role() {
    let mut nurse = professional(NURSE);
    nurse.roles = vec!["NURSE".to_owned()];
    let access = request(nurse, read(None), None);
    let (outcome, decision) = decide_on(&access, None, &Item::Ehr);
    assert_eq!(outcome, Outcome::Admit);
    let roles = of_kind(&decision, "role");
    assert_eq!(roles.len(), 2, "{decision:?}");
    assert_eq!(roles[0].name.as_deref(), Some("access_service"));
    let role = roles[0].reference.as_ref().expect("the ROLE");
    assert_eq!(
        (role.id.as_str(), role.r#type.as_deref()),
        (ROLE, Some("ROLE"))
    );
    assert_eq!(roles[1].name.as_deref(), Some("NURSE"));
    assert!(decision.refused.is_none());
}

/// The subject reading their own record rests on `subject of care`.
#[test]
fn the_subject_reads_under_subject_of_care() {
    let access = request(subject_of_ehr(), read(None), None);
    let (_, decision) = decide_on(&access, None, &Item::Ehr);
    assert_eq!(
        of_kind(&decision, "subject of care").len(),
        1,
        "{decision:?}"
    );
}

/// A permit that admits under `consent_required` is a `consent` ground with
/// the `EHR_ACCESS` version, the directive's position and legal grounds, and
/// that version is the record's authorisation reference.
#[test]
fn a_permit_is_a_consent_ground_with_its_ehr_access_version() {
    let permit = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }],
                 "legal_grounds": [coded("urn:example:law", "gdpr-9-2-a")] }),
    );
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": [{ "party": party("PERSON", DR_B) }] }),
    );
    let access = request(professional(NURSE), read(None), None);
    let (outcome, decision) = decide_on(
        &access,
        Some(settings("consent_required", &[deny, permit], &[], &[])),
        &Item::Ehr,
    );
    assert_eq!(outcome, Outcome::Admit);
    let consents = of_kind(&decision, "consent");
    assert_eq!(consents.len(), 1, "{decision:?}");
    let consent = consents[0];
    assert_eq!(consent.name.as_deref(), Some("EHR_ACCESS"));
    assert_eq!(consent.version.as_deref(), Some(EHR_ACCESS_VERSION));
    let reference = consent.reference.as_ref().expect("the directive");
    assert_eq!(
        (reference.id.as_str(), reference.r#type.as_deref()),
        ("2", Some("position"))
    );
    let grounds = consent.legal_grounds.as_deref().unwrap_or_default();
    assert_eq!(grounds[0].defining_code.code_string, "gdpr-9-2-a");
    assert_eq!(decision.policies, [EHR_ACCESS_VERSION]);
}

/// A gate-keeper of the settings reads under a `consent` ground naming the
/// `EHR_ACCESS` version.
#[test]
fn a_gate_keeper_reads_under_the_ehr_access_settings() {
    let keeper = json!({ "_type": "PARTY_IDENTIFIED", "external_ref": party("PERSON", NURSE) });
    let access = request(professional(NURSE), read(None), None);
    let (outcome, decision) = decide_on(
        &access,
        Some(settings("consent_required", &[], &[], &[keeper])),
        &Item::Ehr,
    );
    assert_eq!(outcome, Outcome::Admit);
    let consents = of_kind(&decision, "consent");
    assert_eq!(consents.len(), 1, "{decision:?}");
    assert_eq!(consents[0].version.as_deref(), Some(EHR_ACCESS_VERSION));
    assert!(consents[0].reference.is_none());
}

/// A representative reads under the `delegation` permit naming them, a
/// `representation` ground.
#[test]
fn a_delegation_is_a_representation_ground() {
    let delegation = directive(
        "delegation",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", GUARDIAN) }],
                 "operations": [term("read")] }),
    );
    let access = request(
        caller(CallerKind::Representative, GUARDIAN),
        read(None),
        None,
    );
    let (outcome, decision) = decide_on(
        &access,
        Some(settings("consent_required", &[delegation], &[], &[])),
        &Item::Ehr,
    );
    assert_eq!(outcome, Outcome::Admit);
    let represented = of_kind(&decision, "representation");
    assert_eq!(represented.len(), 1, "{decision:?}");
    assert_eq!(represented[0].version.as_deref(), Some(EHR_ACCESS_VERSION));
}

/// The break-glass role lifting a restriction in an emergency is an
/// `emergency` ground naming the directive it lifted, and lifting steps 4 to
/// 7 one naming the `ACCESS_POLICY` version; the entry marks the access as
/// an emergency, so the patient sees it (EHDS Art. 11(5)).
#[test]
fn an_emergency_override_is_marked_with_what_it_lifted() {
    let restriction = directive(
        "restriction",
        "deny",
        &json!({ "emergency_override": true }),
    );
    let ward = policy(&json!({ "care_relationship_required": true }));
    let access = request(
        professional(DR_B),
        break_glass(read(Some("ETREAT"))),
        Some(ward),
    );
    let (outcome, decision) = decide_on(
        &access,
        Some(settings("open", &[restriction], &[], &[])),
        &Item::Ehr,
    );
    assert_eq!(outcome, Outcome::Admit);
    let emergencies = of_kind(&decision, "emergency");
    assert_eq!(emergencies.len(), 2, "{decision:?}");
    assert_eq!(emergencies[0].name.as_deref(), Some("EHR_ACCESS"));
    assert_eq!(
        emergencies[0].reference.as_ref().map(|r| r.id.as_str()),
        Some("1")
    );
    assert_eq!(emergencies[1].name.as_deref(), Some("ACCESS_POLICY"));
    assert_eq!(emergencies[1].version.as_deref(), Some(POLICY_VERSION));
    assert_eq!(decision.policies, [EHR_ACCESS_VERSION, POLICY_VERSION]);

    let mut event = event_of(&access);
    event.emergency_access = true;
    let entry =
        entry_of(&event, None, "cdr.example.org", horizon_retention(None)).expect("an entry");
    assert!(entry.is_emergency);
    let kinds: Vec<&str> = entry
        .authorisations
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|ground| ground.kind.value.as_str())
        .collect();
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "emergency").count(),
        2,
        "the override marks the access once per lift, with no bare mark: {kinds:?}"
    );
}

/// `entry` with the link the store gives it, so its invariants hold.
fn linked(mut entry: AccessLogEntry) -> AccessLogEntry {
    entry.link = AccessLogLink {
        sequence: 1,
        algorithm: "SHA-256".to_owned(),
        previous_digest: "0".repeat(64),
        content_digest: "1".repeat(64),
        digest: "2".repeat(64),
        signature: None,
    };
    entry
}

/// The access record of the EHR the tests judge, stamped by `access`.
fn event_of(access: &RequestAccess) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::MinorFailure,
    );
    "dr-b".clone_into(&mut event.user_id);
    event.ehr_id = Some(EHR.to_owned());
    ferroehr::access_eval::record::stamp(&mut event, access);
    event
}

/// One refusal case: the code, the caller, the purpose it declares, the
/// policy, the settings, the item it asks for, and what decided it.
type Row<'a> = (
    Refusal,
    Caller,
    Option<&'a str>,
    Option<AccessPolicy>,
    EhrAccessSettings,
    Item<'a>,
    &'a str,
);

/// Every refusal is recorded with its code and the step that decided it,
/// the absent answers of the opt-out and the restriction included, whose
/// response says nothing (EHDS Art. 8); the entry writes the code as its
/// outcome reason and holds every invariant of the draft.
#[test]
fn every_refusal_is_recorded_with_its_code_and_step() {
    let opt_out = directive("opt_out", "deny", &json!({}));
    let restriction = directive("restriction", "deny", &json!({}));
    let deny = directive("consent", "deny", &json!({}));
    let label = json!({ "_type": "ACCESS_LABEL", "target": object_ref(OBJECT),
                        "sensitivities": [coded(CONFIDENTIALITY, "HIV")] });
    let care = policy(&json!({ "care_relationship_required": true }));
    let purpose = policy(&json!({ "purpose_required": true }));
    let rows: Vec<Row<'_>> = vec![
        (
            Refusal::OptOut,
            caller(CallerKind::AccessService, DR_B),
            None,
            None,
            settings("open", std::slice::from_ref(&opt_out), &[], &[]),
            Item::Ehr,
            "directive 1 of EHR_ACCESS",
        ),
        (
            Refusal::Restricted,
            professional(DR_B),
            None,
            None,
            settings("open", &[deny.clone(), restriction.clone()], &[], &[]),
            Item::Ehr,
            "directive 2 of EHR_ACCESS",
        ),
        (
            Refusal::ConsentDenied,
            professional(DR_B),
            None,
            None,
            settings("open", std::slice::from_ref(&deny), &[], &[]),
            Item::Ehr,
            "directive 1 of EHR_ACCESS",
        ),
        (
            Refusal::Sensitivity,
            professional(DR_B),
            None,
            None,
            settings("open", &[], std::slice::from_ref(&label), &[]),
            object(OBJECT, None),
            "ACCESS_POLICY",
        ),
        (
            Refusal::NoCareRelationship,
            professional(DR_B),
            None,
            Some(care),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "ACCESS_POLICY",
        ),
        (
            Refusal::PurposeMissing,
            professional(DR_B),
            None,
            Some(purpose.clone()),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "ACCESS_POLICY",
        ),
        (
            Refusal::PurposeNotPermitted,
            professional(DR_B),
            Some("HRESCH"),
            Some(purpose),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "ACCESS_POLICY",
        ),
        (
            Refusal::ConsentRequired,
            professional(DR_B),
            None,
            None,
            settings("consent_required", &[], &[], &[]),
            Item::Ehr,
            "the default_access of EHR_ACCESS",
        ),
        (
            Refusal::NotSubject,
            subject_of_other_ehr(),
            None,
            None,
            settings(
                "open",
                &[directive("consent", "permit", &json!({}))],
                &[],
                &[],
            ),
            Item::Ehr,
            "the EHR of the caller's patient context",
        ),
    ];
    assert_eq!(rows.len(), Refusal::ALL.len(), "a row per refusal code");
    for (refusal, who, declared, policy, settings, item, by) in rows {
        let code = refusal.code();
        let access = request(who, read(declared), policy);
        let (outcome, decision) = decide_on(&access, Some(settings), &item);
        let expected = if refusal.is_invisible() {
            Outcome::Absent(refusal)
        } else {
            Outcome::Refused(refusal)
        };
        assert_eq!(outcome, expected, "{code}");
        let refused = decision.refused.as_ref().expect("the refusal is noted");
        assert_eq!(refused.code, refusal, "{code}");
        assert_eq!(refused.step, Step::of(refusal), "{code}");
        assert!(refused.by.starts_with(by), "{code}: {}", refused.by);

        let event = event_of(&access);
        assert_eq!(event.decision, AccessDecision::Refused, "{code}");
        let entry =
            entry_of(&event, None, "cdr.example.org", horizon_retention(None)).expect("an entry");
        assert_eq!(entry.outcome.result.value, "refused", "{code}");
        let reason = entry.outcome.reason.as_ref().expect("the reason");
        assert_eq!(reason.defining_code.code_string, code);
        let Some(DvText::DvText(description)) = entry.outcome.description.as_ref() else {
            panic!("{code}: the step is plain text");
        };
        let step = format!("step {} ", refused.step.number());
        assert!(description.value.starts_with(&step), "{code}");
        openehr_rm::draft::access_log::invariants::check_entry(&linked(entry))
            .unwrap_or_else(|violations| panic!("{code}: {violations:?}"));
    }
}

/// An EHR with no settings that a query reaches through its condition is
/// recorded as a direct read of it is: the subject reading their own record
/// rests on `subject of care` either way (#5581).
#[test]
fn a_query_served_ehr_with_no_settings_names_the_grounds_of_a_direct_read() {
    let subject = || request(subject_of_ehr(), read(None), None);
    let (_, direct) = decide_on(&subject(), None, &Item::Ehr);
    let queried = subject();
    queried.judge_unsettled(EhrId(uuid(EHR)));
    let queried = queried.decision(Some(EhrId(uuid(EHR))));
    assert_eq!(queried, direct);
    assert_eq!(of_kind(&queried, "subject of care").len(), 1, "{queried:?}");
}
