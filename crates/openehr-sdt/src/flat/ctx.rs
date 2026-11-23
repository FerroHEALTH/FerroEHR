// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ctx/` vocabulary (ITS-REST
//! `simplified_formats/master06-context_information.adoc`): [`resolve`] reads
//! the `ctx` child into typed RM pieces ([`CtxDefaults`]);
//! [`crate::flat::ctx_emit::emit`] is the reverse. Coded shortcuts accept a
//! code or a display value ([`crate::flat::map::coded_from_group`]).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::BTreeMap;

use serde_json::Value;

use crate::flat::error::FlatError;
use openehr_rm::v1_2::access;

use crate::flat::map::coded_from_group;
use crate::flat::sim::SimNode;
use crate::flat::typed::{self, Build, Obj, o, s};

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

/// The resolved context of one simplified instance (master06).
#[derive(Debug, Default)]
pub(crate) struct CtxDefaults {
    /// `ENTRY.language` / `COMPOSITION.language` code (master06 §"Language and
    /// Territory"). Not defaulted here — the mandatory check is the caller's.
    pub(crate) language: Option<String>,
    /// `COMPOSITION.territory` code (master06 §"Language and Territory").
    pub(crate) territory: Option<String>,
    /// `COMPOSITION.composer` (`PARTY_PROXY`), built per master06 §Composer.
    pub(crate) composer: Option<Obj>,
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
    /// `EVENT_CONTEXT.setting` (`DV_CODED_TEXT`) the client named; see
    /// [`CtxDefaults::setting`] for the default.
    pub(crate) setting: Option<Obj>,
    /// `EVENT_CONTEXT.location` free-text label (master06 §location).
    pub(crate) location: Option<String>,
    /// `EVENT_CONTEXT.health_care_facility` (`PARTY_IDENTIFIED`).
    pub(crate) health_care_facility: Option<Obj>,
    /// `EVENT_CONTEXT.participations` / `ENTRY.other_participations`
    /// (built `PARTICIPATION`s, master06 §Participation).
    pub(crate) participations: Vec<Obj>,
    /// `ENTRY.workflow_id` (`OBJECT_REF`, master06 §"Workflow ID").
    pub(crate) work_flow_id: Option<Obj>,
    /// `ENTRY.provider` (`PARTY_IDENTIFIED`, master06 §provider).
    pub(crate) provider: Option<Obj>,
    /// `ACTION.ism_transition.current_state` (`DV_CODED_TEXT`, master06
    /// §action_ism_transition_current_state).
    pub(crate) action_ism_current_state: Option<Obj>,
    /// `INSTRUCTION.narrative` text (master06 §instruction_narrative).
    pub(crate) instruction_narrative: Option<String>,
    /// `LOCATABLE.links` (built `LINK`s, master06 §link).
    pub(crate) links: Vec<Obj>,
    /// Default external-reference namespace (master06 §"ID Namespace and Scheme").
    pub(crate) id_namespace: Option<String>,
    /// Default external-reference scheme (master06 §"ID Namespace and Scheme").
    pub(crate) id_scheme: Option<String>,
    /// The `COMPOSITION.language` from `<root>/language|raw`, used when no
    /// `ctx/language` names one.
    pub(crate) language_phrase: Option<Obj>,
    /// The `COMPOSITION.territory` CODE_PHRASE a `<root>/territory|raw` value
    /// carries, used when no `ctx/territory` names one.
    pub(crate) territory_phrase: Option<Obj>,
}

impl CtxDefaults {
    /// The `EVENT_CONTEXT.setting`: the client's, else `238` "other care"
    /// (master06 §setting).
    ///
    /// # Errors
    /// [`FlatError::Rm`], never for these classes.
    pub(crate) fn setting(&self) -> Result<Obj, FlatError> {
        match &self.setting {
            Some(setting) => Ok(setting.clone_box()),
            None => coded_from_group("setting", "other care"),
        }
    }

    /// The `language` CODE_PHRASE in `ISO_639-1` (master06 §"Language and
    /// Territory") from `ctx/language`, else `<root>/language|raw`.
    ///
    /// # Errors
    /// [`FlatError::Rm`], never for these classes.
    pub(crate) fn language_code_phrase(&self) -> Result<Option<Obj>, FlatError> {
        phrase(
            "ISO_639-1",
            self.language.as_deref(),
            self.language_phrase.as_ref(),
        )
    }

