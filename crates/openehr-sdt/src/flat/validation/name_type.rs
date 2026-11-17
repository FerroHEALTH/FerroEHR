// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM type of a node's `name` against the template's constraint on it.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::Value;

use super::{Validator, subtype};
use crate::flat::webtemplate::model::WebTemplateNode;
use crate::rm_instance::ValidationKind;

impl Validator {
    /// Refuses a `name` whose RM type does not conform to the type the
    /// template constrains it to, such as a `DV_TEXT` where the template asks
    /// for a `DV_CODED_TEXT` (AOM 1.4 `master04-constraint_model_package.adoc`:
    /// a `C_COMPLEX_OBJECT` constrains an object of its `rm_type_name`, and
    /// `valid_value` tests the object for conformance). `holder` is the
    /// LOCATABLE the name belongs to, at `path`.
    pub(super) fn check_name_type(&mut self, holder: &Value, wt: &WebTemplateNode, path: &str) {
        let Some(want) = wt.name_rm_type.as_deref() else {
            return;
        };
        let Some(found) = holder
            .get("name")
            .and_then(|n| n.get("_type"))
            .and_then(Value::as_str)
            .or_else(|| holder.get("name").map(|_| "DV_TEXT"))
        else {
            return;
        };
        if !subtype::conforms(found, want) {
            self.push(
                format!("{path}/name"),
                format!("expected RM type conforming to {want} but found {found}"),
                ValidationKind::WrongType,
            );
        }
    }
}
