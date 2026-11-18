// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ctx/` keys a COMPOSITION reads out as (ITS-REST
//! `simplified_formats/master06-context_information.adoc`): the reverse of
//! [`crate::flat::ctx::resolve`], over any [`Instance`].

#![expect(
    clippy::disallowed_types,
    reason = "a `ctx/` datum of the simplified wire is a JSON scalar (ITS-REST Simplified \
              Formats) (#1694, #6003)"
)]

use serde_json::{Value, json};

use crate::flat::instance::Instance;
use crate::flat::sim::SimNode;

/// Derive the `ctx/` keys from a canonical COMPOSITION onto `out` (the `ctx`
/// SimNode). The bidirectional shortcuts (master06 §§"Language and Territory",
/// Composer, time, setting, location, health_care_facility, Participation) are
/// symmetric with [`resolve`]; the per-ENTRY input-only defaults (provider,
/// workflow id, ISM state, timing, narrative, links) keep their structural home
/// on output and are not emitted here (master06 frames them as input shortcuts).
pub(crate) fn emit<'a, I: Instance<'a>>(composition: I, out: &mut SimNode) {
    if let Some(code) = composition
        .attr("language")
        .and_then(|l| l.attr("code_string"))
        .and_then(Instance::to_value)
    {
        set_bare(out, "language", code);
    }
    if let Some(code) = composition
        .attr("territory")
        .and_then(|t| t.attr("code_string"))
        .and_then(Instance::to_value)
    {
        set_bare(out, "territory", code);
    }
    match composition.attr("composer") {
        // NOTE: master06 §Composer has no reference-type shortcut, so such a composer is
        // written as `composer|raw` by the flattener (master04 §Raw canonical JSON).
        Some(c) if !crate::flat::map::parties::ref_type_survives(c, "PERSON") => {}
        Some(c) if c.rm_type() == Some("PARTY_SELF") => {
            set_bare(out, "composer_self", json!(true));
            emit_external_ref(c, out);
        }
        Some(c) => {
            if let Some(name) = c.attr("name").and_then(Instance::to_value) {
                set_bare(out, "composer_name", name);
            }
            emit_external_ref(c, out);
        }
        None => {}
    }

    let context = composition.attr("context");
    let at = |steps: &[&str]| context.and_then(|c| crate::flat::instance::at(c, steps));
    if let Some(t) = at(&["start_time", "value"]).and_then(Instance::to_value) {
        set_bare(out, "time", t);
    }
    if let Some(t) = at(&["end_time", "value"]).and_then(Instance::to_value) {
        set_bare(out, "end_time", t);
    }
    // master06 §setting: emit the code (round-trips through `resolve`, which
    // accepts either a code or a value).
    if let Some(code) =
        at(&["setting", "defining_code", "code_string"]).and_then(Instance::to_value)
    {
        set_bare(out, "setting", code);
    }
    if let Some(loc) = at(&["location"]).and_then(Instance::to_value) {
        set_bare(out, "location", loc);
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
fn emit_external_ref<'a, I: Instance<'a>>(party: I, out: &mut SimNode) {
    let reference = party.attr("external_ref");
    let at = |steps: &[&str]| {
        reference
            .and_then(|r| crate::flat::instance::at(r, steps))
            .and_then(Instance::as_str)
            .filter(|s| !s.is_empty())
    };
    let Some(id) = at(&["id", "value"]) else {
        return;
    };
    set_bare(out, "composer_id", json!(id));
    if let Some(ns) = at(&["namespace"]) {
        set_bare(out, "id_namespace", json!(ns));
    }
    if let Some(scheme) = at(&["id", "scheme"]) {
        set_bare(out, "id_scheme", json!(scheme));
    }
}

fn set_bare(out: &mut SimNode, name: &str, value: Value) {
    out.occurrence_mut(name, None)
        .attrs
        .insert(String::new(), value);
}
