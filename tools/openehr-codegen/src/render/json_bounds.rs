// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-JSON reader expression of an optional container, which
//! refuses a present-but-empty array where the ITS-JSON schema states
//! `minItems: 1` and the BMM does not ([`crate::plan::its_json_bounds`]).

use super::emit_json::{JsonSchema, support_path};
use crate::plan::JsonField;

/// The expression that turns optional-container slot `i` into the field value.
pub(crate) fn optional_final(f: &JsonField, i: usize, schema: &JsonSchema<'_>) -> String {
    if !f.refuses_empty {
        return format!("__s{i}");
    }
    let krate = schema.root.split("::").next().unwrap_or(schema.root);
    let support = support_path(krate);
    format!("{support}::refuse_empty_list(__s{i}, {:?})?", f.wire_name)
}
