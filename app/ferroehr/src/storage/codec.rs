// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Canonical JSON ⇄ node rows — the pure content transform at the heart of the
//! decomposed store.
//!
//! No openEHR spec governs storage; this is our own design
//!. [`decompose`] turns a versioned object's
//! canonical JSON into nested-set-numbered [`NodeRow`]s (structure children
//! pruned out of their parents' fragments, everything else kept verbatim);
//! [`reassemble`] is its lossless inverse over the lean [`crate::storage::row::ReadRow`] the
//! repository fetches back. The codec never re-formats a leaf value — a leaf's
//! lexical form (ISO-8601 partial precision, decimal-comma, timezone suffix,
//! duration form; BASE `foundation_types` master06) survives verbatim inside its
//! `data` fragment.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::{Map, Value};

use crate::storage::error::StorageError;
use crate::storage::row::{NodeContent, NodeRow};
use crate::storage::structure::{archetype_parts, is_structure_type, is_versioned_root_type};

/// Decomposes a versioned object's canonical JSON into node rows.
///
/// # Errors
///
/// Fails when the root has no versioned-root `_type`, or an array mixes
/// structure and non-structure elements (canonical RM JSON never does).
pub fn decompose(root: Value) -> Result<Vec<NodeRow>, StorageError> {
    let root_type = root.get("_type").and_then(Value::as_str);
    if !root_type.is_some_and(is_versioned_root_type) {
        return Err(StorageError::NotAStructureRoot(
            root_type.map(str::to_owned),
        ));
    }

    let mut rows = Vec::new();
    walk(root, String::new(), -1, &mut rows)?;

    // num_cap: children always follow their parents — one reverse pass. `walk`
    // pushes rows in pre-order with `num == index`, so a row's `parent_num` is
    // always the index of an EARLIER row; every access below is fetched rather
    // than indexed, and a violation of that invariant is reported as malformed
    // rows instead of panicking on a write path.
    let mut caps: Vec<i32> = rows.iter().map(|r| r.num).collect();
    for i in (1..rows.len()).rev() {
        let (Some(cap), Some(parent)) = (
            caps.get(i).copied(),
            rows.get(i)
                .and_then(|r| usize::try_from(r.parent_num).ok())
                .filter(|parent| *parent < i),
        ) else {
            return Err(StorageError::InvalidRows(format!(
                "node row {i} does not reference an earlier parent row"
            )));
        };
        if let Some(parent_cap) = caps.get_mut(parent) {
            *parent_cap = (*parent_cap).max(cap);
        }
        if let Some(row) = rows.get_mut(i) {
            row.num_cap = cap;
        }
    }
    if let (Some(row), Some(cap)) = (rows.first_mut(), caps.first()) {
        row.num_cap = *cap;
    }
    Ok(rows)
}

