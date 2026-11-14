// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decision procedure's tests: one table per step and per refusal code
//! (`steps`), and the worked examples of the design (`examples`).

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures are raw canonical JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod examples;
mod grounds;
mod steps;
mod steps_policy;

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, ForeignCode, OperationClass, Refusal,
};
use serde_json::{Value, json};
use uuid::Uuid;

use ferroehr::access_eval::procedure::{
    Asked, Caller, Ehr, Emergency, Judged, Outcome, Policy, Verdict, decide,
};
use ferroehr::access_eval::scope::{CategoryIndex, Item, Object, Scopes};
use ferroehr::party_binding::grants::caller::{CallerFacts, CareTeam};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::policy_register::CategoriesInForce;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::ehr::access_settings::EhrAccessSettings;
use ferroehr::system_log::categories::CategoryMapConfig;

/// Dr B, the ex-partner.
pub(super) const DR_B: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f81";
/// Dr C, the psychiatrist.
pub(super) const DR_C: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f82";
/// A nurse.
pub(super) const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f83";
/// A guardian representing the subject.
pub(super) const GUARDIAN: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f84";
/// The cardiology care team.
pub(super) const CARDIOLOGY: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f90";
/// The nephrology care team.
pub(super) const NEPHROLOGY: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f91";
/// A regional `GROUP` of organisations.
pub(super) const REGION: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f92";
/// A versioned object of the EHR.
pub(super) const OBJECT: &str = "8849182c-82ad-4088-a07f-48ead4180515";
/// Another versioned object of the EHR.
pub(super) const OTHER: &str = "8849182c-82ad-4088-a07f-48ead4180516";

/// The jurisdiction's sensitivity code system.
pub(super) const CONFIDENTIALITY: &str = "2.16.840.1.113883.5.25";
/// The jurisdiction's purpose of use code system.
pub(super) const PURPOSE: &str = "2.16.840.1.113883.5.8";
/// The national category system of the test map.
pub(super) const CATEGORIES: &str = "urn:example:categories";

/// An opt-out reads as absent.
pub(super) const ABSENT_OPT_OUT: Outcome = Outcome::Absent(Refusal::OptOut);
/// A restriction reads as absent.
pub(super) const ABSENT_RESTRICTED: Outcome = Outcome::Absent(Refusal::Restricted);
/// A consent deny refuses.
pub(super) const DENIED: Outcome = Outcome::Refused(Refusal::ConsentDenied);

/// The break-glass role declaring an emergency.
pub(super) fn break_glass(asked: Asked<'_>) -> Asked<'_> {
    Asked {
        emergency: Emergency {
            declared: true,
            break_glass: true,
            opt_out_override: false,
        },
        ..asked
    }
}

/// A term of the access control groups.
pub(super) fn term(code: &str) -> Value {
    coded(TERMINOLOGY, code)
}

/// A coded value of `terminology`.
pub(super) fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A `PARTY_REF` to the party `id` of `type`.
pub(super) fn party(r#type: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "local", "type": r#type,
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// An `OBJECT_REF` to the versioned composition `id`.
pub(super) fn object_ref(id: &str) -> Value {
    json!({ "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// A directive of `kind` and `effect` with `extra` members.
pub(super) fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
    let mut d = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Directive" },
        "archetype_node_id": "at0001",
        "kind": term(kind),
        "effect": term(effect),
        "emergency_override": false
    });
    if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
        d.extend(extra.clone());
    }
    d
}

/// A `DV_INTERVAL<DV_DATE_TIME>` from `lower` to `upper`, each optional.
pub(super) fn interval(lower: Option<&str>, upper: Option<&str>) -> Value {
    let mut i = json!({ "_type": "DV_INTERVAL",
        "lower_unbounded": lower.is_none(), "upper_unbounded": upper.is_none(),
        "lower_included": lower.is_some(), "upper_included": false });
    if let Some(lower) = lower {
        i["lower"] = json!({ "_type": "DV_DATE_TIME", "value": lower });
    }
    if let Some(upper) = upper {
        i["upper"] = json!({ "_type": "DV_DATE_TIME", "value": upper });
    }
    i
}

/// Settings of the default `default` holding `directives`, `labels` and
/// `gate_keepers`, read as the scheme (every invariant and code checked).
pub(super) fn settings(
    default: &str,
    directives: &[Value],
    labels: &[Value],
    gate_keepers: &[Value],
) -> EhrAccessSettings {
    let mut s = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term(default) });
    if !directives.is_empty() {
        s["directives"] = json!(directives);
    }
    if !labels.is_empty() {
        s["labels"] = json!(labels);
    }
    if !gate_keepers.is_empty() {
        s["gate_keepers"] = json!(gate_keepers);
    }
    ferroehr::service::ehr::access_settings::check_written_settings(&s)
        .expect("the test settings hold every invariant");
    EhrAccessSettings::from_ehr_access(&json!({ "_type": "EHR_ACCESS", "settings": s }))
        .expect("the settings read")
        .expect("settings")
}

/// An `ACCESS_POLICY` with `members`.
pub(super) fn policy(members: &Value) -> AccessPolicy {
    let mut p = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Policy" },
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": party("ORGANISATION", REGION),
        "care_relationship_required": false,
        "purpose_required": false
    });
    if let (Some(p), Some(members)) = (p.as_object_mut(), members.as_object()) {
        p.extend(members.clone());
    }
    openehr_its::json::from_canonical_value(&p).expect("the test policy reads")
}

/// A code of `terminology`.
pub(super) fn code(terminology: &str, code: &str) -> ForeignCode {
    ForeignCode {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    }
}

