// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Example-COMPOSITION generation from a [`WebTemplate`] (the `adl1.4/{id}/example`
//! endpoint).
//!
//! [`example_composition`] walks the web-template tree and produces a canonical
//! openEHR JSON COMPOSITION populated with **deterministic** placeholder data
//! (no randomness, no current-time), at one of three [`DetailLevel`]s:
//!
//! * [`Required`](DetailLevel::Required) — only the mandatory skeleton (nodes on
//!   a fully-mandatory chain, plus whatever a min-cardinality forces). Intended
//!   to be committable as-is.
//! * [`Medium`](DetailLevel::Medium) — the fully-populated single-instance
//!   document: every optional branch descended to its leaves, one occurrence of
//!   each node, the first alternative of any choice. Intended to be committable
//!   as-is.
//! * [`Complete`](DetailLevel::Complete) — everything `medium` emits, plus a
//!   second occurrence of each repeating node (demonstrating repetition); not
//!   necessarily committable.
//!
//! The set of populated leaves is monotonic across the levels
//! (`required ⊆ medium ⊆ complete`).
//!
//! The generator emits a FLAT map of deterministic example values and hands it
//! to [`composition_from_flat`], which already materialises the compacted RM
//! structure and fills every RM-mandatory field FLAT never surfaces. The result
//! therefore round-trips through
//! [`composition_to_flat`](crate::flat::convert::composition_to_flat) and
//! deserialises as an `openehr-rm` `Composition`.
//!
//! NOTE: ITS-REST Release-1.1.0 states the example-generation algorithm is
//! non-normative ("vendors may produce different results"), so the value choices
//! here are ours; only the `required` level's mandatory-skeleton-is-committable
//! contract is load-bearing. Reachable `PARTY_*` value leaves are skipped rather
//! than fabricated — an unset `ENTRY.subject` defaults to PARTY_SELF (master05
//! §OBSERVATION `/subject` row Note), and an invented party would put a
//! fictitious identified person into an example document.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::flat::convert::composition_from_flat;
use crate::flat::webtemplate::model::{
    WebTemplate, WebTemplateInput, WebTemplateInputType, WebTemplateNode, WebTemplateRange,
};

/// Fixed example instants used for the RM temporal leaves (deterministic).
///
/// [`EXAMPLE_DATE_TIME`] doubles as the `now` supplied to
/// [`composition_from_flat`] for the
/// `ctx/time` default, so a generated example is reproducible across calls. No
/// openEHR spec governs an example's timestamps — our own design (examples must
/// be deterministic).
const EXAMPLE_DATE_TIME: &str = "2022-02-03T04:05:06Z";
const EXAMPLE_DURATION: &str = "PT1H";

/// The level of detail for a generated example (`detail_level` query parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailLevel {
    /// Mandatory skeleton only; intended to be committable without adjustment.
    Required,
    /// Fully populated, one occurrence of every node (first alternative of any
    /// choice); intended to be committable without adjustment.
    Medium,
    /// `Medium` plus a second occurrence of each repeating node; not expected
    /// to be committable.
    Complete,
}

impl DetailLevel {
    /// Parse the `detail_level` query value, absent meaning
    /// [`Required`](Self::Required).
    ///
    /// The enum is closed and matched exactly — a present value that is not
    /// one of its tokens (the empty string and padded spellings included) is
    /// refused, since the declared default applies only to an absent
    /// parameter (ITS-REST `parameters/query/example_detail_level.yaml`).
    ///
    /// # Errors
    /// A message (→ ITS-REST `400`) for a value outside `required|medium|complete`.
    pub fn from_query(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("required") => Ok(Self::Required),
            Some("medium") => Ok(Self::Medium),
            Some("complete") => Ok(Self::Complete),
            Some(other) => Err(format!(
                "unsupported detail_level '{other}' (expected one of required, medium, complete)"
            )),
        }
    }
}

/// The intended use of a generated example (`type` query parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExampleType {
    /// As submitted to the repository (committable; no `uid`).
    Input,
    /// As retrieved from the repository (carries a populated `uid`).
    Output,
}

impl ExampleType {
    /// Parse the `type` query value, absent meaning [`Input`](Self::Input).
    ///
    /// The enum is closed and matched exactly — a present value that is not
    /// one of its tokens (the empty string and padded spellings included) is
    /// refused, since the declared default applies only to an absent
    /// parameter (ITS-REST `parameters/query/example_type.yaml`).
    ///
    /// # Errors
    /// A message (→ ITS-REST `400`) for a value outside `input|output`.
    pub fn from_query(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("input") => Ok(Self::Input),
            Some("output") => Ok(Self::Output),
            Some(other) => Err(format!(
                "unsupported type '{other}' (expected one of input, output)"
            )),
        }
    }
}

/// Generate an example (input-form) COMPOSITION from `wt` at `level`.
///
/// The result is canonical openEHR JSON that deserialises as an `openehr-rm`
/// `Composition`. For the `output` form, additionally call
/// [`apply_output_uid`].
#[must_use]
pub fn example_composition(wt: &WebTemplate, level: DetailLevel) -> Value {
    let mut flat: Map<String, Value> = Map::new();

    // Composition-level housekeeping the tree does not carry as data leaves.
    // `from_flat`/`apply_ctx` turn these into the RM-mandatory language /
    // territory / composer / context (the tree's `default_language` drives the
    // language; territory has no template constraint, so a fixed valid default).
    flat.insert("ctx/language".to_owned(), json!(wt.default_language));
    flat.insert("ctx/territory".to_owned(), json!("US"));
    flat.insert("ctx/composer_name".to_owned(), json!("Example composer"));
    flat.insert("ctx/time".to_owned(), json!(EXAMPLE_DATE_TIME));

    // Walk the tree from the root (COMPOSITION); the root is never forced empty —
    // an empty-content composition is valid RM, and a real content cardinality is
    // honoured by the per-container cardinality pass in `walk`.
    walk(&wt.tree, &wt.tree.id, 0, level, false, &mut flat);

    match composition_from_flat(&flat, wt, EXAMPLE_DATE_TIME) {
        Ok(value) => value,
        // The build does not fail for a well-formed tree; keep a total function.
        Err(_) => Value::Object(Map::new()),
    }
}

/// Populate a deterministic `uid` (an `OBJECT_VERSION_ID`) on an example
/// composition — the `type=output` form (as retrieved from the repository).
///
/// The version-object UUID is derived deterministically from `template_id` (no
/// randomness), so repeated calls yield an identical example.
pub fn apply_output_uid(composition: &mut Value, template_id: &str) {
    if let Value::Object(map) = composition {
        map.insert(
            "uid".to_owned(),
            json!({
                "_type": "OBJECT_VERSION_ID",
                "value": format!("{}::example.server::1", deterministic_uuid(template_id)),
            }),
        );
    }
}

// ── the tree walk ──────────────────────────────────────────────────────────────

/// Walk a container node, emitting the flat entries for its included leaves.
/// Returns whether anything was emitted under this node.
///
/// `opt_depth` is the number of optional (`min < 1`) nodes on the path to this
/// node inclusive; it drives which optional branches each level includes.
/// `force` requests that a (mandatory) container not be left empty.
fn walk(
    node: &WebTemplateNode,
    prefix: &str,
    opt_depth: usize,
    level: DetailLevel,
    force: bool,
    out: &mut Map<String, Value>,
) -> bool {
    if node.has_input() {
        return emit_leaf(node, prefix, out);
    }

    let groups = child_groups(node);
    let cx = WalkCtx {
        prefix,
        opt_depth,
        level,
    };
    let (mut emitted, included) = walk_children(node, &groups, &cx, out);
    emitted |= satisfy_cardinalities(node, &groups, &included, &cx, out);

    // A forced (mandatory) container with nothing yet: drill its first child to a
    // leaf so the node is not emitted empty.
    if force
        && !emitted
        && let Some(child) = groups.first()
    {
        let child_prefix = format!("{prefix}/{}", seg_for(child));
        emitted |= walk(child, &child_prefix, opt_depth, level, true, out);
    }

    emitted
}

