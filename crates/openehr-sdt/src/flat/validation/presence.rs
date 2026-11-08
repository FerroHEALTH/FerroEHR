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

/// Whether an RM attribute field is absent (missing, JSON `null`, or an empty
/// array) on a node — the negation of "the attribute is present" for the
/// existence check.
pub(super) fn attr_absent(node: &Value, attr: &str) -> bool {
    match node.get(attr) {
        None | Some(Value::Null) => true,
        Some(Value::Array(a)) => a.is_empty(),
        Some(_) => false,
    }
}

impl Validator {
    /// Refuse every attribute of `instance` its template node prohibits
    /// (existence `0..0`: AOM 1.4 `master04-constraint_model_package.adoc`
    /// §existence, AOM2 `master03-archetype_package.adoc` §C_ATTRIBUTE).
    pub(super) fn check_prohibited(&mut self, instance: &Value, wt: &WebTemplateNode) {
        for attribute in &wt.prohibited_attributes {
            if !attr_absent(instance, attribute) {
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
