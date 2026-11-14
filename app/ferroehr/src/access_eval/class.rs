// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Settings classes: what many EHRs' `EHR_ACCESS` settings share, with the
//! values particular to each EHR set aside as its facts.
//!
//! A query is judged over every EHR it could reach, so its condition cannot
//! read each EHR's settings in memory without holding them all. The
//! projection therefore stores each EHR's settings as a class and its facts
//! ([`crate::storage::access_projection`]). The class is the settings with
//! every value the decision reads that is particular to the EHR replaced by a
//! fixed value: the instants of a validity or a `recorded` interval, the
//! party of an actor scope or a gate-keeper, the objects of a data scope, its
//! origins and the objects each sensitivity labels. What the decision does
//! not read, such as a directive's name or its evidence, is left out, and
//! every coded value keeps only its code phrase. The facts are the values
//! set aside, in the order of a depth-first walk ([`Layout`]).
//!
//! A value the decision cannot read (an instant that is no full date and
//! time, a party reference that names no party, an object reference that
//! names no versioned object) stays in the class, so the class holds what a
//! direct read of the EHR would make of it. Every EHR whose settings make the
//! same class shares one row, so a replica holds the classes, never the
//! EHRs. The evaluator decides a class with its facts as atoms of its
//! predicates, which the query reads from each EHR's facts
//! ([`crate::access_eval::scope::Held`], [`crate::access_eval::scope::EhrFact`]).
//! No openEHR spec governs the classes: our own design (#5588).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification), and its class is the same open object"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind};
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::access_eval::condition::CallerKeys;
use crate::access_eval::procedure::{Rule, Verdict};
use crate::access_eval::scope::{DataMatch, Edge, EhrFact, Held, Pred, Recorded};
use crate::service::ehr::access_projection::{Validity, validity};
use crate::service::ehr::access_rule::{SettingsCaller, party_key, proxy_key};
use crate::service::ehr::access_settings::{EhrAccessSettings, UnreadableSettings, object_id};

/// The instant a class holds in place of an instant of its EHRs.
const INSTANT: &str = "1970-01-01T00:00:00Z";
/// The id a class holds in place of a party or an object of its EHRs.
const NIL: &str = "00000000-0000-0000-0000-000000000000";

/// Where each fact of a class's EHRs sits: the position in the facts of
/// each value the class sets aside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    /// Each directive, in the order of the settings.
    pub directives: Vec<DirectiveLayout>,
    /// Each gate-keeper that can name a caller, in the order of the settings.
    pub keepers: Vec<KeeperLayout>,
    /// The objects labelled with each sensitivity.
    pub labels: Vec<(ForeignCode, usize)>,
    /// How many facts the class's EHRs hold.
    pub facts: usize,
}

/// The facts of one directive and its exceptions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectiveLayout {
    /// The lower bound of `time_validity`.
    pub from: Option<usize>,
    /// The upper bound of `time_validity`.
    pub until: Option<usize>,
    /// The party of each actor scope, in order; `None` where the scope names
    /// no party the decision reads.
    pub parties: Vec<Option<usize>>,
    /// Each data scope, in order.
    pub data: Vec<DataLayout>,
    /// Each exception, in order.
    pub exceptions: Vec<DirectiveLayout>,
}

/// The facts of one data scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataLayout {
    /// The objects, when every reference names one.
    pub objects: Option<usize>,
    /// The origins.
    pub origins: Option<usize>,
    /// The lower bound of `recorded`.
    pub from: Option<usize>,
    /// The upper bound of `recorded`.
    pub until: Option<usize>,
}

/// One gate-keeper as the class holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeeperLayout {
    /// `PARTY_SELF`, which names the subject.
    Subject,
    /// A party, at this fact.
    Party(usize),
}

/// The class of one EHR's settings and the facts it sets aside.
#[derive(Debug, Clone, PartialEq)]
pub struct Classified {
    /// The class, as canonical JSON of the settings class.
    pub class: Value,
    /// The SHA-256 of the class's JSON text, which keys it.
    pub digest: Vec<u8>,
    /// The facts, in the order of the class's [`Layout`].
    pub facts: Vec<Value>,
}

