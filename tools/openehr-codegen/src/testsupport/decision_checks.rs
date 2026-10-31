// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decision-map checks that read the loaded schemas: the ordered-hash
//! table and the existence contradictions, each still grounded in a BMM.

use super::{DeclEntry, DeclMap, Error, compose, crate_keys};
use crate::load::bmm::{BmmPropKind, BmmType};
use crate::plan::ordered_hashes::ORDERED_HASHES;
use crate::plan::overrides;

/// The ordered-hash table as a declarative decision map.
pub(super) fn ordered_hash_map() -> DeclMap {
    DeclMap {
        map: "ordered_hash",
        check_existence: true,
        entries: ORDERED_HASHES
            .iter()
            .map(|e| DeclEntry {
                key: format!("{}.{}", e.owner, e.field),
                decision: "indexmap::IndexMap".to_string(),
                citation: e.citation.to_string(),
                reason: e.reason.to_string(),
            })
            .collect(),
    }
}

/// Every declaration of `owner.field` across the loaded schemas, each with the
/// `<crate>/<file>` it comes from and whether it is mandatory and a `Hash`.
fn declarations(owner: &str, field: &str) -> Result<Vec<(String, bool, bool)>, Error> {
    let mut out = Vec::new();
    for key in crate_keys() {
        for u in compose(key)?.generations.iter().flat_map(|g| &g.units) {
            let Some(prop) = u
                .model
                .get(owner)
                .and_then(|cls| cls.properties.iter().find(|p| p.name == field))
            else {
                continue;
            };
            let hash = matches!(&prop.kind,
                BmmPropKind::Single(BmmType::Generic { root, .. }) if root == "Hash");
            out.push((format!("{key}/{}", u.spec.file), prop.is_mandatory, hash));
        }
    }
    Ok(out)
}

/// Every ordered-hash entry whose vendored ground is gone.
///
/// Some loaded schema must declare the attribute on the entry's owner as a
/// `Hash<K, V>`; a release that types it a `List` keeps its `Vec`.
///
/// # Errors
/// Returns an error if any composition's BMM files cannot be loaded.
pub fn stale_ordered_hashes() -> Result<Vec<String>, Error> {
    let mut stale = Vec::new();
    for entry in ORDERED_HASHES {
        let found = declarations(entry.owner, entry.field)?;
        if !found.iter().any(|(_, _, hash)| *hash) {
            stale.push(format!(
                "{}.{}: no loaded schema declares it as a Hash on its owner",
                entry.owner, entry.field
            ));
        }
    }
    Ok(stale)
}

/// Every existence-contradiction entry whose vendored ground is gone, as one
/// message per defect.
///
/// An entry stays live only while some loaded schema declares the attribute on
/// the entry's owner AND every such declaration is still a mandatory
/// `Hash<K, V>`: an upstream fix to `0..1` (or a retyped attribute) leaves the
/// entry stale, and so does an owner that declares no such attribute.
///
/// # Errors
/// Returns an error if any composition's BMM files cannot be loaded.
pub fn stale_existence_contradictions() -> Result<Vec<String>, Error> {
    let mut stale = Vec::new();
    for entry in overrides::SPEC_CONTRADICTIONS
        .iter()
        .filter(|c| c.kind == overrides::ContradictionKind::Existence)
    {
        let found = declarations(entry.owner, entry.field)?;
        for (source, mandatory, hash) in &found {
            if !mandatory || !hash {
                stale.push(format!(
                    "{source}: {}.{} is no longer a mandatory Hash",
                    entry.owner, entry.field
                ));
            }
        }
        if found.is_empty() {
            stale.push(format!(
                "{}.{}: no loaded schema declares the attribute on its owner",
                entry.owner, entry.field
            ));
        }
    }
    Ok(stale)
}