    /// The `territory` CODE_PHRASE in `ISO_3166-1` from `ctx/territory`, else
    /// `<root>/territory|raw`.
    ///
    /// # Errors
    /// [`FlatError::Rm`], never for these classes.
    pub(crate) fn territory_code_phrase(&self) -> Result<Option<Obj>, FlatError> {
        phrase(
            "ISO_3166-1",
            self.territory.as_deref(),
            self.territory_phrase.as_ref(),
        )
    }
}

/// The CODE_PHRASE of `code` in `terminology`, else a copy of `raw`.
fn phrase(
    terminology: &str,
    code: Option<&str>,
    raw: Option<&Obj>,
) -> Result<Option<Obj>, FlatError> {
    match code {
        Some(c) => typed::code_phrase(terminology, c).map(Some),
        None => Ok(raw.map(|p| p.clone_box())),
    }
}

// ── sim → CtxDefaults ─────────────────────────────────────────────────────────

/// Resolves the `ctx` child; `now` is the `ctx/time` default (master06
/// §time). A missing language or territory is the caller's check.
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
        out.setting = Some(coded_from_group("setting", s)?);
    }
    if let Some(s) = bare_str("action_ism_transition_current_state") {
        out.action_ism_current_state = Some(coded_from_group("instruction_states", s)?);
    }

    // Compute into locals first — these read the `id_namespace`/`id_scheme`
    // defaults set above, so `out` must not be mutably reborrowed mid-read.
    let composer = resolve_composer(ctx, &out)?;
    let facility = resolve_facility(ctx, &out)?;
    let provider = resolve_provider(ctx, &out)?;
    let work_flow_id = resolve_workflow(ctx, &out)?;
    let participations = resolve_participations(ctx, &out)?;
    let links = resolve_links(ctx)?;
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
fn resolve_composer(ctx: &SimNode, d: &CtxDefaults) -> Result<Option<Obj>, FlatError> {
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
        let mut p = access::construct("PARTY_SELF")?;
        if let Some(id) = id {
            p.put("external_ref", o(party_ref(id, d, "PERSON")?))?;
        }
        return Ok(Some(p));
    }
    if name.is_none() && id.is_none() {
        return Ok(None);
    }
    party_identified(name, id, d, "PERSON").map(Some)
}

/// master06 §health_care_facility: a PARTY_IDENTIFIED (`|name`/`|id`).
fn resolve_facility(ctx: &SimNode, d: &CtxDefaults) -> Result<Option<Obj>, FlatError> {
    let Some(hcf) = ctx.child("health_care_facility") else {
        return Ok(None);
    };
    let name = hcf.attrs.get("name");
    let id = hcf.attrs.get("id").and_then(Value::as_str);
    if name.is_none() && id.is_none() {
        return Ok(None);
    }
    party_identified(name, id, d, "ORGANISATION").map(Some)
}

/// master06 §provider: a PARTY_IDENTIFIED from `provider_name`/`provider_id`.
fn resolve_provider(ctx: &SimNode, d: &CtxDefaults) -> Result<Option<Obj>, FlatError> {
    let name = ctx.child("provider_name").and_then(SimNode::bare);
    let id = ctx
        .child("provider_id")
        .and_then(SimNode::bare)
        .and_then(Value::as_str);
    if name.is_none() && id.is_none() {
        return Ok(None);
    }
    party_identified(name, id, d, "PERSON").map(Some)
}

/// master06 §"Workflow ID": an OBJECT_REF; `|id_scheme`/`|namespace` fall back to
/// `ctx/id_scheme` / `ctx/id_namespace`.
fn resolve_workflow(ctx: &SimNode, d: &CtxDefaults) -> Result<Option<Obj>, FlatError> {
    let Some(wf) = ctx.child("work_flow_id") else {
        return Ok(None);
    };
    let Some(id) = wf.attrs.get("id").and_then(Value::as_str) else {
        return Ok(None);
    };
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
    typed::generic_object_ref(ns, ty, id, scheme).map(Some)
}

