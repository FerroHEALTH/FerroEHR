// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decision procedure's tests: one table per step and per refusal code
//! (`steps`), and the worked examples of the design (`examples`).

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures are raw canonical JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod care_roles;
mod examples;
mod grounds;
mod steps;
mod steps_confidentiality;
mod steps_episodes;
mod steps_exclusion;
mod steps_justification;
mod steps_policy;
mod steps_purpose;
mod steps_rules;

use std::sync::{Arc, LazyLock};

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::care::CarePath;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, Confidentiality, ForeignCode, OperationClass, Refusal,
};
use serde_json::{Value, json};
use uuid::Uuid;

use ferroehr::access_eval::care_tie::{Cared, Tie};
use ferroehr::access_eval::class::{Class, classify, resolve_verdict};
use ferroehr::access_eval::condition::CallerKeys;
use ferroehr::access_eval::procedure::{
    Asked, Caller, Ehr, Judged, Lifted, Outcome, Policy, Verdict, decide, rules_of,
};
use ferroehr::access_eval::scope::{CategoryIndex, Item, Object, Scopes};
use ferroehr::access_policy::care::CareFacts;
use ferroehr::party_binding::grants::caller::CareTeam;
use ferroehr::party_binding::grants::caller::care_of::{CareOf, ProviderKey};
use ferroehr::party_binding::grants::caller::facts::CallerFacts;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::policy_register::CategoriesInForce;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::ehr::access_settings::EhrAccessSettings;
use ferroehr::system_log::categories::CategoryMapConfig;
use ferroehr::versioning::episode::EpisodeKey;

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
pub(super) const CONFIDENTIALITY: &str = "HL7_ActCode";
/// The jurisdiction's purpose of use code system.
pub(super) const PURPOSE: &str = "HL7_ActReason";
/// The jurisdiction's role code system.
pub(super) const ROLES: &str = "urn:example:roles";
/// The national category system of the test map.
pub(super) const CATEGORIES: &str = "urn:example:categories";

/// An opt-out reads as absent.
pub(super) const ABSENT_OPT_OUT: Outcome = Outcome::Absent(Refusal::OptOut);
/// A restriction reads as absent.
pub(super) const ABSENT_RESTRICTED: Outcome = Outcome::Absent(Refusal::Restricted);
/// A consent deny refuses.
pub(super) const DENIED: Outcome = Outcome::Refused(Refusal::ConsentDenied);

/// The purpose of use a [`break_glass`] ask declares when it names none.
pub(super) const EMERGENCY: &str = "ETREAT";

