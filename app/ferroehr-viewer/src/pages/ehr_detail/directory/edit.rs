// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Pure, component-free helpers for the structured directory tree editor.
//!
//! Navigation and mutation of a canonical `FOLDER` tree held as
//! `serde_json::Value`, `OBJECT_REF` construction, and tree statistics. Kept
//! out of the view code so the editing logic is unit-tested directly
//! (business logic lives in plain types). The time-travel panel's
//! `version_at_time` value goes through the viewer's one normalizer,
//! [`crate::format::datetime_local_to_rfc3339`].
//!
//! The FOLDER shape these operate on IS spec-bound (ITS-REST
//! `specifications/schemas/ehr/Folder.yaml`; RM common
//! `master05-directory_package`): a FOLDER carries `folders` (child FOLDERs)
//! and `items` (`OBJECT_REF`s). Node paths address a folder by the chain of
//! child indices into successive `folders` arrays from the root
//! (`[]` = the root FOLDER, `[0]` = its first child, `[0,1]` = that child's
//! second child).
//!
//! ## Ephemeral node identity (`_key`)
//!
//! `<For>` rows and the collapse / rename / picker UI state MUST be keyed by a
//! stable, unique, data-derived identity — never a positional path or index — or
//! a sibling that shifts into a deleted slot inherits the vacated row's state and
//! `<For>` view (`view/04_iteration`). The working copy therefore stamps every
//! FOLDER object **and every `items` `OBJECT_REF`** with a `_key` string
//! (`stamp_keys`), all drawn from one counter so folder and item keys can never
//! collide. It is a **client-only artifact of the in-memory editing tree**,
//! STRIPPED (`strip_keys`) from every body serialized for the CDR and from the
//! advanced-JSON view — it never appears on the wire, and no openEHR spec governs
//! it (our own design/extension). Positions are re-derived from the `_key` on
//! demand (`find_path_by_key`, `find_item_index`), so mutations keep targeting
//! the `&[usize]` path + index pair after siblings shift.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde_json::{Value, json};

use crate::pages::ehr_detail::directory::{FOLDER_NODE_ID, folder_json};

/// Stamp every node in `tree` — the root FOLDER, every descendant FOLDER, and
/// every `items` `OBJECT_REF` — with an ephemeral, client-only `_key` identity
/// string (`"n0"`, `"n1"`, …) drawn from `counter`, inserting one only where
/// absent — so it is idempotent and preserves the keys already carried by
/// surviving nodes. The key is the stable, data-derived node identity that
/// `<For>` rows and per-folder UI state key on; folders and items draw from
/// the same `counter`, so no item can ever be handed a folder's key. It is a
/// viewer-local artifact of the working copy: never persisted to the CDR (see
/// [`strip_keys`]), never rendered, and no openEHR spec governs it (our own
/// design/extension). The counter is a plain monotonic one, no randomness or
/// clock.
pub(super) fn stamp_keys(tree: &mut Value, counter: &mut u64) {
    if let Some(obj) = tree.as_object_mut() {
        if !obj.contains_key("_key") {
            obj.insert("_key".to_owned(), Value::String(format!("n{counter}")));
            *counter = counter.saturating_add(1);
        }
        // An `OBJECT_REF` carries no `folders`/`items` of its own, so the same
        // recursion stamps items without descending any further.
        if let Some(list) = obj.get_mut("items").and_then(Value::as_array_mut) {
            for item in list {
                stamp_keys(item, counter);
            }
        }
        if let Some(list) = obj.get_mut("folders").and_then(Value::as_array_mut) {
            for child in list {
                stamp_keys(child, counter);
            }
        }
    }
}

