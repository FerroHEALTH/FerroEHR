// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The function signatures of a BMM class (`BMM_CLASS.functions`), read from
//! their persisted form (LANG `org.openehr.lang.bmm_persistence.p_bmm_function`:
//! `parameters`, `result`, `is_nullable`).
//!
//! The BMM carries a function's signature and no body. Its name is what a
//! hand-written behaviour sibling must realize, per generation; a function with
//! no parameters is also a computed attribute of its class (AOM2 `master04.2`
//! §Attribute Nodes: "computed attributes (represented by functions with no
//! arguments in most programming languages)").

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM bundles) — not the application (#1694)"
)]
use serde_json::{Map, Value};

use super::bmm::{BmmPropKind, parse_container_item, parse_type_def};

/// One `BMM_CLASS.functions` entry.
#[derive(Debug, Clone)]
pub(crate) struct BmmFunction {
    /// The function name, verbatim (`is_integral`).
    pub name: String,
    /// The number of formal `parameters`; zero makes the function a computed
    /// attribute.
    pub parameter_count: usize,
    /// The `result` type in the shape of a property (`Single` or `Container`,
    /// with no cardinality), or `None` for a procedure.
    pub result: Option<BmmPropKind>,
    /// `is_nullable`: whether the result may be Void (default false).
    pub is_nullable: bool,
}

/// Parse a class node's `functions` object into declaration-ordered
/// [`BmmFunction`]s (`serde_json`'s `preserve_order` keeps file order). Absent →
/// empty.
pub(super) fn parse_functions(node: &Value) -> Vec<BmmFunction> {
    node.get("functions")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(name, f)| BmmFunction {
                    name: name.clone(),
                    parameter_count: f
                        .get("parameters")
                        .and_then(Value::as_object)
                        .map_or(0, Map::len),
                    result: f.get("result").map(parse_result_type),
                    is_nullable: f
                        .get("is_nullable")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A function `result` (`P_BMM_SIMPLE_TYPE`, `P_BMM_GENERIC_TYPE` or
/// `P_BMM_CONTAINER_TYPE`) in the shape of a property.
fn parse_result_type(result: &Value) -> BmmPropKind {
    if result.get("_type").and_then(Value::as_str) == Some("P_BMM_CONTAINER_TYPE") {
        BmmPropKind::Container {
            container_type: result
                .get("container_type")
                .and_then(Value::as_str)
                .unwrap_or("List")
                .to_string(),
            item: parse_container_item(result),
            cardinality: None,
        }
    } else {
        BmmPropKind::Single(parse_type_def(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::bmm::BmmType;

    #[test]
    fn a_function_keeps_its_arity_result_and_nullability() {
        let node: Value = serde_json::from_str(
            r#"{"functions": {
                "is_integral": {"name": "is_integral",
                    "result": {"_type": "P_BMM_SIMPLE_TYPE", "type": "Boolean"}},
                "names": {"name": "names", "is_nullable": true,
                    "result": {"_type": "P_BMM_CONTAINER_TYPE", "container_type": "List", "type": "DV_TEXT"}},
                "add": {"name": "add",
                    "parameters": {"other": {"_type": "P_BMM_SINGLE_FUNCTION_PARAMETER", "name": "other", "type": "DV_QUANTITY"}},
                    "result": {"_type": "P_BMM_SIMPLE_TYPE", "type": "DV_QUANTITY"}}
            }}"#,
        )
        .expect("test JSON");
        let functions = parse_functions(&node);
        let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["is_integral", "names", "add"]);
        let [is_integral, list, add] = functions.as_slice() else {
            panic!("three functions");
        };
        assert_eq!(is_integral.parameter_count, 0);
        assert!(!is_integral.is_nullable);
        assert!(matches!(
            &is_integral.result,
            Some(BmmPropKind::Single(BmmType::Simple(t))) if t == "Boolean"
        ));
        assert!(list.is_nullable);
        assert!(matches!(
            &list.result,
            Some(BmmPropKind::Container { container_type, item: BmmType::Simple(t), cardinality: None })
                if container_type == "List" && t == "DV_TEXT"
        ));
        assert_eq!(add.parameter_count, 1);
    }
}
