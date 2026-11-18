// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ctx/` context vocabulary + defaulting.
//!
//! ITS-REST `simplified_formats/master06-context_information.adoc` (and the
//! `ctx/` overview in `master04-basic_concepts.adoc` §Context) define a set of
//! `ctx/`-prefixed shortcuts that set default values in the RM tree the
//! Web Template does not surface as leaf nodes — composition/entry language and
//! territory, the composer, the event-context time/setting/location/facility/
//! participations, and per-ENTRY defaults (provider, workflow id, ISM state,
//! activity timing, instruction narrative, links).
//!
//! [`resolve`] parses the `ctx` child of a parsed simplified document into a
//! typed [`CtxDefaults`] carrying **ready canonical-JSON pieces** the
//! composition builder drops into place; [`emit`] is the reverse (a canonical
//! COMPOSITION → the `ctx/` keys).
//!
//! Terminology-coded shortcuts (`setting`, `action_ism_transition_current_state`)
//! accept either a code or a display value (master06 §§setting,
//! action_ism_transition_current_state) and resolve through the openEHR
//! terminology bundle via [`crate::flat::legacy::map::coded_from_group`].

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::flat::error::FlatError;
use crate::flat::legacy::map::coded_from_group;
use crate::flat::sim::SimNode;

/// The `ctx/` child names of master06 (+ the `master04 §Context` overview). A
/// `ctx/` key outside this vocabulary is a [`FlatError::UnknownContext`].
const KNOWN_CTX: &[&str] = &[
    "language",
    "territory",
    "time",
    "end_time",
    "history_origin",
    "action_time",
    "activity_timing",
    "composer_name",
    "composer_self",
    "composer_id",
    "id_namespace",
    "id_scheme",
    "work_flow_id",
    "participation_name",
    "participation_function",
    "participation_mode",
    "participation_id",
    "participation_identifiers",
    "health_care_facility",
    "location",
    "setting",
    "provider_name",
    "provider_id",
    "action_ism_transition_current_state",
    "instruction_narrative",
    "link",
];

/// The typed, resolved context of one simplified data instance
/// (master06-context_information.adoc). Fields hold ready canonical-JSON pieces
/// (or the raw strings a piece is built from on demand).
#[derive(Debug, Clone)]
pub(crate) struct CtxDefaults {
    /// `ENTRY.language` / `COMPOSITION.language` code (master06 §"Language and
    /// Territory"). Not defaulted here — the mandatory check is the caller's.
    pub(crate) language: Option<String>,
    /// `COMPOSITION.territory` code (master06 §"Language and Territory").
    pub(crate) territory: Option<String>,
    /// `COMPOSITION.composer` (`PARTY_PROXY`), built per master06 §Composer.
    pub(crate) composer: Option<Value>,
    /// `EVENT_CONTEXT.start_time` / `ACTION.time` / `OBSERVATION.history.origin`
    /// default (master06 §time; defaults to `now`).
    pub(crate) time: String,
    /// `EVENT_CONTEXT.end_time` (master06 §end_time).
    pub(crate) end_time: Option<String>,
    /// `OBSERVATION.history.origin` override (master06 §history_origin).
    pub(crate) history_origin: Option<String>,
    /// `ACTION.time` override (master06 §action_time).
    pub(crate) action_time: Option<String>,
    /// `ACTIVITY.timing` default (master06 §activity_timing).
    pub(crate) activity_timing: Option<String>,
    /// `EVENT_CONTEXT.setting` (`DV_CODED_TEXT`), default "other care" (238).
    pub(crate) setting: Value,
    /// `EVENT_CONTEXT.location` free-text label (master06 §location).
    pub(crate) location: Option<String>,
    /// `EVENT_CONTEXT.health_care_facility` (`PARTY_IDENTIFIED`).
    pub(crate) health_care_facility: Option<Value>,
    /// `EVENT_CONTEXT.participations` / `ENTRY.other_participations`
    /// (built `PARTICIPATION`s, master06 §Participation).
    pub(crate) participations: Vec<Value>,
    /// `ENTRY.workflow_id` (`OBJECT_REF`, master06 §"Workflow ID").
    pub(crate) work_flow_id: Option<Value>,
    /// `ENTRY.provider` (`PARTY_IDENTIFIED`, master06 §provider).
    pub(crate) provider: Option<Value>,
    /// `ACTION.ism_transition.current_state` (`DV_CODED_TEXT`, master06
    /// §action_ism_transition_current_state).
    pub(crate) action_ism_current_state: Option<Value>,
    /// `INSTRUCTION.narrative` text (master06 §instruction_narrative).
    pub(crate) instruction_narrative: Option<String>,
    /// `LOCATABLE.links` (built `LINK`s, master06 §link).
    pub(crate) links: Vec<Value>,
    /// Default external-reference namespace (master06 §"ID Namespace and Scheme").
    pub(crate) id_namespace: Option<String>,
    /// Default external-reference scheme (master06 §"ID Namespace and Scheme").
    pub(crate) id_scheme: Option<String>,
    /// The `COMPOSITION.language` CODE_PHRASE a `<root>/language|raw` value
    /// carries, used when no `ctx/language` names one (master04 §Raw
    /// canonical JSON).
    pub(crate) language_phrase: Option<Value>,
    /// The `COMPOSITION.territory` CODE_PHRASE a `<root>/territory|raw` value
    /// carries, used when no `ctx/territory` names one.
    pub(crate) territory_phrase: Option<Value>,
}