/// The settings classes a store holds, each by its id, as one replica copies
/// them.
#[derive(Debug, Clone, Default)]
pub struct Classes {
    /// Each class by its id.
    pub classes: Vec<(i64, Class)>,
}

/// One class as the evaluator reads it.
#[derive(Debug, Clone)]
pub struct Class {
    /// The class's settings.
    pub settings: Arc<Option<EhrAccessSettings>>,
    /// Where each fact of its EHRs sits.
    pub layout: Arc<Layout>,
}

impl Class {
    /// The class stored as the JSON text `text`.
    ///
    /// # Errors
    /// [`UnreadableSettings`] when the text does not decode as settings.
    pub fn read(text: &str) -> Result<Self, UnreadableSettings> {
        let settings = EhrAccessSettings::from_projected(text)?;
        let layout = match &settings {
            EhrAccessSettings::Directives(read) => Walk::default().settings(read).layout,
            EhrAccessSettings::HeldClosed => Layout::default(),
        };
        Ok(Self {
            settings: Arc::new(Some(settings)),
            layout: Arc::new(layout),
        })
    }

    /// Whether a gate-keeper of the class names `caller`: `PARTY_SELF` the
    /// subject, a party the one the caller is bound to.
    #[must_use]
    pub fn keeper(&self, caller: &SettingsCaller) -> Pred {
        keeper(&self.layout, caller)
    }
}

/// The classes whose gate-keepers may name a caller, grouped by the
/// predicate over their facts that says so: who reads an EHR's `EHR_ACCESS`
/// objects in a query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keepers {
    /// The classes, by id, grouped by the predicate; a class whose
    /// gate-keepers never name the caller is in none.
    pub groups: Vec<(Vec<i64>, Pred)>,
    /// The party the caller is bound to.
    pub party: Option<String>,
}

impl Keepers {
    /// The classes of `classes` whose gate-keepers may name `caller`.
    #[must_use]
    pub fn of(classes: &Classes, caller: &SettingsCaller) -> Self {
        let mut groups: BTreeMap<Pred, Vec<i64>> = BTreeMap::new();
        for (id, class) in &classes.classes {
            let named = class.keeper(caller);
            if named != Pred::False {
                groups.entry(named).or_default().push(*id);
            }
        }
        Self {
            groups: groups.into_iter().map(|(pred, ids)| (ids, pred)).collect(),
            party: caller.party().map(str::to_owned),
        }
    }
}

/// Whether a gate-keeper `layout` holds names `caller`.
#[must_use]
pub fn keeper(layout: &Layout, caller: &SettingsCaller) -> Pred {
    Pred::any(
        layout
            .keepers
            .iter()
            .map(|keeper| match keeper {
                KeeperLayout::Subject => Pred::of(caller.subject()),
                KeeperLayout::Party(at) => Pred::Ehr(EhrFact::Keeper(*at)),
            })
            .collect(),
    )
}

/// `verdict`, a verdict of a class, as it reads for the EHR whose facts are
/// `facts`, judged against `keys`: every atom answered and every fact known,
/// the in-memory reading of what a query reads in SQL.
#[must_use]
pub fn resolve_verdict(verdict: &Verdict, facts: &[Value], keys: &CallerKeys) -> Verdict {
    Verdict {
        rules: verdict
            .rules
            .iter()
            .map(|rule| Rule {
                when: resolve(&rule.when, facts, keys),
                then: rule.then,
                by: rule.by.clone(),
            })
            .collect(),
        ..verdict.clone()
    }
}

