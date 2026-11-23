// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Web Template JSON model (`simplified_formats master04-basic_concepts.adoc`
//! §"Web Template Metadata"), served as `application/openehr.wt+json`.
//!
//! The serde names, field order and `skip_serializing_if` guards are a fixed
//! wire contract. `#[serde(skip)]` fields are build-time scratch and
//! validation-only constraints.
//!
//! NOTE: the fields `schemas/web_template/*.yaml` does not list (`cardinalities`,
//! `proportionTypes`, `dependsOn`, `listOpen`, `otherDetails` and others) are our
//! own design, legal since those schemas set no `additionalProperties: false`.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::Serialize;

use super::constraint::{
    WebTemplateClosedAttribute, WebTemplateCodeList, WebTemplateConstraintBinding,
    WebTemplateExistence, WebTemplateSlot, WebTemplateStructuralStub,
};

/// A single Web Template metadata document (master04 §"Web Template Metadata").
///
/// `version` is the format version string, not the template version.
#[derive(Debug, Clone, Serialize)]
pub struct WebTemplate {
    /// The template id this document describes (wire member `templateId`).
    #[serde(rename = "templateId")]
    pub template_id: String,
    /// The template's semantic version, when the definition carries one (wire member `semVer`).
    #[serde(rename = "semVer", skip_serializing_if = "Option::is_none")]
    pub sem_ver: Option<String>,
    /// The Web Template **format** version string — not the template version.
    pub version: String,
    /// The template's default language code (wire member `defaultLanguage`).
    #[serde(rename = "defaultLanguage")]
    pub default_language: String,
    /// Every language the template carries translations for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// The root node of the template tree.
    #[serde(serialize_with = "super::root::serialize_root")]
    pub tree: WebTemplateNode,
    /// Additive template metadata — no openEHR spec governs this member; our own design/extension (wire member `otherDetails`).
    #[serde(rename = "otherDetails", skip_serializing_if = "IndexMap::is_empty")]
    pub other_details: IndexMap<String, String>,
    /// For an OPT2, every archetype reference it inlined mapped to the interface id
    /// its root's `nodeId` carries (BASE `master07-referencing.adoc`
    /// §Reconstitutability); `None` for an OPT 1.4.
    #[serde(skip)]
    pub archetype_refs: Option<BTreeMap<String, String>>,
}

/// The `defining_code` a coded-name node must carry (RM common
/// `master03-archetyped_package.adoc` §"The `LOCATABLE` class"); the display
/// value is [`WebTemplateNode::name`].
#[derive(Debug, Clone)]
pub struct CodedName {
    /// The `defining_code` terminology id (e.g. `local`, `openehr`).
    pub terminology: String,
    /// The `defining_code` code string (an `atNNNN` / openEHR code).
    pub code: String,
    /// Whether the template fixes a `name/value` equal to no listed code's rubric.
    /// The example generator omits optional incoherent nodes.
    pub incoherent: bool,
}

