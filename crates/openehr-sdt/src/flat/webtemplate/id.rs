// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Web-template node `id` generation and sibling de-duplication.
//!
//! Implements the node-id algorithm of the openEHR Simplified Formats
//! specification (`ITS-REST simplified_formats master04-basic_concepts.adoc`
//! §"Node ID Generation Rules") — the seven ordered steps that turn an archetype
//! node name into the `id` segment used to build FLAT/STRUCTURED paths
//! (master04 §"Field Identifiers", §"Path Construction"):
//!
//! 1. character normalisation — replace every char that is not
//!    `\p{Alphabetic}`, `0-9`, `_`, `.` or `-` with `_`;
//! 2. underscore consolidation — collapse runs of `_` to one;
//! 3. case normalisation — lowercase;
//! 4. trim leading/trailing `_`;
//! 5. empty result → `"id"`;
//! 6. a leading digit → prepend `"a"`;
//! 7. sibling uniqueness — append an `_`-separated numeric suffix (`blood_pressure`,
//!    `blood_pressure_1`, …) counting from 1.
//!
//! Steps 1–6 are [`generate_node_id`]; step 7 is [`Deduplicator`].
//!
//! Beyond those seven steps, [`build_ids`] derives the BASE NAME a node's id is
//! generated from, orders a polymorphic `ELEMENT`'s alternative `DV_*` children,
//! and resolves the `cardinalities`/`dependsOn` child-id references. The spec
//! defines only the name→id transform, so those mechanics are our own
//! design/extension; the base-name fallback mirrors the metadata example
//! (master04 §"Web Template Metadata").

use std::collections::{HashMap, HashSet};

use super::model::WebTemplateNode;

/// Steps 1–6 of the node-id algorithm (`ITS-REST simplified_formats master04
/// §"Node ID Generation Rules"`): turn an archetype node `name` into an `id`
/// (sibling uniqueness — step 7 — is then applied by [`Deduplicator`]).
fn generate_node_id(name: &str) -> String {
    // Step 1 (character normalisation) folded with step 3 (case normalisation):
    // keep `\p{Alphabetic}` / `0-9` / `_` / `.` / `-`, lowercasing the survivors;
    // replace everything else with `_`. Lowercasing an alphabetic char never adds
    // or removes an `_`, so folding step 3 into this pass leaves steps 2 and 4
    // unaffected. `char::is_alphabetic` is the Unicode `Alphabetic` property.
    let mut normalised = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_alphabetic() || ch.is_ascii_digit() || ch == '_' || ch == '.' || ch == '-' {
            normalised.extend(ch.to_lowercase());
        } else {
            normalised.push('_');
        }
    }
    // Step 2 (underscore consolidation): collapse runs of `_` to one.
    let mut collapsed = String::with_capacity(normalised.len());
    let mut prev_underscore = false;
    for ch in normalised.chars() {
        if ch == '_' {
            if !prev_underscore {
                collapsed.push('_');
            }
            prev_underscore = true;
        } else {
            collapsed.push(ch);
            prev_underscore = false;
        }
    }
    // Step 4 (trim underscores): drop leading/trailing `_`.
    let trimmed = collapsed.trim_matches('_');
    // Step 5 (empty id handling): empty result → "id".
    if trimmed.is_empty() {
        return "id".to_owned();
    }
    // Step 6 (numeric prefix handling): a leading digit → prepend "a".
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("a{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

/// Step 7 (sibling uniqueness) of the node-id algorithm (`ITS-REST
/// simplified_formats master04 §"Node ID Generation Rules"`): append an
/// `_`-separated numeric suffix, counting from 1, so an `id` is unique among its
/// siblings (a duplicate `blood_pressure` → `blood_pressure_1`,
/// `blood_pressure_2`, …). Uniqueness is scoped per parent id.
#[derive(Default)]
struct Deduplicator {
    used: HashMap<String, HashSet<String>>,
}

impl Deduplicator {
    fn unique(&mut self, parent_id: &str, base: &str) -> String {
        let set = self.used.entry(parent_id.to_owned()).or_default();
        if set.insert(base.to_owned()) {
            return base.to_owned();
        }
        // Append the first free `_<n>` suffix (from 1). The loop always
        // terminates: each iteration either claims a fresh suffix or advances,
        // and the sibling set is finite.
        let mut i = 1_usize;
        loop {
            let candidate = format!("{base}_{i}");
            if set.insert(candidate.clone()) {
                return candidate;
            }
            i += 1;
        }
    }
}

/// The typed id for a polymorphic (choice) `ELEMENT`'s `DV_*` alternative
/// (`DV_QUANTITY` → `quantity_value`, `DV_INTERVAL<DV_QUANTITY>` →
/// `interval_of_quantity_value`). No openEHR spec governs how a choice ELEMENT's
/// alternatives are named at the FLAT level — our own design/extension.
fn typed_id(rm_type: &str) -> String {
    if let Some(rest) = rm_type
        .strip_prefix("DV_INTERVAL<DV_")
        .and_then(|s| s.strip_suffix('>'))
    {
        format!("interval_of_{}_value", rest.to_lowercase())
    } else if let Some(rest) = rm_type.strip_prefix("DV_") {
        format!("{}_value", rest.to_lowercase())
    } else {
        format!("{}_value", rm_type.to_lowercase())
    }
}

/// The base *name* a node's id is generated from: the node rubric `name` when
/// present; otherwise `"value"` for a leaf under an `ELEMENT` (master04
/// §"Flat format": "there is no distinction between ELEMENT and its value —
/// elements ARE their values"), except a `DV_INTERVAL` value which keeps its own
/// name; otherwise the last RM attribute path segment (the metadata example's
/// `context`/`category`/`language` shape). The name→id transform is applied by
/// [`generate_node_id`]. No openEHR spec governs this fallback source — our own
/// design/extension.
fn base_name(node: &WebTemplateNode, parent_rm_type: Option<&str>) -> String {
    match &node.name {
        Some(n) if !n.trim().is_empty() => n.clone(),
        _ => {
            if parent_rm_type == Some("ELEMENT") && !node.rm_type.starts_with("DV_INTERVAL") {
                "value".to_owned()
            } else {
                last_path_segment(&node.aql_path)
            }
        }
    }
}

fn last_path_segment(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((_, last)) => last.to_owned(),
        None => path.to_owned(),
    }
}

