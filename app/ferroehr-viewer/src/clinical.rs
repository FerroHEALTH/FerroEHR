// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Template-free rendering of a canonical openEHR JSON document into a
//! human-readable tree.
//!
//! Section headings for the structural RM nodes, label/value rows for the
//! `ELEMENT` leaves, and honest formatting for each `DV_*` data value.
//!
//! It is **template-free on purpose**: the viewer never needs the operational
//! template to show a document, so any composition — including one whose
//! template was since removed — reads the same way. It is also a pure,
//! deterministic function of the document text (no clock, no locale, no
//! network), which is what makes the pane hydration-safe, and a READ-ONLY
//! projection: nothing is stored, nothing is sent anywhere.
//!
//! Every hard-coded attribute name below is the RM attribute of the same name.
//! The attribute tables are cited at their definitions; the vendored class
//! tables are `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.*.adoc`,
//! with the narrative in `docs/specs/openehr/RM/docs/ehr/`,
//! `.../data_structures/` and `.../data_types/`.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde_json::Value;

/// One node of the rendered document.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderedNode {
    /// A structural RM node rendered as a titled section.
    Section(RenderedSection),
    /// A leaf rendered as a label/value row.
    Row(RenderedRow),
}

impl RenderedNode {
    /// The node's RM path key — unique within a document, derived from the
    /// walked attribute names and list indices, and therefore a stable `<For>`
    /// key.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Section(section) => &section.key,
            Self::Row(row) => &row.key,
        }
    }
}

/// A structural RM node: its heading, its RM type, its archetype node id and
/// its children.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedSection {
    /// The RM path of this node (the `<For>` key).
    pub key: String,
    /// The heading: `name.value` where the node has one (`LOCATABLE._name_`),
    /// otherwise the humanized attribute or RM type name.
    pub title: String,
    /// The `_type` of the node, shown as a small type chip.
    pub rm_type: String,
    /// `LOCATABLE._archetype_node_id_`, where present.
    pub archetype_node_id: Option<String>,
    /// The nodes below this one, in RM attribute order.
    pub children: Vec<RenderedNode>,
}

/// A leaf: one label, one rendered value, and the terminology code behind it
/// where the value was coded.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedRow {
    /// The RM path of this leaf (the `<For>` key).
    pub key: String,
    /// The leaf's label: `name.value` for an `ELEMENT`, otherwise the humanized
    /// RM attribute name.
    pub label: String,
    /// The rendered value (see `format_value` for the per-type shapes).
    pub value: String,
    /// `terminology::code` for a coded value, so the code is visible without
    /// leaving the rendered view.
    pub code: Option<String>,
}

/// Render `body` as a clinical document, or `None` when it is not a JSON
/// object carrying a `_type` (an XML body, a FLAT body, an AQL string, a
/// non-RM JSON payload).
///
/// The pane hides the rendered mode in that case.
///
/// The walk below recurses with the document; it cannot recurse unboundedly
/// because `serde_json` refuses to parse beyond its own nesting limit (128
/// levels — <https://docs.rs/serde_json/1/serde_json/#recursion-limit>), so an
/// absurdly nested body fails at the parse instead.
#[must_use]
pub fn render(body: &str) -> Option<RenderedSection> {
    let document: Value = serde_json::from_str(body).ok()?;
    type_of(&document)?;
    Some(section(&document, None, String::new()))
}