/// The invariant context of one walk level: the flat-path prefix, the optional
/// depth reached, and the requested detail level.
struct WalkCtx<'a> {
    prefix: &'a str,
    opt_depth: usize,
    level: DetailLevel,
}

/// Walks every child group of a container, returning whether anything was
/// emitted and which child paths were included.
fn walk_children<'a>(
    node: &'a WebTemplateNode,
    groups: &[&'a WebTemplateNode],
    cx: &WalkCtx<'_>,
    out: &mut Map<String, Value>,
) -> (bool, Vec<&'a str>) {
    let mut emitted = false;
    let mut included: Vec<&str> = Vec::new();
    // Materialised instances per container cardinality (`max != -1`), so the
    // populated levels never overrun a container's upper bound — an *optional*
    // child is skipped once its cardinality is full (a mandatory one still
    // materialises; a template whose mandatory children alone exceed the bound
    // is contradictory and the validator's job to report).
    let mut card_counts = vec![0usize; node.cardinalities.len()];
    for child in groups {
        // A template may constrain an attribute the RM does not declare — the
        // deployed OPT 1.4 corpus carries `ELEMENT.null_flavor` (the US
        // archetype-tooling spelling of RM `null_flavour`,
        // `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc`
        // §Attributes) and other RM-1.0.x leftovers. Such a constraint is
        // tolerated when VALIDATING a template, but materializing it here would
        // author an example that is not a conformant RM instance, so the
        // example generator skips it. The generated RM model is the oracle.
        if !rm_declares(&child.aql_path) {
            continue;
        }
        let child_opt = cx.opt_depth + usize::from(is_optional(child));
        let include = match cx.level {
            DetailLevel::Required => child_opt == 0,
            DetailLevel::Medium | DetailLevel::Complete => true,
        };
        if !include || skips_optional(child) {
            continue;
        }
        let ci = cardinality_index(node, child);
        if is_optional(child) && cardinality_full(node, ci, &card_counts) {
            continue;
        }
        included.push(child.aql_path.as_str());
        emitted |= walk_child(node, child, cx, &mut card_counts, out);
    }
    (emitted, included)
}

/// Whether an OPTIONAL child is omitted from the example.
///
/// A node whose coded-name constraint is display/rubric-incoherent (see
/// `CodedName::incoherent`) or whose leaf is unsatisfiable has no instance
/// form every conforming consumer accepts, and an example exists to be
/// committable everywhere. A MANDATORY one is still emitted in our
/// spec-faithful form.
fn skips_optional(child: &WebTemplateNode) -> bool {
    is_optional(child)
        && (child.name_coded.as_ref().is_some_and(|cn| cn.incoherent)
            || is_unsatisfiable_leaf(child))
}

/// The index of the bounded container cardinality a child sits under, if any.
fn cardinality_index(node: &WebTemplateNode, child: &WebTemplateNode) -> Option<usize> {
    node.cardinalities
        .iter()
        .position(|c| c.max != -1 && child.aql_path.starts_with(c.path.as_str()))
}

/// Whether the container cardinality at `index` has reached its upper bound.
fn cardinality_full(node: &WebTemplateNode, index: Option<usize>, counts: &[usize]) -> bool {
    index.is_some_and(|i| {
        let max = node
            .cardinalities
            .get(i)
            .map_or(usize::MAX, |c| usize::try_from(c.max).unwrap_or(usize::MAX));
        counts.get(i).is_some_and(|&n| n >= max)
    })
}

/// Walks one included child, materialising a second occurrence at the
/// `Complete` level to demonstrate repetition (cardinality permitting).
fn walk_child(
    node: &WebTemplateNode,
    child: &WebTemplateNode,
    cx: &WalkCtx<'_>,
    card_counts: &mut [usize],
    out: &mut Map<String, Value>,
) -> bool {
    let ci = cardinality_index(node, child);
    let child_opt = cx.opt_depth + usize::from(is_optional(child));
    let child_prefix = format!("{}/{}", cx.prefix, seg_for(child));
    // A mandatory child must materialise even when all of *its* children are
    // optional (else the mandatory node would go missing).
    let child_force = !is_optional(child);
    let mut emitted = walk(child, &child_prefix, child_opt, cx.level, child_force, out);
    if emitted && let Some(slot) = ci.and_then(|i| card_counts.get_mut(i)) {
        *slot += 1;
    }
    if cx.level == DetailLevel::Complete
        && emitted
        && is_repeating(child)
        && !cardinality_full(node, ci, card_counts)
    {
        let second_prefix = format!("{}/{}:1", cx.prefix, child.id);
        let second_emitted = walk(child, &second_prefix, child_opt, cx.level, false, out);
        emitted |= second_emitted;
        if second_emitted && let Some(slot) = ci.and_then(|i| card_counts.get_mut(i)) {
            *slot += 1;
        }
    }
    emitted
}

/// Cardinality satisfaction: for every container attribute constrained to
/// `min >= 1`, ensure at least one child under it is materialised (a
/// committable skeleton), even where the level would otherwise skip it.
fn satisfy_cardinalities(
    node: &WebTemplateNode,
    groups: &[&WebTemplateNode],
    included: &[&str],
    cx: &WalkCtx<'_>,
    out: &mut Map<String, Value>,
) -> bool {
    let mut emitted = false;
    for card in &node.cardinalities {
        if card.min.unwrap_or(0) < 1 {
            continue;
        }
        let satisfied = included.iter().any(|p| p.starts_with(card.path.as_str()));
        if !satisfied
            && let Some(child) = groups.iter().find(|c| c.aql_path.starts_with(&card.path))
        {
            let child_prefix = format!("{}/{}", cx.prefix, seg_for(child));
            emitted |= walk(child, &child_prefix, cx.opt_depth, cx.level, true, out);
        }
    }
    emitted
}

/// The children to consider under a container: one per distinct `aql_path` (the
/// first alternative of any polymorphic choice), excluding the `EVENT_CONTEXT`
/// subtree (rebuilt wholesale from the `ctx/…` context keys by `from_flat`).
fn child_groups(node: &WebTemplateNode) -> Vec<&WebTemplateNode> {
    let mut seen: HashSet<&str> = HashSet::new();
    node.children
        .iter()
        .filter(|c| c.rm_type != "EVENT_CONTEXT")
        .filter(|c| seen.insert(c.aql_path.as_str()))
        .collect()
}

/// Whether a node is optional (occurrences lower bound `< 1`).
fn is_optional(node: &WebTemplateNode) -> bool {
    node.min.unwrap_or(0) < 1
}

/// Whether a node may occur more than once (Better's `isRepeating`).
fn is_repeating(node: &WebTemplateNode) -> bool {
    node.max == -1 || node.max > 1
}