/// One node of the Web Template tree (master04 §"Web Template Metadata").
///
/// `id` is the local json-id segment (master04 §"Node ID Generation Rules"),
/// `aqlPath` the archetype path root to node, and `max == -1` unbounded.
#[derive(Debug, Clone, Serialize)]
pub struct WebTemplateNode {
    /// The local json-id segment, serialized as `id` (master04 §"Node ID
    /// Generation Rules").
    pub id: String,
    /// The node's runtime `name` constraint, when the template fixes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The node name in the document's default language (wire member `localizedName`).
    #[serde(rename = "localizedName", skip_serializing_if = "Option::is_none")]
    pub localized_name: Option<String>,
    /// The constrained RM type name (wire member `rmType`).
    #[serde(rename = "rmType")]
    pub rm_type: String,
    /// The archetype node id (`atNNNN`, or an archetype id at a root; wire member `nodeId`).
    #[serde(rename = "nodeId", skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Occurrences lower bound (master04 §"Web Template Metadata": `min`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i32>,
    /// Occurrences upper bound; `-1` = unbounded (master04 §"Web Template
    /// Metadata": `max`, where `max = -1` marks an unbounded occurrence).
    pub max: i32,
    /// Whether the node is filled from the composition context rather than from user input (wire member `inContext`).
    #[serde(rename = "inContext", skip_serializing_if = "Option::is_none")]
    pub in_context: Option<bool>,
    /// The archetype path root -> node (wire member `aqlPath`).
    #[serde(rename = "aqlPath")]
    pub aql_path: String,
    /// The node name per language (wire member `localizedNames`).
    #[serde(rename = "localizedNames", skip_serializing_if = "IndexMap::is_empty")]
    pub localized_names: IndexMap<String, String>,
    /// The node description per language (wire member `localizedDescriptions`).
    #[serde(
        rename = "localizedDescriptions",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub localized_descriptions: IndexMap<String, String>,
    /// Archetype annotations carried on this node.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub annotations: IndexMap<String, String>,
    /// Terminology bindings on this node, keyed by terminology id (wire member `termBindings`).
    #[serde(rename = "termBindings", skip_serializing_if = "IndexMap::is_empty")]
    pub term_bindings: IndexMap<String, WebTemplateBindingCodedValue>,
    /// The `DV_PROPORTION` types this node admits — an additive member; no openEHR spec governs it (wire member `proportionTypes`).
    #[serde(rename = "proportionTypes", skip_serializing_if = "Vec::is_empty")]
    pub proportion_types: Vec<String>,
    /// The leaf input descriptors of this node.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<WebTemplateInput>,
    /// Sibling json-ids this node's presence depends on — an additive member; no openEHR spec governs it (wire member `dependsOn`).
    #[serde(rename = "dependsOn", skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    /// Container cardinalities surfaced in the metadata document.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cardinalities: Vec<WebTemplateCardinality>,
    /// The node's child nodes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WebTemplateNode>,

    /// AOM 1.4 `C_ATTRIBUTE.existence` constraints on this node's mandatory RM
    /// attributes — captured for the validation walk, **not** part of the Web
    /// Template metadata document, so `#[serde(skip)]`.
    #[serde(skip)]
    pub existence: Vec<WebTemplateExistence>,

    /// Every constraining `C_MULTIPLE_ATTRIBUTE.cardinality` on this node (AOM 1.4
    /// §cardinality), a superset of the serialized `cardinalities`.
    #[serde(skip)]
    pub card_all: Vec<WebTemplateCardinality>,

    /// RM-type narrowings of the wrapper constraints the compactor hoisted away; an
    /// instance node at `path` must conform to one of them.
    #[serde(skip)]
    pub slots: Vec<WebTemplateSlot>,
    /// Built for an RM attribute the template leaves implicit. Example-only.
    #[serde(skip)]
    pub implicit: bool,

    /// `C_INTEGER.list`/`C_REAL.list` constraints on a leaf's numeric datum
    /// (`magnitude`, `value`, …), keyed by RM attribute name — the `inputs`
    /// document does not carry these lists, so they are validation-only.
    #[serde(skip)]
    pub numeric_lists: Vec<(String, Vec<f64>)>,

    /// `C_INTEGER`/`C_REAL` ranges the `inputs` do not carry, keyed by RM attribute:
    /// `DV_MULTIMEDIA.size` and an ADL 2 union of ranges (AOM2 `master04.5` §C_ORDERED).
    #[serde(skip)]
    pub numeric_ranges: Vec<(String, WebTemplateRange)>,

    /// `C_DURATION.range` (ISO-8601 duration bounds) on a `DV_DURATION` leaf's
    /// `value` — the `inputs` document encodes only the allowed-field split into
    /// per-field inputs, so the range is validation-only.
    #[serde(skip)]
    pub duration_range: Option<WebTemplateRange>,

    /// `C_TIME`/`C_DATE_TIME.timezone_validity` on a temporal leaf's `value`
    /// (`org.openehr.am.aom14.c_time.adoc`; OPT 1.4 encodes `1001` mandatory,
    /// `1002` optional, `1003` disallowed).
    #[serde(skip)]
    pub tz_validity: Option<i32>,

    /// `C_QUANTITY.property` code on a `DV_QUANTITY` leaf with no unit list; the
    /// instance's `units` must belong to the property's unit set (`openehr_term::bundle`).
    #[serde(skip)]
    pub quantity_property: Option<String>,

    /// Whether this leaf's `C_CODE_PHRASE` explicitly names the `local` terminology
    /// with a closed `code_list`, which the `inputs` lose
    /// (`org.openehr.am.aom14.c_coded_text.adoc` §C_CODED_TEXT).
    #[serde(skip)]
    pub coded_terminology_local: bool,