impl Default for CtxDefaults {
    /// `setting` is the one context field the spec gives a value to when it is
    /// absent: openEHR `setting` group code `238` "other care" (master06
    /// §setting — "will be set to 'other care' if not set"). Every other field
    /// defaults to absent, and `time` is filled by [`resolve`] from `now`.
    fn default() -> Self {
        Self {
            language: None,
            territory: None,
            composer: None,
            time: String::new(),
            end_time: None,
            history_origin: None,
            action_time: None,
            activity_timing: None,
            setting: coded_from_group("setting", "other care"),
            location: None,
            health_care_facility: None,
            participations: Vec::new(),
            work_flow_id: None,
            provider: None,
            action_ism_current_state: None,
            instruction_narrative: None,
            links: Vec::new(),
            id_namespace: None,
            id_scheme: None,
            language_phrase: None,
            territory_phrase: None,
        }
    }
}

impl CtxDefaults {
    /// The `COMPOSITION`/`ENTRY` `language` as a CODE_PHRASE in `ISO_639-1`
    /// (master06 §"Language and Territory"), when `ctx/language` was set,
    /// else the `<root>/language|raw` CODE_PHRASE.
    pub(crate) fn language_code_phrase(&self) -> Option<Value> {
        self.language
            .as_deref()
            .map(|c| code_phrase("ISO_639-1", c))
            .or_else(|| self.language_phrase.clone())
    }

    /// The `COMPOSITION.territory` as a CODE_PHRASE in `ISO_3166-1`, when
    /// `ctx/territory` was set, else the `<root>/territory|raw` CODE_PHRASE.
    pub(crate) fn territory_code_phrase(&self) -> Option<Value> {
        self.territory
            .as_deref()
            .map(|c| code_phrase("ISO_3166-1", c))
            .or_else(|| self.territory_phrase.clone())
    }
}

// ── sim → CtxDefaults ─────────────────────────────────────────────────────────