/// The flat path segment for a node: `id:0` for a repeating node (`max == -1 ||
/// max > 1`), else the bare `id` — matching
/// [`composition_to_flat`](crate::flat::convert::composition_to_flat) so the example
/// round-trips.
/// Does the generated RM model declare the attribute `aql_path` ends in?
///
/// The tail of an AQL path is the RM attribute the node sits under (predicates
/// stripped). An attribute name no RM class declares cannot be materialized
/// into a conformant instance — see the call site for the `null_flavor` case.
/// The check is name-only, deliberately: FLAT level removal means a Web
/// Template child may sit several RM levels below its Web Template parent, so
/// the parent's class is not the right scope to resolve against.
fn rm_declares(aql_path: &str) -> bool {
    static DECLARED: std::sync::LazyLock<std::collections::BTreeSet<&str>> =
        std::sync::LazyLock::new(|| {
            openehr_rm::v1_2::model::classes()
                .flat_map(|c| c.attributes.iter().map(|a| a.name))
                .collect()
        });
    let tail = aql_path.rsplit('/').next().unwrap_or(aql_path);
    let attr = tail.split('[').next().unwrap_or(tail);
    attr.is_empty() || DECLARED.contains(attr)
}

fn seg_for(node: &WebTemplateNode) -> String {
    if is_repeating(node) {
        format!("{}:0", node.id)
    } else {
        node.id.clone()
    }
}

// ── leaf value emission ─────────────────────────────────────────────────────────

/// Emit the flat `path|suffix` entries for one leaf value at `base`. Returns
/// whether anything was emitted.
fn emit_leaf(node: &WebTemplateNode, base: &str, out: &mut Map<String, Value>) -> bool {
    let rm = node.rm_type.split('<').next().unwrap_or(&node.rm_type);
    match rm {
        "DV_TEXT" | "DV_PARAGRAPH" => {
            put(out, base, "", json!(example_text(node)));
        }
        "DV_URI" => {
            // Honour a C_STRING value pattern (master05 §DV_URI: bare value) and
            // any closed value list before the type default, so the example
            // satisfies the leaf's AOM 1.4 `C_STRING.valid_value`.
            let value = first_input(node)
                .and_then(|i| i.validation.as_ref())
                .and_then(|v| v.pattern.as_deref())
                .and_then(example_for_pattern)
                .or_else(|| list_value(first_input(node)))
                .unwrap_or_else(|| "http://example.org/resource".to_owned());
            put(out, base, "", json!(value));
        }
        "DV_EHR_URI" => {
            // A DV_EHR_URI value MUST have scheme `ehr` (RM data_types
            // `UML/classes/org.openehr.rm.data_types.dv_ehr_uri.adoc`
            // §`Scheme_valid`: `scheme.is_equal(Ehr_scheme)`); this is
            // non-negotiable, so the value is always a valid `ehr:` URI even when a
            // C_STRING pattern also constrains it — an ehr URI that also matches the
            // pattern when one exists ([`ehr_uri_example`]), else the default. (A
            // pattern no ehr URI can match is a defective OPT constraint; the
            // OPTIONAL leaf carrying it is omitted upstream in [`walk`].)
            put(out, base, "", json!(ehr_uri_example(node)));
        }
        "DV_CODED_TEXT" | "DV_STATE" => emit_coded_text(node, base, out),
        "CODE_PHRASE" => emit_code_phrase(node, base, out),
        "DV_QUANTITY" => emit_quantity(node, base, out),
        "DV_COUNT" => {
            let range = first_input(node).and_then(input_range);
            put(out, base, "", json!(pick_count(range)));
        }
        "DV_PROPORTION" => emit_proportion(node, base, out),
        "DV_ORDINAL" => emit_ordinal(node, base, out, false),
        "DV_SCALE" => emit_ordinal(node, base, out, true),
        "DV_BOOLEAN" => put(out, base, "", json!(example_boolean(node))),
        "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => {
            put(out, base, "", json!(example_temporal(node, rm)));
        }
        "DV_DURATION" => put(out, base, "", json!(example_duration(node))),
        "DV_IDENTIFIER" => {
            // `id` is RM-mandatory (RM `data_types` §`DV_IDENTIFIER`); honour a
            // C_STRING constraint on it, else a placeholder.
            let id = input_with_suffix(node, "id")
                .and_then(constrained_string_example)
                .unwrap_or_else(|| "example-id".to_owned());
            put(out, base, "id", json!(id));
            // Fill any OPT-constrained (hence, per the builder, mandated)
            // sub-attribute with a conforming value so the example validates
            // (AOM 1.4 `C_STRING.valid_value`; the validator enforces a
            // constrained-and-mandatory DV_IDENTIFIER sub-attribute's presence).
            for part in ["issuer", "assigner", "type"] {
                if let Some(value) =
                    input_with_suffix(node, part).and_then(constrained_string_example)
                {
                    put(out, base, part, json!(value));
                }
            }
        }
        "DV_MULTIMEDIA" => {
            put(out, base, "", json!("http://example.org/media"));
            // Honour a `C_CODE_PHRASE` list on `media_type` (captured in
            // `code_lists` — the constraint the validator enforces).
            let media = node
                .code_lists
                .iter()
                .find(|cl| cl.attr == "media_type")
                .and_then(|cl| cl.codes.first())
                .map_or("text/plain", String::as_str);
            put(out, base, "mediatype", json!(media));
            // A plausible non-zero size. `Size_valid` is `size >= 0`
            // (`RM docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`
            // §Invariants), so 0 is spec-valid — but a referenced resource of
            // zero bytes is unreal, and these are illustrative examples.
            put(out, base, "size", json!(1024));
        }
        "DV_PARSABLE" => {
            // master05 §DV_PARSABLE: bare `value` + `|formalism`. The formalism is
            // an RM-mandatory String the archetype usually constrains to a closed
            // C_STRING list (e.g. `text/html`); pick its first member so the
            // example is valid — never the bare `"text"` default when a list
            // constrains it (AOM 1.4 `C_STRING.valid_value`).
            let value = input_with_suffix(node, "value")
                .or_else(|| first_input(node))
                .and_then(|i| i.validation.as_ref())
                .and_then(|v| v.pattern.as_deref())
                .and_then(example_for_pattern)
                .unwrap_or_else(|| "example".to_owned());
            let formalism = input_with_suffix(node, "formalism")
                .filter(|i| i.list_open != Some(true))
                .and_then(|i| i.list.first())
                .map_or_else(|| "text".to_owned(), |cv| cv.value.clone());
            put(out, base, "", json!(value));
            put(out, base, "formalism", json!(formalism));
        }
        // A slot NARROWED to a concrete non-default party subtype must
        // materialize: the builder's unset-subject default is PARTY_SELF
        // (master05 §OBSERVATION `/subject` row Note), which the template's own
        // constraint then refuses — so the example emits the minimal members
        // the narrowed subtype mandates. PARTY_RELATED additionally carries
        // `relationship` 1..1 (RM common `party_related.adoc`), whose example
        // code comes from the node's own relationship child.
        "PARTY_IDENTIFIED" => put(out, base, "name", json!("Example party")),
        "PARTY_RELATED" => {
            put(out, base, "name", json!("Example party"));
            let rel_base = format!("{base}/relationship");
            if let Some(rel) = node
                .children
                .iter()
                .find(|c| c.id == "relationship" || c.aql_path.ends_with("/relationship"))
            {
                emit_coded_text(rel, &rel_base, out);
            } else {
                // Unconstrained: the openEHR `related party relationship`
                // group's canonical unknown (TERM 3.1.0).
                put(out, &rel_base, "code", json!("253"));
                put(out, &rel_base, "value", json!("unknown"));
                put(out, &rel_base, "terminology", json!("openehr"));
            }
        }
        // Every OTHER party leaf (an unnarrowed PARTY_PROXY, PARTY_SELF) is
        // skipped rather than fabricated: an unset `ENTRY.subject` defaults
        // to PARTY_SELF and a `COMPOSITION.composer` comes from
        // `ctx/composer_*`, so inventing a named party here would put a
        // fictitious person in the example. See the module NOTE.
        _ => return false,
    }
    true
}