/// `asked` invoking the tests' emergency justification with a reason: the
/// case's policy then holds an emergency justification for its purpose
/// ([`in_force`]).
pub(super) fn break_glass(asked: Asked<'_>) -> Asked<'_> {
    Asked {
        purpose: Some(asked.purpose.unwrap_or(EMERGENCY)),
        reasoned: true,
        ..asked
    }
}

/// The reason a [`break_glass`] request states.
pub(super) fn reason_of(asked: Asked<'_>) -> Option<ferroehr::access_eval::justification::Reason> {
    asked.reasoned.then(|| {
        ferroehr::access_eval::justification::Reason::new("suspected sepsis, unconscious")
            .expect("a reason")
    })
}

/// `policy` as a [`break_glass`] ask meets it: with an emergency
/// justification of every professional and access service for the purpose
/// `asked` declares, unless the policy states its own justifications.
pub(super) fn in_force(policy: Option<&AccessPolicy>, asked: Asked<'_>) -> Option<AccessPolicy> {
    let purpose = asked.purpose.filter(|_| asked.reasoned);
    let Some(purpose) = purpose else {
        return policy.cloned();
    };
    if policy.is_some_and(|p| p.justifications.is_some()) {
        return policy.cloned();
    }
    let mut value = policy.map_or_else(
        || json!({}),
        |p| serde_json::to_value(p).expect("the policy writes"),
    );
    value["justifications"] = json!([{ "_type": "ACCESS_JUSTIFICATION",
        "purposes": [coded(PURPOSE, purpose)], "emergency": true,
        "applies_to": [
            { "_type": "ACCESS_ACTOR_SCOPE", "caller_kind": term("professional") },
            { "_type": "ACCESS_ACTOR_SCOPE", "caller_kind": term("access_service") }
        ] }]);
    Some(self::policy(&value))
}

/// A term of the access control groups.
pub(super) fn term(code: &str) -> Value {
    coded(TERMINOLOGY, code)
}

/// An `access effect`: a code of HL7 FHIR `HL7_ConsentProvisionType`.
pub(super) fn provision(code: &str) -> Value {
    coded(
        openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE,
        code,
    )
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
        "effect": provision(effect),
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

/// An exclusion of the controller naming `scope`, with `extra` members.
pub(super) fn exclusion(scope: &Value, extra: &Value) -> Value {
    let mut scope = scope.clone();
    scope["_type"] = json!("ACCESS_ACTOR_SCOPE");
    let mut members = json!({ "applies_to": [scope] });
    if let (Some(members), Some(extra)) = (members.as_object_mut(), extra.as_object()) {
        members.extend(extra.clone());
    }
    directive("exclusion", "deny", &members)
}

/// Open settings with `directives` and `gate_keepers`, carrying `exclusions`.
pub(super) fn excluding(
    exclusions: &[Value],
    directives: &[Value],
    gate_keepers: &[Value],
) -> EhrAccessSettings {
    let mut s = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("open"),
                        "exclusions": exclusions });
    if !directives.is_empty() {
        s["directives"] = json!(directives);
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

/// A rule of the controller's policy of `effect` with `extra` members.
pub(super) fn rule(effect: &str, extra: &Value) -> Value {
    directive("controller_rule", effect, extra)
}

/// A role code of [`ROLES`] as an actor scope.
pub(super) fn role_scope(role: &str) -> Value {
    json!({ "role": coded(ROLES, role) })
}

/// A caller of `kind` bound to `party`, performing the role `role`.
pub(super) fn performing(kind: CallerKind, party: &str, role: &str) -> Caller {
    with_facts(caller(kind, party), &[code(ROLES, role)], &[])
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
        own_ehrs: Vec::new(),
        patient_grants_only: false,
        acts_for: Vec::new(),
        facts: CallerFacts::default(),
        administrator: false,
        settings: SettingsCaller::new(false, kind == CallerKind::Subject, Some(party.to_owned())),
        lacks_consent_reference: false,
    }
}

/// `caller` performing `ROLE`s named `roles` and member of `teams`, each
/// relationship from 1 November 2026 with no end, acting for [`provider`].
pub(super) fn with_facts(mut caller: Caller, roles: &[ForeignCode], teams: &[&str]) -> Caller {
    let teams = teams
        .iter()
        .map(|team| care_team(team, "2026-11-01", None))
        .collect();
    caller.facts = CallerFacts::new(roles.to_vec(), teams).with_providers([provider()]);
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

/// The provider the tests' care relationships are in the care of.
pub(super) fn provider() -> ProviderKey {
    ProviderKey::new("urn:example:providers", "hospital")
}

/// A care relationship to `carer` in the role `PART`, in the care of
/// [`provider`], over the dates `[lower, upper)`.
pub(super) fn care_of(carer: &str, lower: &str, upper: Option<&str>) -> CareOf {
    let role = ForeignCode {
        terminology: "HL7_ParticipationType".to_owned(),
        code: "PART".to_owned(),
    };
    CareOf::spanning(
        uuid(carer),
        (
            Some(lower.parse().expect("a date")),
            upper.map(|upper| upper.parse().expect("a date")),
        ),
        role,
        provider(),
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
        reasoned: false,
        opt_out_override: false,
        episodes: None,
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
    /// The ties by which the caller holds the subject, in place of
    /// [`Self::cared`]'s one team tie.
    pub(super) ties: Option<Vec<Tie>>,
    pub(super) now: Option<jiff::Timestamp>,
    /// The EHR is not the subject caller's own.
    pub(super) foreign: bool,
    /// The EHR's subject is the `PERSON` the caller is bound to.
    pub(super) subject: bool,
    /// The kinships that join the caller to the EHR's subject.
    pub(super) kinships: Vec<ForeignCode>,
}

impl Case {
    /// A case of `settings`.
    pub(super) fn of(settings: EhrAccessSettings) -> Self {
        Self {
            settings: Some(settings),
            ..Self::default()
        }
    }

    /// The caller's care for the subject: [`Self::ties`], or for
    /// [`Self::cared`] one team tie in the role `PART`.
    fn care(&self) -> Cared {
        match &self.ties {
            Some(ties) => Cared::of(ties.clone()),
            None if self.cared => Cared::of(vec![Tie {
                path: CarePath::Team,
                role: code("HL7_ParticipationType", "PART"),
                grant: None,
            }]),
            None => Cared::No,
        }
    }

    /// The verdict for `caller` asking `asked`, checked against the verdict
    /// of the settings' class with the settings' facts resolved, which a
    /// query reads in SQL (#5588): both answer every probe item alike.
    pub(super) fn verdict(&self, caller: &Caller, asked: Asked<'_>) -> Verdict {
        let verdict = self.direct(caller, asked);
        let policy = in_force(self.policy.as_ref(), asked);
        if let Some(EhrAccessSettings::Directives(settings)) = &self.settings {
            let classified = classify(settings).expect("the settings classify");
            let class = Class::read(&classified.class.to_string()).expect("the class reads");
            let classifications = policy
                .as_ref()
                .and_then(|p| p.classifications.as_deref())
                .unwrap_or_default();
            let scopes = Scopes::of_class(categories(), Arc::clone(&class.layout), classifications)
                .marked(
                    (*class.settings)
                        .as_ref()
                        .map_or(Confidentiality::Normal, EhrAccessSettings::marking),
                );
            let care = self.care();
            let symbolic = decide(&Judged {
                caller,
                asked,
                policy: Policy::of(policy.as_ref()),
                rules: rules_of(policy.as_ref()),
                conflict_rules: CareFacts::of(policy.as_ref()).conflict_allowances(),
                ehr: Ehr {
                    settings: (*class.settings).as_ref(),
                    server_default: self.server_default.unwrap_or(AccessDefault::Open),
                    care: &care,
                    own: !self.foreign,
                    subject: self.subject,
                    kinships: &self.kinships,
                },
                scopes: &scopes,
                now: self.now.unwrap_or_else(now),
            });
            let mut parties = caller.acts_for.clone();
            parties.extend(caller.party.clone());
            let keys = CallerKeys {
                now: self.now.unwrap_or_else(now),
                parties: parties.into(),
                keeper: caller.settings.party().map(str::to_owned),
            };
            let resolved = resolve_verdict(&symbolic, &classified.facts, &keys);
            for item in probes() {
                assert_eq!(
                    resolved.outcome(&item),
                    verdict.outcome(&item),
                    "the class verdict answers {item:?} as the direct read does: {resolved:?}"
                );
            }
            assert_eq!(
                resolved.reaches().constant(),
                Some(verdict.admits_any()),
                "the class verdict reaches the EHR as the direct read does"
            );
        }
        verdict
    }

    /// The verdict for `caller` asking `asked`, read from the settings.
    fn direct(&self, caller: &Caller, asked: Asked<'_>) -> Verdict {
        let labels = match &self.settings {
            Some(EhrAccessSettings::Directives(s)) => s.labels.as_deref().unwrap_or_default(),
            _ => &[],
        };
        let policy = in_force(self.policy.as_ref(), asked);
        let classifications = policy
            .as_ref()
            .and_then(|p| p.classifications.as_deref())
            .unwrap_or_default();
        let scopes = Scopes::new(categories(), labels, classifications).marked(
            self.settings
                .as_ref()
                .map_or(Confidentiality::Normal, EhrAccessSettings::marking),
        );
        let care = self.care();
        decide(&Judged {
            caller,
            asked,
            policy: Policy::of(policy.as_ref()),
            rules: rules_of(policy.as_ref()),
            conflict_rules: CareFacts::of(policy.as_ref()).conflict_allowances(),
            ehr: Ehr {
                settings: self.settings.as_ref(),
                server_default: self.server_default.unwrap_or(AccessDefault::Open),
                care: &care,
                own: !self.foreign,
                subject: self.subject,
                kinships: &self.kinships,
            },
            scopes: &scopes,
            now: self.now.unwrap_or_else(now),
        })
    }
}

/// The cardiology episode.
pub(super) static CARDIOLOGY_EPISODE: LazyLock<[EpisodeKey; 1]> =
    LazyLock::new(|| [EpisodeKey::of(HOSPITAL, "C-17")]);
/// The dermatology episode.
pub(super) static DERMATOLOGY_EPISODE: LazyLock<[EpisodeKey; 1]> =
    LazyLock::new(|| [EpisodeKey::of(HOSPITAL, "D-4")]);
/// The issuer of the hospital's episode identifiers.
pub(super) const HOSPITAL: &str = "urn:example:hospital";

/// The items a class verdict is checked on: the EHR itself, and each test
/// object under each test template, with and without origins and a commit
/// time, inside and outside November 2026, unplaced, in the cardiology
/// episode and read without its episodes.
fn probes() -> Vec<Item<'static>> {
    const STAMPED: &[String] = &[];
    let mut items = vec![Item::Ehr];
    for id in [OBJECT, OTHER] {
        for template in [Some("psych_note.v1"), Some("lab_hiv.v1"), None] {
            for origins in [Some(STAMPED), None] {
                for episodes in [Some(&[][..]), Some(&CARDIOLOGY_EPISODE[..]), None] {
                    for recorded in ["2026-11-12T08:00:00Z", "2025-01-01T00:00:00Z"] {
                        items.push(Item::Object(Object {
                            vo_id: uuid(id),
                            template,
                            archetype: None,
                            origins,
                            recorded: Some(recorded.parse().expect("an instant")),
                            episodes,
                        }));
                    }
                    items.push(Item::Object(Object {
                        vo_id: uuid(id),
                        template,
                        archetype: None,
                        origins,
                        recorded: None,
                        episodes,
                    }));
                }
            }
        }
    }
    items
}

/// An `ACCESS_DATA_SCOPE` naming the hospital's episodes `ids`.
pub(super) fn episodes(ids: &[&str]) -> Value {
    json!({ "episodes": ids.iter().map(|id| json!({ "_type": "DV_IDENTIFIER",
        "issuer": HOSPITAL, "id": id })).collect::<Vec<_>>() })
}

/// The object `id` of `template`, committed on 12 November 2026.
pub(super) fn object<'a>(id: &str, template: Option<&'a str>) -> Item<'a> {
    Item::Object(Object {
        vo_id: uuid(id),
        template,
        archetype: None,
        origins: Some(&[]),
        recorded: Some("2026-11-12T08:00:00Z".parse().expect("an instant")),
        episodes: Some(&[]),
    })
}