/// Resolve the `ctx` child of a parsed simplified document. `now` supplies the
/// default timestamp (master04 §Context / master06 §time: `ctx/time` defaults to
/// `now()`).
///
/// Missing `language`/`territory` is NOT an error here — the caller (validation)
/// owns the mandatory-field check (master04 §Context).
///
/// # Errors
/// - [`FlatError::UnknownContext`] — a `ctx/` key outside the master06 vocabulary.
/// - [`FlatError::InvalidValue`] — a malformed compact participation-identifier
///   list (master06 §Participation).
pub(crate) fn resolve(ctx: Option<&SimNode>, now: &str) -> Result<CtxDefaults, FlatError> {
    let mut out = CtxDefaults {
        time: now.to_owned(),
        ..CtxDefaults::default()
    };
    let Some(ctx) = ctx else { return Ok(out) };

    for name in ctx.children.keys() {
        if !KNOWN_CTX.contains(&name.as_str()) {
            return Err(FlatError::UnknownContext(name.clone()));
        }
    }

    let bare = |name: &str| ctx.child(name).and_then(SimNode::bare);
    let bare_str = |name: &str| bare(name).and_then(Value::as_str);

    out.id_namespace = bare_str("id_namespace").map(str::to_owned);
    out.id_scheme = bare_str("id_scheme").map(str::to_owned);
    out.language = bare_str("language").map(str::to_owned);
    out.territory = bare_str("territory").map(str::to_owned);
    if let Some(t) = bare_str("time") {
        t.clone_into(&mut out.time);
    }
    out.end_time = bare_str("end_time").map(str::to_owned);
    out.history_origin = bare_str("history_origin").map(str::to_owned);
    out.action_time = bare_str("action_time").map(str::to_owned);
    out.activity_timing = bare_str("activity_timing").map(str::to_owned);
    out.location = bare_str("location").map(str::to_owned);
    out.instruction_narrative = bare_str("instruction_narrative").map(str::to_owned);

    if let Some(s) = bare_str("setting") {
        out.setting = coded_from_group("setting", s);
    }
    if let Some(s) = bare_str("action_ism_transition_current_state") {
        out.action_ism_current_state = Some(coded_from_group("instruction_states", s));
    }

    // Compute into locals first — these read the `id_namespace`/`id_scheme`
    // defaults set above, so `out` must not be mutably reborrowed mid-read.
    let composer = resolve_composer(ctx, &out);
    let facility = resolve_facility(ctx, &out);
    let provider = resolve_provider(ctx, &out);
    let work_flow_id = resolve_workflow(ctx, &out);
    let participations = resolve_participations(ctx, &out)?;
    let links = resolve_links(ctx);
    out.composer = composer;
    out.health_care_facility = facility;
    out.provider = provider;
    out.work_flow_id = work_flow_id;
    out.participations = participations;
    out.links = links;
    Ok(out)
}

/// master06 §Composer: `composer_self` → PARTY_SELF; `composer_name` →
/// PARTY_IDENTIFIED name; `composer_id` → `external_ref.id.value`.
fn resolve_composer(ctx: &SimNode, d: &CtxDefaults) -> Option<Value> {
    let self_flag = ctx
        .child("composer_self")
        .and_then(SimNode::bare)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let name = ctx.child("composer_name").and_then(SimNode::bare);
    let id = ctx
        .child("composer_id")
        .and_then(SimNode::bare)
        .and_then(Value::as_str);
    if self_flag {
        let mut p = Map::new();
        p.insert("_type".to_owned(), json!("PARTY_SELF"));
        if let Some(id) = id {
            p.insert("external_ref".to_owned(), party_ref(id, d, "PERSON"));
        }
        return Some(Value::Object(p));
    }
    if name.is_none() && id.is_none() {
        return None;
    }
    Some(party_identified(name, id, d, "PERSON"))
}

/// master06 §health_care_facility: a PARTY_IDENTIFIED (`|name`/`|id`).
fn resolve_facility(ctx: &SimNode, d: &CtxDefaults) -> Option<Value> {
    let hcf = ctx.child("health_care_facility")?;
    let name = hcf.attrs.get("name");
    let id = hcf.attrs.get("id").and_then(Value::as_str);
    if name.is_none() && id.is_none() {
        return None;
    }
    Some(party_identified(name, id, d, "ORGANISATION"))
}

/// master06 §provider: a PARTY_IDENTIFIED from `provider_name`/`provider_id`.
fn resolve_provider(ctx: &SimNode, d: &CtxDefaults) -> Option<Value> {
    let name = ctx.child("provider_name").and_then(SimNode::bare);
    let id = ctx
        .child("provider_id")
        .and_then(SimNode::bare)
        .and_then(Value::as_str);
    if name.is_none() && id.is_none() {
        return None;
    }
    Some(party_identified(name, id, d, "PERSON"))
}