    /// `C_CODE_PHRASE` code lists on coded RM attributes the `inputs` mapping
    /// does not model (e.g. `DV_MULTIMEDIA.media_type`). `defining_code` is
    /// excluded (already covered by the coded-text `inputs`).
    #[serde(skip)]
    pub code_lists: Vec<WebTemplateCodeList>,

    /// Archetype constraint bindings in force on this node
    /// (`AM/docs/ADL1.4/master08-adl.adoc` §Constraint_bindings), resolvable only by
    /// a terminology service: see
    /// [`crate::flat::validation::collect_constraint_binding_checks`].
    #[serde(skip)]
    pub constraint_bindings: Vec<WebTemplateConstraintBinding>,

    /// Closed-archetype constraints: per constrained attribute, the admissible
    /// child identities, re-homed on the parent when a wrapper is hoisted.
    #[serde(skip)]
    pub closed_attributes: Vec<WebTemplateClosedAttribute>,

    /// The RM attributes of this node the template prohibits (existence `0..0`, or
    /// every constraint object at occurrences `0..0`; AOM 1.4 §existence).
    #[serde(skip)]
    pub prohibited_attributes: Vec<String>,

    /// Structural stubs for the constrained `data`, `description`, `protocol` and
    /// `state` attributes the compactor dropped, so the builder fills them with the
    /// constrained identity (AOM 1.4 `master04-constraint_model_package.adoc`
    /// §`Valid_value`).
    #[serde(skip)]
    pub structural_stubs: Vec<WebTemplateStructuralStub>,

    /// The other object constraints on this node's attribute that the builder
    /// folded into it; an instance may match any one (ADL 1.4 `master05-cadl.adoc`
    /// §"Single-valued Attributes"; master04 §"Open Value-Sets and the `|other`
    /// Suffix").
    #[serde(skip)]
    pub alternatives: Vec<WebTemplateNode>,

    /// The template's constraints on the ELEMENT's `null_flavour` and `null_reason`,
    /// each with its FLAT attribute id (RM `data_structures` §ELEMENT; master05
    /// §ELEMENT).
    #[serde(skip)]
    pub null_constraints: Vec<WebTemplateNode>,

    // ── build-time scratch (never serialized) ────────────────────────────────
    /// Full `parent/segment` id chain, used to scope dedup and cardinality ids.
    #[serde(skip)]
    pub full_id: String,
    /// Polymorphic alternate json-id (`value`/`value2`) for a choice ELEMENT's
    /// alternatives (no openEHR spec governs choice-ELEMENT ids: our own design).
    #[serde(skip)]
    pub alt_json_id: Option<String>,
    /// The RM name-constraint code, when the node is name-constrained.
    #[serde(skip)]
    pub name_code: Option<String>,
    /// The `(terminology, code)` of a `DV_CODED_TEXT` name constraint; see
    /// [`CodedName`].
    #[serde(skip)]
    pub name_coded: Option<CodedName>,
    /// The RM type the template constrains the node's `name` to, when it
    /// constrains it (`DV_TEXT` or `DV_CODED_TEXT`; AOM 1.4 `C_COMPLEX_OBJECT`
    /// `rm_type_name`). Build-time only (`#[serde(skip)]`).
    #[serde(skip)]
    pub name_rm_type: Option<String>,
    /// The `C_STRING` a template puts on a non-data RM String attribute such as
    /// `ACTIVITY.action_archetype_id`, as the input carrying its pattern or list.
    #[serde(skip)]
    pub string_constraint: Option<Box<WebTemplateInput>>,
    /// For an OPT2 archetype root, the complete identifier data records (BASE
    /// `master07-referencing.adoc` §Reconstitutability); [`Self::node_id`] is the
    /// interface form.
    #[serde(skip)]
    pub archetype_ref: Option<String>,
    /// On an OPT2 root, the template's `rm_release`, recorded as
    /// `ARCHETYPED.rm_version`.
    #[serde(skip)]
    pub rm_release: Option<String>,
    /// What the operational template states about this node that the
    /// document has no member for. Build-time only (`#[serde(skip)]`).
    #[serde(skip)]
    pub facts: super::facts::WebTemplateNodeFacts,

    /// The pre-parsed archetype-conformance walk plan, built once by
    /// [`crate::flat::webtemplate::builder::build_web_template`].
    #[serde(skip)]
    pub(crate) walk: Option<Box<crate::flat::validation::plan::NodeWalk>>,
}