/// Insert a flat entry at `base` (bare) or `base|suffix`.
fn put(out: &mut Map<String, Value>, base: &str, suffix: &str, value: Value) {
    let key = if suffix.is_empty() {
        base.to_owned()
    } else {
        format!("{base}|{suffix}")
    };
    out.insert(key, value);
}

/// A deterministic text value: the first enumerated value (a non-open string
/// list), else the node's rubric name, else a generic placeholder.
fn example_text(node: &WebTemplateNode) -> String {
    if let Some(input) = first_input(node)
        && input.input_type == WebTemplateInputType::Text
        && input.list_open != Some(true)
        && let Some(cv) = input.list.first()
    {
        return cv.value.clone();
    }
    // Honour a C_STRING value pattern when one constrains the free text
    // (AOM 1.4 `C_STRING.valid_value`).
    if let Some(value) = first_input(node)
        .and_then(|i| i.validation.as_ref())
        .and_then(|v| v.pattern.as_deref())
        .and_then(example_for_pattern)
    {
        return value;
    }
    node.name
        .clone()
        .unwrap_or_else(|| "Example text".to_owned())
}

/// A deterministic string satisfying a leaf's `C_STRING` value pattern, for the
/// literal-ish patterns the corpus carries (`/abcdef/`, `/xyz/`). Returns `None`
/// when the pattern uses regex metacharacters (no single canonical instance) —
/// the caller then falls back to its type default. AOM 1.4 `C_STRING.valid_value`
/// (`AM/docs/AOM1.4/master04-constraint_model_package.adoc` §`C_STRING`): a value
/// is valid iff it matches the pattern; the ADL regex is `/`-delimited
/// (`ADL1.4/master05-cadl.adoc` §Regular Expression).
fn example_for_pattern(pattern: &str) -> Option<String> {
    const META: &[char] = &[
        '.', '^', '$', '*', '+', '?', '(', ')', '[', ']', '{', '}', '|', '\\',
    ];
    let body = pattern
        .strip_prefix('/')
        .and_then(|p| p.strip_suffix('/'))
        .unwrap_or(pattern);
    if body.is_empty() || body.chars().any(|c| META.contains(&c)) {
        return None;
    }
    // A literal pattern matches its own text; confirm before committing to it.
    if pattern_matches(body, body) {
        Some(body.to_owned())
    } else {
        None
    }
}

/// A deterministic value satisfying a leaf sub-attribute's `C_STRING` constraint
/// (a closed value list or a value pattern), or `None` when the input carries no
/// enforceable string constraint. Used to fill an OPT-constrained (hence
/// mandated) `DV_IDENTIFIER` sub-attribute (AOM 1.4
/// `master04-constraint_model_package.adoc` §`C_STRING`). For a pattern, the
/// literal pattern body is offered when it satisfies its own regex (e.g. `gov.si`,
/// where `.` matches the literal dot) — a conforming instance even when the
/// pattern carries regex metacharacters that [`example_for_pattern`] declines.
fn constrained_string_example(input: &WebTemplateInput) -> Option<String> {
    if input.list_open != Some(true)
        && let Some(cv) = input.list.first()
    {
        return Some(cv.value.clone());
    }
    let pattern = input.validation.as_ref()?.pattern.as_deref()?;
    example_for_pattern(pattern).or_else(|| {
        let body = pattern
            .strip_prefix('/')
            .and_then(|p| p.strip_suffix('/'))
            .unwrap_or(pattern);
        (!body.is_empty() && pattern_matches(pattern, body)).then(|| body.to_owned())
    })
}

/// Whether `value` satisfies an ADL `C_STRING` regex `pattern` (`/`-delimited,
/// anchored full-match — mirroring the leaf validator's `matches_pattern`). An
/// uninterpretable pattern is treated as matching (it cannot be evaluated, so it
/// does not over-constrain the example).
fn pattern_matches(pattern: &str, value: &str) -> bool {
    let body = pattern
        .strip_prefix('/')
        .and_then(|p| p.strip_suffix('/'))
        .unwrap_or(pattern);
    match regex::Regex::new(&format!("^(?:{body})$")) {
        Ok(re) => re.is_match(value),
        Err(_) => true,
    }
}

/// The C_STRING value pattern constraining a leaf, if any.
fn value_pattern(node: &WebTemplateNode) -> Option<&str> {
    node.inputs
        .first()
        .and_then(|i| i.validation.as_ref())
        .and_then(|v| v.pattern.as_deref())
}

/// The default DV_EHR_URI value — scheme `ehr` (RM `DV_EHR_URI.Scheme_valid`).
const EHR_URI_DEFAULT: &str = "ehr://example.org/composition";

/// A deterministic DV_EHR_URI value: always an `ehr:`-scheme URI (RM data_types
/// `UML/classes/org.openehr.rm.data_types.dv_ehr_uri.adoc` §`Scheme_valid`), and
/// matching the leaf's C_STRING pattern when the default (or a literal instance
/// that is itself an ehr URI) satisfies it. When no ehr URI can satisfy the
/// pattern the default is returned — a valid RM value; the un-committable OPTIONAL
/// leaf carrying such a defective OPT constraint is omitted in [`walk`].
fn ehr_uri_example(node: &WebTemplateNode) -> String {
    let Some(pattern) = value_pattern(node) else {
        return EHR_URI_DEFAULT.to_owned();
    };
    if pattern_matches(pattern, EHR_URI_DEFAULT) {
        return EHR_URI_DEFAULT.to_owned();
    }
    // A literal pattern instance that is itself an `ehr:` URI satisfies both.
    match example_for_pattern(pattern) {
        Some(lit) if lit.starts_with("ehr:") => lit,
        _ => EHR_URI_DEFAULT.to_owned(),
    }
}

/// Whether an OPTIONAL leaf has no committable value because its archetype
/// C_STRING constraint contradicts an RM invariant of the leaf type — currently a
/// DV_EHR_URI whose value pattern no `ehr:`-scheme URI can match (the OPT's
/// C_STRING contradicts RM `DV_EHR_URI.Scheme_valid`). Such a leaf is omitted from
/// the example when optional (the example must be committable — the same posture
/// as an incoherent coded-name node), never fabricated into an invalid value.
fn is_unsatisfiable_leaf(node: &WebTemplateNode) -> bool {
    let rm = node.rm_type.split('<').next().unwrap_or(&node.rm_type);
    if rm != "DV_EHR_URI" {
        return false;
    }
    value_pattern(node).is_some_and(|p| !pattern_matches(p, &ehr_uri_example(node)))
}

fn emit_coded_text(node: &WebTemplateNode, base: &str, out: &mut Map<String, Value>) {
    let input = input_with_suffix(node, "code").or_else(|| first_input(node));
    let (code, value, terminology) = coded_example(input);
    put(out, base, "code", json!(code));
    put(out, base, "value", json!(value));
    put(out, base, "terminology", json!(terminology));
}

fn emit_code_phrase(node: &WebTemplateNode, base: &str, out: &mut Map<String, Value>) {
    let input = input_with_suffix(node, "code").or_else(|| first_input(node));
    let (code, _value, terminology) = coded_example(input);
    put(out, base, "code", json!(code));
    put(out, base, "terminology", json!(terminology));
}

