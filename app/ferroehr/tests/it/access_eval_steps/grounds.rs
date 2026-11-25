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
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use serde_json::json;

use super::*;
use ferroehr::access_eval::condition::{Asking, EhrEntry, InForce, RecordFacts, RequestAccess};
use ferroehr::access_eval::record::{Decision, Step};
use ferroehr::access_log::entry::entry_of;
use ferroehr::access_log::retention::stated;
use ferroehr::ids::EhrId;
use ferroehr::storage::access_projection::EhrSubject;
use ferroehr::system_log::event::{
    AccessDecision, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::retention::LogRetention;

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
/// The uid of a controller rule.
const RULE_UID: &str = "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f";

/// The request of `caller` asking `asked` under `policy`, naming the grant
/// of `access_service` by [`ROLE`] and the policy version in its record.
fn request(caller: Caller, asked: Asked<'_>, policy: Option<AccessPolicy>) -> RequestAccess {
    RequestAccess::new(
        caller,
        Asking {
            operation: asked.operation,
            purpose: asked.purpose.map(str::to_owned),
            reason: reason_of(asked),
            opt_out_override: asked.opt_out_override,
            episodes: asked.episodes.map(Arc::from),
        },
        InForce {
            policy: policy
                .map_or_else(|| in_force(None, asked), |p| in_force(Some(&p), asked))
                .map(Arc::new),
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

/// The evaluator of a request by `caller`, as [`request`], with grants that
/// resolve a subject in this CDR's own namespace, and the subject reference
/// of the `PERSON` the caller is bound to.
fn bound(
    caller: Caller,
    asked: Asked<'_>,
    policy: Option<AccessPolicy>,
) -> (RequestAccess, Option<EhrSubject>) {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:9/nowhere")
        .expect("a lazy pool builds without connecting");
    let grants = Arc::new(ferroehr::party_binding::grants::live::LiveGrants::new(pool));
    let subject = caller
        .party
        .clone()
        .map(|party| EhrSubject::new(&*grants.namespace(), party));
    let access = RequestAccess::new(
        caller,
        Asking {
            operation: asked.operation,
            purpose: asked.purpose.map(str::to_owned),
            reason: reason_of(asked),
            opt_out_override: asked.opt_out_override,
            episodes: asked.episodes.map(Arc::from),
        },
        InForce {
            policy: policy
                .map_or_else(|| in_force(None, asked), |p| in_force(Some(&p), asked))
                .map(Arc::new),
            categories: categories(),
            server_default: AccessDefault::Open,
            now: now(),
            grants: Some(grants),
        },
    )
    .with_record_facts(RecordFacts {
        policy_version: Some(POLICY_VERSION.to_owned()),
        capabilities: vec![("access_service", uuid(ROLE))],
    });
    (access, subject)
}

/// The subject of [`EHR`], through its patient context.
fn subject_of_ehr() -> Caller {
    Caller {
        own_ehrs: vec![EhrId(uuid(EHR))],
        ..caller(CallerKind::Subject, GUARDIAN)
    }
}

/// The subject of another patient's EHR, through a patient context naming
/// that EHR.
fn subject_of_other_ehr() -> Caller {
    Caller {
        own_ehrs: vec![EhrId(uuid(OTHER_EHR))],
        ..caller(CallerKind::Subject, GUARDIAN)
    }
}

/// A representative of the subject of [`EHR`], the EHR its recorded
/// representation names.
fn representative_of_ehr() -> Caller {
    Caller {
        own_ehrs: vec![EhrId(uuid(EHR))],
        ..caller(CallerKind::Representative, GUARDIAN)
    }
}

/// A representative of the subject of another patient's EHR.
fn representative_of_other_ehr() -> Caller {
    Caller {
        own_ehrs: vec![EhrId(uuid(OTHER_EHR))],
        ..caller(CallerKind::Representative, GUARDIAN)
    }
}

/// The decision `access` records for the EHR with `settings`, after it
/// answered `item`.
fn decide_on(
    access: &RequestAccess,
    settings: Option<EhrAccessSettings>,
    item: &Item<'_>,
) -> (Outcome, Decision) {
    decide_on_subject(access, settings, item, None)
}

/// [`decide_on`] for an EHR whose `EHR_STATUS` names `subject`.
fn decide_on_subject(
    access: &RequestAccess,
    settings: Option<EhrAccessSettings>,
    item: &Item<'_>,
    subject: Option<EhrSubject>,
) -> (Outcome, Decision) {
    let ehr_id = EhrId(uuid(EHR));
    let version = settings.as_ref().map(|_| EHR_ACCESS_VERSION.to_owned());
    let entry = Arc::new(EhrEntry {
        settings: Arc::new(settings),
        subject,
        version,
        revision: ferroehr::storage::access_projection::Revision::default(),
    });
    // NOTE: an EHR with no subject reads no care relationship, so the judgement awaits nothing.
    let verdict = futures::executor::block_on(access.judge_ehr(ehr_id, &entry))
        .expect("an EHR with no subject reads no care relationship");
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

/// The capabilities the caller held and the roles it performs are `role`
/// grounds: each grant by its operation code and `ROLE`, each role by its
/// terminology and code.
#[test]
fn a_role_ground_names_each_capability_and_role() {
    let nurse = performing(CallerKind::Professional, NURSE, "NURSE");
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
    assert_eq!(roles[1].name.as_deref(), Some("urn:example:roles::NURSE"));
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
    let access = request(representative_of_ehr(), read(None), None);
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

/// An emergency justification lifting a restriction is an `emergency`
/// ground naming the directive it lifted, and the justification itself one
/// naming the `ACCESS_POLICY` version with the reason; the entry names both,
/// so the patient sees the access and why (EHDS Art. 11(5)).
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
    assert_eq!(
        emergencies[1].reason.as_deref(),
        Some("suspected sepsis, unconscious")
    );
    assert_eq!(
        emergencies[0].reason, None,
        "a lifted directive states none"
    );
    assert_eq!(decision.policies, [EHR_ACCESS_VERSION, POLICY_VERSION]);

    let event = event_of(&access);
    let justification = event.justification.as_ref().expect("a justified access");
    assert!(justification.emergency);
    let entry = entry_of(
        &event,
        None,
        "cdr.example.org",
        stated(None, &LogRetention::default()),
    )
    .expect("an entry");
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

/// An emergency lifts a consent deny over the mental-health template; a read
/// of another template names no `emergency` ground of that directive, only
/// the `ACCESS_POLICY` of the lifted steps 6 to 8, and a read of the
/// mental-health note names it (EHDS Art. 11(5) and Art. 9(1), #5630).
#[test]
fn the_emergency_grounds_name_only_the_directives_covering_the_data_read() {
    let deny = directive(
        "consent",
        "deny",
        &json!({ "emergency_override": true,
                 "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": ["Mental health note"] }] }),
    );
    let lifted = |template: &str| {
        let access = request(professional(DR_B), break_glass(read(Some("ETREAT"))), None);
        let item = object(OBJECT, Some(template));
        let (outcome, decision) = decide_on(
            &access,
            Some(settings("open", std::slice::from_ref(&deny), &[], &[])),
            &item,
        );
        assert_eq!(outcome, Outcome::Admit, "{template}");
        of_kind(&decision, "emergency")
            .iter()
            .map(|ground| ground.name.clone().unwrap_or_default())
            .collect::<Vec<_>>()
    };
    assert_eq!(lifted("HIV result"), ["ACCESS_POLICY"]);
    assert_eq!(
        lifted("Mental health note"),
        ["EHR_ACCESS", "ACCESS_POLICY"]
    );
}

/// An emergency lifts a controller deny rule over the mental-health
/// template; a read of another template names no `emergency` ground of that
/// rule, and a read of the mental-health note names it by its uid, else its
/// position (EHDS Art. 11(5) and Art. 9(1), #5676).
#[test]
fn the_emergency_grounds_name_only_the_rules_covering_the_data_read() {
    let deny = |uid: Option<&str>| {
        let mut rule = rule(
            "deny",
            &json!({ "applies_to": [role_scope("NURSE")], "emergency_override": true,
                     "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": ["Mental health note"] }] }),
        );
        if let Some(uid) = uid {
            rule["uid"] = json!({ "_type": "HIER_OBJECT_ID", "value": uid });
        }
        rule
    };
    let lifted = |rule: &Value, template: &str| {
        let access = request(
            performing(CallerKind::Professional, NURSE, "NURSE"),
            break_glass(read(Some("ETREAT"))),
            Some(policy(&json!({ "rules": [rule] }))),
        );
        let item = object(OBJECT, Some(template));
        let (outcome, decision) = decide_on(&access, None, &item);
        assert_eq!(outcome, Outcome::Admit, "{template}");
        // NOTE: the justification's own ground states the reason; the lifted rules state none.
        of_kind(&decision, "emergency")
            .iter()
            .filter(|ground| ground.reason.is_none())
            .filter_map(|ground| ground.reference.as_ref())
            .map(|reference| (reference.r#type.clone(), reference.id.clone()))
            .collect::<Vec<_>>()
    };
    let by_position = deny(None);
    assert_eq!(lifted(&by_position, "HIV result"), []);
    assert_eq!(
        lifted(&by_position, "Mental health note"),
        [(Some("position".to_owned()), "1".to_owned())]
    );
    let by_uid = deny(Some(RULE_UID));
    assert_eq!(lifted(&by_uid, "HIV result"), []);
    assert_eq!(
        lifted(&by_uid, "Mental health note"),
        [(Some("uid".to_owned()), RULE_UID.to_owned())]
    );
}

/// `entry` as the store keeps it.
fn linked(entry: AccessLogEntry) -> AccessLogEntry {
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
    let justified = policy(&json!({ "care_relationship_required": true,
        "justifications": [{ "_type": "ACCESS_JUSTIFICATION", "emergency": false,
            "purposes": [coded(PURPOSE, "TREAT")] }] }));
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
            Refusal::Confidentiality,
            professional(DR_B),
            None,
            None,
            steps_confidentiality::marked(Some("R"), &[], &[]),
            Item::Ehr,
            "the marking R of EHR_ACCESS",
        ),
        (
            Refusal::NoCareRelationship,
            professional(DR_B),
            None,
            Some(care.clone()),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "ACCESS_POLICY",
        ),
        (
            Refusal::OutsideEpisode,
            professional(DR_B),
            None,
            Some(care),
            settings("open", &[], &[], &[]),
            placed(OBJECT, None, &DERMATOLOGY_EPISODE[..]),
            "ACCESS_POLICY",
        ),
        (
            Refusal::PurposeMissing,
            professional(DR_B),
            None,
            Some(purpose),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "ACCESS_POLICY",
        ),
        (
            Refusal::ReasonMissing,
            professional(DR_B),
            Some("TREAT"),
            Some(justified),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "ACCESS_POLICY",
        ),
        (
            Refusal::PolicyDenied,
            performing(CallerKind::Professional, DR_B, "NURSE"),
            None,
            Some(policy(&json!({ "rules": [
                rule("permit", &json!({})),
                rule("deny", &json!({ "applies_to": [role_scope("NURSE")] }))] }))),
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "rule 2 of ACCESS_POLICY",
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
        (
            Refusal::NotRepresented,
            representative_of_other_ehr(),
            None,
            None,
            settings(
                "open",
                &[directive("consent", "permit", &json!({}))],
                &[],
                &[],
            ),
            Item::Ehr,
            "the EHR of the subject the caller represents",
        ),
        (
            Refusal::ConsentReferenceMissing,
            Caller {
                lacks_consent_reference: true,
                ..caller(CallerKind::AccessService, DR_B)
            },
            None,
            None,
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "the consent condition of the caller's agreement",
        ),
        (
            Refusal::Excluded,
            professional(DR_B),
            None,
            None,
            excluding(
                &[exclusion(
                    &json!({ "party": party("PERSON", DR_B) }),
                    &json!({}),
                )],
                &[],
                &[],
            ),
            Item::Ehr,
            "exclusion 1 of EHR_ACCESS",
        ),
        (
            Refusal::Conflict,
            Caller {
                facts: CallerFacts::new(vec![code(ROLES, "NURSE")], Vec::new()),
                ..professional(DR_B)
            },
            None,
            None,
            settings("open", &[], &[], &[]),
            Item::Ehr,
            "the conflict rules of ACCESS_POLICY",
        ),
    ];
    // NOTE: the conflict row's subject is Dr B's own PERSON, which grants resolve in this
    // CDR's namespace with no read, so its lazy pool never connects.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let _context = runtime.enter();
    assert_eq!(rows.len(), Refusal::ALL.len(), "a row per refusal code");
    for (refusal, who, declared, policy, settings, item, by) in rows {
        let code = refusal.code();
        let (access, subject) = if refusal == Refusal::Conflict {
            bound(who, read(declared), policy)
        } else {
            (request(who, read(declared), policy), None)
        };
        let (outcome, decision) = if refusal == Refusal::OutsideEpisode {
            let ehr_id = EhrId(uuid(EHR));
            let cared = Cared::of(vec![tie_in(Some("C-17"))]);
            let verdict = access.judge(Some(ehr_id), Some(&settings), &cared);
            (
                access.note(ehr_id, &verdict, &item),
                access.decision(Some(ehr_id)),
            )
        } else {
            decide_on_subject(&access, Some(settings), &item, subject)
        };
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
        let entry = entry_of(
            &event,
            None,
            "cdr.example.org",
            stated(None, &LogRetention::default()),
        )
        .expect("an entry");
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

/// A refusal by a controller rule names the rule as a legal obligation
/// ground carrying its legal grounds, and the request answers them for the
/// problem body; a refusal by another step names none (#5691).
#[test]
fn a_rule_refusal_carries_the_legal_grounds_of_the_rule() {
    let ground = coded("urn:example:law", "exchange-1");
    let rules = policy(&json!({ "rules": [
        rule("deny", &json!({ "applies_to": [role_scope("NURSE")],
                              "legal_grounds": [ground] }))] }));
    let access = request(
        performing(CallerKind::Professional, DR_B, "NURSE"),
        read(None),
        Some(rules),
    );
    let (outcome, decision) = decide_on(&access, Some(settings("open", &[], &[], &[])), &Item::Ehr);
    assert_eq!(outcome, Outcome::Refused(Refusal::PolicyDenied));
    let obligations = of_kind(&decision, "legal obligation");
    assert_eq!(obligations.len(), 1, "{decision:?}");
    let named: Vec<(&str, &str)> = obligations[0]
        .legal_grounds
        .iter()
        .flatten()
        .map(|g| {
            (
                g.defining_code.terminology_id.value.as_str(),
                g.defining_code.code_string.as_str(),
            )
        })
        .collect();
    assert_eq!(named, [("urn:example:law", "exchange-1")]);
    let answered = access.refusal_grounds(Some(EhrId(uuid(EHR))));
    assert_eq!(answered.len(), 1);
    assert_eq!(answered[0].defining_code.code_string, "exchange-1");

    let other = request(professional(DR_B), read(None), None);
    let deny = directive(
        "consent",
        "deny",
        &json!({ "legal_grounds": [coded("urn:example:law", "x")] }),
    );
    let (_, decision) = decide_on(
        &other,
        Some(settings("open", std::slice::from_ref(&deny), &[], &[])),
        &Item::Ehr,
    );
    assert!(
        of_kind(&decision, "legal obligation").is_empty(),
        "{decision:?}"
    );
    assert_eq!(other.refusal_grounds(Some(EhrId(uuid(EHR)))).len(), 0);
}
