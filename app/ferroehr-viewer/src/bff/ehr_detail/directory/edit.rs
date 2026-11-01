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
//! A posted rename, move or delete MUST name its folder by a stable, unique,
//! data-derived identity — never a positional path or index — or a sibling that
//! shifts into a deleted slot receives the change meant for the vacated row. The working copy therefore stamps every
//! FOLDER object **and every `items` `OBJECT_REF`** with a `_key` string
//! (`stamp_keys`), all drawn from one counter so folder and item keys can never
//! collide. It is a **client-only artifact of the in-memory editing tree**,
//! STRIPPED (`strip_keys`) from every body serialized for the CDR and from the
//! advanced-JSON view — it never appears on the wire, and no openEHR spec governs
//! it (our own design/extension). Positions are re-derived from the `_key` on
//! demand (`find_path_by_key`, `find_item_by_key`), so mutations keep targeting
//! the `&[usize]` path + index pair after siblings shift.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde_json::{Value, json};

use crate::bff::ehr_detail::directory::{FOLDER_NODE_ID, folder_json};

/// Stamp every node in `tree` — the root FOLDER, every descendant FOLDER, and
/// every `items` `OBJECT_REF` — with an ephemeral, client-only `_key` identity
/// string (`"n0"`, `"n1"`, …) drawn from `counter`, inserting one only where
/// absent — so it is idempotent and preserves the keys already carried by
/// surviving nodes. The key is the stable, data-derived node identity that
/// a posted change names a folder or item by; folders and items draw from
/// the same `counter`, so no item can ever be handed a folder's key. It is a
/// viewer-local artifact of the working copy: never persisted to the CDR (see
/// [`strip_keys`]), never rendered, and no openEHR spec governs it (our own
/// design/extension). The counter is a plain monotonic one, no randomness or
/// clock.
pub(crate) fn stamp_keys(tree: &mut Value, counter: &mut u64) {
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
pub(crate) fn strip_keys(tree: &mut Value) {
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
/// from it on every read and mutation so a folder's row stays
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

/// The folder path and the place of the item reference carrying the
/// ephemeral identity `key`, or `None` when no item carries it.
#[must_use]
pub(crate) fn find_item_by_key(root: &Value, key: &str) -> Option<(Vec<usize>, usize)> {
    fn walk(node: &Value, path: &mut Vec<usize>, key: &str) -> Option<(Vec<usize>, usize)> {
        if let Some(items) = node.get("items").and_then(Value::as_array)
            && let Some(index) = items
                .iter()
                .position(|item| item.get("_key").and_then(Value::as_str) == Some(key))
        {
            return Some((path.clone(), index));
        }
        if let Some(folders) = node.get("folders").and_then(Value::as_array) {
            for (index, child) in folders.iter().enumerate() {
                path.push(index);
                if let Some(found) = walk(child, path, key) {
                    return Some(found);
                }
                path.pop();
            }
        }
        None
    }
    walk(root, &mut Vec::new(), key)
}

/// The counter a further [`stamp_keys`] pass starts from: one past the
/// highest `n{number}` key in `tree`, so a node added to a working copy that
/// came back from the form takes a key no node holds.
#[must_use]
pub(crate) fn next_counter(tree: &Value) -> u64 {
    fn walk(node: &Value, high: &mut u64) {
        if let Some(number) = node
            .get("_key")
            .and_then(Value::as_str)
            .and_then(|key| key.strip_prefix('n'))
            .and_then(|digits| digits.parse::<u64>().ok())
        {
            *high = (*high).max(number.saturating_add(1));
        }
        for list in ["folders", "items"] {
            if let Some(children) = node.get(list).and_then(Value::as_array) {
                for child in children {
                    walk(child, high);
                }
            }
        }
    }
    let mut high = 0;
    walk(tree, &mut high);
    high
}

/// Mutably borrow the folder node at `path`.
fn folder_at_mut<'a>(root: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let mut node = root;
    for &idx in path {
        node = node.get_mut("folders")?.get_mut(idx)?;
    }
    Some(node)
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
/// The parent's emptied `folders` goes too: ITS-JSON `Common/FOLDER.json`
/// gives the array `minItems: 1`.
pub(crate) fn delete_folder(root: &mut Value, path: &[usize]) {
    if let Some((&idx, parent_path)) = path.split_last()
        && let Some(parent) = folder_at_mut(root, parent_path).and_then(Value::as_object_mut)
        && let Some(list) = parent.get_mut("folders").and_then(Value::as_array_mut)
        && idx < list.len()
    {
        list.remove(idx);
        if list.is_empty() {
            parent.remove("folders");
        }
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
/// from the row's ephemeral item identity ([`find_item_by_key`]) at click time,
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
        add_item, add_subfolder, count_tree, delete_folder, find_item_by_key, find_path_by_key,
        next_counter, object_ref, remove_item, rename_folder, stamp_keys, strip_keys,
    };
    use crate::bff::ehr_detail::directory::{DIRECTORY_ARCHETYPE, folder_json};

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

    /// The folder at `path`.
    fn folder<'a>(tree: &'a Value, path: &[usize]) -> &'a Value {
        let mut node = tree;
        for &index in path {
            node = &node["folders"][index];
        }
        node
    }

    /// The `_key` of the folder at `path`.
    fn key_at(tree: &Value, path: &[usize]) -> Option<String> {
        folder(tree, path)["_key"].as_str().map(str::to_owned)
    }

    /// The name of the folder at `path`.
    fn name_at(tree: &Value, path: &[usize]) -> String {
        folder(tree, path)["name"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// The `_key` of every item of the folder at `path`.
    fn item_keys(tree: &Value, path: &[usize]) -> Vec<String> {
        folder(tree, path)["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["_key"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
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
        let unique: std::collections::BTreeSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), 4, "every folder _key is unique");
        for path in [&[][..], &[0], &[0, 0], &[1]] {
            assert!(key_at(&tree, path).is_some());
        }
    }

    #[test]
    fn stamp_assigns_a_unique_key_to_every_item_too() {
        let mut tree = root_with_items();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        // 4 folders + 3 item references, each stamped once from ONE counter.
        assert_eq!(counter, 7);
        let items = item_keys(&tree, &[0]);
        assert_eq!(items.len(), 3);
        let mut all = items.clone();
        collect_keys(&tree, &mut all);
        let unique: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(
            unique.len(),
            7,
            "folder and item keys share one namespace and never collide"
        );
        // Idempotent for items as well: a second pass re-stamps nothing.
        stamp_keys(&mut tree, &mut counter);
        assert_eq!(counter, 7);
        assert_eq!(item_keys(&tree, &[0]), items);
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
        assert!(key_at(&tree, &[1, 0]).is_some());
    }

    #[test]
    fn identity_key_survives_a_sibling_delete() {
        let mut tree = root();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        let root_key = key_at(&tree, &[]).expect("stamped root");
        let b_key = key_at(&tree, &[1]).expect("stamped b");
        assert_eq!(find_path_by_key(&tree, &root_key), Some(Vec::new()));
        assert_eq!(find_path_by_key(&tree, &b_key), Some(vec![1]));
        assert_eq!(find_path_by_key(&tree, "nope"), None);
        // Deleting the first child shifts `b` from index 1 to index 0; its
        // identity key now resolves to the new path.
        delete_folder(&mut tree, &[0]);
        assert_eq!(find_path_by_key(&tree, &b_key), Some(vec![0]));
        assert_eq!(name_at(&tree, &[0]), "b");
    }

    #[test]
    fn add_and_rename_and_delete_folders() {
        let mut tree = root();
        add_subfolder(&mut tree, &[1], "b1");
        assert_eq!(name_at(&tree, &[1, 0]), "b1");
        rename_folder(&mut tree, &[1, 0], "renamed");
        assert_eq!(name_at(&tree, &[1, 0]), "renamed");
        // Deleting the first child shifts the second into index 0.
        delete_folder(&mut tree, &[0]);
        assert_eq!(tree["folders"].as_array().map(Vec::len), Some(1));
        assert_eq!(name_at(&tree, &[0]), "b");
        // Deleting the root is a no-op.
        delete_folder(&mut tree, &[]);
        assert_eq!(name_at(&tree, &[]), "root");
        // Deleting the last child leaves no empty `folders` array behind.
        delete_folder(&mut tree, &[0]);
        assert!(tree.get("folders").is_none(), "{tree}");
    }

    #[test]
    fn items_add_and_remove() {
        let mut tree = root();
        add_item(
            &mut tree,
            &[0],
            object_ref("local", "COMPOSITION", "HIER_OBJECT_ID", "abc"),
        );
        assert_eq!(tree["folders"][0]["items"][0]["id"]["value"], "abc");
        remove_item(&mut tree, &[0], 0);
        assert_eq!(
            tree["folders"][0]["items"].as_array().map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn an_item_is_found_by_its_key_after_a_sibling_removal() {
        let mut tree = root_with_items();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        let before = item_keys(&tree, &[0]);
        assert_eq!(find_item_by_key(&tree, &before[2]), Some((vec![0], 2)));
        assert_eq!(find_item_by_key(&tree, "nope"), None);
        // Remove the MIDDLE item, addressing it by identity rather than by a
        // captured positional index.
        let (path, doomed) = find_item_by_key(&tree, &before[1]).expect("the middle item");
        remove_item(&mut tree, &path, doomed);
        assert_eq!(
            item_keys(&tree, &[0]),
            vec![before[0].clone(), before[2].clone()]
        );
        assert_eq!(find_item_by_key(&tree, &before[1]), None);
        assert_eq!(find_item_by_key(&tree, &before[2]), Some((vec![0], 1)));
        assert_eq!(tree["folders"][0]["items"][1]["id"]["value"], "z");
    }

    #[test]
    fn a_node_added_to_a_posted_tree_takes_a_fresh_key() {
        let mut tree = root_with_items();
        let mut counter = 0u64;
        stamp_keys(&mut tree, &mut counter);
        // The working copy comes back from the form; the next pass must not
        // reuse a key a surviving node holds.
        let mut next = next_counter(&tree);
        assert_eq!(next, counter);
        add_subfolder(&mut tree, &[1], "new");
        stamp_keys(&mut tree, &mut next);
        let mut keys = Vec::new();
        collect_keys(&tree, &mut keys);
        keys.extend(item_keys(&tree, &[0]));
        let unique: std::collections::BTreeSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len());
        assert_eq!(next_counter(&json!({})), 0);
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