/// The `(code, value, terminology)` for an example coded value: the first entry
/// of the input's coded list, else a placeholder local code. The list labels
/// already carry the display text — term definitions, then the TERM 3.1.0
/// rubric for `openehr` codes (`webtemplate::inputs::coded_value`).
fn coded_example(input: Option<&WebTemplateInput>) -> (String, String, String) {
    match input {
        Some(i) if !i.list.is_empty() => {
            let Some(cv) = i.list.first() else {
                return (String::new(), String::new(), "local".to_owned());
            };
            let terminology = i.terminology.clone().unwrap_or_else(|| "local".to_owned());
            let value = cv.label.clone().unwrap_or_else(|| cv.value.clone());
            (cv.value.clone(), value, terminology)
        }
        Some(i) => (
            "at0000".to_owned(),
            "Example".to_owned(),
            i.terminology.clone().unwrap_or_else(|| "local".to_owned()),
        ),
        None => (
            "at0000".to_owned(),
            "Example".to_owned(),
            "local".to_owned(),
        ),
    }
}

fn emit_quantity(node: &WebTemplateNode, base: &str, out: &mut Map<String, Value>) {
    let unit_input = input_with_suffix(node, "unit");
    let unit = unit_input
        .and_then(|i| i.list.first())
        .map(|cv| cv.value.clone())
        .or_else(|| {
            // No enumerated C_QUANTITY_ITEM unit list: if the constraint fixes a
            // measurement `property`, emit a valid unit for it (the primary/base
            // unit, else the first) so the generated example satisfies the
            // property→units check (openEHR `PropertyUnitData.xml` table). A
            // property with no table entry falls through to the "1" default.
            node.quantity_property.as_deref().and_then(|p| {
                let units = openehr_term::bundle::openehr().units_for_property(p);
                units
                    .iter()
                    .find(|u| u.primary)
                    .or_else(|| units.first())
                    .map(|u| u.text.clone())
            })
        })
        .unwrap_or_else(|| "1".to_owned());
    // Prefer the magnitude input's own range, else the range scoped to the unit.
    let range = input_with_suffix(node, "magnitude")
        .and_then(input_range)
        .or_else(|| {
            unit_input
                .and_then(|i| i.list.first())
                .and_then(|cv| cv.validation.as_ref())
                .and_then(|v| v.range.as_ref())
        });
    put(out, base, "magnitude", json!(pick_decimal(range)));
    put(out, base, "unit", json!(unit));
}

fn emit_proportion(node: &WebTemplateNode, base: &str, out: &mut Map<String, Value>) {
    // Choose values satisfying the DV_PROPORTION invariants for the first
    // allowed kind AND the archetype's numerator/denominator ranges (RM
    // data_types `dv_proportion.adoc`: `Unitary_validity` denominator = 1,
    // `Percent_validity` 100, `Fraction_validity` integral, `Valid_denominator`
    // ≠ 0). `type` codes (PROPORTION_KIND): ratio 0, unitary 1, percent 2,
    // fraction 3, integer_fraction 4. The ranges ride on the
    // `numerator`/`denominator` inputs (master05 §DV_PROPORTION), which the leaf
    // validator range-checks, so the picked values must land inside them.
    let type_code = match node.proportion_types.first().map(String::as_str) {
        Some("unitary") => 1,
        Some("percent") => 2,
        Some("fraction") => 3,
        Some("integer_fraction") => 4,
        _ => 0,
    };
    let num_input = input_with_suffix(node, "numerator");
    let den_input = input_with_suffix(node, "denominator");
    // Fraction kinds force integral numerator+denominator; the builder also types
    // the numerator/denominator inputs as `Integer` when the archetype constrains
    // `is_integral` (RM `Is_integral_validity`).
    let integral = matches!(type_code, 3 | 4)
        || num_input.is_some_and(|i| i.input_type == WebTemplateInputType::Integer)
        || den_input.is_some_and(|i| i.input_type == WebTemplateInputType::Integer);
    let numerator = pick_proportion_part(num_input.and_then(input_range), integral, false);
    let denominator = match type_code {
        1 => 1.0,   // Unitary_validity: denominator = 1
        2 => 100.0, // Percent_validity: denominator = 100
        _ => pick_proportion_part(den_input.and_then(input_range), integral, true),
    };
    put(out, base, "numerator", json!(numerator));
    put(out, base, "denominator", json!(denominator));
    put(out, base, "type", json!(type_code));
}

/// Pick a DV_PROPORTION numerator/denominator inside `range` (deterministic).
/// `integral` forces a whole number (fraction kinds / `is_integral`), `avoid_zero`
/// forbids `0` (a denominator must be non-zero — `Valid_denominator`). Falls back
/// to the nearest in-range decimal when no in-range integer exists (a contradictory
/// integral range — none in the corpus).
fn pick_proportion_part(range: Option<&WebTemplateRange>, integral: bool, avoid_zero: bool) -> f64 {
    let v = pick_decimal(range);
    if !integral {
        return v;
    }
    for cand in [v.round(), v.floor(), v.ceil()] {
        if in_decimal_range(cand, range) && !(avoid_zero && cand == 0.0) {
            return cand;
        }
    }
    v
}

/// Whether `v` satisfies a `WebTemplateRange` (honouring `minOp`/`maxOp`; missing
/// bounds are unbounded) — mirrors the leaf validator's `in_range`.
fn in_decimal_range(v: f64, range: Option<&WebTemplateRange>) -> bool {
    let Some(r) = range else { return true };
    if let Some(min) = r.min.as_ref().and_then(Value::as_f64) {
        let ok = if r.min_op.as_deref() == Some(">") {
            v > min
        } else {
            v >= min
        };
        if !ok {
            return false;
        }
    }
    if let Some(max) = r.max.as_ref().and_then(Value::as_f64) {
        let ok = if r.max_op.as_deref() == Some("<") {
            v < max
        } else {
            v <= max
        };
        if !ok {
            return false;
        }
    }
    true
}

fn emit_ordinal(node: &WebTemplateNode, base: &str, out: &mut Map<String, Value>, scale: bool) {
    let numeric_suffix = if scale { "scale" } else { "ordinal" };
    if let Some(cv) = first_input(node).and_then(|i| i.list.first()) {
        let numeric = if scale {
            json!(cv.scale.unwrap_or(0.0))
        } else {
            json!(cv.ordinal.unwrap_or(0))
        };
        put(out, base, numeric_suffix, numeric);
        put(out, base, "code", json!(cv.value));
        put(
            out,
            base,
            "value",
            json!(cv.label.clone().unwrap_or_else(|| cv.value.clone())),
        );
    } else {
        put(
            out,
            base,
            numeric_suffix,
            if scale { json!(0.0) } else { json!(0) },
        );
        put(out, base, "code", json!("at0000"));
        put(out, base, "value", json!("Example"));
    }
}

/// A deterministic temporal example honouring the leaf's AOM 1.4 temporal
/// pattern (`yyyy-??-XX` etc.: literal = mandatory, `??` = optional/kept,
/// `XX` = prohibited/omitted — mirroring the validator's presence-only
/// judgement) and the `timezone_validity` constraint (`1003` = the designator
/// must be absent; the fixed example instants already carry `Z` for the rest).
fn example_temporal(node: &WebTemplateNode, rm: &str) -> String {
    let pattern = node
        .inputs
        .first()
        .and_then(|i| i.validation.as_ref())
        .and_then(|v| v.pattern.as_deref());
    let (pat_date, pat_time) = match pattern {
        Some(p) if p.contains('T') => {
            let (d, t) = p.split_once('T').unwrap_or((p, ""));
            (d, t)
        }
        Some(p) if p.contains(':') => ("", p),
        Some(p) => (p, ""),
        None => ("", ""),
    };
    let tz = if node.tz_validity == Some(1003) {
        ""
    } else {
        "Z"
    };
    let date = temporal_segments(pat_date, DATE_SEGMENTS, '-');
    match rm {
        "DV_DATE" => date,
        "DV_TIME" => format!("{}{tz}", temporal_segments(pat_time, TIME_SEGMENTS, ':')),
        // DV_DATE_TIME: a time part prohibited outright (pattern `…TXX:…`)
        // truncates to the date.
        _ if pat_time.starts_with("XX") => date,
        _ => format!(
            "{date}T{}{tz}",
            temporal_segments(pat_time, TIME_SEGMENTS, ':')
        ),
    }
}