/// The leaf attributes surfaced as label/value rows at the head of a section,
/// per RM type — the facts a reader wants before the content, in RM order.
///
/// Spec (attribute names, one file per class under
/// `docs/specs/openehr/RM/docs/UML/classes/`):
/// `COMPOSITION._composer_` (`org.openehr.rm.composition.composition.adoc`);
/// `EVENT_CONTEXT._start_time_`/`_end_time_`/`_setting_`/`_location_`/
/// `_health_care_facility_` (`...composition.event_context.adoc`);
/// `INSTRUCTION._narrative_`/`_expiry_time_` (`...composition.instruction.adoc`);
/// `ACTION._time_`/`_ism_transition_` (`...composition.action.adoc`);
/// `ISM_TRANSITION._current_state_`/`_transition_`/`_careflow_step_`
/// (`...composition.ism_transition.adoc`);
/// `ACTIVITY._timing_`/`_action_archetype_id_` (`...composition.activity.adoc`);
/// `HISTORY._origin_`/`_period_`/`_duration_` (`...data_structures.history.adoc`);
/// `EVENT._time_` (`...data_structures.event.adoc`), `INTERVAL_EVENT._width_`
/// (`...data_structures.interval_event.adoc`); `EHR_STATUS._subject_`/
/// `_is_queryable_`/`_is_modifiable_` (RM ehr `master04-ehr_package.adoc`
/// §`EHR_STATUS`).
fn header_attributes(rm_type: &str) -> &'static [&'static str] {
    match rm_type {
        "COMPOSITION" => &["composer"],
        "EVENT_CONTEXT" => &[
            "start_time",
            "end_time",
            "setting",
            "location",
            "health_care_facility",
        ],
        "INSTRUCTION" => &["narrative", "expiry_time"],
        // `ACTION._time_` and `EVENT._time_` are the same attribute name.
        "ACTION" | "EVENT" | "POINT_EVENT" => &["time"],
        "ISM_TRANSITION" => &["current_state", "transition", "careflow_step"],
        "ACTIVITY" => &["timing", "action_archetype_id"],
        "HISTORY" => &["origin", "period", "duration"],
        "INTERVAL_EVENT" => &["time", "width", "math_function"],
        "EHR_STATUS" => &["subject", "is_queryable", "is_modifiable"],
        _ => &[],
    }
}

/// The attributes walked as structure, per RM type, in RM declaration order.
///
/// Spec (same class files as [`header_attributes`]):
/// `COMPOSITION._context_`/`_content_`; `EVENT_CONTEXT._other_context_`;
/// `SECTION._items_` (`...composition.section.adoc`);
/// `OBSERVATION._data_`/`_state_` + `CARE_ENTRY._protocol_`
/// (`...composition.observation.adoc`, `...composition.care_entry.adoc`);
/// `EVALUATION._data_`, `ADMIN_ENTRY._data_`, `INSTRUCTION._activities_`,
/// `ACTION._description_`/`_ism_transition_`; `HISTORY._events_`/`_summary_`;
/// `EVENT._data_`/`_state_`; `ITEM_TREE._items_`/`ITEM_LIST._items_`/
/// `CLUSTER._items_`/`ITEM_TABLE._rows_`/`ITEM_SINGLE._item_`
/// (`...data_structures.item_tree.adoc` and siblings);
/// `EHR_STATUS._other_details_`; `FOLDER._items_`/`_folders_` (RM common
/// `master05-directory_package.adoc`).
fn child_attributes(rm_type: &str) -> &'static [&'static str] {
    match rm_type {
        "COMPOSITION" => &["context", "content"],
        "EVENT_CONTEXT" => &["other_context"],
        // `SECTION`, the `ITEM_STRUCTURE` list variants and `CLUSTER` all name
        // their children `items`.
        "SECTION" | "ITEM_TREE" | "ITEM_LIST" | "CLUSTER" => &["items"],
        "OBSERVATION" => &["data", "state", "protocol"],
        "EVALUATION" | "ADMIN_ENTRY" | "GENERIC_ENTRY" => &["data", "protocol"],
        "INSTRUCTION" => &["activities", "protocol"],
        "ACTIVITY" => &["description"],
        "ACTION" => &[
            "description",
            "ism_transition",
            "instruction_details",
            "protocol",
        ],
        "HISTORY" => &["events", "summary"],
        "EVENT" | "POINT_EVENT" | "INTERVAL_EVENT" => &["data", "state"],
        "ITEM_TABLE" => &["rows"],
        "ITEM_SINGLE" => &["item"],
        "EHR_STATUS" => &["other_details"],
        "FOLDER" => &["items", "folders"],
        _ => &[],
    }
}

