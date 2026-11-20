// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care roles and the care path in a rule's actor scope, the care purposes,
//! and the relationship the access log names, over the evaluator itself
//! (#6075).
//!
//! The worked example is the draft package's Who Sees What table (draft
//! access control package, Appendix, Who Sees What; #6060 B.4.3): one patient
//! under the care of the hospital H, a policy with a 30-day grace and four
//! rules, each caller acting for H. Until the record's confidentiality is
//! decided (#6057) the levels `R` and `V` stand as two sensitivities the
//! policy classifies by template, so a datum of a level no applying permit
//! names is refused at step 6 as `refused-sensitivity`, the refusal
//! `refused-confidentiality` takes there.

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::care::CarePath;
use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass, Refusal};
use serde_json::{Value, json};

use super::*;
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess};
use ferroehr::access_eval::procedure::{Emergency, Lifted};
use ferroehr::party_binding::grants::caller::care_of::{CarerKind, Cover};

/// Dr. Jansen, the responsible practitioner.
const JANSEN: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa1";
/// Dr. Bakker, second in chain.
const BAKKER: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa2";
/// Dr. Visser, a consultant for 14 days.
const VISSER: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa3";
/// Nurse De Vries, a member of cardiology.
const DE_VRIES: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa4";
/// Dr. Meijer, covering for Dr. Jansen from 21 to 23 November.
const MEIJER: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa5";
/// Dr. Smit, with no relationship.
const SMIT: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7fa6";

/// The code system the stand-in levels are coded in.
const LEVELS: &str = "urn:example:levels";
/// The template whose data stand at the level `R`.
const AT_R: &str = "restricted_note.v1";
/// The template whose data stand at the level `V`.
const AT_V: &str = "very_restricted_note.v1";
/// The discharge summary template.
const DISCHARGE: &str = "discharge_summary.v1";

/// A care role of `HL7_ParticipationType`.
fn care_role(code: &str) -> Value {
    coded("HL7_ParticipationType", code)
}

/// An actor scope naming a care relationship with `extra` members.
fn in_care(extra: &Value) -> Value {
    let mut scope = json!({ "relationship": term("care") });
    if let (Some(scope), Some(extra)) = (scope.as_object_mut(), extra.as_object()) {
        scope.extend(extra.clone());
    }
    scope
}

/// The four rules of the worked example, with a 30-day grace, the stand-in
/// levels classified by template, and `members` beside them.
fn hospital(members: &Value) -> AccessPolicy {
    let level = |template: &str, code: &str| {
        json!({ "_type": "ACCESS_CLASSIFICATION",
            "name": { "_type": "DV_TEXT", "value": code }, "archetype_node_id": "at0002",
            "data": { "_type": "ACCESS_DATA_SCOPE", "templates": [template] },
            "sensitivities": [coded(LEVELS, code)] })
    };
    let responsible = in_care(
        &json!({ "care_roles": [care_role("RESP"), care_role("ATND")],
        "care_path": term("direct") }),
    );
    let mut p = json!({
        "care_relationship_required": true,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P30D" },
        "classifications": [level(AT_R, "R"), level(AT_V, "V")],
        "rules": [
            rule("permit", &json!({ "applies_to": [in_care(&json!({}))],
                "data": [{ "sensitivities": [coded(LEVELS, "R")] }],
                "purposes": [coded(PURPOSE, "TREAT")] })),
            rule("permit", &json!({ "applies_to": [responsible],
                "data": [{ "sensitivities": [coded(LEVELS, "V")] }],
                "purposes": [coded(PURPOSE, "TREAT")], "emergency_override": true })),
            rule("deny", &json!({ "operations": [term("write")],
                "data": [{ "templates": [DISCHARGE] }],
                "exceptions": [directive("controller_rule", "permit", &json!({
                    "applies_to": [in_care(&json!({ "care_roles": [care_role("RESP")],
                        "care_path": term("direct") }))] }))] }))
        ]
    });
    if let (Some(p), Some(members)) = (p.as_object_mut(), members.as_object()) {
        p.extend(members.clone());
    }
    policy(&p)
}