/// The example date's segments, coarsest first.
const DATE_SEGMENTS: [&str; 3] = ["2022", "02", "03"];

/// The example time's segments, coarsest first.
const TIME_SEGMENTS: [&str; 3] = ["04", "05", "06"];

/// Renders one temporal half, dropping every segment the pattern prohibits.
///
/// The leading segment is always present; the rest are kept until the pattern
/// marks one `XX`, since ISO 8601 partial precision truncates right-to-left —
/// an omitted month forces the day off with it.
fn temporal_segments(pattern: &str, full: [&str; 3], separator: char) -> String {
    let segments: Vec<&str> = if pattern.is_empty() {
        Vec::new()
    } else {
        pattern.splitn(3, separator).collect()
    };
    let mut out = String::from(full[0]);
    for (i, part) in full.iter().enumerate().skip(1) {
        if segments.get(i).is_some_and(|s| *s == "XX") {
            break;
        }
        out.push(separator);
        out.push_str(part);
    }
    out
}

/// A deterministic duration example honouring the `C_DURATION` allowed-fields
/// pattern (encoded by the builder as which per-field inputs exist — mirroring
/// the validator) and the ISO duration range: the range minimum when one is
/// declared, else one unit of the first allowed field, else `PT1H`.
fn example_duration(node: &WebTemplateNode) -> String {
    // ISO-8601 duration fields, coarsest → finest, with the RM's nominal field
    // lengths in seconds (matching the leaf validator's `duration_seconds`, so a
    // picked value agrees with the range check byte-for-byte): year = 365.25 d,
    // month = 30.4375 d (RM data_types §DV_DURATION magnitude semantics).
    const ISO: &[(&str, bool, char, f64)] = &[
        ("year", false, 'Y', 31_557_600.0),
        ("month", false, 'M', 2_629_800.0),
        ("week", false, 'W', 604_800.0),
        ("day", false, 'D', 86_400.0),
        ("hour", true, 'H', 3_600.0),
        ("minute", true, 'M', 60.0),
        ("second", true, 'S', 1.0),
    ];
    let allowed: Vec<&str> = node
        .inputs
        .iter()
        .filter_map(|i| i.suffix.as_deref())
        .collect();
    // The fields the C_DURATION pattern permits (all when the node lists none or
    // the full seven), coarsest → finest.
    let usable: Vec<&(&str, bool, char, f64)> = ISO
        .iter()
        .filter(|f| allowed.is_empty() || allowed.len() >= 7 || allowed.contains(&f.0))
        .collect();
    // A whole count `n` of `field` (kept as f64 to avoid a lossy float→int cast;
    // printed with no fractional digits).
    let fmt = |field: &(&str, bool, char, f64), n: f64| -> String {
        let (_, is_time, letter, _) = *field;
        if is_time {
            format!("PT{n:.0}{letter}")
        } else {
            format!("P{n:.0}{letter}")
        }
    };
    // Unconstrained: one unit of the coarsest usable field (the historic default).
    let Some(range) = node.duration_range.as_ref() else {
        return usable
            .first()
            .map_or_else(|| EXAMPLE_DURATION.to_owned(), |field| fmt(field, 1.0));
    };
    // Ranged: express a whole count of the FINEST usable field (maximal
    // granularity) inside [min, max] honouring the AOM interval inclusivity
    // (`minOp`/`maxOp` `>`/`<` = excluded bound; BASE foundation_types Interval).
    let field = usable
        .last()
        .copied()
        .unwrap_or(&("hour", true, 'H', 3_600.0));
    let unit = field.3;
    let min_secs = range.min.as_ref().and_then(Value::as_str).map(iso_seconds);
    let max_secs = range.max.as_ref().and_then(Value::as_str).map(iso_seconds);
    let min_excl = range.min_op.as_deref() == Some(">");
    let max_excl = range.max_op.as_deref() == Some("<");
    // The whole-count bounds (in field units) satisfying the second bounds,
    // kept in f64 to avoid a lossy float→int cast.
    let n_lo = match min_secs {
        Some(m) if min_excl => (m / unit).floor() + 1.0,
        Some(m) => (m / unit).ceil(),
        None => 0.0,
    }
    .max(0.0);
    let n_hi = match max_secs {
        Some(m) if max_excl => (m / unit).ceil() - 1.0,
        Some(m) => (m / unit).floor(),
        None => f64::INFINITY,
    };
    if n_lo > n_hi {
        // Contradictory bounds for this field — no in-range whole count exists.
        // Best effort: the literal minimum bound (or one unit).
        return range
            .min
            .as_ref()
            .and_then(Value::as_str)
            .map_or_else(|| fmt(field, 1.0), str::to_owned);
    }
    // Target: the midpoint when bounded above, else just above the minimum.
    let target = match max_secs {
        Some(mx) => f64::midpoint(min_secs.unwrap_or(0.0), mx),
        None => min_secs.unwrap_or(0.0) + unit,
    };
    let n = (target / unit).round().max(n_lo).min(n_hi);
    fmt(field, n)
}

/// Total seconds of an ISO-8601 duration string, using the RM's nominal field
/// lengths (the same constants the leaf validator's `duration_seconds` uses, so a
/// picked example duration and the range check agree). A part that does not parse
/// contributes nothing (the RM-invariant pass owns well-formedness).
fn iso_seconds(value: &str) -> f64 {
    let rest = value.strip_prefix('-').unwrap_or(value);
    let Some(rest) = rest.strip_prefix('P') else {
        return 0.0;
    };
    let (date_part, time_part) = rest.split_once('T').unwrap_or((rest, ""));
    iso_part_seconds(date_part, false) + iso_part_seconds(time_part, true)
}

/// Total seconds of one half of an ISO-8601 duration string.
///
/// `in_time` selects the time-half reading of the ambiguous `M` designator
/// (minutes rather than months).
fn iso_part_seconds(part: &str, in_time: bool) -> f64 {
    let mut total = 0.0;
    let mut num = String::new();
    for ch in part.chars() {
        if ch.is_ascii_digit() || ch == '.' || ch == ',' {
            num.push(if ch == ',' { '.' } else { ch });
            continue;
        }
        let n: f64 = num.parse().unwrap_or(0.0);
        num.clear();
        total += n * iso_designator_seconds(ch, in_time);
    }
    total
}

/// The RM's nominal length in seconds of one ISO-8601 duration designator.
fn iso_designator_seconds(designator: char, in_time: bool) -> f64 {
    match (designator, in_time) {
        ('Y', false) => 31_557_600.0,
        ('M', false) => 2_629_800.0,
        ('W', false) => 604_800.0,
        ('D', false) => 86_400.0,
        ('H', true) => 3_600.0,
        ('M', true) => 60.0,
        ('S', true) => 1.0,
        _ => 0.0,
    }
}