/// Remove the ephemeral `_key` identity from every node in `tree` (the root
/// FOLDER, every descendant FOLDER, and every `items` `OBJECT_REF`),
/// recursively — the inverse of [`stamp_keys`]. Applied to the body serialized
/// for every save path and to the advanced-JSON view so the viewer-local
/// identity never reaches the CDR or the user.
pub(super) fn strip_keys(tree: &mut Value) {
    if let Some(obj) = tree.as_object_mut() {
        obj.remove("_key");
        if let Some(list) = obj.get_mut("items").and_then(Value::as_array_mut) {
            for item in list {
                strip_keys(item);
            }
        }
        if let Some(list) = obj.get_mut("folders").and_then(Value::as_array_mut) {
            for child in list {
                strip_keys(child);
            }
        }
    }
}

/// The live positional path of the folder carrying the ephemeral identity
/// `key`, searched from `root` (`Some([])` for the root, `None` if no folder
/// carries it). The `_key` is the durable identity; the path is re-derived
/// from it on every reactive read and mutation so a folder's row stays
/// correct after a sibling delete shifts indices.
#[must_use]
pub(crate) fn find_path_by_key(root: &Value, key: &str) -> Option<Vec<usize>> {
    fn walk(node: &Value, key: &str, path: &mut Vec<usize>) -> bool {
        if node.get("_key").and_then(Value::as_str) == Some(key) {
            return true;
        }
        if let Some(list) = node.get("folders").and_then(Value::as_array) {
            for (i, child) in list.iter().enumerate() {
                path.push(i);
                if walk(child, key, path) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    let mut path = Vec::new();
    walk(root, key, &mut path).then_some(path)
}

/// The ephemeral identity key of the folder at `path`, if stamped.
#[must_use]
pub(crate) fn node_key_at(root: &Value, path: &[usize]) -> Option<String> {
    folder_at(root, path)
        .and_then(|f| f.get("_key"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Borrow the folder node at `path`, following `folders` arrays from `root`.
#[must_use]
pub(crate) fn folder_at<'a>(root: &'a Value, path: &[usize]) -> Option<&'a Value> {
    let mut node = root;
    for &idx in path {
        node = node.get("folders")?.get(idx)?;
    }
    Some(node)
}

/// Mutably borrow the folder node at `path`.
fn folder_at_mut<'a>(root: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let mut node = root;
    for &idx in path {
        node = node.get_mut("folders")?.get_mut(idx)?;
    }
    Some(node)
}

/// The display name (`name/value`) of the folder at `path`, or a placeholder.
#[must_use]
pub(crate) fn node_name(root: &Value, path: &[usize]) -> String {
    folder_at(root, path)
        .and_then(|f| f.get("name"))
        .and_then(|n| n.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("(folder)")
        .to_owned()
}

/// The ephemeral identity keys of the child folders of `path`, in order — the
/// stable, data-derived `<For>` keys for the folder rows. Every folder in the
/// working copy is stamped ([`stamp_keys`]), so each entry is a real
/// identity.
#[must_use]
pub(crate) fn child_node_keys(root: &Value, path: &[usize]) -> Vec<String> {
    folder_at(root, path)
        .and_then(|f| f.get("folders"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|child| {
                    child
                        .get("_key")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The number of item references in the folder at `path`.
#[must_use]
pub(crate) fn item_count(root: &Value, path: &[usize]) -> usize {
    folder_at(root, path)
        .and_then(|f| f.get("items"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

/// The ephemeral identity of the `idx`-th entry of a folder's `items` array:
/// its stamped `_key`, or — for an entry [`stamp_keys`] could not stamp because
/// it is not a JSON object (reachable only by hand-typing the advanced JSON, a
/// body the CDR rejects on save) — a positional fallback that cannot collide
/// with a stamped `"n…"` key, so the `<For>` keys stay unique either way.
fn item_key(item: &Value, idx: usize) -> String {
    item.get("_key")
        .and_then(Value::as_str)
        .map_or_else(|| format!("#{idx}"), str::to_owned)
}

/// The ephemeral identity keys of the item references of the folder at `path`,
/// in order — the stable, data-derived `<For>` keys for the item rows. Every
/// item in the working copy is stamped ([`stamp_keys`]), so each entry is a
/// real identity that survives a sibling removal shifting the remaining items
/// down.
#[must_use]
pub(crate) fn item_node_keys(root: &Value, path: &[usize]) -> Vec<String> {
    folder_at(root, path)
        .and_then(|f| f.get("items"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .enumerate()
                .map(|(i, item)| item_key(item, i))
                .collect()
        })
        .unwrap_or_default()
}

/// The live index, within the `items` array of the folder at `path`, of the
/// item carrying the ephemeral identity `key` (`None` if no item there carries
/// it). The `_key` is the durable identity; the index is re-derived from it for
/// every reactive read and mutation so an item row stays correct after a
/// sibling removal shifts indices — the item-side counterpart of
/// [`find_path_by_key`].
#[must_use]
pub(crate) fn find_item_index(root: &Value, path: &[usize], key: &str) -> Option<usize> {
    folder_at(root, path)?
        .get("items")?
        .as_array()?
        .iter()
        .enumerate()
        .find_map(|(i, item)| (item_key(item, i) == key).then_some(i))
}

/// The `(type, id-value)` summary of the `idx`-th item of the folder at
/// `path`, for row display.
#[must_use]
pub(crate) fn item_summary(root: &Value, path: &[usize], idx: usize) -> (String, String) {
    let item = folder_at(root, path)
        .and_then(|f| f.get("items"))
        .and_then(Value::as_array)
        .and_then(|items| items.get(idx));
    let ref_type = item
        .and_then(|i| i.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("OBJECT")
        .to_owned();
    let id = item
        .and_then(|i| i.get("id"))
        .and_then(|i| i.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("(ref)")
        .to_owned();
    (ref_type, id)
}

/// Append a new empty child folder named `name` to the folder at `path`.
pub(crate) fn add_subfolder(root: &mut Value, path: &[usize], name: &str) {
    if let Some(node) = folder_at_mut(root, path).and_then(Value::as_object_mut) {
        let folders = node
            .entry("folders")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(list) = folders.as_array_mut() {
            list.push(folder_json(FOLDER_NODE_ID, name, &[]));
        }
    }
}

/// Rename the folder at `path` (sets `name/value`, keeping it a `DV_TEXT`).
pub(crate) fn rename_folder(root: &mut Value, path: &[usize], name: &str) {
    if let Some(node) = folder_at_mut(root, path).and_then(Value::as_object_mut) {
        let name_obj = node
            .entry("name")
            .or_insert_with(|| json!({ "_type": "DV_TEXT", "value": "" }));
        if let Some(obj) = name_obj.as_object_mut() {
            obj.insert("value".to_owned(), Value::String(name.to_owned()));
            obj.entry("_type")
                .or_insert_with(|| Value::String("DV_TEXT".to_owned()));
        }
    }
}

/// Remove the folder at `path` from its parent. A no-op for the root
/// (`path` empty) — the root FOLDER is never deleted from within the tree.
pub(crate) fn delete_folder(root: &mut Value, path: &[usize]) {
    if let Some((&idx, parent_path)) = path.split_last()
        && let Some(list) = folder_at_mut(root, parent_path)
            .and_then(|p| p.get_mut("folders"))
            .and_then(Value::as_array_mut)
        && idx < list.len()
    {
        list.remove(idx);
    }
}

/// Append an item reference to the folder at `path`.
pub(crate) fn add_item(root: &mut Value, path: &[usize], item: Value) {
    if let Some(node) = folder_at_mut(root, path).and_then(Value::as_object_mut) {
        let items = node
            .entry("items")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(list) = items.as_array_mut() {
            list.push(item);
        }
    }
}

/// Remove the `idx`-th item from the folder at `path`. Callers re-derive `idx`
/// from the row's ephemeral item identity ([`find_item_index`]) at click time,
/// so a shifted index can never remove the wrong sibling.
pub(crate) fn remove_item(root: &mut Value, path: &[usize], idx: usize) {
    if let Some(list) = folder_at_mut(root, path)
        .and_then(|f| f.get_mut("items"))
        .and_then(Value::as_array_mut)
        && idx < list.len()
    {
        list.remove(idx);
    }
}

/// Build a canonical `OBJECT_REF` from its parts, per RM common
/// `master03-support` (`OBJECT_REF` = `namespace` + `type` +
/// `id: OBJECT_ID`). `id_type` is the concrete `OBJECT_ID` subtype
/// (`HIER_OBJECT_ID`, `OBJECT_VERSION_ID`, …).
#[must_use]
pub(crate) fn object_ref(namespace: &str, ref_type: &str, id_type: &str, id_value: &str) -> Value {
    json!({
        "_type": "OBJECT_REF",
        "namespace": namespace,
        "type": ref_type,
        "id": { "_type": id_type, "value": id_value },
    })
}

/// The total descendant folder count (excluding the root) and total item
/// count across the whole tree, for the version-history summary.
#[must_use]
pub(crate) fn count_tree(root: &Value) -> (i32, i32) {
    fn walk(node: &Value, folders: &mut i32, items: &mut i32) {
        if let Some(list) = node.get("items").and_then(Value::as_array) {
            *items = items.saturating_add(i32::try_from(list.len()).unwrap_or(i32::MAX));
        }
        if let Some(subs) = node.get("folders").and_then(Value::as_array) {
            for sub in subs {
                *folders = folders.saturating_add(1);
                walk(sub, folders, items);
            }
        }
    }
    let (mut folders, mut items) = (0, 0);
    walk(root, &mut folders, &mut items);
    (folders, items)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        add_item, add_subfolder, child_node_keys, count_tree, delete_folder, find_item_index,
        find_path_by_key, item_count, item_node_keys, item_summary, node_key_at, node_name,
        object_ref, remove_item, rename_folder, stamp_keys, strip_keys,
    };
    use crate::pages::ehr_detail::directory::{DIRECTORY_ARCHETYPE, folder_json};

    fn root() -> Value {
        folder_json(
            DIRECTORY_ARCHETYPE,
            "root",
            &[
                folder_json("at0001", "a", &[folder_json("at0001", "a1", &[])]),
                folder_json("at0001", "b", &[]),
            ],
        )
    }

    /// A root whose first child folder holds three item references (`x`, `y`,
    /// `z`), for the item-identity assertions.
    fn root_with_items() -> Value {
        let mut tree = root();
        for id in ["x", "y", "z"] {
            add_item(
                &mut tree,
                &[0],
                object_ref("local", "COMPOSITION", "HIER_OBJECT_ID", id),
            );
        }
        tree
    }

    /// Collect every folder's `_key` (root first, depth-first) for assertions.
    fn collect_keys(node: &Value, out: &mut Vec<String>) {
        if let Some(k) = node.get("_key").and_then(Value::as_str) {
            out.push(k.to_owned());
        }
        if let Some(list) = node.get("folders").and_then(Value::as_array) {
            for child in list {
                collect_keys(child, out);
            }
        }
    }

    #[test]
    fn stamp_assigns_a_unique_key_to_every_folder() {
        let mut tree = root();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        // root + a + a1 + b = 4 folders, each stamped once.
        assert_eq!(counter, 4);
        let mut keys = Vec::new();
        collect_keys(&tree, &mut keys);
        assert_eq!(keys.len(), 4);
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), 4, "every folder _key is unique");
        for path in [&[][..], &[0], &[0, 0], &[1]] {
            assert!(node_key_at(&tree, path).is_some());
        }
    }

    #[test]
    fn stamp_assigns_a_unique_key_to_every_item_too() {
        let mut tree = root_with_items();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        // 4 folders + 3 item references, each stamped once from ONE counter.
        assert_eq!(counter, 7);
        let item_keys = item_node_keys(&tree, &[0]);
        assert_eq!(item_keys.len(), 3);
        let mut all = item_keys.clone();
        collect_keys(&tree, &mut all);
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(
            unique.len(),
            7,
            "folder and item keys share one namespace and never collide"
        );
        // Idempotent for items as well: a second pass re-stamps nothing.
        stamp_keys(&mut tree, &mut counter);
        assert_eq!(counter, 7);
        assert_eq!(item_node_keys(&tree, &[0]), item_keys);
    }

    #[test]
    fn strip_is_the_inverse_of_stamp() {
        let before = root();
        let mut after = before.clone();
        let mut counter = 0u64;
        stamp_keys(&mut after, &mut counter);
        strip_keys(&mut after);
        assert_eq!(after, before, "stripped tree equals the pre-stamp tree");
    }

    #[test]
    fn strip_removes_item_keys_too() {
        let before = root_with_items();
        let mut after = before.clone();
        let mut counter = 0u64;
        stamp_keys(&mut after, &mut counter);
        assert_ne!(after, before, "stamping did mark the items");
        strip_keys(&mut after);
        assert_eq!(
            after, before,
            "no item _key survives into the body sent to the CDR"
        );
    }

    #[test]
    fn stamp_is_idempotent_and_keeps_existing_keys() {
        let mut tree = root();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        let stamped = tree.clone();
        let advanced = counter;
        // A second pass adds nothing and advances the counter for no folder.
        stamp_keys(&mut tree, &mut counter);
        assert_eq!(tree, stamped);
        assert_eq!(counter, advanced);
        // Only a newly added (unstamped) folder is stamped on the next pass.
        add_subfolder(&mut tree, &[1], "b1");
        stamp_keys(&mut tree, &mut counter);
        assert_eq!(counter, advanced + 1);
        assert!(node_key_at(&tree, &[1, 0]).is_some());
    }

    #[test]
    fn identity_key_survives_a_sibling_delete() {
        let mut tree = root();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        let root_key = node_key_at(&tree, &[]).unwrap();
        let b_key = node_key_at(&tree, &[1]).unwrap();
        assert_eq!(find_path_by_key(&tree, &root_key), Some(Vec::new()));
        assert_eq!(find_path_by_key(&tree, &b_key), Some(vec![1]));
        assert_eq!(find_path_by_key(&tree, "nope"), None);
        // Deleting the first child shifts `b` from index 1 to index 0 — its
        // identity key now resolves to the new path (the reviewed defect fix).
        delete_folder(&mut tree, &[0]);
        assert_eq!(find_path_by_key(&tree, &b_key), Some(vec![0]));
        assert_eq!(
            node_name(&tree, &find_path_by_key(&tree, &b_key).unwrap()),
            "b"
        );
    }

    #[test]
    fn navigation_reads_names() {
        let tree = root();
        assert_eq!(node_name(&tree, &[]), "root");
        assert_eq!(node_name(&tree, &[0]), "a");
        assert_eq!(node_name(&tree, &[0, 0]), "a1");
    }

    #[test]
    fn child_node_keys_and_item_count_read_structure() {
        let mut tree = root();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        assert_eq!(child_node_keys(&tree, &[]).len(), 2);
        assert_eq!(child_node_keys(&tree, &[0]).len(), 1);
        assert!(child_node_keys(&tree, &[1]).is_empty());
        // The child keys are the children's stamped identities, in order.
        let keys = child_node_keys(&tree, &[]);
        assert_eq!(keys[0], node_key_at(&tree, &[0]).unwrap());
        assert_eq!(keys[1], node_key_at(&tree, &[1]).unwrap());
        assert_eq!(item_count(&tree, &[0]), 0);
    }

    #[test]
    fn add_and_rename_and_delete_folders() {
        let mut tree = root();
        add_subfolder(&mut tree, &[1], "b1");
        assert_eq!(node_name(&tree, &[1, 0]), "b1");
        rename_folder(&mut tree, &[1, 0], "renamed");
        assert_eq!(node_name(&tree, &[1, 0]), "renamed");
        // Deleting the first child shifts the second into index 0.
        delete_folder(&mut tree, &[0]);
        assert_eq!(child_node_keys(&tree, &[]).len(), 1);
        assert_eq!(node_name(&tree, &[0]), "b");
        // Deleting the root is a no-op.
        delete_folder(&mut tree, &[]);
        assert_eq!(node_name(&tree, &[]), "root");
    }

    #[test]
    fn items_add_summarize_and_remove() {
        let mut tree = root();
        add_item(
            &mut tree,
            &[0],
            object_ref("local", "COMPOSITION", "HIER_OBJECT_ID", "abc"),
        );
        assert_eq!(item_count(&tree, &[0]), 1);
        assert_eq!(
            item_summary(&tree, &[0], 0),
            ("COMPOSITION".to_owned(), "abc".to_owned())
        );
        remove_item(&mut tree, &[0], 0);
        assert_eq!(item_count(&tree, &[0]), 0);
    }

    #[test]
    fn item_identity_key_survives_a_sibling_removal() {
        let mut tree = root_with_items();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        let before = item_node_keys(&tree, &[0]);
        assert_eq!(find_item_index(&tree, &[0], &before[2]), Some(2));
        assert_eq!(find_item_index(&tree, &[0], "nope"), None);
        // Remove the MIDDLE item, addressing it by identity (what the row's
        // remove button does) rather than by a captured positional index.
        let doomed = find_item_index(&tree, &[0], &before[1]).unwrap();
        remove_item(&mut tree, &[0], doomed);
        // The regression this pins: the surviving items keep exactly the keys
        // (and therefore the `<For>` rows) they had — only positions shift.
        assert_eq!(
            item_node_keys(&tree, &[0]),
            vec![before[0].clone(), before[2].clone()]
        );
        assert_eq!(find_item_index(&tree, &[0], &before[1]), None);
        assert_eq!(find_item_index(&tree, &[0], &before[2]), Some(1));
        // …and each surviving key still resolves to its OWN reference, never
        // the removed sibling's slot.
        for (key, id) in [(&before[0], "x"), (&before[2], "z")] {
            let idx = find_item_index(&tree, &[0], key).unwrap();
            assert_eq!(item_summary(&tree, &[0], idx).1, id);
        }
    }

    #[test]
    fn item_keys_stay_unique_for_unstampable_entries() {
        let mut tree = root_with_items();
        // Advanced-JSON mode can hand the working tree `items` entries that are
        // not JSON objects and therefore cannot carry a `_key`; the positional
        // fallback keeps the `<For>` keys unique (such a body is rejected by the
        // CDR on save, so this is only about not corrupting the row identity).
        tree["folders"][0]["items"] = json!([1, 2]);
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        let keys = item_node_keys(&tree, &[0]);
        assert_eq!(keys, vec!["#0".to_owned(), "#1".to_owned()]);
        assert_eq!(find_item_index(&tree, &[0], "#1"), Some(1));
    }

    #[test]
    fn object_ref_is_spec_shaped() {
        let r = object_ref("local", "COMPOSITION", "HIER_OBJECT_ID", "abc::sys::1");
        assert_eq!(r["_type"], "OBJECT_REF");
        assert_eq!(r["namespace"], "local");
        assert_eq!(r["type"], "COMPOSITION");
        assert_eq!(r["id"]["_type"], "HIER_OBJECT_ID");
        assert_eq!(r["id"]["value"], "abc::sys::1");
    }

    #[test]
    fn count_tree_counts_descendants_and_items() {
        let mut tree = root();
        add_item(
            &mut tree,
            &[0],
            object_ref("local", "COMPOSITION", "HIER_OBJECT_ID", "x"),
        );
        // Folders a, a1, b = 3; one item.
        assert_eq!(count_tree(&tree), (3, 1));
    }
}