/// `pred`, a predicate of a class, as it reads for the EHR whose facts are
/// `facts`, judged against `keys`. A fact that is missing or does not read
/// answers no atom and names no value.
#[must_use]
pub fn resolve(pred: &Pred, facts: &[Value], keys: &CallerKeys) -> Pred {
    let text = |at: usize| facts.get(at).and_then(Value::as_str);
    let instant = |at: usize| text(at).and_then(|t| t.parse::<jiff::Timestamp>().ok());
    let strings = |at: usize| -> Vec<String> {
        facts
            .get(at)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    match pred {
        Pred::True | Pred::False => pred.clone(),
        Pred::Not(inner) => Pred::negate(resolve(inner, facts, keys)),
        Pred::All(preds) => Pred::all(preds.iter().map(|p| resolve(p, facts, keys)).collect()),
        Pred::Any(preds) => Pred::any(preds.iter().map(|p| resolve(p, facts, keys)).collect()),
        Pred::Ehr(EhrFact::Within { from, until }) => {
            let above = from.is_none_or(|(at, included)| {
                instant(at).is_some_and(|b| {
                    if included {
                        b <= keys.now
                    } else {
                        b < keys.now
                    }
                })
            });
            let below = until.is_none_or(|(at, included)| {
                instant(at).is_some_and(|b| {
                    if included {
                        keys.now <= b
                    } else {
                        keys.now < b
                    }
                })
            });
            Pred::of(above && below)
        }
        Pred::Ehr(EhrFact::Party(at)) => {
            Pred::of(text(*at).is_some_and(|key| keys.parties.iter().any(|p| p == key)))
        }
        Pred::Ehr(EhrFact::Keeper(at)) => {
            Pred::of(text(*at).is_some_and(|key| keys.keeper.as_deref() == Some(key)))
        }
        Pred::Data(data, polarity) => {
            let edge = |edge: Option<Edge>| {
                edge.map(|e| match e.at {
                    Held::Fact(at) => Edge {
                        at: instant(at).map_or(e.at, Held::Known),
                        included: e.included,
                    },
                    Held::Known(_) => e,
                })
            };
            let known = DataMatch {
                objects: data.objects.as_ref().map(|objects| match objects {
                    Held::Fact(at) => Held::Known(
                        strings(*at)
                            .iter()
                            .filter_map(|id| id.parse().ok())
                            .collect(),
                    ),
                    Held::Known(_) => objects.clone(),
                }),
                origins: data.origins.as_ref().map(|origins| match origins {
                    Held::Fact(at) => Held::Known(strings(*at)),
                    Held::Known(_) => origins.clone(),
                }),
                recorded: data.recorded.map(|r| Recorded {
                    from: edge(r.from),
                    until: edge(r.until),
                }),
                ..(**data).clone()
            };
            Pred::Data(Arc::new(known), *polarity)
        }
    }
}

/// The class and facts of `settings`.
///
/// # Errors
/// [`serde_json::Error`] when the settings do not serialize.
pub fn classify(settings: &AccessDirectiveSettings) -> Result<Classified, serde_json::Error> {
    let walked = Walk::default().settings(settings);
    let mut class = serde_json::to_value(settings)?;
    normalize_settings(&mut class);
    for edit in &walked.edits {
        edit.apply(&mut class);
    }
    let keepers = class
        .get("gate_keepers")
        .and_then(Value::as_array)
        .map(|keepers| {
            walked
                .keepers
                .iter()
                .filter_map(|(at, subject)| keepers.get(*at).map(|k| keeper_class(k, *subject)))
                .collect()
        })
        .unwrap_or_default();
    let labels = class
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            walked
                .labels
                .iter()
                .filter_map(|(at, coded)| labels.get(*at).map(|l| label_class(l, *coded)))
                .collect()
        })
        .unwrap_or_default();
    set_or_remove(&mut class, "gate_keepers", keepers);
    set_or_remove(&mut class, "labels", labels);
    let digest = Sha256::digest(serde_json::to_string(&class)?.as_bytes()).to_vec();
    Ok(Classified {
        class,
        digest,
        facts: walked.facts,
    })
}

/// What one walk of a settings yields: the layout, the facts, the edits that
/// make the class, each gate-keeper kept by its position and whether it is
/// `PARTY_SELF`, and for each sensitivity the label and the code that first
/// name it.
#[derive(Debug, Default)]
struct Walked {
    layout: Layout,
    facts: Vec<Value>,
    edits: Vec<Edit>,
    keepers: Vec<(usize, bool)>,
    labels: Vec<(usize, usize)>,
}

