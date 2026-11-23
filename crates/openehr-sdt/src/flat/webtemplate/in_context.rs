// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The in-context children of the Web Template (master04 §"Web Template
//! Metadata", the `inContext` marker), run after [`super::shape::compact`].

use super::model::{WebTemplateInput, WebTemplateInputType, WebTemplateNode};

fn is_entry_family(rm_type: &str) -> bool {
    matches!(
        rm_type,
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY" | "GENERIC_ENTRY"
    )
}

fn is_event_family(rm_type: &str) -> bool {
    matches!(rm_type, "EVENT" | "POINT_EVENT" | "INTERVAL_EVENT")
}

/// Synthesize the in-context RM children the master04 §"Web Template Metadata"
/// example carries and an OPT commonly leaves unconstrained, so their simplified
/// path keys resolve: COMPOSITION `context`, `category`, `language`,
/// `territory`, `composer`; ENTRY `language`, `encoding`, `subject`; EVENT
/// `time`. An existing child at the same `aqlPath` wins.
///
/// NOTE: the master04 example is the shape oracle; its unmarked `context`
/// wrapper is reproduced.
pub(super) fn synthesize_in_context(node: &mut WebTemplateNode) {
    for child in &mut node.children {
        synthesize_in_context(child);
    }
    synth_collapsed_event(node);
    if node.rm_type == "COMPOSITION" {
        synth_composition(node);
    } else if is_entry_family(&node.rm_type) {
        synth_entry(node);
    } else if is_event_family(&node.rm_type) {
        synth_event(node);
    }
}

/// The COMPOSITION-level in-context children, in the example's order; a
/// template that prohibits `context` gets none of them.
fn synth_composition(node: &mut WebTemplateNode) {
    if node.prohibited_attributes.iter().any(|a| a == "context") {
        node.children.retain(|c| c.aql_path != "/context");
    } else {
        ensure_context(node);
    }
    ensure_child(
        node,
        "/category",
        "DV_CODED_TEXT",
        None,
        category_inputs(),
        true,
    );
    ensure_child(
        node,
        "/language",
        "CODE_PHRASE",
        Some("Language"),
        vec![],
        false,
    );
    ensure_child(
        node,
        "/territory",
        "CODE_PHRASE",
        Some("Territory"),
        vec![],
        false,
    );
    ensure_child(
        node,
        "/composer",
        "PARTY_PROXY",
        Some("Composer"),
        party_inputs(),
        false,
    );
}

/// The COMPOSITION `context` node, found or created and placed first, with its
/// `start_time` and `setting` leaves.
///
/// A created wrapper's `min` is the template's existence, else the RM's `0..1`
/// (`org.openehr.rm.composition.composition.adoc`; ADL2 `master04.3` §Existence).
fn ensure_context(node: &mut WebTemplateNode) {
    let idx = if let Some(i) = node.children.iter().position(|c| c.aql_path == "/context") {
        i
    } else {
        let mut ctx = WebTemplateNode::new("EVENT_CONTEXT".to_owned(), "/context".to_owned());
        // The example gives the context wrapper `nodeId: ""` and no `inContext`
        // marker; its leaf children carry `inContext: true`.
        ctx.node_id = Some(String::new());
        ctx.min = Some(i32::from(
            node.existence
                .iter()
                .any(|ex| ex.path == "/context" && ex.min >= 1),
        ));
        ctx.max = 1;
        node.children.insert(0, ctx);
        0
    };
    let Some(ctx) = node.children.get_mut(idx) else {
        return;
    };
    ensure_child(
        ctx,
        "/context/start_time",
        "DV_DATE_TIME",
        Some("Start_time"),
        datetime_inputs(),
        false,
    );
    ensure_child(
        ctx,
        "/context/setting",
        "DV_CODED_TEXT",
        Some("Setting"),
        setting_inputs(),
        false,
    );
}