/// The category map of the tests: psychiatric notes are mental health, the
/// HIV laboratory template a laboratory report.
pub(super) fn categories() -> CategoryIndex {
    let map: CategoryMapConfig = toml::from_str(&format!(
        "national = [\"{CATEGORIES}|mental-health\"]\n\
         [templates]\n\
         \"psych_note.v1\" = [\"{CATEGORIES}|mental-health\"]\n\
         \"lab_hiv.v1\" = [\"Laboratory-Reports\"]\n"
    ))
    .expect("the test map parses");
    let classifier = map.compile().expect("the test map compiles");
    CategoryIndex::new(Arc::new(CategoriesInForce {
        version: 1,
        map,
        classifier,
    }))
}

/// A professional bound to `party`, holding nothing.
pub(super) fn professional(party: &str) -> Caller {
    caller(CallerKind::Professional, party)
}

/// A caller of `kind` bound to `party`, holding nothing.
pub(super) fn caller(kind: CallerKind, party: &str) -> Caller {
    Caller {
        kind,
        party: Some(party.to_owned()),
        own_ehr: None,
        acts_for: Vec::new(),
        roles: Vec::new(),
        facts: CallerFacts::default(),
        administrator: false,
        settings: SettingsCaller::new(false, kind == CallerKind::Subject, Some(party.to_owned())),
    }
}

/// `caller` holding `clearances`, `purposes` and member of `teams`, each
/// relationship from 1 November 2026 with no end.
pub(super) fn with_facts(
    mut caller: Caller,
    clearances: &[ForeignCode],
    purposes: &[ForeignCode],
    teams: &[&str],
) -> Caller {
    let teams = teams
        .iter()
        .map(|team| care_team(team, "2026-11-01", None))
        .collect();
    caller.facts = CallerFacts::new(clearances.to_vec(), purposes.to_vec(), teams);
    caller.acts_for = caller
        .facts
        .care_teams()
        .iter()
        .map(|team| team.team().to_string())
        .collect();
    caller
}

/// A care relationship to `team` over the dates `[lower, upper)`.
pub(super) fn care_team(team: &str, lower: &str, upper: Option<&str>) -> CareTeam {
    CareTeam::spanning(
        uuid(team),
        Some(lower.parse().expect("a date")),
        upper.map(|upper| upper.parse().expect("a date")),
    )
}

/// `text` as a UUID.
pub(super) fn uuid(text: &str) -> Uuid {
    text.parse().expect("a UUID")
}

/// A read of `purpose` with no emergency.
pub(super) fn read(purpose: Option<&str>) -> Asked<'_> {
    asked(OperationClass::Read, purpose)
}

/// An `operation` of `purpose` with no emergency.
pub(super) fn asked(operation: OperationClass, purpose: Option<&str>) -> Asked<'_> {
    Asked {
        operation,
        purpose,
        emergency: Emergency::default(),
    }
}

/// The moment of the tests: 13 November 2026, noon UTC.
pub(super) fn now() -> jiff::Timestamp {
    "2026-11-13T12:00:00Z".parse().expect("an instant")
}

/// One decision's inputs beside the caller and the ask.
#[derive(Default)]
pub(super) struct Case {
    pub(super) settings: Option<EhrAccessSettings>,
    pub(super) server_default: Option<AccessDefault>,
    pub(super) policy: Option<AccessPolicy>,
    pub(super) cared: bool,
    pub(super) now: Option<jiff::Timestamp>,
    /// The EHR is not the subject caller's own.
    pub(super) foreign: bool,
}

impl Case {
    /// A case of `settings`.
    pub(super) fn of(settings: EhrAccessSettings) -> Self {
        Self {
            settings: Some(settings),
            ..Self::default()
        }
    }

    /// The verdict for `caller` asking `asked`.
    pub(super) fn verdict(&self, caller: &Caller, asked: Asked<'_>) -> Verdict {
        let labels = match &self.settings {
            Some(EhrAccessSettings::Directives(s)) => s.labels.as_deref().unwrap_or_default(),
            _ => &[],
        };
        let classifications = self
            .policy
            .as_ref()
            .and_then(|p| p.classifications.as_deref())
            .unwrap_or_default();
        let scopes = Scopes::new(categories(), labels, classifications);
        decide(&Judged {
            caller,
            asked,
            policy: Policy::of(self.policy.as_ref()),
            ehr: Ehr {
                settings: self.settings.as_ref(),
                server_default: self.server_default.unwrap_or(AccessDefault::Open),
                cared: self.cared,
                own: !self.foreign,
            },
            scopes: &scopes,
            now: self.now.unwrap_or_else(now),
        })
    }
}

/// The object `id` of `template`, committed on 12 November 2026.
pub(super) fn object<'a>(id: &str, template: Option<&'a str>) -> Item<'a> {
    Item::Object(Object {
        vo_id: uuid(id),
        template,
        archetype: None,
        origins: Some(&[]),
        recorded: Some("2026-11-12T08:00:00Z".parse().expect("an instant")),
    })
}

/// Checks that `verdict` answers `expected` for `item`, and that its admitted
/// predicate, which a query renders as SQL, agrees.
pub(super) fn assert_outcome(verdict: &Verdict, item: &Item<'_>, expected: Outcome, case: &str) {
    assert_eq!(verdict.outcome(item), expected, "{case}: {verdict:?}");
    assert_eq!(
        verdict.admitted().holds(item),
        expected == Outcome::Admit,
        "{case}: the admitted predicate agrees"
    );
}