/// One change the class makes to the settings at a JSON pointer.
#[derive(Debug)]
enum Edit {
    /// Replace the value with the string.
    Set(String, &'static str),
    /// Hold the reference as one to no particular party or object.
    Reference(String),
    /// Hold the list of references as one reference to no particular object.
    Objects(String),
    /// Hold the list of origins as one fixed origin.
    Origins(String),
}

impl Edit {
    /// Applies the edit to `class`.
    fn apply(&self, class: &mut Value) {
        match self {
            Self::Set(pointer, value) => {
                if let Some(at) = class.pointer_mut(pointer) {
                    *at = Value::String((*value).to_owned());
                }
            }
            Self::Reference(pointer) => {
                if let Some(at) = class.pointer_mut(pointer) {
                    nil_reference(at);
                }
            }
            Self::Objects(pointer) | Self::Origins(pointer) => {
                let Some(Value::Array(items)) = class.pointer_mut(pointer) else {
                    return;
                };
                items.truncate(1);
                if let Some(first) = items.first_mut() {
                    if matches!(self, Self::Origins(_)) {
                        *first = Value::String("-".to_owned());
                    } else {
                        nil_reference(first);
                        set(first, "_type", "OBJECT_REF");
                    }
                }
            }
        }
    }
}

/// The depth-first walk that places every fact.
#[derive(Debug, Default)]
struct Walk {
    out: Walked,
}

impl Walk {
    /// Walks `settings`.
    fn settings(mut self, settings: &AccessDirectiveSettings) -> Walked {
        for (at, directive) in settings
            .directives
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let placed = self.directive(directive, &format!("/directives/{at}"));
            self.out.layout.directives.push(placed);
        }
        for (index, keeper) in settings
            .gate_keepers
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            match keeper {
                PartyProxy::PartySelf(_) => {
                    self.out.layout.keepers.push(KeeperLayout::Subject);
                    self.out.keepers.push((index, true));
                }
                PartyProxy::PartyIdentified(_) => {
                    // NOTE: our own design, a gate-keeper that names no party names no caller,
                    // so the class leaves it out.
                    if let Some(key) = proxy_key(keeper) {
                        let at = self.fact(Value::String(key));
                        self.out.layout.keepers.push(KeeperLayout::Party(at));
                        self.out.keepers.push((index, false));
                    }
                }
            }
        }
        let mut labelled: BTreeMap<ForeignCode, (Vec<String>, (usize, usize))> = BTreeMap::new();
        for (index, label) in settings
            .labels
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let Some(vo_id) = object_id(&label.target) else {
                continue;
            };
            for (position, coded) in label.sensitivities.iter().enumerate() {
                let Ok(code) = ForeignCode::read(ForeignKind::Sensitivity, coded) else {
                    continue;
                };
                labelled
                    .entry(code)
                    .or_insert_with(|| (Vec::new(), (index, position)))
                    .0
                    .push(vo_id.to_string());
            }
        }
        for (code, (objects, first)) in labelled {
            let at = self.fact(Value::from(objects));
            self.out.layout.labels.push((code, at));
            self.out.labels.push(first);
        }
        self.out.layout.facts = self.out.facts.len();
        self.out
    }

    /// Walks `directive` at the JSON pointer `at`.
    fn directive(&mut self, directive: &AccessDirective, at: &str) -> DirectiveLayout {
        let mut placed = DirectiveLayout::default();
        if let Some(Validity::Within { from, until }) =
            directive.time_validity.as_ref().map(validity)
        {
            placed.from =
                from.map(|b| self.instant(&format!("{at}/time_validity/lower/value"), b.at));
            placed.until =
                until.map(|b| self.instant(&format!("{at}/time_validity/upper/value"), b.at));
        }
        for (scope_at, scope) in directive
            .applies_to
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let key = scope.party.as_ref().and_then(party_key);
            placed.parties.push(key.map(|key| {
                self.edited(
                    Value::String(key),
                    Edit::Reference(format!("{at}/applies_to/{scope_at}/party")),
                )
            }));
        }
        for (scope_at, scope) in directive
            .data
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let pointer = format!("{at}/data/{scope_at}");
            let mut data = DataLayout::default();
            let objects: Option<Option<Vec<String>>> = scope.objects.as_deref().map(|objects| {
                objects
                    .iter()
                    .map(|o| object_id(o).map(|id| id.to_string()))
                    .collect()
            });
            if let Some(Some(objects)) = objects {
                data.objects = Some(self.edited(
                    Value::from(objects),
                    Edit::Objects(format!("{pointer}/objects")),
                ));
            }
            if let Some(origins) = scope.origins.as_deref() {
                data.origins = Some(self.edited(
                    Value::from(origins.to_vec()),
                    Edit::Origins(format!("{pointer}/origins")),
                ));
            }
            if let Some(Validity::Within { from, until }) = scope.recorded.as_ref().map(validity) {
                data.from =
                    from.map(|b| self.instant(&format!("{pointer}/recorded/lower/value"), b.at));
                data.until =
                    until.map(|b| self.instant(&format!("{pointer}/recorded/upper/value"), b.at));
            }
            placed.data.push(data);
        }
        for (exception_at, exception) in directive
            .exceptions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let exception = self.directive(exception, &format!("{at}/exceptions/{exception_at}"));
            placed.exceptions.push(exception);
        }
        placed
    }

    /// Sets aside the instant `at` found at `pointer`.
    fn instant(&mut self, pointer: &str, at: jiff::Timestamp) -> usize {
        self.edited(
            Value::String(at.to_string()),
            Edit::Set(pointer.to_owned(), INSTANT),
        )
    }

    /// Sets aside `fact`, which the class holds as `edit` makes it.
    fn edited(&mut self, fact: Value, edit: Edit) -> usize {
        self.out.edits.push(edit);
        self.fact(fact)
    }

    /// Sets aside `fact`, returning its position.
    fn fact(&mut self, fact: Value) -> usize {
        self.out.facts.push(fact);
        self.out.facts.len() - 1
    }
}

