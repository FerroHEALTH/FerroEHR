// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identity a built `LOCATABLE` carries: its `name`, its
//! `archetype_node_id` and, at an archetype root, its `archetype_details` (RM
//! `UML/classes/org.openehr.rm.common.locatable.adoc`,
//! `org.openehr.rm.common.archetyped.adoc`).

use openehr_rm::v1_2::access::RmObject;

use crate::flat::build::is_locatable;
use crate::flat::error::FlatError;
use crate::flat::typed::{self, Build, Obj, o, s};
use crate::flat::webtemplate::model::{CodedName, WebTemplateNode};

/// The `name` value for a locatable node: a DV_CODED_TEXT carrying the
/// template's constrained `defining_code` when the name is coded (RM common
/// `master03-archetyped_package.adoc` §"The LOCATABLE class"), else a plain
/// DV_TEXT.
pub(crate) fn name_value(display: &str, coded: Option<&CodedName>) -> Result<Obj, FlatError> {
    match coded {
        Some(CodedName {
            terminology, code, ..
        }) => typed::dv_coded_text(display, terminology, code),
        None => typed::dv_text(display),
    }
}

/// Stamp the two LOCATABLE identity members (`name`, `archetype_node_id`) on a
/// built node — and ONLY when its class is a LOCATABLE (see [`is_locatable`]).
pub(crate) fn stamp_locatable_identity(
    obj: &mut dyn RmObject,
    node: &WebTemplateNode,
    rm_type: &str,
) -> Result<(), FlatError> {
    if !is_locatable(rm_type) {
        return Ok(());
    }
    obj.or_put("name", || {
        let text = node
            .name
            .as_deref()
            .map_or_else(|| default_name(rm_type), str::to_owned);
        Ok(o(name_value(&text, node.name_coded.as_ref())?))
    })?;
    if let Some(nid) = recorded_archetype_id(node) {
        obj.or_put("archetype_node_id", || Ok(s(nid)))?;
    }
    Ok(())
}

/// The `archetype_node_id` data records for `node`: the complete identifier an
/// OPT2 inlined an archetype root by (BASE `artefact_identification`
/// `master07-referencing.adoc` §Reconstitutability), else its Web Template
/// `nodeId`.
pub(crate) fn recorded_archetype_id(node: &WebTemplateNode) -> Option<&str> {
    node.archetype_ref.as_deref().or(node.node_id.as_deref())
}

/// The name of a `LOCATABLE` whose template node has no term text, which ADL2
/// allows for a single node under a single-valued attribute
/// (`master04.3-cadl_complex_types.adoc` §Node Identifiers): `History`,
/// `Tree`, `List`, `Single` or `Table` for the structures, else the RM type in
/// sentence case, never the bare node code (RM `LOCATABLE.name`).
///
/// NOTE: no openEHR spec names such a node; the RM type is our own design.
pub(crate) fn default_name(rm_type: &str) -> String {
    let words = rm_type
        .strip_prefix("ITEM_")
        .unwrap_or(rm_type)
        .to_lowercase()
        .replace('_', " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Stamps `archetype_details` on an archetype-root node, naming `rm_version`
/// as the RM release the object was created to (RM common
/// `master03-archetyped_package.adoc` §ARCHETYPED `rm_version`).
pub(crate) fn stamp_archetype_details(
    obj: &mut dyn RmObject,
    node: &WebTemplateNode,
    is_root: bool,
    template_id: &str,
    rm_version: &str,
) -> Result<(), FlatError> {
    let is_root_arch = node
        .node_id
        .as_deref()
        .is_some_and(openehr_rm::v1_2::paths::is_archetype_root_node_id);
    if !is_root_arch {
        return Ok(());
    }
    obj.or_put("archetype_details", || {
        let mut a = typed::new("ARCHETYPED")?;
        if let Some(id) = recorded_archetype_id(node) {
            a.put(
                "archetype_id",
                o(typed::make("ARCHETYPE_ID", [("value", s(id))])?),
            )?;
        }
        if is_root && !template_id.is_empty() {
            a.put(
                "template_id",
                o(typed::make("TEMPLATE_ID", [("value", s(template_id))])?),
            )?;
        }
        a.put("rm_version", s(rm_version))?;
        Ok(o(a))
    })
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the test reads a built name through its canonical JSON"
)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canonical(object: Result<Obj, FlatError>) -> serde_json::Value {
        typed::canonical_of(&*object.unwrap()).unwrap()
    }

    #[test]
    fn a_node_without_term_text_is_named_for_its_rm_type() {
        let names: Vec<String> = [
            "HISTORY",
            "ITEM_TREE",
            "ITEM_LIST",
            "ITEM_SINGLE",
            "ITEM_TABLE",
            "POINT_EVENT",
        ]
        .into_iter()
        .map(default_name)
        .collect();
        assert_eq!(
            names,
            ["History", "Tree", "List", "Single", "Table", "Point event"]
        );
    }

    #[test]
    fn name_value_stamps_coded_and_plain_names() {
        let coded = CodedName {
            terminology: "local".to_owned(),
            code: "at0007".to_owned(),
            incoherent: false,
        };
        let v = canonical(name_value("Exclusion", Some(&coded)));
        assert_eq!(v["_type"], "DV_CODED_TEXT");
        assert_eq!(v["defining_code"]["code_string"], "at0007");
        assert_eq!(
            canonical(name_value("Systolic", None)),
            json!({"_type": "DV_TEXT", "value": "Systolic"})
        );
    }
}
