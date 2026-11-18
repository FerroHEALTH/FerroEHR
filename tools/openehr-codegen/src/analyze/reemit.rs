// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cross-schema re-emission closure: the upstream classes a downstream
//! schema re-emits locally, and the schema copy that carries them.

use std::collections::{BTreeMap, BTreeSet};

use crate::analyze::{Model, class_paths};
use crate::load::bmm::BmmSchema;

/// Compute the closure of *upstream* (dependency-schema) classes a downstream
/// schema must **re-emit locally**, because the downstream schema declares
/// subtypes of upstream polymorphic classes across an `includes` boundary.
///
/// This realizes the owner ruling (2026-07-19):
/// cross-component subtype extension (e.g. the AM 2.4 `rules` leaves
/// `EXPR_ARCHETYPE_REF`/`EXPR_CONSTRAINT` extending LANG's `EXPR_VALUE_REF`/
/// `EXPR_LEAF`) is re-opened at the **downstream** crate boundary — an
/// extender-level enum composing the upstream variants + the downstream leaves —
/// while the upstream crate stays byte-identical (dependency arrows point one
/// way, so an upstream enum never gains a downstream variant).
///
/// `model` is the downstream crate's merged include-closure (BASE + LANG + AM);
/// `schema` is the downstream schema (its own classes are "downstream"). A class
/// `C` defined only upstream is re-emitted iff its emitted Rust form **differs**
/// in the downstream view (a least fixpoint): (a) `C` is a polymorphic parent
/// whose concrete-descendant (variant) set gains a downstream class or an
/// already-in-closure class, or (b) `C` **contains**, through one of its own
/// (flattened) field types, a downstream or already-in-closure class. This is
/// deliberately **complete, not minimal** (owner ruling 2026-07-19): code
/// generation exists to emit *every* cross-`includes` extension the vendored
/// BMM implies — e.g. both the beom expression/statement subtree (extended by
/// the AM `rules` leaves) and `AUTHORED_RESOURCE`/`RESOURCE_DESCRIPTION` (the
/// resource metatype `AUTHORED_ARCHETYPE` etc. extend) are re-emitted. Upstream
/// classes whose form is unchanged (they gain no downstream descendant and touch
/// no widened type — e.g. `EXPR_LITERAL`, the whole EL/BMM3 tree) stay external.
/// Returns the empty set when the downstream schema adds no cross-boundary
/// subtypes (e.g. AM 1.4).
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

/// Produce an augmented copy of `schema` in which the [`cross_schema_reemit`]
/// closure classes (pulled from the merged `model`) are added to the schema's
/// own class set and grafted into its package tree at the **same package path
/// they occupy in their source schema** (`sources`) — so a re-emitted
/// `EXPRESSION` lands under `<prefix>/beom/core` mirroring LANG, an
/// `AUTHORED_RESOURCE` under its BASE package, etc. — emitting them as
/// crate-local types the downstream fields resolve against.
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

/// Descend/create the nested [`BmmPackage`](crate::load::bmm::BmmPackage) chain named
/// by `segments` and add `class` to the leaf package's class list (idempotent).
/// An empty `segments` drops `class` at the schema root (no package prefix).
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