/// Holds the reference `value` as one to no particular party or object: its
/// namespace local and its id the nil UUID.
fn nil_reference(value: &mut Value) {
    retain(value, &["_type", "namespace", "type", "id"]);
    set(value, "namespace", "local");
    if let Some(id) = value.get_mut("id") {
        retain(id, &["_type", "value"]);
        set(id, "_type", "HIER_OBJECT_ID");
        set(id, "value", NIL);
    }
}

/// The class of the gate-keeper `keeper`: `PARTY_SELF` alone, or a party
/// reference to no particular party.
fn keeper_class(keeper: &Value, subject: bool) -> Value {
    let mut keeper = keeper.clone();
    if subject {
        retain(&mut keeper, &["_type"]);
    } else {
        retain(&mut keeper, &["_type", "external_ref"]);
        set(&mut keeper, "_type", "PARTY_IDENTIFIED");
        if let Some(reference) = keeper.get_mut("external_ref") {
            nil_reference(reference);
        }
    }
    keeper
}

/// The class of the label `label` for the sensitivity at `coded`: a label
/// of that one code on no particular object.
fn label_class(label: &Value, coded: usize) -> Value {
    let mut label = label.clone();
    retain(&mut label, &["_type", "target", "sensitivities"]);
    if let Some(target) = label.get_mut("target") {
        nil_reference(target);
        set(target, "_type", "OBJECT_REF");
    }
    let code = label
        .get("sensitivities")
        .and_then(|codes| codes.get(coded))
        .map(normal_coded);
    if let (Some(object), Some(code)) = (label.as_object_mut(), code) {
        object.insert("sensitivities".to_owned(), Value::Array(vec![code]));
    }
    label
}

/// Keeps of the object `value` only the members `keys`.
fn retain(value: &mut Value, keys: &[&str]) {
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, _| keys.contains(&key.as_str()));
    }
}

/// Sets the member `key` of the object `value` to the string `text`.
fn set(value: &mut Value, key: &str, text: &str) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_owned(), Value::String(text.to_owned()));
    }
}

/// Sets `key` of `object` to `members`, or removes it when there are none.
fn set_or_remove(object: &mut Value, key: &str, members: Vec<Value>) {
    if let Some(object) = object.as_object_mut() {
        if members.is_empty() {
            object.remove(key);
        } else {
            object.insert(key.to_owned(), Value::Array(members));
        }
    }
}