/// The relationships of the patient, each in the care of the hospital, the
/// upper date the first one it no longer holds.
fn relationships() -> Vec<CareOf> {
    let held = |carer: &str, role: &str, from: &str, until: Option<&str>| {
        CareOf::spanning(
            uuid(carer),
            (
                Some(from.parse().expect("a date")),
                until.map(|until| until.parse().expect("a date")),
            ),
            code("HL7_ParticipationType", role),
            provider(),
        )
    };
    vec![
        held(JANSEN, "RESP", "2026-11-01", None),
        held(BAKKER, "SPRF", "2026-11-10", None),
        held(VISSER, "CON", "2026-11-19", Some("2026-12-04")),
        held(CARDIOLOGY, "PART", "2026-11-18", None),
        held(MEIJER, "RESP", "2026-11-21", Some("2026-11-24")).stored_as(
            (uuid(OBJECT), Some(format!("{OBJECT}::cdr.example.org::1"))),
            CarerKind::Person,
            Some(Cover::new(
                ProviderKey::new("urn:example:staff", "jansen"),
                CarerKind::Person,
            )),
        ),
    ]
}

/// A professional bound to the `PERSON` `person`, acting for `acting_for`,
/// a member of each of `teams` from 1 November 2026.
fn clinician(person: &str, acting_for: ProviderKey, teams: &[&str]) -> Caller {
    let mut caller = professional(person);
    let teams = teams
        .iter()
        .map(|team| care_team(team, "2026-11-01", None))
        .collect();
    caller.facts = CallerFacts::new(Vec::new(), teams)
        .with_party(uuid(person))
        .with_providers([acting_for]);
    caller.acts_for = caller
        .facts
        .care_teams()
        .iter()
        .map(|team| team.team().to_string())
        .collect();
    caller
}

/// The evaluator of `operation` by `caller` for `purpose` on `day`, under
/// `policy`, with the patient's relationships judged.
fn judged(
    caller: &Caller,
    asked: (OperationClass, Option<&str>, Emergency),
    policy: &AccessPolicy,
    day: &str,
) -> Verdict {
    let (operation, purpose, emergency) = asked;
    let access = RequestAccess::new(
        caller.clone(),
        Asking {
            operation,
            purpose: purpose.map(str::to_owned),
            emergency,
        },
        InForce {
            policy: Some(Arc::new(policy.clone())),
            categories: categories(),
            server_default: AccessDefault::Open,
            now: format!("{day}T12:00:00Z").parse().expect("an instant"),
            grants: None,
        },
    );
    access.judge(None, None, &access.cared(&relationships()))
}

/// A read for `purpose`.
fn reading(purpose: &str) -> (OperationClass, Option<&str>, Emergency) {
    (OperationClass::Read, Some(purpose), Emergency::default())
}

/// The datum of `template`, committed on 12 November 2026.
fn datum(template: &str) -> Item<'_> {
    object(OBJECT, Some(template))
}

const ADMIT: Outcome = Outcome::Admit;
const NO_CARE: Outcome = Outcome::Refused(Refusal::NoCareRelationship);
const LEVEL: Outcome = Outcome::Refused(Refusal::Sensitivity);

/// The Who Sees What table of the draft, with the provider: each caller's
/// answer at `N`, `R` and `V` on a day every relationship holds.
#[test]
fn the_who_sees_what_table_holds_with_the_provider() {
    let hospital_policy = hospital(&json!({}));
    let h = provider;
    let rows: [(&str, Caller, [Outcome; 3]); 7] = [
        (
            "Dr. Jansen, RESP direct",
            clinician(JANSEN, h(), &[]),
            [ADMIT, ADMIT, ADMIT],
        ),
        (
            "Dr. Bakker, SPRF direct",
            clinician(BAKKER, h(), &[]),
            [ADMIT, ADMIT, LEVEL],
        ),
        (
            "Dr. Visser, CON direct",
            clinician(VISSER, h(), &[]),
            [ADMIT, ADMIT, LEVEL],
        ),
        (
            "Nurse De Vries, PART by the team",
            clinician(DE_VRIES, h(), &[CARDIOLOGY]),
            [ADMIT, ADMIT, LEVEL],
        ),
        (
            "Dr. Meijer, covering",
            clinician(MEIJER, h(), &[]),
            [ADMIT, ADMIT, ADMIT],
        ),
        (
            "Dr. Smit, no relationship",
            clinician(SMIT, h(), &[]),
            [NO_CARE, LEVEL, LEVEL],
        ),
        (
            "Dr. Jansen, acting for another provider",
            clinician(
                JANSEN,
                ProviderKey::new("urn:example:providers", "clinic"),
                &[],
            ),
            [NO_CARE, LEVEL, LEVEL],
        ),
    ];
    for (name, caller, expected) in rows {
        let verdict = judged(&caller, reading("TREAT"), &hospital_policy, "2026-11-22");
        for ((level, template), outcome) in [("N", "vitals.v1"), ("R", AT_R), ("V", AT_V)]
            .into_iter()
            .zip(expected)
        {
            assert_outcome(
                &verdict,
                &datum(template),
                outcome,
                &format!("{name} at {level}"),
            );
        }
    }
    let own = Case {
        policy: Some(hospital_policy),
        ..Case::default()
    }
    .verdict(&caller(CallerKind::Subject, SMIT), read(Some("TREAT")));
    for template in ["vitals.v1", AT_R, AT_V] {
        assert_outcome(&own, &datum(template), ADMIT, "the patient");
    }
}

