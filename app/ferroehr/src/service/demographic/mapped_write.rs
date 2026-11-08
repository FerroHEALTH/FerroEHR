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

use openehr_base::prelude::{HierObjectId, ObjectId, PartyRef};
use openehr_rm::prelude::{DvIdentifier, DvUri, DvUriData};
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use serde_json::{Map, Value};

use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
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
    /// A body with no identity takes the template's first `PARTY_IDENTITY`,
    /// with its empty `details`.
    /// Each path names an `ELEMENT` (or its `value`) of the template. Several
    /// identifiers can share one path: a `DV_IDENTIFIER` value replaces the
    /// one of its `type` and `issuer`, and leaves the others as they are.
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
        let named = body
            .get("identities")
            .and_then(Value::as_array)
            .is_some_and(|identities| !identities.is_empty());
        if let Some(object) = body.as_object_mut()
            && !named
        {
            object.insert("identities".to_owned(), Value::Array(vec![identity(&wt)?]));
        }
        Ok(body)
    }

    /// Returns the data value `text` takes at the `ELEMENT` `path` of
    /// `template_id`, by the RM type the template draws there: a
    /// `DV_IDENTIFIER` of `identifier`'s issuer and type, a `DV_TEXT` or a
    /// `DV_URI`.
    ///
    /// # Errors
    /// The [`Self::web_template`] refusals, and a `422` naming the path when
    /// the template draws no data value there that a text fills.
    pub async fn mapped_text_value(
        &self,
        template_id: &str,
        path: &str,
        text: &str,
        identifier: (&str, &str),
    ) -> Result<Value, ServiceError> {
        let wt = self.web_template(template_id).await?;
        let types = super::mapping::value_types(&wt.tree, path);
        let drawn = |rm_type: &str| types.iter().any(|t| t == rm_type);
        if drawn("DV_IDENTIFIER") {
            let (issuer, identifier_type) = identifier;
            return Ok(openehr_its::json::to_canonical_value(&DvIdentifier {
                issuer: Some(issuer.to_owned()),
                assigner: None,
                id: text.to_owned(),
                r#type: Some(identifier_type.to_owned()),
            }));
        }
        if drawn("DV_TEXT") {
            return Ok(openehr_its::json::to_canonical_value(
                &crate::versioning::audit::dv_text(text),
            ));
        }
        if drawn("DV_URI") {
            return Ok(openehr_its::json::to_canonical_value(&DvUri::DvUri(
                DvUriData {
                    value: text.to_owned(),
                },
            )));
        }
        Err(ServiceError::content_invalid(
            Violation::new(format!(
                "the template draws {} here, which a text does not fill",
                if types.is_empty() {
                    "no data value".to_owned()
                } else {
                    types.join(" or ")
                }
            ))
            .with_path(path),
        ))
    }
}

/// The data value of the first `ELEMENT` at `path` in the party `body`, the
/// one of `identifier`'s `type` and `issuer` when given.
#[must_use]
pub fn mapped_value<'a>(
    body: &'a Value,
    path: &str,
    identifier: Option<(&str, &str)>,
) -> Option<&'a Value> {
    let steps: Vec<&str> = path
        .trim_end_matches("/value")
        .trim_start_matches('/')
        .split('/')
        .collect();
    let mut current = vec![body];
    for step in steps {
        let (attribute, node_id) = match step.split_once('[') {
            Some((attribute, predicate)) => (attribute, Some(predicate.trim_end_matches(']'))),
            None => (step, None),
        };
        let held = |item: &&Value| {
            node_id
                .is_none_or(|id| item.get("archetype_node_id").and_then(Value::as_str) == Some(id))
        };
        current = current
            .into_iter()
            .filter_map(|node| node.get(attribute))
            .flat_map(|slot| match slot {
                Value::Array(items) => items.iter().filter(held).collect::<Vec<_>>(),
                single => std::iter::once(single).filter(held).collect(),
            })
            .collect();
    }
    current
        .into_iter()
        .filter_map(|element| element.get("value"))
        .find(|value| {
            identifier.is_none_or(|(identifier_type, issuer)| {
                identifier_slot(Some(value)) == Some((Some(identifier_type), Some(issuer)))
            })
        })
}

