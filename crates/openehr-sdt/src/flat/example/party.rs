// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party-specific steps of the example generator: which root is a party
//! class, and the identity RM demographic §PARTY requires of every party
//! (`UML/classes/org.openehr.rm.demographic.party.adoc`).

use serde_json::{Map, Value};

use super::{DetailLevel, child_groups, seg_for, tail_attribute, walk};
use crate::flat::error::FlatError;
use crate::flat::webtemplate::model::WebTemplateNode;

/// The party class `rm_type` names, if it names one.
pub(super) fn party_class(rm_type: &str) -> Option<&'static str> {
    crate::flat::build::PARTY_CLASSES
        .into_iter()
        .find(|class| *class == rm_type)
}

/// Whether `out` holds a flat entry at `base` or under it: the node at `base`
/// itself, one of its attributes (`|`), a child (`/`) or an occurrence (`:`).
pub(super) fn emitted_under(base: &str, out: &Map<String, Value>) -> bool {
    out.keys().any(|k| {
        k.strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['|', '/', ':']))
    })
}

/// Materialises one `identities` member of a party when the template walk left
/// none, as RM demographic §PARTY `Identities_valid` (`not identities.is_empty`)
/// requires one whatever occurrences the template states
/// (`UML/classes/org.openehr.rm.demographic.party.adoc`).
///
/// The first `PARTY_IDENTITY` the template constrains is walked as a forced
/// node, so it drills to a leaf the way a mandatory node does.
///
/// # Errors
///
/// [`FlatError::NoExampleIdentity`] when the template constrains no
/// `identities` member to make one from.
pub(super) fn satisfy_party_identities(
    tree: &WebTemplateNode,
    class: &str,
    level: DetailLevel,
    out: &mut Map<String, Value>,
) -> Result<(), FlatError> {
    let root = &tree.id;
    let identities: Vec<&WebTemplateNode> = child_groups(tree)
        .into_iter()
        .filter(|c| tail_attribute(&c.aql_path) == "identities")
        .collect();
    if identities
        .iter()
        .any(|c| emitted_under(&format!("{root}/{}", c.id), out))
    {
        return Ok(());
    }
    let identity = identities
        .first()
        .ok_or_else(|| FlatError::NoExampleIdentity {
            class: class.to_owned(),
        })?;
    let prefix = format!("{root}/{}", seg_for(identity));
    walk(identity, &prefix, 0, level, true, out);
    Ok(())
}