/// A consultant, and the professional covering for the responsible one,
/// read after their relationship ends only within the grace; the record of
/// such an access marks it as admitted in grace.
#[test]
fn an_ended_relationship_admits_within_the_grace_alone() {
    let hospital_policy = hospital(&json!({}));
    for (name, carer, day, expected) in [
        ("Dr. Visser in the grace", VISSER, "2026-12-20", ADMIT),
        ("Dr. Visser after the grace", VISSER, "2027-01-10", NO_CARE),
        ("Dr. Meijer in the grace", MEIJER, "2026-12-20", ADMIT),
        ("Dr. Meijer after the grace", MEIJER, "2026-12-24", NO_CARE),
    ] {
        let verdict = judged(
            &clinician(carer, provider(), &[]),
            reading("TREAT"),
            &hospital_policy,
            day,
        );
        assert_outcome(&verdict, &datum("vitals.v1"), expected, name);
        if expected == ADMIT {
            let grant = verdict
                .grounds
                .granting
                .as_ref()
                .and_then(|tie| tie.grant.clone())
                .expect("the record names the relationship");
            assert!(grant.in_grace, "{name}: marked as admitted in grace");
        }
    }
    let covering = judged(
        &clinician(MEIJER, provider(), &[]),
        reading("TREAT"),
        &hospital_policy,
        "2026-11-22",
    );
    let grant = covering
        .grounds
        .granting
        .as_ref()
        .and_then(|tie| tie.grant.clone())
        .expect("the record names the relationship");
    assert_eq!(
        grant.covers_for,
        Some(Cover::new(
            ProviderKey::new("urn:example:staff", "jansen"),
            CarerKind::Person
        )),
        "the record names whom the relationship covers for, with its kind"
    );
    assert_eq!(grant.relationship, Some(uuid(OBJECT)));
    assert!(!grant.in_grace);
}

/// Only the responsible practitioner, directly, signs the discharge summary:
/// a deny rule whose permit exception names `RESP` and `direct` refuses the
/// second in chain, the team and the consultant.
#[test]
fn only_the_responsible_practitioner_directly_writes_the_discharge_summary() {
    let hospital_policy = hospital(&json!({}));
    let writing = (OperationClass::Write, Some("TREAT"), Emergency::default());
    let denied = Outcome::Refused(Refusal::PolicyDenied);
    for (name, caller, expected) in [
        ("Dr. Jansen", clinician(JANSEN, provider(), &[]), ADMIT),
        (
            "Dr. Meijer, covering",
            clinician(MEIJER, provider(), &[]),
            ADMIT,
        ),
        ("Dr. Bakker", clinician(BAKKER, provider(), &[]), denied),
        ("Dr. Visser", clinician(VISSER, provider(), &[]), denied),
        (
            "Nurse De Vries",
            clinician(DE_VRIES, provider(), &[CARDIOLOGY]),
            denied,
        ),
    ] {
        let verdict = judged(&caller, writing, &hospital_policy, "2026-11-22");
        assert_outcome(&verdict, &datum(DISCHARGE), expected, name);
        assert_outcome(
            &verdict,
            &datum("vitals.v1"),
            ADMIT,
            &format!("{name}, other data"),
        );
    }
}

/// A declared emergency lifts steps 6 to 8 and the rule that sets its
/// override, and creates no relationship: the caller with none reads at
/// every level, marked, and the record names no relationship.
#[test]
fn an_emergency_reads_without_a_relationship_and_names_none() {
    let hospital_policy = hospital(&json!({}));
    let emergency = (
        OperationClass::Read,
        Some("ETREAT"),
        Emergency {
            declared: true,
            break_glass: true,
            opt_out_override: false,
        },
    );
    let verdict = judged(
        &clinician(SMIT, provider(), &[]),
        emergency,
        &hospital_policy,
        "2026-11-22",
    );
    for template in ["vitals.v1", AT_R, AT_V] {
        assert_outcome(
            &verdict,
            &datum(template),
            ADMIT,
            "Dr. Smit in an emergency",
        );
    }
    assert!(
        verdict.lifted.contains(&Lifted::Policy),
        "the access is marked"
    );
    assert_eq!(verdict.grounds.granting, None);
}

