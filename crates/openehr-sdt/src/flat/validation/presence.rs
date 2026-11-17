// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether an RM attribute is present on an instance node: the absence the
//! existence check refuses on a mandatory attribute, and the presence the
//! prohibition check refuses on an attribute the template prohibits.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::Value;

use super::Validator;
use crate::flat::webtemplate::model::WebTemplateNode;
use crate::rm_instance::ValidationKind;

/// Whether an RM attribute field is absent (missing or JSON `null`) on a node,
/// which the existence check refuses on a mandatory attribute.
///
/// A present empty list is a container that is there: AOM 1.4
/// `master04-constraint_model_package.adoc` (`C_MULTIPLE_ATTRIBUTE`) gives
/// existence "whether the container object will be there (at all)" and leaves
/// how many members it holds to cardinality ([`super::Validator`]'s
/// cardinality check).
pub(super) fn attr_absent(node: &Value, attr: &str) -> bool {
    matches!(node.get(attr), None | Some(Value::Null))
}

/// Whether an RM attribute field carries content: present, not `null`, and,
/// for a list, with at least one member. An empty list of a prohibited
/// attribute carries nothing, so the prohibition check reads it as absent.
fn attr_has_content(node: &Value, attr: &str) -> bool {
    match node.get(attr) {
        None | Some(Value::Null) => false,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    }
}

impl Validator {
    /// Refuse every attribute of `instance` its template node prohibits
    /// (existence `0..0`: AOM 1.4 `master04-constraint_model_package.adoc`
    /// §existence, AOM2 `master03-archetype_package.adoc` §C_ATTRIBUTE).
    pub(super) fn check_prohibited(&mut self, instance: &Value, wt: &WebTemplateNode) {
        for attribute in &wt.prohibited_attributes {
            if attr_has_content(instance, attribute) {
                self.push(
                    format!("{}/{attribute}", wt.aql_path),
                    format!(
                        "attribute '{attribute}' is prohibited by the template (existence 0..0)"
                    ),
                    ValidationKind::Unexpected,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{attr_absent, attr_has_content};

    /// A present empty list is there for the existence check and carries no
    /// content for the prohibition check; a missing or `null` member is absent.
    #[test]
    fn an_empty_list_is_present_and_carries_no_content() {
        let node = json!({ "items": [], "nulled": null, "full": [1] });
        assert!(!attr_absent(&node, "items"));
        assert!(attr_absent(&node, "nulled"));
        assert!(attr_absent(&node, "missing"));
        assert!(!attr_has_content(&node, "items"));
        assert!(!attr_has_content(&node, "nulled"));
        assert!(attr_has_content(&node, "full"));
    }
}
