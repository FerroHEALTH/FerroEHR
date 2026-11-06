// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A write of a mapped party: the next version starts from the head and
//! replaces only the paths the mapping names.
//!
//! SCIM provisioning and just-in-time creation write the values of the
//! attributes they map. Everything else the head holds, an unmapped detail,
//! an identifier another system added, a contact, is carried into the new
//! version unchanged. A path the head lacks is created from the template's
//! own node ids and names. No openEHR spec governs a mapping from SCIM or
//! OpenID Connect to a demographic template: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use serde_json::{Map, Value};

use crate::error::{ServiceError, Violation};
use crate::service::FerroEhrService;

/// The RM attributes of the demographic model that hold a list.
const LISTS: [&str; 7] = [
    "identities",
    "contacts",
    "addresses",
    "items",
    "relationships",
    "roles",
    "capabilities",
];

impl FerroEhrService {
    /// Returns the body of the next version of a party of `template_id`:
    /// `head` (a new party of the template when `None`) with the data value
    /// of each of `values` set at its path, and nothing else changed.
    ///
    /// Each path names an `ELEMENT` (or its `value`) of the template.
    ///
    /// # Errors
    /// The [`Self::web_template`] refusals when the template does not
    /// resolve, and a `422` naming the path when the template has no
    /// `ELEMENT` there.
    pub async fn mapped_party_body(
        &self,
        head: Option<&Value>,
        template_id: &str,
        values: &[(String, Value)],
    ) -> Result<Value, ServiceError> {
        let wt = self.web_template(template_id).await?;
        let mut body = match head {
            Some(head) => head.clone(),
            None => new_party(&wt),
        };
        for (path, value) in values {
            set(&mut body, &wt, path, value.clone())?;
        }
        Ok(body)
    }
}

/// A party of the template `wt` holding only its root.
fn new_party(wt: &WebTemplate) -> Value {
    let mut root = node(&wt.tree);
    if let Some(object) = root.as_object_mut() {
        object.insert(
            "archetype_details".to_owned(),
            openehr_its::json::to_canonical_value(&openehr_rm::prelude::Archetyped {
                archetype_id: openehr_base::prelude::ArchetypeId {
                    value: wt.tree.node_id.clone().unwrap_or_default(),
                },
                template_id: Some(openehr_base::prelude::TemplateId {
                    value: wt.template_id.clone(),
                }),
                rm_version: "1.1.0".to_owned(),
            }),
        );
    }
    root
}

/// The empty RM node of the template node `wt`: its class, node id and name.
fn node(wt: &WebTemplateNode) -> Value {
    let mut object = Map::new();
    object.insert(
        "_type".to_owned(),
        Value::String(
            wt.rm_type
                .split('<')
                .next()
                .unwrap_or(&wt.rm_type)
                .to_owned(),
        ),
    );
    if let Some(node_id) = &wt.node_id {
        object.insert(
            "archetype_node_id".to_owned(),
            Value::String(node_id.clone()),
        );
    }
    let name = wt
        .localized_name
        .clone()
        .or_else(|| wt.name.clone())
        .unwrap_or_else(|| wt.id.clone());
    object.insert(
        "name".to_owned(),
        openehr_its::json::to_canonical_value(&crate::versioning::audit::dv_text(&name)),
    );
    Value::Object(object)
}

/// Sets `value` as the data value of the `ELEMENT` at `path` in `body`,
/// creating each missing node.
///
/// A web template draws an `ELEMENT` as its value node and leaves out the
/// structures between, so a missing node takes the class its RM attribute
/// holds (RM demographic: `details` an `ITEM_TREE`, `identities` a
/// `PARTY_IDENTITY`; an item an `ELEMENT` at the end of the path and a
/// `CLUSTER` before it) and the name the template gives it, else its node id.
fn set(body: &mut Value, wt: &WebTemplate, path: &str, value: Value) -> Result<(), ServiceError> {
    let element_path = path.trim_end_matches("/value");
    if super::mapping::value_types(&wt.tree, element_path).is_empty() {
        return Err(unmapped(path));
    }
    let steps: Vec<&str> = element_path.trim_start_matches('/').split('/').collect();
    let mut current = body;
    let mut walked = String::new();
    for (index, step) in steps.iter().enumerate() {
        let (attribute, node_id) = match step.split_once('[') {
            Some((attribute, predicate)) => (attribute, Some(predicate.trim_end_matches(']'))),
            None => (*step, None),
        };
        walked.push('/');
        walked.push_str(step);
        let last = index + 1 == steps.len();
        let fresh = || created(wt, &walked, attribute, node_id, last);
        let Some(object) = current.as_object_mut() else {
            return Err(unmapped(path));
        };
        let slot = object.entry(attribute.to_owned()).or_insert_with(|| {
            if LISTS.contains(&attribute) {
                Value::Array(Vec::new())
            } else {
                Value::Null
            }
        });
        current = match slot {
            Value::Array(items) => {
                let found = items.iter().position(|item| {
                    node_id.is_none_or(|id| {
                        item.get("archetype_node_id").and_then(Value::as_str) == Some(id)
                    })
                });
                let index = found.unwrap_or_else(|| {
                    items.push(fresh());
                    items.len().saturating_sub(1)
                });
                items.get_mut(index).ok_or_else(|| unmapped(path))?
            }
            slot => {
                if slot.is_null() {
                    *slot = fresh();
                }
                slot
            }
        };
    }
    let Some(element) = current.as_object_mut() else {
        return Err(unmapped(path));
    };
    element.insert("value".to_owned(), value);
    Ok(())
}

/// The node created at `walked`, the step `attribute[node_id]` of a mapped
/// path, the `ELEMENT` when `last`.
fn created(
    wt: &WebTemplate,
    walked: &str,
    attribute: &str,
    node_id: Option<&str>,
    last: bool,
) -> Value {
    let drawn = find(&wt.tree, walked).or_else(|| {
        last.then(|| find(&wt.tree, &format!("{walked}/value")))
            .flatten()
    });
    let class = match (attribute, last) {
        (_, true) => "ELEMENT",
        ("details" | "credentials", false) => "ITEM_TREE",
        ("identities", false) => "PARTY_IDENTITY",
        ("contacts", false) => "CONTACT",
        ("addresses", false) => "ADDRESS",
        ("capabilities", false) => "CAPABILITY",
        _ => "CLUSTER",
    };
    let name = drawn
        .and_then(|node| node.localized_name.clone().or_else(|| node.name.clone()))
        .or_else(|| node_id.map(str::to_owned))
        .unwrap_or_else(|| attribute.to_owned());
    let mut object = Map::new();
    object.insert("_type".to_owned(), Value::String(class.to_owned()));
    if let Some(node_id) = node_id {
        object.insert(
            "archetype_node_id".to_owned(),
            Value::String(node_id.to_owned()),
        );
    }
    object.insert(
        "name".to_owned(),
        openehr_its::json::to_canonical_value(&crate::versioning::audit::dv_text(&name)),
    );
    Value::Object(object)
}

/// The template node at `path`.
fn find<'a>(node: &'a WebTemplateNode, path: &str) -> Option<&'a WebTemplateNode> {
    if super::mapping::plain(&node.aql_path) == path {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, path))
}

/// The `422` of a path the template has no `ELEMENT` at.
fn unmapped(path: &str) -> ServiceError {
    ServiceError::content_invalid(
        Violation::new("the template has no ELEMENT at this mapped path").with_path(path),
    )
}