/// RM housekeeping the clinical view folds away when walking a type it has no
/// table for: identity/meta attributes that carry no clinical reading. The raw
/// modes still show every one of them verbatim.
///
/// `_type`, `name`, `archetype_node_id`, `archetype_details`, `uid`, `links`,
/// `feeder_audit` are `LOCATABLE`/`ARCHETYPED` meta
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`);
/// `language`, `territory`, `category` are `COMPOSITION` bookkeeping and
/// `encoding`/`subject`/`provider`/`other_participations`/`workflow_id` are
/// `ENTRY` bookkeeping
/// (`...composition.composition.adoc`, `...composition.entry.adoc`).
fn is_folded(attribute: &str) -> bool {
    matches!(
        attribute,
        "_type"
            | "name"
            | "archetype_node_id"
            | "archetype_details"
            | "uid"
            | "links"
            | "feeder_audit"
            | "language"
            | "territory"
            | "category"
            | "encoding"
            | "subject"
            | "provider"
            | "other_participations"
            | "workflow_id"
    )
}

/// Whether an RM type is a value object rendered as one row rather than a
/// section: every `DV_*` data value plus the small identifier/party types
/// (RM `data_types`, and `common.party_proxy`/`base_types` id classes).
fn is_leaf_type(rm_type: &str) -> bool {
    rm_type.starts_with("DV_")
        || matches!(
            rm_type,
            "CODE_PHRASE"
                | "PARTY_SELF"
                | "PARTY_IDENTIFIED"
                | "PARTY_RELATED"
                | "PARTY_REF"
                | "OBJECT_REF"
                | "LOCATABLE_REF"
                | "HIER_OBJECT_ID"
                | "OBJECT_VERSION_ID"
                | "TERMINOLOGY_ID"
                | "ARCHETYPE_ID"
                | "TEMPLATE_ID"
                | "GENERIC_ID"
                | "INTERNET_ID"
                | "UUID"
                | "ISO_OID"
        )
}

/// Build the section for one RM object.
fn section(value: &Value, label: Option<&str>, key: String) -> RenderedSection {
    let rm_type = type_of(value).unwrap_or_default().to_owned();
    let title = name_of(value)
        .or_else(|| label.map(humanize))
        .unwrap_or_else(|| humanize(&rm_type));
    let mut children = Vec::new();
    // ARCHETYPED._template_id_ — the template a document was built from is the
    // one piece of `archetype_details` worth reading here
    // (docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.archetyped.adoc).
    if let Some(template) = nested_str(value, &["archetype_details", "template_id", "value"]) {
        children.push(RenderedNode::Row(RenderedRow {
            key: format!("{key}/template_id"),
            label: "template".to_owned(),
            value: template,
            code: None,
        }));
    }
    push_section_children(&mut children, value, &rm_type, &key);
    RenderedSection {
        key,
        title,
        rm_type,
        archetype_node_id: value
            .get("archetype_node_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        children,
    }
}

/// Walks the attributes one RM object contributes to its section.
///
/// A type the display tables cover walks its declared header and container
/// attributes, in that order. A type with no table (a CONTRIBUTION, a VERSION,
/// a demographic PARTY, an extension payload) walks what the document actually
/// carries, minus the folded meta.
fn push_section_children(out: &mut Vec<RenderedNode>, value: &Value, rm_type: &str, key: &str) {
    let headers = header_attributes(rm_type);
    let containers = child_attributes(rm_type);
    if headers.is_empty() && containers.is_empty() {
        let Some(map) = value.as_object() else {
            return;
        };
        for (attribute, child) in map {
            if !is_folded(attribute) {
                push_attribute(out, attribute, child, key);
            }
        }
        return;
    }
    for attribute in headers.iter().chain(containers) {
        if let Some(child) = value.get(*attribute) {
            push_attribute(out, attribute, child, key);
        }
    }
}

/// Walk one attribute of an RM object, expanding a list into one child per
/// item (the index goes into the path key, keeping keys unique).
fn push_attribute(out: &mut Vec<RenderedNode>, attribute: &str, value: &Value, parent: &str) {
    if let Value::Array(items) = value {
        for (index, item) in items.iter().enumerate() {
            let key = format!("{parent}/{attribute}[{index}]");
            out.push(node(item, attribute, key));
        }
    } else {
        let key = format!("{parent}/{attribute}");
        out.push(node(value, attribute, key));
    }
}

/// Decide whether one attribute value reads as a row or a section.
fn node(value: &Value, attribute: &str, key: String) -> RenderedNode {
    match type_of(value) {
        // ELEMENT is the leaf variant of ITEM, carrying one DATA_VALUE
        // (docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc).
        Some("ELEMENT") => RenderedNode::Row(element_row(value, attribute, key)),
        Some(rm_type) if is_leaf_type(rm_type) => {
            let (text, code) = format_value(value);
            RenderedNode::Row(RenderedRow {
                key,
                label: name_of(value).unwrap_or_else(|| humanize(attribute)),
                value: text,
                code,
            })
        }
        Some(_) => RenderedNode::Section(section(value, Some(attribute), key)),
        None => {
            if value.is_object() || value.is_array() {
                RenderedNode::Section(section(value, Some(attribute), key))
            } else {
                let (text, code) = format_value(value);
                RenderedNode::Row(RenderedRow {
                    key,
                    label: humanize(attribute),
                    value: text,
                    code,
                })
            }
        }
    }
}

/// One `ELEMENT` as a label/value row: its `name` labels the row and its
/// `value` renders as the value; an element with no value carries a
/// `null_flavour` instead (RM `ELEMENT` invariant
/// `Inv_null_flavour_indicated`, `...data_structures.element.adoc`).
fn element_row(value: &Value, attribute: &str, key: String) -> RenderedRow {
    let label = name_of(value).unwrap_or_else(|| humanize(attribute));
    let (text, code) = match value.get("value") {
        Some(leaf) => format_value(leaf),
        None => match value.get("null_flavour") {
            Some(flavour) => {
                let (flavour_text, flavour_code) = format_value(flavour);
                (format!("({flavour_text})"), flavour_code)
            }
            None => ("(no value)".to_owned(), None),
        },
    };
    RenderedRow {
        key,
        label,
        value: if text.trim().is_empty() {
            "(no value)".to_owned()
        } else {
            text
        },
        code,
    }
}

/// Render one value honestly: each `DV_*` type by its own attributes, with a
/// compact-JSON fallback so nothing is ever silently dropped.
///
/// Spec (`docs/specs/openehr/RM/docs/UML/classes/`): `DV_TEXT._value_` and
/// `DV_CODED_TEXT._defining_code_` (`...data_types.dv_text.adoc`,
/// `...data_types.dv_coded_text.adoc`); `DV_QUANTITY._magnitude_`/`_units_`
/// (`...data_types.dv_quantity.adoc`); `DV_COUNT`/`DV_SCALE._magnitude_`;
/// `DV_ORDINAL._value_`/`_symbol_` (`...data_types.dv_ordinal.adoc`);
/// `DV_PROPORTION._numerator_`/`_denominator_`
/// (`...data_types.dv_proportion.adoc`); `DV_IDENTIFIER._id_`/`_type_`/
/// `_issuer_` (`...data_types.dv_identifier.adoc`);
/// `DV_MULTIMEDIA._media_type_`/`_size_`/`_uri_`
/// (`...data_types.dv_multimedia.adoc`); `CODE_PHRASE._terminology_id_`/
/// `_code_string_` (`...data_types.code_phrase.adoc`);
/// `PARTY_IDENTIFIED._name_` (`...common.party_identified.adoc`); the
/// date/time and URI types carry a plain `_value_`
/// (RM `data_types` `master07-date_time_package.adoc`,
/// `master10-uri_package.adoc`).
fn format_value(value: &Value) -> (String, Option<String>) {
    match value {
        Value::Object(_) => format_object(value),
        Value::Array(items) => (
            items
                .iter()
                .map(|item| format_value(item).0)
                .collect::<Vec<_>>()
                .join(", "),
            None,
        ),
        _ => (scalar_text(value), None),
    }
}

/// [`format_value`] for the object case: dispatch on `_type`.
fn format_object(value: &Value) -> (String, Option<String>) {
    let rm_type = type_of(value).unwrap_or_default();
    match rm_type {
        "DV_CODED_TEXT" => (
            field_text(value, "value"),
            code_of(value.get("defining_code")),
        ),
        "DV_QUANTITY" => {
            let magnitude = field_text(value, "magnitude");
            let units = field_text(value, "units");
            (join_with_space(&magnitude, &units), None)
        }
        "DV_ORDINAL" | "DV_SCALE" => format_ordinal(value),
        "DV_PROPORTION" => (
            format!(
                "{}/{}",
                field_text(value, "numerator"),
                field_text(value, "denominator")
            ),
            None,
        ),
        "DV_IDENTIFIER" => {
            let id = field_text(value, "id");
            let kind = field_text(value, "type");
            (join_parenthesized(&id, &kind), None)
        }
        "DV_MULTIMEDIA" => format_multimedia(value),
        "DV_INTERVAL" => {
            let lower = value.get("lower").map(format_value).unwrap_or_default();
            let upper = value.get("upper").map(format_value).unwrap_or_default();
            (format!("{} – {}", lower.0, upper.0), None)
        }
        "CODE_PHRASE" => (code_of(Some(value)).unwrap_or_default(), None),
        "PARTY_SELF" => {
            let external = nested_str(value, &["external_ref", "id", "value"]);
            (external.unwrap_or_else(|| "self".to_owned()), None)
        }
        "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            let name = field_text(value, "name");
            let external = nested_str(value, &["external_ref", "id", "value"]).unwrap_or_default();
            if name.is_empty() {
                (external, None)
            } else {
                (join_parenthesized(&name, &external), None)
            }
        }
        "OBJECT_REF" | "PARTY_REF" | "LOCATABLE_REF" => (
            nested_str(value, &["id", "value"]).unwrap_or_default(),
            None,
        ),
        _ => {
            // Every remaining type with a plain `value` (DV_TEXT, the
            // date/time family, DV_BOOLEAN, DV_URI, DV_DURATION, DV_COUNT's
            // magnitude, the id classes) renders that value; anything else
            // falls back to its compact JSON so the row is never a lie.
            let text = field_text(value, "value");
            if !text.is_empty() {
                return (text, None);
            }
            let magnitude = field_text(value, "magnitude");
            if !magnitude.is_empty() {
                return (magnitude, None);
            }
            (serde_json::to_string(value).unwrap_or_default(), None)
        }
    }
}

/// `DV_ORDINAL`/`DV_SCALE`: the symbol's text with the ordinal value beside it,
/// and the symbol's code as the row's code.
fn format_ordinal(value: &Value) -> (String, Option<String>) {
    let symbol = value.get("symbol");
    let symbol_text = symbol.map(|s| field_text(s, "value")).unwrap_or_default();
    let ordinal = field_text(value, "value");
    let text = if symbol_text.is_empty() {
        ordinal
    } else if ordinal.is_empty() {
        symbol_text
    } else {
        format!("{symbol_text} ({ordinal})")
    };
    (text, symbol.and_then(|s| code_of(s.get("defining_code"))))
}

/// `DV_MULTIMEDIA`: what it is and how big, never the payload itself.
fn format_multimedia(value: &Value) -> (String, Option<String>) {
    let media = code_of(value.get("media_type")).unwrap_or_default();
    let size = field_text(value, "size");
    let uri = value
        .get("uri")
        .map(|uri| field_text(uri, "value"))
        .unwrap_or_default();
    let bytes = if size.is_empty() {
        String::new()
    } else {
        format!("{size} bytes")
    };
    (
        join_with_space(&join_parenthesized(&media, &bytes), &uri),
        None,
    )
}

/// `terminology::code` for a `CODE_PHRASE`, or just the code when the
/// terminology is absent.
fn code_of(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let code = value.get("code_string").and_then(Value::as_str)?;
    match nested_str(value, &["terminology_id", "value"]) {
        Some(terminology) => Some(format!("{terminology}::{code}")),
        None => Some(code.to_owned()),
    }
}

/// The `_type` of a JSON object, when it has one.
fn type_of(value: &Value) -> Option<&str> {
    value.get("_type")?.as_str()
}

/// `LOCATABLE._name_._value_`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`).
fn name_of(value: &Value) -> Option<String> {
    nested_str(value, &["name", "value"])
}