fn walk(
    mut json: Value,
    path: String,
    parent: i32,
    rows: &mut Vec<NodeRow>,
) -> Result<(), StorageError> {
    let index = rows.len();
    let num = i32::try_from(index).unwrap_or(i32::MAX);
    let rm_type = json
        .get("_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // openEHR identifiers are case-insensitive (BASE `base_types` master05
    // §Composite Identifiers and Case), so the promoted predicate column stores
    // the lowercase form and AQL archetype equality stays plain indexed equality
    // with honest statistics. The canonical `data` fragment keeps its casing.
    let archetype = json
        .get("archetype_node_id")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    // Parse a full archetype HRID into the promoted subsumption columns
    // (lowercased); at/id-code nodes leave them NULL (BASE `base_types` master05
    // §Archetype Identifiers; querying per master10 §Design-time Relationships).
    let (arch_entity, arch_concept, arch_major) = archetype
        .as_deref()
        .and_then(archetype_parts)
        .map_or((None, None, None), |(e, c, m)| (Some(e), Some(c), Some(m)));
    let name = json
        .get("name")
        .and_then(|n| n.get("value"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    // The coded half of a DV_CODED_TEXT name, promoted so the AQL node
    // predicate matches a coded name without a JSON probe (QUERY
    // master03-syntax.adoc §Node predicate). Absent on a plain DV_TEXT name.
    let defining_code = json.get("name").and_then(|n| n.get("defining_code"));
    let name_code = defining_code
        .and_then(|c| c.get("code_string"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let name_terminology = defining_code
        .and_then(|c| c.get("terminology_id"))
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    // Promoted-leaf capture (our own storage design — no openEHR spec
    // governs it): read the hot leaves off the node's *pre-pruning* JSON (the
    // value may sit inside an about-to-be-split structure child, e.g.
    // `EVENT_CONTEXT`), aligned to `PROMOTED_LEAVES`. Populated only on a
    // versioned-object root (`num == 0`); every other row carries all-`None`.
    let promoted = crate::storage::promoted::extract(num, &rm_type, &json);
    // `path` and `data` land in the row at the END of this call (the fragment
    // after pruning, the path by move — never a second allocation).
    rows.push(NodeRow {
        num,
        num_cap: num,
        parent_num: parent.max(0),
        rm_type,
        archetype,
        arch_entity,
        arch_concept,
        arch_major,
        name,
        name_code,
        name_terminology,
        path: String::new(),
        data: Value::Null,
        promoted,
    });

    if let Value::Object(map) = &mut json {
        prune_children(map, &path, num, rows)?;
    }
    // `index` is the slot this call pushed above, so it exists; fetched rather
    // than indexed so a future restructuring of `walk` cannot panic here.
    let Some(row) = rows.get_mut(index) else {
        return Err(StorageError::InvalidRows(format!(
            "node row {index} vanished while decomposing {path}"
        )));
    };
    row.path = path;
    row.data = json;
    Ok(())
}

/// Prunes structure children out of `map`, recursing in document order.
///
/// Two read passes: the first only IDENTIFIES the structure-carrying
/// attributes (so nothing is cloned for the — typical — attributes that stay
/// in place), the second removes and walks them. Document order among the
/// pruned attributes is preserved.
fn prune_children(
    map: &mut Map<String, Value>,
    path: &str,
    num: i32,
    rows: &mut Vec<NodeRow>,
) -> Result<(), StorageError> {
    let mut structure_attributes: Vec<String> = Vec::new();
    for (attribute, child) in map.iter() {
        if carries_structure(attribute, child)? {
            structure_attributes.push(attribute.clone());
        }
    }
    for attribute in structure_attributes {
        match map.shift_remove(&attribute) {
            Some(child @ Value::Object(_)) => {
                walk(child, format!("{path}{attribute}."), num, rows)?;
            }
            Some(Value::Array(items)) => {
                for (i, item) in items.into_iter().enumerate() {
                    walk(item, format!("{path}{attribute}{i}."), num, rows)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether one attribute's value carries structure children that become their
/// own node rows.
///
/// # Errors
/// [`StorageError::MixedArray`] when an array mixes structure and
/// non-structure members — a shape the node model cannot represent.
fn carries_structure(attribute: &str, child: &Value) -> Result<bool, StorageError> {
    let Value::Array(items) = child else {
        return Ok(is_structure(child));
    };
    let structure_count = items.iter().filter(|c| is_structure(c)).count();
    if structure_count == 0 {
        return Ok(false);
    }
    if structure_count == items.len() {
        return Ok(true);
    }
    Err(StorageError::MixedArray {
        attribute: attribute.to_owned(),
    })
}

fn is_structure(v: &Value) -> bool {
    v.get("_type")
        .and_then(Value::as_str)
        .is_some_and(is_structure_type)
}

/// Reassembles the canonical JSON from node rows (sorted or not — rows are
/// ordered by `num` internally).
///
/// Lossless inverse of [`decompose`]. Generic over [`NodeContent`], so it
/// accepts either the write [`NodeRow`] (from [`decompose`], e.g. to
/// reassemble the served form for signing) or the lean
/// [`crate::storage::row::ReadRow`] the repository fetches back.
///
/// # Errors
///
/// Fails when the rows do not form a single tree rooted at `num = 0`. An empty
/// row set is a caller error here (a version legitimately having no nodes — a
/// logical delete — is handled by
/// [`crate::storage::node_repo::read_version_canonical`], which returns
/// `Value::Null` before calling this).
pub fn reassemble<R: NodeContent>(rows: &[R]) -> Result<Value, StorageError> {
    let mut ordered: Vec<&R> = rows.iter().collect();
    // Both producers already deliver `num` order (the node read's ORDER BY;
    // decompose's pre-order walk), so the usual case is a linear verification,
    // not an O(N log N) sort.
    if !ordered.is_sorted_by_key(|r| r.num()) {
        ordered.sort_by_key(|r| r.num());
    }
    let Some(root_row) = ordered.first() else {
        return Err(StorageError::InvalidRows("no rows".into()));
    };
    if root_row.num() != 0 || !root_row.path().is_empty() {
        return Err(StorageError::InvalidRows(format!(
            "root row must have num=0 and empty path (got num={}, path={:?})",
            root_row.num(),
            root_row.path()
        )));
    }
    let mut root = root_row.data().clone();
    for row in ordered.iter().skip(1) {
        attach(&mut root, row.path(), row.data().clone())?;
    }
    Ok(root)
}

/// Re-attaches one pruned fragment at its materialized path. Parents come
/// before children (`num` order), so every ancestor is already in place.
fn attach(root: &mut Value, path: &str, fragment: Value) -> Result<(), StorageError> {
    let missing =
        |step: &str| StorageError::InvalidRows(format!("missing ancestor {step:?} ({path})"));
    let steps: Vec<&str> = path.trim_end_matches('.').split('.').collect();
    let mut current = root;
    for (i, step) in steps.iter().enumerate() {
        let is_leaf = i == steps.len() - 1;
        let (attribute, index) = split_step(step);
        let Value::Object(map) = current else {
            return Err(missing(step));
        };
        match index {
            None => {
                if is_leaf {
                    map.insert(attribute.to_owned(), fragment);
                    return Ok(());
                }
                current = map.get_mut(attribute).ok_or_else(|| missing(step))?;
            }
            Some(index) => {
                let slot = map
                    .entry(attribute.to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
                let Value::Array(array) = slot else {
                    return Err(missing(step));
                };
                if is_leaf {
                    if array.len() <= index {
                        array.resize(index + 1, Value::Null);
                    }
                    // Resized to cover `index` above; fetched rather than
                    // indexed so the resize stays the only thing to get right.
                    let slot = array.get_mut(index).ok_or_else(|| missing(step))?;
                    *slot = fragment;
                    return Ok(());
                }
                current = array.get_mut(index).ok_or_else(|| missing(step))?;
            }
        }
    }
    Err(StorageError::InvalidRows(format!("empty path ({path})")))
}

/// Splits a path step into (attribute, array index): `content0` → `content`
/// + 0; `context` → no index.
fn split_step(step: &str) -> (&str, Option<usize>) {
    let digits = step.chars().rev().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        (step, None)
    } else {
        let (attribute, number) = step.split_at(step.len() - digits);
        (attribute, number.parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Reassemble a decomposed write-row set directly (`NodeRow: NodeContent`),
    /// exercising the full round-trip.
    fn round_trip(rows: &[NodeRow]) -> Value {
        reassemble(rows).unwrap()
    }

    fn sample() -> Value {
        json!({
            "_type": "COMPOSITION",
            "archetype_node_id": "openEHR-EHR-COMPOSITION.report.v1",
            "name": {"_type": "DV_TEXT", "value": "Report"},
            "context": {
                "_type": "EVENT_CONTEXT",
                "setting": {"_type": "DV_CODED_TEXT", "value": "other care"}
            },
            "content": [
                {
                    "_type": "OBSERVATION",
                    "archetype_node_id": "openEHR-EHR-OBSERVATION.bp.v2",
                    "name": {"_type": "DV_TEXT", "value": "Blood pressure"},
                    "data": {
                        "_type": "HISTORY",
                        "archetype_node_id": "at0001",
                        "name": {"_type": "DV_TEXT", "value": "history"},
                        "events": [{
                            "_type": "POINT_EVENT",
                            "archetype_node_id": "at0006",
                            "name": {"_type": "DV_TEXT", "value": "any event"}
                        }]
                    }
                }
            ]
        })
    }

    type BriefRow<'a> = (&'a str, &'a str, i32, i32, i32);

    #[test]
    fn decomposes_with_nested_set_numbers() {
        let rows = decompose(sample()).unwrap();
        let brief: Vec<BriefRow> = rows
            .iter()
            .map(|r| {
                (
                    r.rm_type.as_str(),
                    r.path.as_str(),
                    r.num,
                    r.num_cap,
                    r.parent_num,
                )
            })
            .collect();
        assert_eq!(
            brief,
            vec![
                ("COMPOSITION", "", 0, 4, 0),
                ("EVENT_CONTEXT", "context.", 1, 1, 0),
                ("OBSERVATION", "content0.", 2, 4, 0),
                ("HISTORY", "content0.data.", 3, 4, 2),
                ("POINT_EVENT", "content0.data.events0.", 4, 4, 3),
            ]
        );
        // Structure children are pruned out of the parent fragments.
        assert!(rows[0].data.get("context").is_none());
        assert!(rows[0].data.get("content").is_none());
        assert!(rows[2].data.get("data").is_none());
        // Non-structure content stays verbatim.
        assert_eq!(rows[0].data["name"]["value"], "Report");
        assert_eq!(rows[1].data["setting"]["value"], "other care");
        // Full archetype HRIDs are parsed into the lowercased subsumption
        // columns; at-code nodes leave them NULL.
        assert_eq!(rows[2].rm_type, "OBSERVATION");
        assert_eq!(
            rows[2].arch_entity.as_deref(),
            Some("openehr-ehr-observation")
        );
        assert_eq!(rows[2].arch_concept.as_deref(), Some("bp"));
        assert_eq!(rows[2].arch_major, Some(2));
        assert_eq!(rows[3].rm_type, "HISTORY"); // archetype_node_id = at0001
        assert_eq!(rows[3].arch_entity, None);
        assert_eq!(rows[3].arch_concept, None);
        assert_eq!(rows[3].arch_major, None);
    }

    #[test]
    fn parses_specialised_archetype_concept() {
        // The specialisation child keeps the full concept incl. its `-` segment,
        // so a parent (`laboratory`) query prefix-matches it (master10 §83).
        let v = json!({
            "_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.laboratory-glucose.v1",
            "name": {"_type": "DV_TEXT", "value": "glucose"}
        });
        let rows = decompose(v).unwrap();
        assert_eq!(
            rows[0].arch_entity.as_deref(),
            Some("openehr-ehr-observation")
        );
        assert_eq!(rows[0].arch_concept.as_deref(), Some("laboratory-glucose"));
        assert_eq!(rows[0].arch_major, Some(1));
    }

    #[test]
    fn round_trips_losslessly() {
        let original = sample();
        let rows = decompose(original.clone()).unwrap();
        assert_eq!(round_trip(&rows), original);
    }

    #[test]
    fn rejects_non_structure_roots() {
        assert!(matches!(
            decompose(json!({"_type": "DV_TEXT", "value": "x"})),
            Err(StorageError::NotAStructureRoot(Some(t))) if t == "DV_TEXT"
        ));
    }

    #[test]
    fn rejects_mixed_arrays() {
        let v = json!({
            "_type": "COMPOSITION",
            "content": [
                {"_type": "OBSERVATION"},
                {"_type": "DV_TEXT", "value": "not a structure"}
            ]
        });
        assert!(matches!(
            decompose(v),
            Err(StorageError::MixedArray { attribute }) if attribute == "content"
        ));
    }

    #[test]
    fn reassembles_out_of_order_rows() {
        let original = sample();
        let mut rows = decompose(original.clone()).unwrap();
        rows.reverse();
        assert_eq!(round_trip(&rows), original);
    }

    #[test]
    fn reassemble_rejects_empty_rows() {
        assert!(matches!(
            reassemble::<NodeRow>(&[]),
            Err(StorageError::InvalidRows(m)) if m == "no rows"
        ));
    }

    /// A realistic PERSON with `identities` (each carrying a nested
    /// `details: ITEM_TREE` inside its `PARTY_IDENTITY`), `contacts` (with an
    /// `ADDRESS.details: ITEM_TREE`), a top-level `details: ITEM_TREE`,
    /// `languages`, and `roles` refs. Every container carries an at-code, so
    /// the archetyped-container case is a separate fixture.
    fn person() -> Value {
        json!({
            "_type": "PERSON",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
            "name": {"_type": "DV_TEXT", "value": "person"},
            "uid": {"_type": "HIER_OBJECT_ID", "value": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"},
            "identities": [{
                "_type": "PARTY_IDENTITY",
                "archetype_node_id": "at0001",
                "name": {"_type": "DV_TEXT", "value": "legal name"},
                "details": {
                    "_type": "ITEM_TREE",
                    "archetype_node_id": "at0002",
                    "name": {"_type": "DV_TEXT", "value": "structure"},
                    "items": [{
                        "_type": "ELEMENT",
                        "archetype_node_id": "at0003",
                        "name": {"_type": "DV_TEXT", "value": "family name"},
                        "value": {"_type": "DV_TEXT", "value": "Doe"}
                    }]
                }
            }],
            "contacts": [{
                "_type": "CONTACT",
                "archetype_node_id": "at0010",
                "name": {"_type": "DV_TEXT", "value": "home"},
                "addresses": [{
                    "_type": "ADDRESS",
                    "archetype_node_id": "at0011",
                    "name": {"_type": "DV_TEXT", "value": "postal"},
                    "details": {
                        "_type": "ITEM_TREE",
                        "archetype_node_id": "at0012",
                        "name": {"_type": "DV_TEXT", "value": "address"},
                        "items": [{
                            "_type": "ELEMENT",
                            "archetype_node_id": "at0013",
                            "name": {"_type": "DV_TEXT", "value": "city"},
                            "value": {"_type": "DV_TEXT", "value": "Amsterdam"}
                        }]
                    }
                }]
            }],
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0020",
                "name": {"_type": "DV_TEXT", "value": "details"},
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0021",
                    "name": {"_type": "DV_TEXT", "value": "note"},
                    "value": {"_type": "DV_TEXT", "value": "vip"}
                }]
            },
            "languages": [{"_type": "DV_TEXT", "value": "en"}],
            "roles": [{
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "ROLE",
                "id": {"_type": "HIER_OBJECT_ID", "value": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}
            }]
        })
    }

    /// A ROLE with a `performer` `PARTY_REF` and `capabilities` (each carrying a
    /// nested `credentials: ITEM_TREE`).
    fn role() -> Value {
        json!({
            "_type": "ROLE",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
            "name": {"_type": "DV_TEXT", "value": "role"},
            "identities": [{
                "_type": "PARTY_IDENTITY",
                "archetype_node_id": "at0001",
                "name": {"_type": "DV_TEXT", "value": "role name"},
                "details": {
                    "_type": "ITEM_TREE",
                    "archetype_node_id": "at0002",
                    "name": {"_type": "DV_TEXT", "value": "structure"},
                    "items": []
                }
            }],
            "performer": {
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "PERSON",
                "id": {"_type": "HIER_OBJECT_ID", "value": "cccccccc-cccc-4ccc-8ccc-cccccccccccc"}
            },
            "capabilities": [{
                "_type": "CAPABILITY",
                "archetype_node_id": "at0030",
                "name": {"_type": "DV_TEXT", "value": "prescribing"},
                "credentials": {
                    "_type": "ITEM_TREE",
                    "archetype_node_id": "at0031",
                    "name": {"_type": "DV_TEXT", "value": "credentials"},
                    "items": [{
                        "_type": "ELEMENT",
                        "archetype_node_id": "at0032",
                        "name": {"_type": "DV_TEXT", "value": "licence"},
                        "value": {"_type": "DV_TEXT", "value": "GMC-12345"}
                    }]
                }
            }]
        })
    }

    /// A party's `identities`, `contacts`, `contacts.addresses` and the
    /// `details` `ITEM_TREE` under each become rows of their own, exactly as
    /// composition content does — the containers are archetypable structures
    /// (RM demographic `master02-demographic_package.adoc` §Archetyping), so
    /// leaving them inline would put their content out of reach of every
    /// row-level predicate.
    #[test]
    fn person_round_trips_losslessly() {
        let original = person();
        let rows = decompose(original.clone()).unwrap();
        assert_eq!(rows[0].rm_type, "PERSON");
        for attribute in ["identities", "contacts", "details"] {
            assert!(
                rows[0].data.get(attribute).is_none(),
                "{attribute} is pruned off the party root"
            );
        }
        let paths: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.rm_type.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("PERSON", ""),
                ("PARTY_IDENTITY", "identities0."),
                ("ITEM_TREE", "identities0.details."),
                ("ELEMENT", "identities0.details.items0."),
                ("CONTACT", "contacts0."),
                ("ADDRESS", "contacts0.addresses0."),
                ("ITEM_TREE", "contacts0.addresses0.details."),
                ("ELEMENT", "contacts0.addresses0.details.items0."),
                ("ITEM_TREE", "details."),
                ("ELEMENT", "details.items0."),
            ]
        );
        assert_eq!(round_trip(&rows), original);
    }

    #[test]
    fn role_round_trips_losslessly() {
        let original = role();
        let rows = decompose(original.clone()).unwrap();
        let paths: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.rm_type.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("ROLE", ""),
                ("PARTY_IDENTITY", "identities0."),
                ("ITEM_TREE", "identities0.details."),
                ("CAPABILITY", "capabilities0."),
                ("ITEM_TREE", "capabilities0.credentials."),
                ("ELEMENT", "capabilities0.credentials.items0."),
            ]
        );
        // `performer` is a PARTY_REF, not a structure: it stays on the root.
        assert!(rows[0].data.get("performer").is_some());
        assert!(rows[0].data.get("capabilities").is_none());
        assert_eq!(round_trip(&rows), original);
    }

    /// A container carrying a full archetype HRID gets the promoted
    /// subsumption columns parsed from it, and the tree still round-trips.
    #[test]
    fn an_archetyped_address_carries_its_own_subsumption_columns() {
        let mut original = person();
        original["contacts"][0]["addresses"][0]["archetype_node_id"] =
            json!("openEHR-DEMOGRAPHIC-ADDRESS.address.v1");
        let rows = decompose(original.clone()).unwrap();
        let row = |path: &str| {
            rows.iter()
                .find(|r| r.path == path)
                .unwrap_or_else(|| panic!("no row at {path}"))
        };
        let address = row("contacts0.addresses0.");
        assert_eq!(
            address.arch_concept.as_deref(),
            Some("address"),
            "the ADDRESS row carries the promoted subsumption columns"
        );
        assert_eq!(
            address.arch_entity.as_deref(),
            Some("openehr-demographic-address"),
            "the entity half is parsed and lowercased"
        );
        assert_eq!(address.arch_major, Some(1));
        // An at-coded sibling leaves them unset: only a full HRID populates
        // them (BASE base_types master05 §Archetype Identifiers).
        let at_coded = row("identities0.details.items0.");
        assert_eq!(at_coded.arch_entity, None);
        assert_eq!(at_coded.arch_concept, None);
        assert_eq!(round_trip(&rows), original);
    }

    #[test]
    fn party_round_trips_out_of_order() {
        let original = person();
        let mut rows = decompose(original.clone()).unwrap();
        rows.reverse();
        assert_eq!(round_trip(&rows), original);
    }
}