/// master06 §"Workflow ID": an OBJECT_REF; `|id_scheme`/`|namespace` fall back to
/// `ctx/id_scheme` / `ctx/id_namespace`.
fn resolve_workflow(ctx: &SimNode, d: &CtxDefaults) -> Option<Value> {
    let wf = ctx.child("work_flow_id")?;
    let id = wf.attrs.get("id").and_then(Value::as_str)?;
    let ty = wf
        .attrs
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("ANY");
    let scheme = wf
        .attrs
        .get("id_scheme")
        .and_then(Value::as_str)
        .or(d.id_scheme.as_deref())
        .unwrap_or("id_scheme");
    let ns = wf
        .attrs
        .get("namespace")
        .and_then(Value::as_str)
        .or(d.id_namespace.as_deref())
        .unwrap_or("EHR");
    Some(json!({
        "_type": "OBJECT_REF",
        "namespace": ns,
        "type": ty,
        "id": {"_type": "GENERIC_ID", "value": id, "scheme": scheme},
    }))
}

/// master06 §link: `LOCATABLE.links` from `ctx/link:i|type`/`|meaning`/`|target`.
fn resolve_links(ctx: &SimNode) -> Vec<Value> {
    let Some(child) = ctx.children.get("link") else {
        return Vec::new();
    };
    child
        .occurrences
        .iter()
        .filter(|o| !o.is_empty())
        .map(|o| {
            let text = |s: &str| o.attrs.get(s).and_then(Value::as_str).unwrap_or("");
            json!({
                "_type": "LINK",
                "type": {"_type": "DV_TEXT", "value": text("type")},
                "meaning": {"_type": "DV_TEXT", "value": text("meaning")},
                "target": {"_type": "DV_EHR_URI", "value": text("target")},
            })
        })
        .collect()
}

/// master06 §Participation: build a `PARTICIPATION` per index from the parallel
/// `participation_name`/`_function`/`_mode`/`_id`/`_identifiers` families.
fn resolve_participations(ctx: &SimNode, d: &CtxDefaults) -> Result<Vec<Value>, FlatError> {
    let occ = |name: &str, i: usize| {
        ctx.children
            .get(name)
            .and_then(|c| c.occurrences.get(i))
            .filter(|o| !o.is_empty())
    };
    let max = [
        "participation_name",
        "participation_function",
        "participation_mode",
        "participation_id",
        "participation_identifiers",
    ]
    .iter()
    .filter_map(|n| ctx.children.get(*n).map(|c| c.occurrences.len()))
    .max()
    .unwrap_or(0);

    let mut out = Vec::new();
    for i in 0..max {
        let name = occ("participation_name", i).and_then(SimNode::bare);
        let function = occ("participation_function", i)
            .and_then(SimNode::bare)
            .and_then(Value::as_str);
        let mode = occ("participation_mode", i)
            .and_then(SimNode::bare)
            .and_then(Value::as_str);
        let id = occ("participation_id", i)
            .and_then(SimNode::bare)
            .and_then(Value::as_str);
        let identifiers = occ("participation_identifiers", i)
            .map(|n| participation_identifiers(n, i))
            .transpose()?
            .unwrap_or_default();

        if name.is_none()
            && function.is_none()
            && mode.is_none()
            && id.is_none()
            && identifiers.is_empty()
        {
            continue;
        }

        // `PARTICIPATION.function` is 1..1 (RM
        // `UML/classes/org.openehr.rm.common.participation.adoc` §Attributes),
        // and master06 §Participation supplies `participation_function:<i>` in
        // every example it gives. The chapter is silent on omitting it, so the
        // class table governs: a participation the client began at this index
        // but gave no function is a client error, refused by name — never
        // completed with a fabricated empty DV_TEXT, which would commit a
        // participation whose mandatory attribute carries no information.
        let Some(function) = function else {
            return Err(FlatError::MissingRequiredSuffix {
                key: format!("ctx/participation_function:{i}"),
            });
        };
        let mut p = Map::new();
        p.insert("_type".to_owned(), json!("PARTICIPATION"));
        p.insert(
            "function".to_owned(),
            json!({"_type": "DV_TEXT", "value": function}),
        );
        // The performer is a PARTY_IDENTIFIED (master06 §Participation); its
        // identifiers make it identified even without a name.
        let mut performer = party_identified(name, id, d, "PERSON");
        if let Value::Object(pm) = &mut performer
            && !identifiers.is_empty()
        {
            pm.insert("_type".to_owned(), json!("PARTY_IDENTIFIED"));
            pm.insert("identifiers".to_owned(), Value::Array(identifiers));
        }
        p.insert("performer".to_owned(), performer);
        if let Some(m) = mode {
            p.insert("mode".to_owned(), coded_from_group("participation_mode", m));
        }
        out.push(Value::Object(p));
    }
    Ok(out)
}