/// Follow a chain of object keys to a string leaf.
fn nested_str(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
}

/// One scalar as display text (objects/arrays fall back to compact JSON).
fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// One named field as display text (empty when absent).
fn field_text(value: &Value, field: &str) -> String {
    value.get(field).map(scalar_text).unwrap_or_default()
}

/// `"a b"`, skipping either side when empty.
fn join_with_space(left: &str, right: &str) -> String {
    match (left.is_empty(), right.is_empty()) {
        (true, _) => right.to_owned(),
        (_, true) => left.to_owned(),
        _ => format!("{left} {right}"),
    }
}

/// `"a (b)"`, skipping either side when empty.
fn join_parenthesized(left: &str, right: &str) -> String {
    match (left.is_empty(), right.is_empty()) {
        (true, _) => right.to_owned(),
        (_, true) => left.to_owned(),
        _ => format!("{left} ({right})"),
    }
}

/// An RM attribute or type name as a reading label: underscores are spaces.
fn humanize(name: &str) -> String {
    name.replace('_', " ")
}

#[cfg(test)]
mod tests {
    use crate::clinical::{RenderedNode, RenderedSection, render};

    /// A small but realistic canonical COMPOSITION: an `EVENT_CONTEXT`, one
    /// OBSERVATION with a `HISTORY`/`POINT_EVENT`/`ITEM_TREE` spine, a
    /// `DV_QUANTITY` leaf, a `DV_CODED_TEXT` leaf and a null-flavoured leaf.
    const COMPOSITION: &str = r#"{
      "_type": "COMPOSITION",
      "name": {"_type": "DV_TEXT", "value": "Vital signs"},
      "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
      "archetype_details": {
        "_type": "ARCHETYPED",
        "archetype_id": {"value": "openEHR-EHR-COMPOSITION.encounter.v1"},
        "template_id": {"value": "vitals.en.v1"},
        "rm_version": "1.1.0"
      },
      "language": {"_type": "CODE_PHRASE", "terminology_id": {"value": "ISO_639-1"}, "code_string": "en"},
      "territory": {"_type": "CODE_PHRASE", "terminology_id": {"value": "ISO_3166-1"}, "code_string": "NL"},
      "category": {"_type": "DV_CODED_TEXT", "value": "event", "defining_code": {"terminology_id": {"value": "openehr"}, "code_string": "433"}},
      "composer": {"_type": "PARTY_IDENTIFIED", "name": "Dr Jane Williams"},
      "context": {
        "_type": "EVENT_CONTEXT",
        "start_time": {"_type": "DV_DATE_TIME", "value": "2026-07-25T09:30:00Z"},
        "setting": {"_type": "DV_CODED_TEXT", "value": "primary medical care", "defining_code": {"terminology_id": {"value": "openehr"}, "code_string": "228"}}
      },
      "content": [
        {
          "_type": "OBSERVATION",
          "name": {"_type": "DV_TEXT", "value": "Pulse"},
          "archetype_node_id": "openEHR-EHR-OBSERVATION.pulse.v2",
          "data": {
            "_type": "HISTORY",
            "name": {"_type": "DV_TEXT", "value": "history"},
            "origin": {"_type": "DV_DATE_TIME", "value": "2026-07-25T09:31:00Z"},
            "events": [
              {
                "_type": "POINT_EVENT",
                "name": {"_type": "DV_TEXT", "value": "any event"},
                "time": {"_type": "DV_DATE_TIME", "value": "2026-07-25T09:31:00Z"},
                "data": {
                  "_type": "ITEM_TREE",
                  "name": {"_type": "DV_TEXT", "value": "tree"},
                  "items": [
                    {
                      "_type": "ELEMENT",
                      "name": {"_type": "DV_TEXT", "value": "Rate"},
                      "value": {"_type": "DV_QUANTITY", "magnitude": 72.0, "units": "/min"}
                    },
                    {
                      "_type": "ELEMENT",
                      "name": {"_type": "DV_TEXT", "value": "Regularity"},
                      "value": {"_type": "DV_CODED_TEXT", "value": "Regular", "defining_code": {"terminology_id": {"value": "local"}, "code_string": "at0006"}}
                    },
                    {
                      "_type": "ELEMENT",
                      "name": {"_type": "DV_TEXT", "value": "Comment"},
                      "null_flavour": {"_type": "DV_CODED_TEXT", "value": "no information", "defining_code": {"terminology_id": {"value": "openehr"}, "code_string": "271"}}
                    }
                  ]
                }
              }
            ]
          }
        }
      ]
    }"#;

    /// Flatten the tree into `(label, value, code)` rows.
    fn rows(section: &RenderedSection) -> Vec<(String, String, Option<String>)> {
        let mut out = Vec::new();
        for child in &section.children {
            match child {
                RenderedNode::Row(row) => {
                    out.push((row.label.clone(), row.value.clone(), row.code.clone()));
                }
                RenderedNode::Section(nested) => out.extend(rows(nested)),
            }
        }
        out
    }

    /// Every key in the tree, in walk order.
    fn keys(section: &RenderedSection) -> Vec<String> {
        let mut out = vec![section.key.clone()];
        for child in &section.children {
            match child {
                RenderedNode::Row(row) => out.push(row.key.clone()),
                RenderedNode::Section(nested) => out.extend(keys(nested)),
            }
        }
        out
    }

    #[test]
    fn a_non_rm_body_has_no_rendered_view() {
        assert!(render("not json at all").is_none());
        assert!(render("<composition/>").is_none());
        // Valid JSON without `_type` is not an RM document.
        assert!(render("{\"a\": 1}").is_none());
        assert!(render("[]").is_none());
    }

    #[test]
    fn the_root_heading_comes_from_the_composition_name() {
        let document = render(COMPOSITION).expect("a COMPOSITION renders");
        assert_eq!(document.title, "Vital signs");
        assert_eq!(document.rm_type, "COMPOSITION");
        assert_eq!(
            document.archetype_node_id.as_deref(),
            Some("openEHR-EHR-COMPOSITION.encounter.v1")
        );
    }

    #[test]
    fn element_leaves_render_as_label_value_rows() {
        let document = render(COMPOSITION).expect("a COMPOSITION renders");
        let rows = rows(&document);
        assert!(
            rows.contains(&("Rate".to_owned(), "72.0 /min".to_owned(), None)),
            "DV_QUANTITY magnitude + units: {rows:#?}"
        );
        assert!(
            rows.contains(&(
                "Regularity".to_owned(),
                "Regular".to_owned(),
                Some("local::at0006".to_owned())
            )),
            "DV_CODED_TEXT value + code: {rows:#?}"
        );
        assert!(
            rows.contains(&(
                "Comment".to_owned(),
                "(no information)".to_owned(),
                Some("openehr::271".to_owned())
            )),
            "a null-flavoured ELEMENT states its flavour: {rows:#?}"
        );
    }

    #[test]
    fn the_context_facts_are_surfaced_and_the_bookkeeping_folded() {
        let document = render(COMPOSITION).expect("a COMPOSITION renders");
        let rows = rows(&document);
        let labels: Vec<&str> = rows.iter().map(|(label, ..)| label.as_str()).collect();
        assert!(labels.contains(&"composer"), "{labels:?}");
        assert!(labels.contains(&"start time"), "{labels:?}");
        assert!(labels.contains(&"setting"), "{labels:?}");
        assert!(labels.contains(&"template"), "{labels:?}");
        assert!(labels.contains(&"origin"), "{labels:?}");
        // Folded: language / territory / category / uid never reach the view.
        assert!(!labels.contains(&"language"), "{labels:?}");
        assert!(!labels.contains(&"territory"), "{labels:?}");
        assert!(!labels.contains(&"category"), "{labels:?}");
        assert_eq!(
            rows.iter()
                .find(|(label, ..)| label == "composer")
                .map(|(_, value, _)| value.as_str()),
            Some("Dr Jane Williams")
        );
    }

    #[test]
    fn keys_are_unique_rm_paths() {
        let document = render(COMPOSITION).expect("a COMPOSITION renders");
        let keys = keys(&document);
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            keys.len(),
            "`<For>` keys must be unique: {keys:#?}"
        );
        assert!(
            keys.iter()
                .any(|key| key == "/content[0]/data/events[0]/data/items[0]"),
            "keys are RM paths: {keys:#?}"
        );
    }

    #[test]
    fn rendering_is_deterministic() {
        assert_eq!(render(COMPOSITION), render(COMPOSITION));
    }

    #[test]
    fn an_ehr_status_surfaces_its_capability_flags() {
        // The pane renders every openEHR document, not only compositions.
        let body = r#"{
          "_type": "EHR_STATUS",
          "name": {"_type": "DV_TEXT", "value": "EHR Status"},
          "uid": {"_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515"},
          "subject": {"_type": "PARTY_SELF", "external_ref": {"_type": "PARTY_REF", "id": {"_type": "GENERIC_ID", "value": "1234"}}},
          "is_queryable": true,
          "is_modifiable": false
        }"#;
        let document = render(body).expect("an EHR_STATUS renders");
        let rows = rows(&document);
        let labels: Vec<&str> = rows.iter().map(|(label, ..)| label.as_str()).collect();
        assert!(labels.contains(&"is queryable"), "{labels:?}");
        assert!(labels.contains(&"is modifiable"), "{labels:?}");
        assert_eq!(
            rows.iter()
                .find(|(label, ..)| label == "is queryable")
                .map(|(_, value, _)| value.as_str()),
            Some("true")
        );
    }
}