/// A deterministic boolean example: `false` when the archetype allows only
/// `false`, else `true`.
fn example_boolean(node: &WebTemplateNode) -> bool {
    let only_false = first_input(node).is_some_and(|i| {
        i.list.iter().any(|c| c.value == "false") && !i.list.iter().any(|c| c.value == "true")
    });
    !only_false
}

// ── input helpers ───────────────────────────────────────────────────────────────

fn first_input(node: &WebTemplateNode) -> Option<&WebTemplateInput> {
    node.inputs.first()
}

fn input_with_suffix<'a>(node: &'a WebTemplateNode, suffix: &str) -> Option<&'a WebTemplateInput> {
    node.inputs
        .iter()
        .find(|i| i.suffix.as_deref() == Some(suffix))
}

/// The first enumerated value of a (non-open) text input, if any.
fn list_value(input: Option<&WebTemplateInput>) -> Option<String> {
    let input = input?;
    if input.list_open == Some(true) {
        return None;
    }
    input.list.first().map(|cv| cv.value.clone())
}

fn input_range(input: &WebTemplateInput) -> Option<&WebTemplateRange> {
    input.validation.as_ref().and_then(|v| v.range.as_ref())
}

// ── numeric picking (deterministic, range-clamped) ──────────────────────────────

/// A plausible, non-zero decimal example value within `range`.
///
/// An unconstrained magnitude gets a deterministic non-zero default
/// (`DEFAULT_DECIMAL`); a range-constrained one is placed at the range
/// **midpoint** when both bounds exist (honouring the open-bound `>`/`<`
/// operators), and clamped toward the single stated bound otherwise. The result
/// never lands on `0.0` when the range admits a non-zero value: a zero example
/// magnitude is clinically unreal and defeats the multiplicative payload jitter
/// the example skeletons feed. Deterministic (no randomness): the same range
/// always yields the same value. The FLAT/STRUCTURED formats have no versioned
/// openEHR spec of their own — this pick policy is our own design.
fn pick_decimal(range: Option<&WebTemplateRange>) -> f64 {
    /// The default when no numeric bound is stated (a plausible non-zero value).
    const DEFAULT_DECIMAL: f64 = 10.0;
    /// The step taken across an open (`>` / `<`) bound.
    const STEP: f64 = 1.0;

    let Some(r) = range else {
        return DEFAULT_DECIMAL;
    };
    // Effective bounds, moved just inside an open (`>` / `<`) constraint.
    let lo = r.min.as_ref().and_then(Value::as_f64).map(|min| {
        if r.min_op.as_deref() == Some(">") {
            min + STEP
        } else {
            min
        }
    });
    let hi = r.max.as_ref().and_then(Value::as_f64).map(|max| {
        if r.max_op.as_deref() == Some("<") {
            max - STEP
        } else {
            max
        }
    });
    match (lo, hi) {
        (Some(lo), Some(hi)) => {
            let mid = f64::midpoint(lo, hi);
            // A midpoint of 0.0 on a range symmetric about zero (e.g. [-5, 5])
            // would land on the forbidden zero even though a non-zero value is
            // in range — bias to the positive half instead.
            if mid == 0.0 && hi > 0.0 {
                hi / 2.0
            } else {
                mid
            }
        }
        // Only a lower bound: sit at the floor, or the default when that is not
        // above zero (so the pick stays non-zero and in range).
        (Some(lo), None) => lo.max(DEFAULT_DECIMAL),
        // Only an upper bound: sit at the ceiling, or the default when that is
        // not below zero; a non-positive ceiling admits only a value below it.
        (None, Some(hi)) => {
            let v = hi.min(DEFAULT_DECIMAL);
            if v == 0.0 { hi - STEP } else { v }
        }
        (None, None) => DEFAULT_DECIMAL,
    }
}

/// An integer example value clamped into `range` (defaulting to `0`).
fn pick_count(range: Option<&WebTemplateRange>) -> i64 {
    let Some(r) = range else { return 0 };
    let mut v: i64 = 0;
    if let Some(min) = r.min.as_ref().and_then(Value::as_i64) {
        let lo = if r.min_op.as_deref() == Some(">") {
            min + 1
        } else {
            min
        };
        if v < lo {
            v = lo;
        }
    }
    if let Some(max) = r.max.as_ref().and_then(Value::as_i64) {
        let hi = if r.max_op.as_deref() == Some("<") {
            max - 1
        } else {
            max
        };
        if v > hi {
            v = hi;
        }
    }
    v
}

// ── deterministic UUID (for the output-form uid) ────────────────────────────────

/// A deterministic, RFC-4122-shaped UUID derived from `seed` (FNV-1a; no
/// randomness). Not a cryptographic v5, but syntactically a valid UUID and stable
/// for a given seed — sufficient for a pedagogical example's `OBJECT_VERSION_ID`.
fn deterministic_uuid(seed: &str) -> String {
    let h1 = fnv1a64(seed.as_bytes(), 0xcbf2_9ce4_8422_2325);
    let h2 = fnv1a64(seed.as_bytes(), 0x8422_2325_cbf2_9ce4);
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&h1.to_be_bytes());
    b[8..].copy_from_slice(&h2.to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x50; // version 5 nibble
    b[8] = (b[8] & 0x3f) | 0x80; // RFC-4122 variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15],
    )
}

