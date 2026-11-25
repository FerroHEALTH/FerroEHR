// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A node's `name` against the template's constraint on it: its RM type and,
//! for a coded name, its code list.

use crate::flat::instance::Instance;

use super::leaf::check_code_membership;
use super::{Validator, subtype};
use crate::flat::webtemplate::model::WebTemplateNode;
use crate::rm_instance::ValidationKind;

impl Validator {
    /// Refuses a `name` whose RM type does not conform to the type the template
    /// constrains it to (AOM 1.4 `master04-constraint_model_package.adoc`
    /// `valid_value`). `holder` is the LOCATABLE at `path`.
    pub(super) fn check_name_type<'i, I: Instance<'i>>(
        &mut self,
        holder: I,
        wt: &WebTemplateNode,
        path: &str,
    ) {
        let Some(want) = wt.name_rm_type.as_deref() else {
            return;
        };
        let Some(found) = holder
            .attr("name")
            .and_then(Instance::rm_type)
            .or_else(|| holder.attr("name").map(|_| "DV_TEXT"))
        else {
            return;
        };
        if !subtype::conforms(found, want) {
            self.push(
                format!("{path}/name"),
                format!("expected RM type conforming to {want} but found {found}"),
                ValidationKind::WrongType,
            );
            return;
        }
        if let (Some(input), Some(name)) = (wt.name_code_input.as_deref(), holder.attr("name")) {
            let code = name.attr("defining_code");
            let code_string = code
                .and_then(|c| c.attr("code_string"))
                .and_then(Instance::as_str);
            let terminology = code
                .and_then(|c| c.attr("terminology_id"))
                .and_then(|t| t.attr("value"))
                .and_then(Instance::as_str);
            check_code_membership(
                self,
                &format!("{path}/name"),
                false,
                input,
                (code_string, terminology),
            );
        }
    }
}
