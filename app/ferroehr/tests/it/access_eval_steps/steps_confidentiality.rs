// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step 6, confidentiality: the record's marking, a label's and a
//! classification's level, the order `N` < `R` < `V`, and the levels a
//! policy conceals (draft access control, The Decision Procedure, step 6
//! and Refusals; the worked example of the appendix, Who Sees What).

use openehr_rm::draft::access_control::terminology::{CallerKind, Refusal};
use serde_json::{Value, json};

use super::*;

/// The code system of the confidentiality levels.
const LEVELS: &str = "HL7_Confidentiality";

/// A `CONFIDENTIALITY_MARKING` of `level`.
fn marking(level: &str) -> Value {
    json!({ "_type": "CONFIDENTIALITY_MARKING", "confidentiality": coded(LEVELS, level),
            "set_by": { "_type": "PARTY_SELF" },
            "set_at": { "_type": "DV_DATE_TIME", "value": "2026-11-19T10:00:00Z" },
            "reason": { "_type": "DV_TEXT", "value": "public figure" } })
}

/// Open settings whose marking is `level`, with `directives` and `labels`.
pub(super) fn marked(
    level: Option<&str>,
    directives: &[Value],
    labels: &[Value],
) -> EhrAccessSettings {
    let mut s = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("open") });
    if let Some(level) = level {
        s["marking"] = marking(level);
    }
    if !directives.is_empty() {
        s["directives"] = json!(directives);
    }
    if !labels.is_empty() {
        s["labels"] = json!(labels);
    }
    ferroehr::service::ehr::access_settings::check_written_settings(&s)
        .expect("the test settings hold every invariant");
    EhrAccessSettings::from_ehr_access(&json!({ "_type": "EHR_ACCESS", "settings": s }))
        .expect("the settings read")
        .expect("settings")
}

/// The four rules of the worked example, with role scopes in place of care
/// roles: a care relationship reads `R` for treatment, the responsible role
/// reads `V` for treatment, with `emergency_override` on the `V` rule, and
/// `concealed` levels answering as absent.
fn example_policy(concealed: &[&str]) -> AccessPolicy {
    let restricted = rule(
        "permit",
        &json!({ "applies_to": [{ "relationship": term("care") }],
                 "purposes": [coded(PURPOSE, "TREAT")],
                 "data": [{ "confidentialities": [coded(LEVELS, "R")] }] }),
    );
    let very = rule(
        "permit",
        &json!({ "applies_to": [role_scope("RESPONSIBLE")],
                 "purposes": [coded(PURPOSE, "TREAT")],
                 "data": [{ "confidentialities": [coded(LEVELS, "V")] }] }),
    );
    let mut members = json!({ "rules": [restricted, very] });
    if !concealed.is_empty() {
        members["concealed_confidentialities"] = json!(
            concealed
                .iter()
                .map(|level| coded(LEVELS, level))
                .collect::<Vec<_>>()
        );
    }
    policy(&members)
}

const REFUSED: Outcome = Outcome::Refused(Refusal::Confidentiality);
const CONCEALED: Outcome = Outcome::Absent(Refusal::Confidentiality);

/// One row of the worked example: who, in care or not, the purpose, and the
/// answer at `N`, `R` and `V`.
type Row = (
    &'static str,
    Caller,
    bool,
    Option<&'static str>,
    [Outcome; 3],
);