/// A care relationship is a ground only for an access that declares no
/// purpose or one of `care_purposes`: a research purpose is admitted by no
/// relationship, and a purpose the policy leaves out of its list neither,
/// while void reads as `TREAT`, `ETREAT` and `CAREMGT` (draft decision
/// procedure, Terms; #6060 F11).
#[test]
fn an_access_under_a_purpose_outside_the_care_purposes_rests_on_no_relationship() {
    let jansen = clinician(JANSEN, provider(), &[]);
    let void = hospital(&json!({}));
    let listed = hospital(&json!({ "care_purposes": [coded(PURPOSE, "TREAT")] }));
    for (name, policy, purpose, expected) in [
        ("research, void", &void, Some("HRESCH"), NO_CARE),
        ("care management, void", &void, Some("CAREMGT"), ADMIT),
        ("no purpose, void", &void, None, ADMIT),
        (
            "care management, listed TREAT",
            &listed,
            Some("CAREMGT"),
            NO_CARE,
        ),
        ("treatment, listed TREAT", &listed, Some("TREAT"), ADMIT),
    ] {
        let verdict = judged(
            &jansen,
            (OperationClass::Read, purpose, Emergency::default()),
            policy,
            "2026-11-22",
        );
        assert_outcome(&verdict, &datum("vitals.v1"), expected, name);
        assert_eq!(
            verdict.grounds.granting.is_some(),
            expected == ADMIT,
            "{name}: the record names a relationship only where one admitted"
        );
    }
    let research = judged(&jansen, reading("HRESCH"), &void, "2026-11-22");
    assert_outcome(
        &research,
        &datum(AT_R),
        LEVEL,
        "research matches no scope naming care",
    );
}

/// Where several relationships admit an access the record names one: direct
/// before team, then by the order of the roles in the scope that matched
/// (draft decision procedure, Holding a Care Relationship).
#[test]
fn the_record_names_the_relationship_the_procedure_chooses() {
    let held = |carer: &str, role: &str| {
        CareOf::spanning(
            uuid(carer),
            (Some("2026-11-01".parse().expect("a date")), None),
            code("HL7_ParticipationType", role),
            provider(),
        )
    };
    let relations = [
        held(CARDIOLOGY, "RESP"),
        held(JANSEN, "RESP"),
        held(JANSEN, "ATND"),
    ];
    let named = |policy: &AccessPolicy| {
        let access = RequestAccess::new(
            clinician(JANSEN, provider(), &[CARDIOLOGY]),
            Asking {
                operation: OperationClass::Read,
                purpose: Some("TREAT".to_owned()),
                emergency: Emergency::default(),
            },
            InForce {
                policy: Some(Arc::new(policy.clone())),
                categories: categories(),
                server_default: AccessDefault::Open,
                now: "2026-11-22T12:00:00Z".parse().expect("an instant"),
                grants: None,
            },
        );
        let verdict = access.judge(None, None, &access.cared(&relations));
        verdict
            .grounds
            .granting
            .map(|tie| (tie.path, tie.role.code))
    };
    let ordered = |roles: &[&str], path: Option<&str>| {
        let mut scope =
            json!({ "care_roles": roles.iter().map(|r| care_role(r)).collect::<Vec<_>>() });
        if let Some(path) = path {
            scope["care_path"] = term(path);
        }
        policy(&json!({ "care_relationship_required": true,
            "rules": [rule("permit", &json!({ "applies_to": [in_care(&scope)],
                "data": [{ "sensitivities": [coded(LEVELS, "V")] }] }))] }))
    };
    assert_eq!(
        named(&ordered(&["ATND", "RESP"], None)),
        Some((CarePath::Direct, "ATND".to_owned())),
        "direct before team, then the rule's first role"
    );
    assert_eq!(
        named(&ordered(&["RESP", "ATND"], None)),
        Some((CarePath::Direct, "RESP".to_owned())),
        "the order of the roles in the rule decides"
    );
    assert_eq!(
        named(&ordered(&["RESP"], Some("team"))),
        Some((CarePath::Team, "RESP".to_owned())),
        "the relationship the rule's path names"
    );
    assert_eq!(
        named(&policy(&json!({ "care_relationship_required": true }))),
        Some((CarePath::Direct, "ATND".to_owned())),
        "with no rule naming roles, direct first"
    );
}