/// The ENTRY-level in-context children (master04 §"Web Template Metadata":
/// `language`, `encoding`, `subject` on every ENTRY node).
fn synth_entry(node: &mut WebTemplateNode) {
    let base = node.aql_path.clone();
    ensure_child(
        node,
        &format!("{base}/language"),
        "CODE_PHRASE",
        Some("Language"),
        vec![],
        false,
    );
    ensure_child(
        node,
        &format!("{base}/encoding"),
        "CODE_PHRASE",
        Some("Encoding"),
        vec![],
        false,
    );
    ensure_child(
        node,
        &format!("{base}/subject"),
        "PARTY_PROXY",
        Some("Subject"),
        party_inputs(),
        false,
    );
}

/// The EVENT-level in-context `time`.
// NOTE: master04 §"Web Template Metadata" renders an EVENT with one inContext
// child; `width` and the rest stay master05 datum paths.
fn synth_event(node: &mut WebTemplateNode) {
    let base = node.aql_path.clone();
    ensure_child(
        node,
        &format!("{base}/time"),
        "DV_DATE_TIME",
        Some("Time"),
        datetime_inputs(),
        false,
    );
}

/// The `time` of a single EVENT the compactor collapsed into `node` (master04
/// §"Conditionally Collapsed Wrapper Types"), at the event's own `aqlPath`.
fn synth_collapsed_event(node: &mut WebTemplateNode) {
    let events: Vec<String> = node
        .slots
        .iter()
        .filter(|slot| is_event_family(&slot.rm_type))
        .map(|slot| slot.path.clone())
        .collect();
    for event in events {
        ensure_child(
            node,
            &format!("{event}/time"),
            "DV_DATE_TIME",
            Some("Time"),
            datetime_inputs(),
            false,
        );
    }
}

/// Append an in-context leaf child at `aql_path` (min 1 / max 1, `inContext:
/// true`) unless a child already occupies that `aqlPath`. `empty_node_id`
/// emits `nodeId: ""` (the master04 shape for `category`).
fn ensure_child(
    parent: &mut WebTemplateNode,
    aql_path: &str,
    rm_type: &str,
    name: Option<&str>,
    inputs: Vec<WebTemplateInput>,
    empty_node_id: bool,
) {
    if parent.children.iter().any(|c| c.aql_path == aql_path) {
        return;
    }
    let mut n = WebTemplateNode::new(rm_type.to_owned(), aql_path.to_owned());
    n.min = Some(1);
    n.max = 1;
    n.in_context = Some(true);
    if let Some(name) = name {
        n.name = Some(name.to_owned());
        n.localized_name = Some(name.to_owned());
    }
    if empty_node_id {
        n.node_id = Some(String::new());
    }
    n.inputs = inputs;
    parent.children.push(n);
}

/// The single `DATETIME` input of a synthesized `start_time`/EVENT `time` leaf.
fn datetime_inputs() -> Vec<WebTemplateInput> {
    vec![WebTemplateInput::new(WebTemplateInputType::Datetime, None)]
}

/// The `code`/`value` TEXT inputs of a synthesized `setting` leaf (master04
/// §"Web Template Metadata": the `setting` node's `inputs`).
fn setting_inputs() -> Vec<WebTemplateInput> {
    vec![
        WebTemplateInput::new(WebTemplateInputType::Text, Some("code")),
        WebTemplateInput::new(WebTemplateInputType::Text, Some("value")),
    ]
}

/// The four PARTY_PROXY TEXT inputs of a synthesized `composer`/`subject` leaf
/// (master04 §"Web Template Metadata": `id`, `id_scheme`, `id_namespace`,
/// `name`).
fn party_inputs() -> Vec<WebTemplateInput> {
    ["id", "id_scheme", "id_namespace", "name"]
        .into_iter()
        .map(|s| WebTemplateInput::new(WebTemplateInputType::Text, Some(s)))
        .collect()
}

/// The single coded `code` input of a synthesized COMPOSITION `category` leaf,
/// carrying the openEHR `433`/`event` coded value verbatim from the master04
/// §"Web Template Metadata" example (localized label `en: event`).
fn category_inputs() -> Vec<WebTemplateInput> {
    let mut cv = super::model::WebTemplateCodedValue::new("433", Some("event".to_owned()));
    cv.localized_labels
        .insert("en".to_owned(), "event".to_owned());
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = vec![cv];
    input.terminology = Some("openehr".to_owned());
    vec![input]
}