/// Order a choice `ELEMENT`'s children so `DV_CODED_TEXT` precedes `DV_TEXT`, so
/// the coded alternative takes the primary `value` id and the free-text one the
/// `value2` alternate. No openEHR spec governs choice-ELEMENT rendering — our own
/// design/extension.
fn fix_polymorphic_order(children: &mut Vec<WebTemplateNode>) {
    let coded = children.iter().position(|c| c.rm_type == "DV_CODED_TEXT");
    let text = children.iter().position(|c| c.rm_type == "DV_TEXT");
    if let (Some(coded), Some(text)) = (coded, text)
        && coded > text
    {
        let node = children.remove(coded);
        children.insert(text, node);
    }
}

/// Assign the node `id` (and full path chain) across the whole tree, then resolve
/// the `cardinalities`/`dependsOn` child-id references.
pub(crate) fn build_ids(root: &mut WebTemplateNode) {
    let mut dedup = Deduplicator::default();
    assign(root, "", &mut dedup, None, None);
}

fn assign(
    node: &mut WebTemplateNode,
    parent_prefix: &str,
    dedup: &mut Deduplicator,
    forced_base: Option<String>,
    parent_rm_type: Option<&str>,
) {
    let raw_name = forced_base.unwrap_or_else(|| base_name(node, parent_rm_type));
    let base = generate_node_id(&raw_name);
    let id = dedup.unique(parent_prefix, &base);
    node.full_id = format!("{parent_prefix}{id}");
    node.id = id;

    let is_choice = node.rm_type == "ELEMENT" && node.children.len() > 1;
    if is_choice {
        fix_polymorphic_order(&mut node.children);
    }
    let child_prefix = format!("{}/", node.full_id);
    let rm_type = node.rm_type.clone();
    for (index, child) in node.children.iter_mut().enumerate() {
        let fb = if is_choice && child.rm_type.starts_with("DV_") {
            // The polymorphic alternate id (`value`/`value2`/…) the FLAT
            // converters and validation walk match against — our own
            // design/extension (no openEHR spec governs choice-ELEMENT ids).
            child.alt_json_id = Some(if index > 0 {
                format!("value{}", index + 1)
            } else {
                "value".to_owned()
            });
            Some(typed_id(&child.rm_type))
        } else {
            None
        };
        assign(child, &child_prefix, dedup, fb, Some(&rm_type));
    }

    resolve_cardinalities(node);
    resolve_depends_on(node);
}

