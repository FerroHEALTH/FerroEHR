// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ctx/` defaults a COMPOSITION build applies (ITS-REST
//! `simplified_formats/master06-context_information.adoc`): the composition
//! language, territory, composer and EVENT_CONTEXT, the root links, and the
//! per-entry defaults the build's completion walk applies node by node.

use openehr_rm::v1_2::access;
use openehr_rm::v1_2::access::{RmObject, RmRef};

use crate::flat::build::{DEFAULT_TIME, is_entry_family};
use crate::flat::ctx;
use crate::flat::error::FlatError;
use crate::flat::typed::{self, Build, o, s};

/// Apply the composition-level context (`master06`: language, territory,
/// composer, and the EVENT_CONTEXT when warranted).
pub(crate) fn apply_composition_ctx(
    comp: &mut dyn RmObject,
    defaults: &ctx::CtxDefaults,
    explicit_event_context: bool,
) -> Result<(), FlatError> {
    if !comp.is_set("language")
        && let Some(language) = defaults.language_code_phrase()?
    {
        comp.put("language", o(language))?;
    }
    if !comp.is_set("territory")
        && let Some(territory) = defaults.territory_code_phrase()?
    {
        comp.put("territory", o(territory))?;
    }
    if let Some(composer) = &defaults.composer {
        comp.or_put("composer", || Ok(o(composer.clone_box())))?;
    }
    // An EVENT_CONTEXT is built only when the client expressed event-context
    // content (explicit ctx/ keys per master06, or archetyped other_context
    // tree data) — never fabricated from the category alone. NOTE: no openEHR
    // spec mandates synthesizing a context; the master06 time/setting
    // defaults apply when a context is being expressed, and round-trip
    // stability requires not inventing one (RM ehr master05 §"Persistent
    // Compositions may optionally have an Event context").
    if !explicit_event_context && !comp.is_set("context") {
        return Ok(());
    }
    comp.or_put("context", || Ok(o(access::construct("EVENT_CONTEXT")?)))?;
    let Some(context) = comp.object_mut("context") else {
        return Ok(());
    };
    context.or_put("start_time", || Ok(o(typed::dv_date_time(&defaults.time)?)))?;
    context.or_put("setting", || Ok(o(defaults.setting()?)))?;
    if let Some(end_time) = &defaults.end_time {
        context.or_put("end_time", || Ok(o(typed::dv_date_time(end_time)?)))?;
    }
    if let Some(location) = &defaults.location {
        context.or_put("location", || Ok(s(location.as_str())))?;
    }
    if let Some(facility) = &defaults.health_care_facility {
        context.or_put("health_care_facility", || Ok(o(facility.clone_box())))?;
    }
    if !defaults.participations.is_empty() && !context.is_set("participations") {
        for participation in &defaults.participations {
            context.push_obj("participations", participation.clone_box())?;
        }
    }
    Ok(())
}

/// The `ctx/link` LOCATABLE links on the root (master06 §link); the
/// per-entry defaults follow in [`crate::flat::build`]'s completion walk.
pub(crate) fn apply_root_links(
    root: &mut dyn RmObject,
    defaults: &ctx::CtxDefaults,
) -> Result<(), FlatError> {
    if !defaults.links.is_empty() && !root.is_set("links") {
        for link in &defaults.links {
            root.push_obj("links", link.clone_box())?;
        }
    }
    Ok(())
}