/// The performer identifiers for one participation, in both master06 forms:
/// the *compact* `"issuer::assigner::id::TYPE;…"` bare string, or the
/// *non-compact* `|issuer:j`/`|assigner:j`/`|id:j`/`|type:j` attrs.
fn participation_identifiers(node: &SimNode, index: usize) -> Result<Vec<Value>, FlatError> {
    // Compact bare form (master06 §Participation example, index 0).
    if let Some(compact) = node.bare().and_then(Value::as_str) {
        return parse_compact_identifiers(compact, index);
    }
    // Non-compact form: `|<field>:j` attrs grouped by `j`.
    let mut by_index: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
    for (key, value) in &node.attrs {
        let Some((field, j)) = key.split_once(':') else {
            continue;
        };
        let Ok(j) = j.parse::<u32>() else { continue };
        if !matches!(field, "id" | "issuer" | "assigner" | "type") {
            continue;
        }
        by_index
            .entry(j)
            .or_default()
            .insert(field.to_owned(), value.clone());
    }
    Ok(by_index
        .into_values()
        .map(|mut fields| {
            fields.insert("_type".to_owned(), json!("DV_IDENTIFIER"));
            fields.entry("id".to_owned()).or_insert_with(|| json!(""));
            Value::Object(fields)
        })
        .collect())
}

/// Parse the compact `"issuer::assigner::id::TYPE;issuer2::…"` participation
/// identifier list (master06 §Participation). Each `;`-separated entry must have
/// exactly four `::`-separated parts.
fn parse_compact_identifiers(compact: &str, index: usize) -> Result<Vec<Value>, FlatError> {
    let mut out = Vec::new();
    for entry in compact.split(';').filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = entry.split("::").collect();
        let [issuer, assigner, id, id_type] = parts.as_slice() else {
            return Err(FlatError::InvalidValue {
                path: format!("ctx/participation_identifiers:{index}"),
                reason: format!("compact identifier {entry:?} must be issuer::assigner::id::type"),
            });
        };
        out.push(json!({
            "_type": "DV_IDENTIFIER",
            "issuer": issuer,
            "assigner": assigner,
            "id": id,
            "type": id_type,
        }));
    }
    Ok(out)
}

/// A PARTY_IDENTIFIED (or PARTY_SELF when neither name nor id is given),
/// honouring `ctx/id_scheme`/`ctx/id_namespace` (master06 §§Composer, "ID
/// Namespace and Scheme").
fn party_identified(
    name: Option<&Value>,
    id: Option<&str>,
    d: &CtxDefaults,
    party_type: &str,
) -> Value {
    let mut p = Map::new();
    match (name, id) {
        (_, Some(id)) => {
            p.insert("_type".to_owned(), json!("PARTY_IDENTIFIED"));
            if let Some(n) = name {
                p.insert("name".to_owned(), n.clone());
            }
            p.insert("external_ref".to_owned(), party_ref(id, d, party_type));
        }
        (Some(n), None) => {
            p.insert("_type".to_owned(), json!("PARTY_IDENTIFIED"));
            p.insert("name".to_owned(), n.clone());
        }
        (None, None) => {
            p.insert("_type".to_owned(), json!("PARTY_SELF"));
        }
    }
    Value::Object(p)
}

