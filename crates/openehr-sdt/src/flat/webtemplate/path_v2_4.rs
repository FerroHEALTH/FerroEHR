// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data path of a `v2_4` Web Template node, with the name predicate that
//! tells same-archetype siblings apart.

use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use super::builder_v2_4::{attr_children, cstring_under};
use super::model::WebTemplateNode;

/// The data path of a node below `parent`. An `[idN]` predicate stands
/// for `[@archetype_node_id = 'idN']` (ADL2 `master05-paths.adoc` §"Relationship
/// with W3C Xpath"), which a `DATA_VALUE` lacks, being no `LOCATABLE` (RM
/// `data_types`; RM `common` §Archetyped Package), so no data-value step takes
/// one: `COMPOSITION.category` reads `/category`, and a `DV_INTERVAL`'s parts
/// read `/lower` and `/upper` (ITS-REST
/// `simplified_formats/master05-rm_mapping.adoc` §DV_INTERVAL).
pub(super) fn build_path(
    parent: Option<&WebTemplateNode>,
    attr: Option<&str>,
    rm_type: &str,
    id: &str,
    name: Option<&str>,
) -> String {
    let (Some(parent), Some(a)) = (parent, attr) else {
        return String::new();
    };
    let path = &parent.aql_path;
    if id.is_empty() || rm_type.starts_with("DV_") {
        format!("{path}/{a}")
    } else if let Some(name) = name {
        format!("{path}/{a}[{id},'{name}']")
    } else {
        format!("{path}/{a}[{id}]")
    }
}

/// The single fixed name a node's `name` constraint gives it, for the
/// `[id,'Name']` path predicate: a `name/value` `C_STRING` holding one literal.
///
/// Sibling archetype roots of one archetype carry the same
/// `archetype_node_id` in data (RM common `master03-archetyped_package.adoc`
/// §"The `LOCATABLE` class": at a root point it is the archetype id), so the
/// runtime name is what tells them apart (BASE
/// `architecture_overview/master11-paths.adoc` §"Using a Name-based
/// Predicate"); the twin of the OPT 1.4 builder's `name_constraint`.
pub(super) fn name_constraint(co: &CObject) -> Option<String> {
    attr_children(co, "name").find_map(|name| {
        let value = cstring_under(name, "value")?;
        match value.constraint.as_deref() {
            Some([literal]) if !literal.starts_with('/') => Some(literal.clone()),
            _ => None,
        }
    })
}

/// The OPT path of an object with `node_id` under the attribute at
/// `attr_path`: `attr_path[node_id]`, or `attr_path` for an object with no node
/// id (ADL2 `master05-paths.adoc` §Overview), the form
/// `openehr_adl::opt::RawOpt` writes its prohibited attributes in.
pub(super) fn object_path(attr_path: &str, node_id: &str) -> String {
    if node_id.is_empty() {
        attr_path.to_owned()
    } else {
        format!("{attr_path}[{node_id}]")
    }
}

/// The names of the attributes `prohibited` lists directly under the object at
/// OPT path `owner`.
pub(super) fn prohibited_under(prohibited: &[String], owner: &str) -> Vec<String> {
    prohibited
        .iter()
        .filter_map(|path| path.strip_prefix(owner)?.strip_prefix('/'))
        .filter(|name| !name.contains(['/', '[']))
        .map(str::to_owned)
        .collect()
}