/// Fill each cardinality's `ids` from the child json-ids whose aql path is under
/// the cardinality attribute path; drop cardinalities that match nothing. The
/// `cardinalities` field carries container `{min, max, ids}` bounds; no openEHR
/// spec governs this id back-reference — our own design/extension (consumed by
/// interop consumers and the validation walk).
fn resolve_cardinalities(node: &mut WebTemplateNode) {
    if node.cardinalities.is_empty() {
        return;
    }
    let child_ids: Vec<(String, String)> = node
        .children
        .iter()
        .map(|c| (c.aql_path.clone(), c.id.clone()))
        .collect();
    node.cardinalities.retain_mut(|card| {
        let ids: Vec<String> = child_ids
            .iter()
            .filter(|(path, _)| path.starts_with(&card.path))
            .map(|(_, id)| id.clone())
            .collect();
        if ids.is_empty() {
            false
        } else {
            card.ids = Some(ids);
            true
        }
    });
}

/// Resolve each child's `dependsOn` RM paths to sibling json-ids: a dependency
/// resolves to the ids of non-`inContext` siblings whose aql path starts with the
/// dependency path. `dependsOn` is part of the metadata document shape (master04
/// §"Web Template Metadata": the `position`/`method` nodes carry a `dependsOn`
/// list); the RM-path→sibling-id resolution itself is our own design/extension.
fn resolve_depends_on(node: &mut WebTemplateNode) {
    let siblings: Vec<(String, String, bool)> = node
        .children
        .iter()
        .map(|c| (c.aql_path.clone(), c.id.clone(), c.in_context == Some(true)))
        .collect();
    for child in &mut node.children {
        let Some(paths) = child.depends_on.take() else {
            continue;
        };
        let mut ids: Vec<String> = Vec::new();
        for path in &paths {
            for (spath, sid, in_context) in &siblings {
                if !in_context && spath.starts_with(path) && !ids.contains(sid) {
                    ids.push(sid.clone());
                }
            }
        }
        child.depends_on = if ids.is_empty() { None } else { Some(ids) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_generation_examples() {
        // The worked example table of `ITS-REST simplified_formats master04
        // §"Node ID Generation Rules"`.
        assert_eq!(generate_node_id("Body temperature"), "body_temperature");
        assert_eq!(generate_node_id("Problem/diagnosis"), "problem_diagnosis");
        assert_eq!(generate_node_id("Tests (1, 2, 3)"), "tests_1_2_3");
        assert_eq!(generate_node_id("1st visit"), "a1st_visit");
        assert_eq!(generate_node_id("Blood Pressure"), "blood_pressure");
    }

    #[test]
    fn empty_result_becomes_id() {
        // Step 5: a name that normalises to empty → "id".
        assert_eq!(generate_node_id(""), "id");
        assert_eq!(generate_node_id("()"), "id");
        assert_eq!(generate_node_id("___"), "id");
    }

    #[test]
    fn preserves_allowed_punctuation_and_consolidates_underscores() {
        // Steps 1/2/4: dots and dashes are allowed and survive; runs of `_`
        // collapse to one; leading/trailing `_` are trimmed.
        assert_eq!(generate_node_id("keeps.dot-dash"), "keeps.dot-dash");
        assert_eq!(generate_node_id("__weird__name__"), "weird_name");
        assert_eq!(generate_node_id("  a / b  "), "a_b");
    }

    #[test]
    fn duplicate_siblings_get_numeric_suffix() {
        // Step 7 (master04 example table): a duplicate "Blood Pressure" among the
        // same siblings becomes `blood_pressure_1`, then `_2`, …
        let mut dedup = Deduplicator::default();
        assert_eq!(dedup.unique("root", "blood_pressure"), "blood_pressure");
        assert_eq!(dedup.unique("root", "blood_pressure"), "blood_pressure_1");
        assert_eq!(dedup.unique("root", "blood_pressure"), "blood_pressure_2");
        // Uniqueness is scoped per parent — a different parent starts fresh.
        assert_eq!(dedup.unique("other", "blood_pressure"), "blood_pressure");
    }

    #[test]
    fn typed_ids_for_polymorphic_values() {
        assert_eq!(typed_id("DV_QUANTITY"), "quantity_value");
        assert_eq!(typed_id("DV_CODED_TEXT"), "coded_text_value");
        assert_eq!(
            typed_id("DV_INTERVAL<DV_QUANTITY>"),
            "interval_of_quantity_value"
        );
    }
}