/// The record at `N`, `R` and `V`, judged for one caller per row, on the
/// EHR itself and on one of its objects alike.
#[test]
fn step_6_confidentiality_who_sees_what() {
    let responsible = || performing(CallerKind::Professional, DR_B, "RESPONSIBLE");
    let rows: Vec<Row> = vec![
        (
            "a treating professional in care",
            professional(NURSE),
            true,
            Some("TREAT"),
            [Outcome::Admit, Outcome::Admit, REFUSED],
        ),
        (
            "the responsible role, whose V permit covers R",
            responsible(),
            false,
            Some("TREAT"),
            [Outcome::Admit; 3],
        ),
        (
            "in care, no purpose",
            professional(NURSE),
            true,
            None,
            [Outcome::Admit, REFUSED, REFUSED],
        ),
        (
            "no relationship",
            professional(DR_C),
            false,
            Some("TREAT"),
            [Outcome::Admit, REFUSED, REFUSED],
        ),
        (
            "the subject",
            caller(CallerKind::Subject, GUARDIAN),
            false,
            None,
            [Outcome::Admit; 3],
        ),
    ];
    let item = object(OBJECT, Some("vitals.v1"));
    for (name, who, cared, purpose, outcomes) in rows {
        for (level, expected) in [None, Some("R"), Some("V")].into_iter().zip(outcomes) {
            let case = Case {
                policy: Some(example_policy(&[])),
                cared,
                ..Case::of(marked(level, &[], &[]))
            };
            let verdict = case.verdict(&who, read(purpose));
            let at = format!("{name} at {level:?}");
            assert_outcome(&verdict, &Item::Ehr, expected, &at);
            assert_outcome(&verdict, &item, expected, &at);
        }
    }
}

/// With no rule naming a confidentiality, a marked record is read by its
/// subject alone, and a declared emergency lifts the step.
#[test]
fn a_marked_record_is_read_by_its_subject_alone_and_an_emergency_lifts_it() {
    let case = Case::of(marked(Some("R"), &[], &[]));
    assert_outcome(
        &case.verdict(&professional(NURSE), read(None)),
        &Item::Ehr,
        REFUSED,
        "a professional",
    );
    assert_outcome(
        &case.verdict(&caller(CallerKind::Subject, GUARDIAN), read(None)),
        &Item::Ehr,
        Outcome::Admit,
        "the subject",
    );
    assert_outcome(
        &case.verdict(&professional(NURSE), break_glass(read(Some("ETREAT")))),
        &Item::Ehr,
        Outcome::Admit,
        "an emergency",
    );
}

/// A level the policy conceals answers as absent, and the other level stays
/// forbidden.
#[test]
fn a_concealed_level_answers_as_absent() {
    for (level, expected) in [("R", REFUSED), ("V", CONCEALED)] {
        let case = Case {
            policy: Some(example_policy(&["V"])),
            ..Case::of(marked(Some(level), &[], &[]))
        };
        assert_outcome(
            &case.verdict(&professional(DR_C), read(Some("TREAT"))),
            &Item::Ehr,
            expected,
            level,
        );
    }
}

/// A label marks one object and a classification one template; a permit
/// directive of the subject naming the level clears them, as a permit rule
/// does.
#[test]
fn a_label_and_a_classification_mark_their_data() {
    let label = json!({ "_type": "ACCESS_LABEL", "target": object_ref(OBJECT),
                        "confidentiality": marking("V") });
    let classified = policy(&json!({ "classifications": [{
        "_type": "ACCESS_CLASSIFICATION",
        "name": { "_type": "DV_TEXT", "value": "HIV results" },
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["lab_hiv.v1"] },
        "confidentiality": coded(LEVELS, "R") }] }));
    let naming = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }],
                 "data": [{ "confidentialities": [coded(LEVELS, "V")] }] }),
    );
    let labelled = object(OBJECT, Some("vitals.v1"));
    let lab = object(OTHER, Some("lab_hiv.v1"));
    let plain = object(OTHER, Some("vitals.v1"));
    let rows: Vec<(&str, EhrAccessSettings, [Outcome; 4])> = vec![
        (
            "no permit",
            marked(None, &[], std::slice::from_ref(&label)),
            [Outcome::Admit, REFUSED, REFUSED, Outcome::Admit],
        ),
        (
            "a permit of the subject naming V",
            marked(None, &[naming], &[label]),
            [Outcome::Admit; 4],
        ),
    ];
    for (name, settings, [on_ehr, on_labelled, on_lab, on_plain]) in rows {
        let case = Case {
            policy: Some(classified.clone()),
            ..Case::of(settings)
        };
        let verdict = case.verdict(&professional(NURSE), read(None));
        assert_outcome(&verdict, &Item::Ehr, on_ehr, name);
        assert_outcome(&verdict, &labelled, on_labelled, name);
        assert_outcome(&verdict, &lab, on_lab, name);
        assert_outcome(&verdict, &plain, on_plain, name);
    }
}