/// The object `id` of `template`, committed on 12 November 2026 and placed in
/// `episodes`.
pub(super) fn placed<'a>(
    id: &str,
    template: Option<&'a str>,
    episodes: &'a [EpisodeKey],
) -> Item<'a> {
    Item::Object(Object {
        vo_id: uuid(id),
        template,
        archetype: None,
        origins: Some(&[]),
        recorded: Some("2026-11-12T08:00:00Z".parse().expect("an instant")),
        episodes: Some(episodes),
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

/// A team tie in the role `PART`, scoped to the hospital's episode `episode`,
/// or to none.
pub(super) fn tie_in(episode: Option<&str>) -> Tie {
    use ferroehr::access_eval::care_tie::Grant;
    use ferroehr::party_binding::grants::caller::care_of::CarerKind;
    use ferroehr::party_binding::grants::caller::care_ref::Episode;
    Tie {
        path: CarePath::Team,
        role: code("HL7_ParticipationType", "PART"),
        grant: Some(Arc::new(Grant {
            relationship: None,
            version: None,
            carer: uuid(CARDIOLOGY),
            carer_kind: Some(CarerKind::Group),
            provider: ProviderKey::new("urn:example:providers", "hospital"),
            in_grace: false,
            covers_for: None,
            episode: episode.map(|id| Arc::new(Episode::new(HOSPITAL, id))),
        })),
    }
}