fn fnv1a64(data: &[u8], basis: u64) -> u64 {
    let mut hash = basis;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::webtemplate::builder::build_web_template;

    fn web_template(opt_rel: &str) -> WebTemplate {
        let path = format!("{}/{opt_rel}", env!("CARGO_MANIFEST_DIR"));
        let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let opt = openehr_its::opt14::from_xml(&xml).expect("parse OPT");
        build_web_template(&opt).expect("build web template")
    }

    /// The Demo Vitals template (OBSERVATION with a history of events) vendored
    /// for the FLAT tests.
    fn demo_vitals() -> WebTemplate {
        web_template("tests/fixtures/better/Demo Vitals.opt")
    }

    fn leaf_count(comp: &Value) -> usize {
        // Count populated DATA_VALUE leaves as a coarse coverage metric.
        fn rec(v: &Value, n: &mut usize) {
            match v {
                Value::Object(m) => {
                    if m.get("_type")
                        .and_then(Value::as_str)
                        .is_some_and(|t| t.starts_with("DV_"))
                    {
                        *n += 1;
                    }
                    for val in m.values() {
                        rec(val, n);
                    }
                }
                Value::Array(a) => {
                    for e in a {
                        rec(e, n);
                    }
                }
                _ => {}
            }
        }
        let mut n = 0;
        rec(comp, &mut n);
        n
    }

    #[test]
    fn parses_detail_level() {
        assert_eq!(DetailLevel::from_query(None), Ok(DetailLevel::Required));
        assert_eq!(
            DetailLevel::from_query(Some("required")),
            Ok(DetailLevel::Required)
        );
        assert_eq!(
            DetailLevel::from_query(Some("medium")),
            Ok(DetailLevel::Medium)
        );
        assert_eq!(
            DetailLevel::from_query(Some("complete")),
            Ok(DetailLevel::Complete)
        );
        assert!(DetailLevel::from_query(Some("full")).is_err());
    }

    #[test]
    fn parses_example_type() {
        assert_eq!(ExampleType::from_query(None), Ok(ExampleType::Input));
        assert_eq!(
            ExampleType::from_query(Some("input")),
            Ok(ExampleType::Input)
        );
        assert_eq!(
            ExampleType::from_query(Some("output")),
            Ok(ExampleType::Output)
        );
        assert!(ExampleType::from_query(Some("bogus")).is_err());
    }

    // A present out-of-enum value is refused even when it is empty or a
    // whitespace-padded spelling of a token: the declared default applies
    // only to an ABSENT parameter (ITS-REST
    // parameters/query/example_detail_level.yaml, example_type.yaml — closed
    // enums), so no present value may silently become one.
    #[test]
    fn refuses_present_out_of_enum_values() {
        for bad in ["", " ", " required ", "REQUIRED", "required\n"] {
            assert!(
                DetailLevel::from_query(Some(bad)).is_err(),
                "detail_level {bad:?} must refuse"
            );
        }
        for bad in ["", " ", " input ", "INPUT", "output "] {
            assert!(
                ExampleType::from_query(Some(bad)).is_err(),
                "type {bad:?} must refuse"
            );
        }
    }

    #[test]
    fn is_deterministic() {
        let wt = demo_vitals();
        let a = example_composition(&wt, DetailLevel::Medium);
        let b = example_composition(&wt, DetailLevel::Medium);
        assert_eq!(a, b, "two calls produce identical JSON");
    }

    #[test]
    fn is_a_composition_with_housekeeping() {
        let wt = demo_vitals();
        let comp = example_composition(&wt, DetailLevel::Required);
        assert_eq!(
            comp.get("_type").and_then(Value::as_str),
            Some("COMPOSITION")
        );
        assert!(comp.get("language").is_some(), "mandatory language present");
        assert!(
            comp.get("territory").is_some(),
            "mandatory territory present"
        );
        assert!(comp.get("composer").is_some(), "mandatory composer present");
        assert!(comp.get("category").is_some(), "mandatory category present");
        assert_eq!(
            comp.pointer("/composer/name").and_then(Value::as_str),
            Some("Example composer")
        );
        assert_eq!(
            comp.pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str),
            Some(wt.template_id.as_str()),
            "self-describing template id"
        );
    }

    #[test]
    fn medium_populates_optional_content() {
        // The level's whole point: a template whose content chain is entirely
        // optional still yields a populated document at `medium` (the empty
        // `required` skeleton is the committable *minimum*, not the example).
        let wt = demo_vitals();
        let comp = example_composition(&wt, DetailLevel::Medium);
        let content = comp.get("content").and_then(Value::as_array);
        assert!(
            content.is_some_and(|c| !c.is_empty()),
            "medium example carries content entries"
        );
        assert!(
            leaf_count(&comp) > leaf_count(&example_composition(&wt, DetailLevel::Required)),
            "medium populates strictly more leaves than required"
        );
    }

    #[test]
    fn complete_demonstrates_repetition() {
        let wt = demo_vitals();
        let medium = example_composition(&wt, DetailLevel::Medium);
        let complete = example_composition(&wt, DetailLevel::Complete);
        assert!(
            leaf_count(&complete) > leaf_count(&medium),
            "complete adds second occurrences of repeating nodes"
        );
    }

    #[test]
    fn levels_are_monotonic() {
        let wt = demo_vitals();
        let required = leaf_count(&example_composition(&wt, DetailLevel::Required));
        let medium = leaf_count(&example_composition(&wt, DetailLevel::Medium));
        let complete = leaf_count(&example_composition(&wt, DetailLevel::Complete));
        assert!(
            required <= medium && medium <= complete,
            "coverage grows with detail: required={required}, medium={medium}, complete={complete}"
        );
        assert!(
            complete > required,
            "a rich template exposes more leaves at complete than required \
             (required={required}, complete={complete})"
        );
    }

    #[test]
    fn output_populates_a_deterministic_uid() {
        let wt = demo_vitals();
        let mut input = example_composition(&wt, DetailLevel::Required);
        assert!(input.get("uid").is_none(), "input form has no uid");

        let mut output = input.clone();
        apply_output_uid(&mut output, &wt.template_id);
        let uid = output
            .pointer("/uid/value")
            .and_then(Value::as_str)
            .expect("output uid");
        assert!(
            uid.ends_with("::example.server::1"),
            "OBJECT_VERSION_ID form: {uid}"
        );
        assert_eq!(
            output.pointer("/uid/_type").and_then(Value::as_str),
            Some("OBJECT_VERSION_ID")
        );

        // Deterministic across calls.
        apply_output_uid(&mut input, &wt.template_id);
        assert_eq!(input.get("uid"), output.get("uid"));
    }

    fn range(
        min: Option<f64>,
        min_op: Option<&str>,
        max: Option<f64>,
        max_op: Option<&str>,
    ) -> WebTemplateRange {
        WebTemplateRange {
            min_op: min_op.map(str::to_owned),
            min: min.map(|m| json!(m)),
            max_op: max_op.map(str::to_owned),
            max: max.map(|m| json!(m)),
        }
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the example picker returns exactly representable values, so bit-equality is the intended test"
    )]
    fn pick_decimal_is_non_zero_and_in_range() {
        // Unconstrained: a plausible non-zero default, not 0.0.
        assert_eq!(pick_decimal(None), 10.0);
        // Both bounds present: the midpoint (never the min, never 0.0).
        assert_eq!(
            pick_decimal(Some(&range(Some(0.0), None, Some(10.0), None))),
            5.0
        );
        assert_eq!(
            pick_decimal(Some(&range(Some(60.0), None, Some(80.0), None))),
            70.0
        );
        // A range symmetric about zero must not yield the forbidden 0.0.
        let v = pick_decimal(Some(&range(Some(-5.0), None, Some(5.0), None)));
        assert!(
            v != 0.0 && (-5.0..=5.0).contains(&v),
            "in range, non-zero: {v}"
        );
        // Only a lower bound: at the floor when it is above zero, else the default.
        assert_eq!(
            pick_decimal(Some(&range(Some(50.0), None, None, None))),
            50.0
        );
        assert_eq!(
            pick_decimal(Some(&range(Some(0.0), None, None, None))),
            10.0
        );
        // Only an upper bound: below the ceiling, non-zero.
        assert_eq!(
            pick_decimal(Some(&range(None, None, Some(100.0), None))),
            10.0
        );
        assert_eq!(pick_decimal(Some(&range(None, None, Some(4.0), None))), 4.0);
        // Open bounds are honoured (`>` / `<` exclude the stated value).
        assert_eq!(
            pick_decimal(Some(&range(Some(0.0), Some(">"), None, None))),
            10.0
        );
        let hi = pick_decimal(Some(&range(None, None, Some(1.0), Some("<"))));
        assert!(hi < 1.0, "strictly below an open upper bound: {hi}");
    }

    #[test]
    fn pick_decimal_populated_magnitudes_are_non_zero() {
        fn collect(v: &Value, out: &mut Vec<f64>) {
            match v {
                Value::Object(m) => {
                    if m.get("_type").and_then(Value::as_str) == Some("DV_QUANTITY")
                        && let Some(x) = m.get("magnitude").and_then(Value::as_f64)
                    {
                        out.push(x);
                    }
                    m.values().for_each(|c| collect(c, out));
                }
                Value::Array(a) => a.iter().for_each(|e| collect(e, out)),
                _ => {}
            }
        }
        // A populated example must not seed a zero-valued quantity magnitude
        // (clinically unreal; defeats multiplicative jitter).
        let wt = demo_vitals();
        let comp = example_composition(&wt, DetailLevel::Medium);
        let mut mags = Vec::new();
        collect(&comp, &mut mags);
        assert!(!mags.is_empty(), "Demo Vitals exposes DV_QUANTITY leaves");
        assert!(
            mags.iter().all(|m| *m != 0.0),
            "no example magnitude is zero: {mags:?}"
        );
    }
}
