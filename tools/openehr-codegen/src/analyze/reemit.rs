// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cross-schema re-emission closure: the upstream classes a downstream
//! schema re-emits locally, and the schema copy that carries them.

use std::collections::{BTreeMap, BTreeSet};

use crate::analyze::{Model, class_paths};
use crate::load::bmm::BmmSchema;

/// The upstream classes a downstream schema must re-emit locally because its
/// own classes extend them across an `includes` boundary.
///
/// A class defined only upstream is re-emitted when its Rust form differs
/// downstream, as a least fixpoint: its variant set gains a downstream or
/// re-emitted class, or a field type contains one. Empty for AM 1.4.
#[must_use]
pub(crate) fn cross_schema_reemit(model: &Model, schema: &BmmSchema) -> BTreeSet<String> {
    let downstream: BTreeSet<&str> = schema.classes.keys().map(String::as_str).collect();
    let is_upstream =
        |n: &str| model.get(n).is_some() && !downstream.contains(n) && !Model::is_mapped(n);

    let mut reemit: BTreeSet<String> = BTreeSet::new();
    loop {
        let mut added = false;
        let names: Vec<String> = model.class_iter().map(|(n, _)| n.clone()).collect();
        let is_target = |n: &str, r: &BTreeSet<String>| downstream.contains(n) || r.contains(n);
        for name in names {
            if reemit.contains(&name) || !is_upstream(&name) {
                continue;
            }
            // (a) polymorphic parent whose variant set widens downstream …
            let widens = model
                .enum_variants(&name)
                .iter()
                .any(|v| is_target(v, &reemit));
            // … or (b) container whose own field types touch a widened/downstream
            // type (so the field must resolve to the re-emitted crate-local type).
            let contains = model
                .field_roots(&name)
                .iter()
                .any(|r| is_target(r, &reemit));
            if widens || contains {
                reemit.insert(name);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    reemit
}

/// An augmented copy of `schema` with the [`cross_schema_reemit`] classes
/// grafted at the package paths they occupy in `sources`.
#[must_use]
pub(crate) fn augment_with_reemit(
    schema: &BmmSchema,
    model: &Model,
    reemit: &BTreeSet<String>,
    sources: &[&BmmSchema],
) -> BmmSchema {
    // Source package path (slash-joined) per class, from the include sources.
    let mut src_path: BTreeMap<String, String> = BTreeMap::new();
    for src in sources {
        for (cls, path) in class_paths(src) {
            src_path.entry(cls).or_insert(path);
        }
    }
    let mut s = schema.clone();
    for name in reemit {
        if let Some(c) = model.get(name) {
            s.classes.insert(name.clone(), c.clone());
        }
        let path = src_path.get(name).cloned().unwrap_or_default();
        let segments: Vec<&str> = path.split('/').filter(|seg| !seg.is_empty()).collect();
        insert_class_into_packages(&mut s.packages, &segments, name);
    }
    s
}

/// Adds `class` to the package chain `segments` names, creating it; empty
/// `segments` means the schema root.
fn insert_class_into_packages(
    packages: &mut Vec<crate::load::bmm::BmmPackage>,
    segments: &[&str],
    class: &str,
) {
    let Some((head, rest)) = segments.split_first() else {
        return;
    };
    if !packages.iter().any(|p| p.name.as_str() == *head) {
        packages.push(crate::load::bmm::BmmPackage {
            name: (*head).to_string(),
            classes: Vec::new(),
            packages: Vec::new(),
        });
    }
    // Found or just pushed, so the lookup succeeds; `find` keeps it index-free.
    let Some(pkg) = packages.iter_mut().find(|p| p.name.as_str() == *head) else {
        return;
    };
    if rest.is_empty() {
        if !pkg.classes.iter().any(|c| c == class) {
            pkg.classes.push(class.to_string());
        }
    } else {
        insert_class_into_packages(&mut pkg.packages, rest, class);
    }
}