/// master06 §link: `LOCATABLE.links` from `ctx/link:i|type`/`|meaning`/`|target`.
fn resolve_links(ctx: &SimNode) -> Result<Vec<Obj>, FlatError> {
    let Some(child) = ctx.children.get("link") else {
        return Ok(Vec::new());
    };
    child
        .occurrences
        .iter()
        .filter(|occ| !occ.is_empty())
        .map(|occ| {
            let text = |key: &str| occ.attrs.get(key).and_then(Value::as_str).unwrap_or("");
            typed::make(
                "LINK",
                [
                    ("type", o(typed::dv_text(text("type"))?)),
                    ("meaning", o(typed::dv_text(text("meaning"))?)),
                    (
                        "target",
                        o(typed::make("DV_EHR_URI", [("value", s(text("target")))])?),
                    ),
                ],
            )
        })
        .collect()
}

/// master06 §Participation: build a `PARTICIPATION` per index from the parallel
/// `participation_name`/`_function`/`_mode`/`_id`/`_identifiers` families.
fn resolve_participations(ctx: &SimNode, d: &CtxDefaults) -> Result<Vec<Obj>, FlatError> {
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

        // `PARTICIPATION.function` is 1..1 (`org.openehr.rm.common.participation.adoc`),
        // so a participation without one is refused by name.
        let Some(function) = function else {
            return Err(FlatError::MissingRequiredSuffix {
                key: format!("ctx/participation_function:{i}"),
            });
        };
        let mut p = typed::make(
            "PARTICIPATION",
            [("function", o(typed::dv_text(function)?))],
        )?;
        // The performer is a PARTY_IDENTIFIED (master06 §Participation); its
        // identifiers make it identified even without a name.
        let performer = if identifiers.is_empty() {
            party_identified(name, id, d, "PERSON")?
        } else {
            let mut performer = access::construct("PARTY_IDENTIFIED")?;
            if let Some(n) = name {
                performer.put_datum("name", n)?;
            }
            if let Some(id) = id {
                performer.put("external_ref", o(party_ref(id, d, "PERSON")?))?;
            }
            for identifier in identifiers {
                performer.push_obj("identifiers", identifier)?;
            }
            performer
        };
        p.put("performer", o(performer))?;
        if let Some(m) = mode {
            p.put("mode", o(coded_from_group("participation_mode", m)?))?;
        }
        out.push(p);
    }
    Ok(out)
}

/// The performer identifiers of one participation, in the compact or the
/// non-compact master06 form.
fn participation_identifiers(node: &SimNode, index: usize) -> Result<Vec<Obj>, FlatError> {
    // Compact bare form (master06 §Participation example, index 0).
    if let Some(compact) = node.bare().and_then(Value::as_str) {
        return parse_compact_identifiers(compact, index);
    }
    // Non-compact form: `|<field>:j` attrs grouped by `j`.
    let mut by_index: BTreeMap<u32, Vec<(&str, &Value)>> = BTreeMap::new();
    for (key, value) in &node.attrs {
        let Some((field, j)) = key.split_once(':') else {
            continue;
        };
        let Ok(j) = j.parse::<u32>() else { continue };
        if !matches!(field, "id" | "issuer" | "assigner" | "type") {
            continue;
        }
        by_index.entry(j).or_default().push((field, value));
    }
    by_index
        .into_values()
        .map(|fields| crate::flat::map::parties::identifier_from_fields(&fields))
        .collect()
}

/// Parses the compact `"issuer::assigner::id::TYPE;…"` list (master06
/// §Participation), four parts per entry.
fn parse_compact_identifiers(compact: &str, index: usize) -> Result<Vec<Obj>, FlatError> {
    let mut out = Vec::new();
    for entry in compact.split(';').filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = entry.split("::").collect();
        let [issuer, assigner, id, id_type] = parts.as_slice() else {
            return Err(FlatError::InvalidValue {
                path: format!("ctx/participation_identifiers:{index}"),
                reason: format!("compact identifier {entry:?} must be issuer::assigner::id::type"),
            });
        };
        out.push(typed::make(
            "DV_IDENTIFIER",
            [
                ("issuer", s(*issuer)),
                ("assigner", s(*assigner)),
                ("id", s(*id)),
                ("type", s(*id_type)),
            ],
        )?);
    }
    Ok(out)
}