impl FerroEhrService {
    /// Returns the body of a new professional `ROLE` of `template_id`
    /// performed by the `PERSON` `performer`, named under `namespace`.
    ///
    /// The `ROLE` carries the template's first `PARTY_IDENTITY` with its
    /// empty `details`; a template that requires an item in it makes the
    /// `ROLE` incomplete until an administrator fills it.
    ///
    /// # Errors
    /// The [`Self::web_template`] refusals, and a `422` when the template
    /// draws no `PARTY_IDENTITY` with its `details`.
    pub async fn professional_role_body(
        &self,
        template_id: &str,
        performer: VoId,
        namespace: &str,
    ) -> Result<Value, ServiceError> {
        let wt = self.web_template(template_id).await?;
        let mut body = new_party(&wt);
        if let Some(object) = body.as_object_mut() {
            object.insert("identities".to_owned(), Value::Array(vec![identity(&wt)?]));
            object.insert(
                "performer".to_owned(),
                party_ref(namespace, "PERSON", performer),
            );
        }
        Ok(body)
    }
}

/// The canonical `PARTY_REF` to the party container `id` of the RM class
/// `class`, named under `namespace`.
pub(crate) fn party_ref(namespace: &str, class: &str, id: VoId) -> Value {
    openehr_its::json::to_canonical_value(&PartyRef {
        namespace: namespace.to_owned(),
        r#type: class.to_owned(),
        id: ObjectId::HierObjectId(HierObjectId::from(id.0)),
    })
}

/// The first `PARTY_IDENTITY` the template `wt` draws, with its `details`
/// structure, each named as the template names it and holding no item.
///
/// RM demographic `PARTY` requires an identity (`Identities_valid`) and
/// `PARTY_IDENTITY` its `details`; a template that requires an item makes the
/// party incomplete until the item is written.
fn identity(wt: &WebTemplate) -> Result<Value, ServiceError> {
    let undrawn = || {
        ServiceError::content_invalid(
            Violation::new("the template draws no PARTY_IDENTITY with its details")
                .with_path("/identities"),
        )
    };
    let identity = wt
        .tree
        .children
        .iter()
        .find(|child| base_type(&child.rm_type) == "PARTY_IDENTITY")
        .ok_or_else(undrawn)?;
    // NOTE: a web template leaves the structure out and draws its items, so
    // the `details` node id is read from the path of an item under it.
    let identity_path = super::mapping::plain(&identity.aql_path);
    let details_id = identity
        .children
        .iter()
        .find_map(|child| {
            let path = super::mapping::plain(&child.aql_path);
            let rest = path
                .strip_prefix(&identity_path)?
                .strip_prefix("/details[")?;
            rest.split(']').next().map(str::to_owned)
        })
        .ok_or_else(undrawn)?;
    let details = created(
        wt,
        &format!("{identity_path}/details[{details_id}]"),
        "details",
        Some(&details_id),
        false,
    );
    let mut drawn = node(identity);
    if let Some(object) = drawn.as_object_mut() {
        object.insert("details".to_owned(), details);
    }
    Ok(drawn)
}

/// `rm_type` without its generic parameters.
fn base_type(rm_type: &str) -> &str {
    rm_type.split('<').next().unwrap_or(rm_type)
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
        Value::String(base_type(&wt.rm_type).to_owned()),
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
///
/// A `DV_IDENTIFIER` replaces only the identifier of its own `type` and
/// `issuer` at the path, and is added beside the others when there is none.
fn set(body: &mut Value, wt: &WebTemplate, path: &str, value: Value) -> Result<(), ServiceError> {
    let element_path = path.trim_end_matches("/value");
    if super::mapping::value_types(&wt.tree, element_path).is_empty() {
        return Err(unmapped(path));
    }
    let steps: Vec<&str> = element_path.trim_start_matches('/').split('/').collect();
    let identifier = identifier_slot(Some(&value));
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
                    }) && (!last
                        || identifier
                            .is_none_or(|held| identifier_slot(item.get("value")) == Some(held)))
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

/// The `type` and `issuer` of a `DV_IDENTIFIER` `value`, which tell apart
/// the identifiers of two selectors at one path; `None` for any other value.
/// Both are attributes of the identifier (RM
/// `org.openehr.rm.data_types.dv_identifier.adoc`), and a selector holds the
/// identifiers of one type and issuer set.
fn identifier_slot(value: Option<&Value>) -> Option<(Option<&str>, Option<&str>)> {
    let value = value?;
    (value.get("_type").and_then(Value::as_str) == Some("DV_IDENTIFIER")).then(|| {
        (
            value.get("type").and_then(Value::as_str),
            value.get("issuer").and_then(Value::as_str),
        )
    })
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
