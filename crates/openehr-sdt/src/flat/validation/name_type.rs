// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM type of a node's `name` against the template's constraint on it.

use crate::flat::instance::Instance;

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
        }
    }
}