impl WebTemplateNode {
    /// A fresh node with the given rm type / aql path; all other fields empty.
    #[must_use]
    pub fn new(rm_type: String, aql_path: String) -> Self {
        Self {
            id: String::new(),
            name: None,
            localized_name: None,
            rm_type,
            node_id: None,
            min: None,
            max: -1,
            in_context: None,
            aql_path,
            localized_names: IndexMap::new(),
            localized_descriptions: IndexMap::new(),
            annotations: IndexMap::new(),
            term_bindings: IndexMap::new(),
            proportion_types: Vec::new(),
            inputs: Vec::new(),
            depends_on: None,
            cardinalities: Vec::new(),
            children: Vec::new(),
            existence: Vec::new(),
            card_all: Vec::new(),
            slots: Vec::new(),
            implicit: false,
            numeric_lists: Vec::new(),
            numeric_ranges: Vec::new(),
            duration_range: None,
            tz_validity: None,
            quantity_property: None,
            coded_terminology_local: false,
            code_lists: Vec::new(),
            constraint_bindings: Vec::new(),
            closed_attributes: Vec::new(),
            prohibited_attributes: Vec::new(),
            structural_stubs: Vec::new(),
            alternatives: Vec::new(),
            null_constraints: Vec::new(),
            full_id: String::new(),
            alt_json_id: None,
            name_code: None,
            name_coded: None,
            name_rm_type: None,
            string_constraint: None,
            archetype_ref: None,
            rm_release: None,
            facts: super::facts::WebTemplateNodeFacts::default(),
            walk: None,
        }
    }

    /// Whether the node carries an input (a leaf value node).
    pub(crate) fn has_input(&self) -> bool {
        !self.inputs.is_empty()
    }

    /// The first input's type, if any.
    pub(crate) fn first_input_type(&self) -> Option<WebTemplateInputType> {
        self.inputs.first().map(|i| i.input_type)
    }
}

/// The Web Template input `type` enum. Serializes as the SCREAMING constant name
/// (`"TEXT"`, `"CODED_TEXT"`, `"DATETIME"`, …), the `inputs[].type` values of the
/// master04 §"Web Template Metadata" example.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WebTemplateInputType {
    /// The `TEXT` input type.
    Text,
    /// The `CODED_TEXT` input type.
    CodedText,
    /// The `DATE` input type.
    Date,
    /// The `TIME` input type.
    Time,
    /// The `DATETIME` input type.
    Datetime,
    /// The `BOOLEAN` input type.
    Boolean,
    /// The `INTEGER` input type.
    Integer,
    /// The `DECIMAL` input type.
    Decimal,
    /// The `DURATION` input type.
    Duration,
    /// The `QUANTITY` input type.
    Quantity,
    /// The `COUNT` input type.
    Count,
    /// The `PROPORTION` input type.
    Proportion,
}

/// A leaf input descriptor (master04 §"Web Template Metadata": `inputs[]`).
///
/// NOTE: `listOpen`, `terminology` and `defaultValue` are our own design.
#[derive(Debug, Clone, Serialize)]
pub struct WebTemplateInput {
    /// The FLAT key suffix this input fills (`magnitude`, `unit`, `code`, …), or `None` for the node's bare value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// The input's value type (wire member `type`).
    #[serde(rename = "type")]
    pub input_type: WebTemplateInputType,
    /// The closed list of coded options, when the constraint enumerates one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<WebTemplateCodedValue>,
    /// Whether the list admits an unlisted value via the `|other` suffix — an additive member; no openEHR spec governs it (wire member `listOpen`).
    #[serde(rename = "listOpen", skip_serializing_if = "Option::is_none")]
    pub list_open: Option<bool>,
    /// Value-level validation (pattern / range / precision) on this input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<WebTemplateValidation>,
    /// The constraint's default value, when it fixes one — an additive member; no openEHR spec governs it (wire member `defaultValue`).
    #[serde(rename = "defaultValue", skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    /// The constrained terminology id, when the constraint names one — an additive member; no openEHR spec governs it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminology: Option<String>,
}

impl WebTemplateInput {
    /// A fresh input of the given type and FLAT key suffix, with no list, validation, or default.
    #[must_use]
    pub fn new(input_type: WebTemplateInputType, suffix: Option<&str>) -> Self {
        Self {
            suffix: suffix.map(str::to_owned),
            input_type,
            list: Vec::new(),
            list_open: None,
            validation: None,
            default_value: None,
            terminology: None,
        }
    }
}