/// The per-node `ctx/` defaults of [`crate::flat::build`]'s completion walk's walk: the ENTRY,
/// OBSERVATION, ACTION, INSTRUCTION and ACTIVITY defaults of master06.
pub(crate) fn apply_node_defaults(
    obj: &mut dyn RmObject,
    defaults: &ctx::CtxDefaults,
) -> Result<(), FlatError> {
    let rm_type = obj.rm_type();
    apply_common_entry_defaults(obj, rm_type, defaults)?;
    match rm_type {
        "OBSERVATION" => apply_observation_defaults(obj, defaults),
        "ACTION" => apply_action_defaults(obj, defaults),
        "INSTRUCTION" => match &defaults.instruction_narrative {
            Some(narrative) => obj.or_put("narrative", || Ok(o(typed::dv_text(narrative)?))),
            None => Ok(()),
        },
        "ACTIVITY" => match &defaults.activity_timing {
            Some(timing) => obj.or_put("timing", || {
                Ok(o(typed::make(
                    "DV_PARSABLE",
                    [("value", s(timing.as_str())), ("formalism", s("timing"))],
                )?))
            }),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// The defaults every ENTRY subtype takes: language, encoding, provider and
/// workflow id.
///
/// NOTE: master06 §Participation also names `ENTRY.other_participations`, but
/// master05's explicit `_other_participation:i` path form wins, so ctx lands
/// here only.
fn apply_common_entry_defaults(
    obj: &mut dyn RmObject,
    rm_type: &str,
    defaults: &ctx::CtxDefaults,
) -> Result<(), FlatError> {
    if !is_entry_family(rm_type) {
        return Ok(());
    }
    if !obj.is_set("language")
        && let Some(language) = defaults.language_code_phrase()?
    {
        obj.put("language", o(language))?;
    }
    obj.or_put("encoding", || {
        Ok(o(typed::code_phrase("IANA_character-sets", "UTF-8")?))
    })?;
    if let Some(provider) = &defaults.provider {
        obj.or_put("provider", || Ok(o(provider.clone_box())))?;
    }
    if let Some(wf) = &defaults.work_flow_id {
        obj.or_put("workflow_id", || Ok(o(wf.clone_box())))?;
    }
    Ok(())
}

/// The OBSERVATION defaults: the HISTORY origin, and each EVENT's time, which
/// defaults to that origin (master06 §time).
///
/// An origin the input does not set is the `ctx/history_origin` default, else
/// the time of the earliest event that states one ("will be set to the time of
/// the earliest event if not explicitly set", master05 §OBSERVATION
/// `/history_origin`), else the composition's context time.
fn apply_observation_defaults(
    obj: &mut dyn RmObject,
    defaults: &ctx::CtxDefaults,
) -> Result<(), FlatError> {
    let Some(history) = obj.object_mut("data") else {
        return Ok(());
    };
    let origin = defaults
        .history_origin
        .clone()
        .or_else(|| earliest_event_time(history))
        .unwrap_or_else(|| defaults.time.clone());
    if !history.is_set("origin")
        || typed::str_at(history, &["origin", "value"]) == Some(DEFAULT_TIME)
    {
        history.put("origin", o(typed::dv_date_time(&origin)?))?;
    }
    let Some(RmRef::Object(origin_now)) = history.attribute("origin") else {
        return Ok(());
    };
    let origin_now = origin_now.clone_box();
    let Some(events) = history.items_mut("events") else {
        return Ok(());
    };
    for ev in events {
        if !ev.is_set("time") || typed::str_at(ev, &["time", "value"]) == Some(DEFAULT_TIME) {
            ev.put("time", o(origin_now.clone_box()))?;
        }
    }
    Ok(())
}

/// The earliest time an EVENT of `history` states, as written; `None` when no
/// event states a time this reader can order.
///
/// Times are compared as instants: each is read as an ISO 8601 extended date
/// time with its offset, and one that does not read leaves the comparison.
fn earliest_event_time(history: &dyn RmObject) -> Option<String> {
    history
        .items("events")?
        .filter_map(|event| typed::str_at(event, &["time", "value"]))
        .filter(|time| *time != DEFAULT_TIME)
        .filter_map(|time| super::instant::instant(time).map(|at| (at, time)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, time)| time.to_owned())
}

/// The ACTION defaults: the action time and the ISM transition's current
/// state.
fn apply_action_defaults(
    obj: &mut dyn RmObject,
    defaults: &ctx::CtxDefaults,
) -> Result<(), FlatError> {
    let time = defaults
        .action_time
        .clone()
        .unwrap_or_else(|| defaults.time.clone());
    if !obj.is_set("time") || typed::str_at(obj, &["time", "value"]) == Some(DEFAULT_TIME) {
        obj.put("time", o(typed::dv_date_time(&time)?))?;
    }
    let Some(state) = &defaults.action_ism_current_state else {
        return Ok(());
    };
    obj.or_put("ism_transition", || {
        Ok(o(access::construct("ISM_TRANSITION")?))
    })?;
    if let Some(ism) = obj.object_mut("ism_transition") {
        ism.or_put("current_state", || Ok(o(state.clone_box())))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::typed::Obj;

    fn event(time: Option<&str>) -> Obj {
        let mut ev = access::construct("POINT_EVENT").unwrap();
        if let Some(time) = time {
            ev.put("time", o(typed::dv_date_time(time).unwrap()))
                .unwrap();
        }
        ev
    }

    fn observation(events: Vec<Obj>) -> Obj {
        let mut history = access::construct("HISTORY").unwrap();
        for ev in events {
            history.push_obj("events", ev).unwrap();
        }
        typed::make("OBSERVATION", [("data", o(history))]).unwrap()
    }

    fn event_time(obs: &dyn RmObject, i: usize) -> Option<String> {
        let RmRef::Object(history) = obs.attribute("data")? else {
            return None;
        };
        let ev = history.items("events")?.nth(i)?;
        typed::str_at(ev, &["time", "value"]).map(str::to_owned)
    }

    /// master05 §OBSERVATION `/history_origin`: an origin the input does not
    /// set is "the time of the earliest event", compared as instants, not as
    /// text.
    #[test]
    fn an_unset_history_origin_is_the_earliest_event_time() {
        let mut obs = observation(vec![
            event(Some("2026-03-02T10:00:00+01:00")),
            event(Some("2026-03-02T08:30:00Z")),
            event(None),
        ]);
        let defaults = ctx::CtxDefaults {
            time: "2026-03-05T12:00:00Z".to_owned(),
            ..ctx::CtxDefaults::default()
        };
        apply_observation_defaults(&mut *obs, &defaults).unwrap();
        assert_eq!(
            typed::str_at(&*obs, &["data", "origin", "value"]),
            Some("2026-03-02T08:30:00Z")
        );
        assert_eq!(
            event_time(&*obs, 2).as_deref(),
            Some("2026-03-02T08:30:00Z"),
            "an event without a time takes the origin"
        );

        let mut none = observation(Vec::new());
        apply_observation_defaults(&mut *none, &defaults).unwrap();
        assert_eq!(
            typed::str_at(&*none, &["data", "origin", "value"]),
            Some("2026-03-05T12:00:00Z"),
            "with no event time the context time stands"
        );
    }
}