/// A PARTY_REF for `id` honouring the ctx scheme/namespace defaults. A scheme
/// makes the id a `GENERIC_ID`; without one it is a `HIER_OBJECT_ID` (never a
/// fabricated scheme).
fn party_ref(id: &str, d: &CtxDefaults, party_type: &str) -> Value {
    let id_obj = match d.id_scheme.as_deref() {
        Some(scheme) => json!({"_type": "GENERIC_ID", "value": id, "scheme": scheme}),
        None => json!({"_type": "HIER_OBJECT_ID", "value": id}),
    };
    json!({
        "_type": "PARTY_REF",
        "id": id_obj,
        "namespace": d.id_namespace.as_deref().unwrap_or("EHR"),
        "type": party_type,
    })
}

fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code,
    })
}

// ── COMPOSITION → ctx sim ─────────────────────────────────────────────────────

/// Derive the `ctx/` keys from a canonical COMPOSITION onto `out` (the `ctx`
/// SimNode). The bidirectional shortcuts (master06 §§"Language and Territory",
/// Composer, time, setting, location, health_care_facility, Participation) are
/// symmetric with [`resolve`]; the per-ENTRY input-only defaults (provider,
/// workflow id, ISM state, timing, narrative, links) keep their structural home
/// on output and are not emitted here (master06 frames them as input shortcuts).
pub(crate) fn emit(composition: &Value, out: &mut SimNode) {
    if let Some(code) = composition.pointer("/language/code_string") {
        set_bare(out, "language", code.clone());
    }
    if let Some(code) = composition.pointer("/territory/code_string") {
        set_bare(out, "territory", code.clone());
    }
    match composition.get("composer") {
        // NOTE: master06 §Composer has no reference-type shortcut, so such a composer is
        // written as `composer|raw` by the flattener (master04 §Raw canonical JSON).
        Some(c) if !crate::flat::legacy::map::parties::ref_type_survives(c, "PERSON") => {}
        Some(c) if c.get("_type").and_then(Value::as_str) == Some("PARTY_SELF") => {
            set_bare(out, "composer_self", json!(true));
            emit_external_ref(c, out);
        }
        Some(c) => {
            if let Some(name) = c.get("name").filter(|v| !v.is_null()) {
                set_bare(out, "composer_name", name.clone());
            }
            emit_external_ref(c, out);
        }
        None => {}
    }

    let context = composition.get("context");
    if let Some(t) = context.and_then(|c| c.pointer("/start_time/value")) {
        set_bare(out, "time", t.clone());
    }
    if let Some(t) = context.and_then(|c| c.pointer("/end_time/value")) {
        set_bare(out, "end_time", t.clone());
    }
    // master06 §setting: emit the code (round-trips through `resolve`, which
    // accepts either a code or a value).
    if let Some(code) = context.and_then(|c| c.pointer("/setting/defining_code/code_string")) {
        set_bare(out, "setting", code.clone());
    }
    if let Some(loc) = context
        .and_then(|c| c.get("location"))
        .filter(|v| !v.is_null())
    {
        set_bare(out, "location", loc.clone());
    }
    // NOTE: health_care_facility and participations are NOT emitted as ctx/
    // shortcuts — the master06 shortcut vocabulary is lossy for them, so the
    // lossless master05 §EVENT_CONTEXT path rows own the output.
}

/// Emit a party's `external_ref` id/namespace/scheme as `ctx/composer_id` +
/// `ctx/id_namespace`/`ctx/id_scheme` (master06 §Composer). Namespace and
/// scheme emit only when an id value exists — a ref without an id cannot be
/// rebuilt from the shortcuts (master06 §Composer: `composer_id` sets
/// `external_ref.id.value`), so emitting its satellites alone would break
/// the round-trip.
fn emit_external_ref(party: &Value, out: &mut SimNode) {
    let Some(id) = party
        .pointer("/external_ref/id/value")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    set_bare(out, "composer_id", json!(id));
    if let Some(ns) = party
        .pointer("/external_ref/namespace")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        set_bare(out, "id_namespace", json!(ns));
    }
    if let Some(scheme) = party
        .pointer("/external_ref/id/scheme")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        set_bare(out, "id_scheme", json!(scheme));
    }
}

fn set_bare(out: &mut SimNode, name: &str, value: Value) {
    out.occurrence_mut(name, None)
        .attrs
        .insert(String::new(), value);
}