/// A coded option (master04 §"Web Template Metadata": `inputs[].list[]`).
///
/// NOTE: `ordinal` and `scale` are our own design.
#[derive(Debug, Clone, Serialize)]
pub struct WebTemplateCodedValue {
    /// The option's code string.
    pub value: String,
    /// The option's display label in the default language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The option's label per language (wire member `localizedLabels`).
    #[serde(rename = "localizedLabels", skip_serializing_if = "IndexMap::is_empty")]
    pub localized_labels: IndexMap<String, String>,
    /// The option's description per language (wire member `localizedDescriptions`).
    #[serde(
        rename = "localizedDescriptions",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub localized_descriptions: IndexMap<String, String>,
    /// Terminology bindings on this option, keyed by terminology id (wire member `termBindings`).
    #[serde(rename = "termBindings", skip_serializing_if = "IndexMap::is_empty")]
    pub term_bindings: IndexMap<String, WebTemplateBindingCodedValue>,
    /// Value-level validation on this option.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<WebTemplateValidation>,
    /// The `DV_ORDINAL` ordinal of this option, when the constraint is ordinal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<i32>,
    /// The `DV_SCALE` scale value of this option, when the constraint is a scale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// The terminology the option's code is drawn from (`#[serde(skip)]`).
    #[serde(skip)]
    pub terminology: Option<String>,
}

impl WebTemplateCodedValue {
    /// A coded option with the given code and display label.
    pub fn new(value: impl Into<String>, label: Option<String>) -> Self {
        Self {
            value: value.into(),
            label,
            localized_labels: IndexMap::new(),
            localized_descriptions: IndexMap::new(),
            term_bindings: IndexMap::new(),
            validation: None,
            ordinal: None,
            scale: None,
            terminology: None,
        }
    }
}

/// A terminology binding (master04 §"Web Template Metadata": a `termBindings`
/// entry, `{value, terminologyId}`).
#[derive(Debug, Clone, Serialize)]
pub struct WebTemplateBindingCodedValue {
    /// The bound code in the target terminology.
    pub value: String,
    /// The target terminology id (wire member `terminologyId`).
    #[serde(rename = "terminologyId")]
    pub terminology_id: String,
}

/// A leaf input's `validation` (master04 §"Web Template Metadata":
/// `inputs[].validation`): `pattern`, `range`, `precision`; null members omitted.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WebTemplateValidation {
    /// A regular expression the value must match (wire member `pattern`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The permitted value range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<WebTemplateRange>,
    /// The permitted decimal-precision range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<WebTemplateRange>,
}

impl WebTemplateValidation {
    pub(crate) fn is_empty(&self) -> bool {
        self.pattern.is_none() && self.range.is_none() && self.precision.is_none()
    }
}

/// A validation range (master04 §"Web Template Metadata"); `min` and `max` are
/// numbers or ISO strings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WebTemplateRange {
    /// The lower-bound comparison operator (wire member `minOp`).
    #[serde(rename = "minOp", skip_serializing_if = "Option::is_none")]
    pub min_op: Option<String>,
    /// The lower bound — a number or an ISO 8601 string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<serde_json::Value>,
    /// The upper-bound comparison operator (wire member `maxOp`).
    #[serde(rename = "maxOp", skip_serializing_if = "Option::is_none")]
    pub max_op: Option<String>,
    /// The upper bound — a number or an ISO 8601 string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<serde_json::Value>,
}

/// A container cardinality: `{min, max, ids}`. `path` is build-time scratch used
/// to resolve `ids` from the child json-ids (never serialized).
#[derive(Debug, Clone, Serialize)]
pub struct WebTemplateCardinality {
    /// The container's minimum member count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i32>,
    /// The container's maximum member count; `-1` means unbounded.
    pub max: i32,
    /// The child json-ids the cardinality applies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    /// Build-time scratch: the archetype path used to resolve `ids` from child json-ids (never serialized).
    #[serde(skip)]
    pub path: String,
    /// Whether no two container members may be equal by value (ADL 1.4
    /// `master05-cadl.adoc`, ADL2 `master04.3` §Container attributes).
    #[serde(skip)]
    pub unique: bool,
}