/// Keeps of the settings `value` only what the decision reads.
fn normalize_settings(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(default) = object.get_mut("default_access") {
        *default = normal_coded(default);
    }
    if let Some(Value::Array(directives)) = object.get_mut("directives") {
        directives.iter_mut().for_each(normalize_directive);
    }
}

/// The members of a directive the decision reads.
const DIRECTIVE: [&str; 10] = [
    "_type",
    "kind",
    "effect",
    "applies_to",
    "operations",
    "purposes",
    "data",
    "time_validity",
    "emergency_override",
    "exceptions",
];

/// Keeps of the directive `value` only what the decision reads.
fn normalize_directive(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.retain(|key, _| DIRECTIVE.contains(&key.as_str()) || key == "name");
    if let Some(name) = object.get_mut("name") {
        retain(name, &["_type", "value"]);
        set(name, "_type", "DV_TEXT");
        set(name, "value", "directive");
    }
    object.insert(
        "archetype_node_id".to_owned(),
        Value::String("at0000".to_owned()),
    );
    for key in ["kind", "effect"] {
        if let Some(coded) = object.get_mut(key) {
            *coded = normal_coded(coded);
        }
    }
    for key in ["operations", "purposes"] {
        if let Some(Value::Array(codes)) = object.get_mut(key) {
            for c in codes.iter_mut() {
                *c = normal_coded(c);
            }
        }
    }
    if let Some(validity) = object.get_mut("time_validity") {
        normalize_interval(validity);
    }
    if let Some(Value::Array(scopes)) = object.get_mut("applies_to") {
        for scope in scopes.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["role", "relationship", "caller_kind"] {
                if let Some(coded) = scope.get_mut(key) {
                    *coded = normal_coded(coded);
                }
            }
        }
    }
    if let Some(Value::Array(scopes)) = object.get_mut("data") {
        for scope in scopes.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["categories", "sensitivities"] {
                if let Some(Value::Array(codes)) = scope.get_mut(key) {
                    for c in codes.iter_mut() {
                        *c = normal_coded(c);
                    }
                }
            }
            if let Some(recorded) = scope.get_mut("recorded") {
                normalize_interval(recorded);
            }
        }
    }
    if let Some(Value::Array(exceptions)) = object.get_mut("exceptions") {
        exceptions.iter_mut().for_each(normalize_directive);
    }
}

/// Keeps of a readable `DV_INTERVAL<DV_DATE_TIME>` only its bounds and their
/// flags, dropping a bound the interval marks unbounded; an unreadable one
/// stays as it is, so the class holds what the decision makes of it.
fn normalize_interval(value: &mut Value) {
    let readable = serde_json::from_value(value.clone())
        .is_ok_and(|interval| matches!(validity(&interval), Validity::Within { .. }));
    let Some(object) = value.as_object_mut().filter(|_| readable) else {
        return;
    };
    for (bound, unbounded) in [("lower", "lower_unbounded"), ("upper", "upper_unbounded")] {
        let open = object
            .get(unbounded)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if open {
            object.remove(bound);
        } else if let Some(at) = object.get_mut(bound) {
            retain(at, &["_type", "value"]);
        }
    }
    object.retain(|key, _| {
        [
            "_type",
            "lower",
            "upper",
            "lower_unbounded",
            "upper_unbounded",
            "lower_included",
            "upper_included",
        ]
        .contains(&key.as_str())
    });
}

/// A `DV_CODED_TEXT` by its code phrase alone, its value the code.
fn normal_coded(coded: &Value) -> Value {
    let mut coded = coded.clone();
    retain(&mut coded, &["_type", "value", "defining_code"]);
    let code = coded
        .pointer("/defining_code/code_string")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = coded.as_object_mut() {
        object.insert("value".to_owned(), code);
    }
    if let Some(phrase) = coded.get_mut("defining_code") {
        retain(phrase, &["_type", "terminology_id", "code_string"]);
        if let Some(terminology) = phrase.get_mut("terminology_id") {
            retain(terminology, &["_type", "value"]);
        }
    }
    coded
}