/// A PARTY_IDENTIFIED, or PARTY_SELF with neither name nor id, honouring
/// `ctx/id_scheme` and `ctx/id_namespace` (master06 §Composer).
fn party_identified(
    name: Option<&Value>,
    id: Option<&str>,
    d: &CtxDefaults,
    party_type: &str,
) -> Result<Obj, FlatError> {
    match (name, id) {
        (_, Some(id)) => {
            let mut p = access::construct("PARTY_IDENTIFIED")?;
            if let Some(n) = name {
                p.put_datum("name", n)?;
            }
            p.put("external_ref", o(party_ref(id, d, party_type)?))?;
            Ok(p)
        }
        (Some(n), None) => {
            let mut p = access::construct("PARTY_IDENTIFIED")?;
            p.put_datum("name", n)?;
            Ok(p)
        }
        (None, None) => Ok(access::construct("PARTY_SELF")?),
    }
}

/// A PARTY_REF for `id`: a `GENERIC_ID` under a scheme, else a
/// `HIER_OBJECT_ID`.
fn party_ref(id: &str, d: &CtxDefaults, party_type: &str) -> Result<Obj, FlatError> {
    let id_obj = match d.id_scheme.as_deref() {
        Some(scheme) => typed::make("GENERIC_ID", [("value", s(id)), ("scheme", s(scheme))])?,
        None => typed::hier_object_id(id, "ctx/composer_id")?,
    };
    typed::make(
        "PARTY_REF",
        [
            ("id", o(id_obj)),
            ("namespace", s(d.id_namespace.as_deref().unwrap_or("EHR"))),
            ("type", s(party_type)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The canonical JSON of a built piece.
    fn v(object: &Obj) -> Value {
        typed::canonical_of(&**object).unwrap()
    }

    /// Build a `ctx` SimNode from FLAT-style `ctx/...`-relative pairs.
    /// `(child, index, suffix, value)` — suffix `""` is the bare value.
    fn ctx_node(entries: &[(&str, Option<u32>, &str, Value)]) -> SimNode {
        let mut ctx = SimNode::default();
        for (name, index, suffix, value) in entries {
            let child = ctx.occurrence_mut(name, *index);
            child.attrs.insert((*suffix).to_owned(), value.clone());
        }
        ctx
    }

    // master06 §time: `ctx/time` defaults to now when unset.
    #[test]
    fn time_defaults_to_now() {
        let d = resolve(None, "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(d.time, "2026-01-01T00:00:00Z");
        // master06 §setting: default "other care" resolves to openEHR 238.
        assert_eq!(
            v(&d.setting().unwrap())["defining_code"]["code_string"],
            json!("238")
        );
        assert_eq!(v(&d.setting().unwrap())["value"], json!("other care"));
    }

    // master06 §"unknown key" — a key outside the vocabulary is rejected.
    #[test]
    fn unknown_ctx_key_rejected() {
        let ctx = ctx_node(&[("nonsense", None, "", json!("x"))]);
        assert!(matches!(
            resolve(Some(&ctx), "now"),
            Err(FlatError::UnknownContext(_))
        ));
    }

    // master06 §Composer: composer_self → PARTY_SELF.
    #[test]
    fn composer_self() {
        let ctx = ctx_node(&[
            ("composer_self", None, "", json!(true)),
            ("composer_id", None, "", json!("123")),
            ("id_namespace", None, "", json!("HOSPITAL-NS")),
            ("id_scheme", None, "", json!("HOSPITAL-NS")),
        ]);
        let d = resolve(Some(&ctx), "now").unwrap();
        let composer = v(&d.composer.unwrap());
        assert_eq!(composer["_type"], json!("PARTY_SELF"));
        assert_eq!(composer["external_ref"]["id"]["value"], json!("123"));
        assert_eq!(
            composer["external_ref"]["id"]["scheme"],
            json!("HOSPITAL-NS")
        );
    }

    // master06 §Composer: composer_name → PARTY_IDENTIFIED.
    #[test]
    fn composer_name_identified() {
        let ctx = ctx_node(&[("composer_name", None, "", json!("Silvia Blake"))]);
        let composer = v(&resolve(Some(&ctx), "now").unwrap().composer.unwrap());
        assert_eq!(composer["_type"], json!("PARTY_IDENTIFIED"));
        assert_eq!(composer["name"], json!("Silvia Blake"));
    }

    // master06 §setting: a code is accepted and resolved to its value.
    #[test]
    fn setting_code_or_value() {
        let ctx = ctx_node(&[("setting", None, "", json!("238"))]);
        let d = resolve(Some(&ctx), "now").unwrap();
        assert_eq!(
            v(&d.setting().unwrap())["defining_code"]["code_string"],
            json!("238")
        );
        assert_eq!(v(&d.setting().unwrap())["value"], json!("other care"));
    }

    // master06 §action_ism_transition_current_state: value accepted.
    #[test]
    fn ism_current_state_from_value() {
        let ctx = ctx_node(&[(
            "action_ism_transition_current_state",
            None,
            "",
            json!("completed"),
        )]);
        let d = resolve(Some(&ctx), "now").unwrap();
        let ism = v(&d.action_ism_current_state.unwrap());
        assert_eq!(ism["value"], json!("completed"));
        assert_eq!(ism["defining_code"]["code_string"], json!("532"));
    }

    // master06 §Participation: the compact identifier form.
    #[test]
    fn participation_compact_identifiers() {
        let ctx = ctx_node(&[
            (
                "participation_name",
                Some(0),
                "",
                json!("Dr. Marcus Johnson"),
            ),
            ("participation_function", Some(0), "", json!("requester")),
            ("participation_id", Some(0), "", json!("199")),
            (
                "participation_identifiers",
                Some(0),
                "",
                json!("issuer1::assigner1::id1::PERSON;issuer2::assigner2::id2::PERSON"),
            ),
        ]);
        let d = resolve(Some(&ctx), "now").unwrap();
        assert_eq!(d.participations.len(), 1);
        let p = &v(&d.participations[0]);
        assert_eq!(p["function"]["value"], json!("requester"));
        assert_eq!(p["performer"]["identifiers"][0]["id"], json!("id1"));
        assert_eq!(p["performer"]["identifiers"][0]["issuer"], json!("issuer1"));
        assert_eq!(p["performer"]["identifiers"][1]["id"], json!("id2"));
    }

    // master06 §Participation: the non-compact `|issuer:j` form.
    #[test]
    fn participation_noncompact_identifiers() {
        let mut ctx = SimNode::default();
        let node = ctx.occurrence_mut("participation_identifiers", Some(1));
        node.attrs.insert("issuer:0".to_owned(), json!("issuer3"));
        node.attrs
            .insert("assigner:0".to_owned(), json!("assigner3"));
        node.attrs.insert("id:0".to_owned(), json!("id3"));
        node.attrs.insert("type:0".to_owned(), json!("PERSON"));
        ctx.occurrence_mut("participation_name", Some(1))
            .attrs
            .insert(String::new(), json!("Lara Markham"));
        // master06 §Participation's index-1 example carries `participation_function:1`.
        ctx.occurrence_mut("participation_function", Some(1))
            .attrs
            .insert(String::new(), json!("performer"));
        let d = resolve(Some(&ctx), "now").unwrap();
        // index 0 is an empty placeholder → one real participation at index 1.
        let with_ids: Vec<Value> = d
            .participations
            .iter()
            .map(v)
            .filter(|p| {
                p.get("performer")
                    .and_then(|pf| pf.get("identifiers"))
                    .is_some()
            })
            .collect();
        assert_eq!(with_ids.len(), 1);
        assert_eq!(
            with_ids[0]["performer"]["identifiers"][0]["id"],
            json!("id3")
        );
    }

    // master06 §Participation: a malformed compact list is rejected.
    #[test]
    fn participation_malformed_compact_rejected() {
        let ctx = ctx_node(&[(
            "participation_identifiers",
            Some(0),
            "",
            json!("issuer::onlytwo"),
        )]);
        assert!(matches!(
            resolve(Some(&ctx), "now"),
            Err(FlatError::InvalidValue { .. })
        ));
    }

    /// A participation without a function is refused by name.
    #[test]
    fn participation_without_function_rejected() {
        let partials = [
            ("participation_name", json!("Lara Markham")),
            ("participation_id", json!("199")),
            ("participation_mode", json!("face-to-face communication")),
            (
                "participation_identifiers",
                json!("issuer1::assigner1::id1::PERSON"),
            ),
        ];
        for (key, value) in partials {
            let ctx = ctx_node(&[(key, Some(0), "", value)]);
            let err = resolve(Some(&ctx), "now")
                .expect_err("a participation without its mandatory function is refused");
            assert!(
                matches!(&err, FlatError::MissingRequiredSuffix { key }
                         if key == "ctx/participation_function:0"),
                "should name the missing key, got {err:?}"
            );
        }
    }

    // The happy path of the same shape: the function is present, so the whole
    // participation resolves (master06 §Participation).
    #[test]
    fn participation_with_function_resolves() {
        let ctx = ctx_node(&[
            ("participation_name", Some(0), "", json!("Lara Markham")),
            ("participation_function", Some(0), "", json!("performer")),
        ]);
        let d = resolve(Some(&ctx), "now").unwrap();
        assert_eq!(d.participations.len(), 1);
        assert_eq!(
            v(&d.participations[0])["function"]["value"],
            json!("performer")
        );
        assert_eq!(
            v(&d.participations[0])["performer"]["name"],
            json!("Lara Markham")
        );
    }

    // master06 §"Workflow ID": scheme/namespace fall back to ctx defaults.
    #[test]
    fn workflow_id_falls_back_to_ctx_scheme() {
        let mut ctx = SimNode::default();
        ctx.occurrence_mut("id_scheme", None)
            .attrs
            .insert(String::new(), json!("HOSPITAL-NS"));
        ctx.occurrence_mut("id_namespace", None)
            .attrs
            .insert(String::new(), json!("HOSPITAL-NS"));
        let wf = ctx.occurrence_mut("work_flow_id", None);
        wf.attrs.insert("id".to_owned(), json!("567"));
        wf.attrs.insert("type".to_owned(), json!("ORGANISATION"));
        let d = resolve(Some(&ctx), "now").unwrap();
        let w = v(&d.work_flow_id.unwrap());
        assert_eq!(w["id"]["scheme"], json!("HOSPITAL-NS"));
        assert_eq!(w["namespace"], json!("HOSPITAL-NS"));
        assert_eq!(w["type"], json!("ORGANISATION"));
    }

    // emit → resolve round-trip for the bidirectional core.
    #[test]
    fn emit_roundtrips_core_context() {
        let comp = json!({
            "_type": "COMPOSITION",
            "language": {"_type": "CODE_PHRASE", "code_string": "en",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-1"}},
            "territory": {"_type": "CODE_PHRASE", "code_string": "US",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_3166-1"}},
            "composer": {"_type": "PARTY_IDENTIFIED", "name": "Silvia Blake"},
            "context": {"_type": "EVENT_CONTEXT",
                "start_time": {"_type": "DV_DATE_TIME", "value": "2021-12-21T14:19:31+01:00"},
                "setting": {"_type": "DV_CODED_TEXT", "value": "other care",
                    "defining_code": {"_type": "CODE_PHRASE", "code_string": "238",
                        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}}}}
        });
        let mut ctx = SimNode::default();
        crate::flat::ctx_emit::emit(&comp, &mut ctx);
        let d = resolve(Some(&ctx), "now").unwrap();
        assert_eq!(d.language.as_deref(), Some("en"));
        assert_eq!(d.territory.as_deref(), Some("US"));
        assert_eq!(d.time, "2021-12-21T14:19:31+01:00");
        assert_eq!(
            v(&d.setting().unwrap())["defining_code"]["code_string"],
            json!("238")
        );
        assert_eq!(
            v(&d.language_code_phrase().unwrap().unwrap())["code_string"],
            json!("en")
        );
        assert_eq!(v(&d.composer.unwrap())["name"], json!("Silvia Blake"));
    }
}
