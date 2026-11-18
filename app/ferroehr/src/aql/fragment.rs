// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The fragment path of a data leaf: the residual path into a node's
//! canonical fragment, written as SQL/JSON path text for a `WHERE` or
//! `ORDER BY` read, and evaluated in Rust for a projected cell.
//!
//! The evaluator follows the lax-mode semantics PostgreSQL gives the text
//! (PostgreSQL 18 docs §9.16.2.1 "Strict and Lax Modes", §9.16.2.2 "Regular
//! Expressions" aside): a member accessor unwraps one array level and skips
//! an item with no such member, `[*]` treats a non-array as a one-element
//! array, a filter unwraps one array level and keeps an item only when every
//! condition is true, and a comparison is true when some item of its operand
//! compares true with the variable. Items of different JSON types compare
//! unknown, except that `!=` with a `null` on one side is true. A
//! differential test holds the evaluator to `jsonb_path_query` on every path
//! the emitter writes. No openEHR spec governs the lowering: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result cells are arbitrary \
              projections of a stored fragment by specification (QUERY 1.1)"
)]

use std::cmp::Ordering;
use std::fmt::Write as _;

use openehr_query::lexer::CompOp;
use serde_json::Value;

use crate::storage::header::jsonb_key_order;

/// A fragment path: one step per attribute below the anchor node.
#[derive(Debug, Clone, PartialEq)]
pub struct FragmentPath {
    /// The steps, root first.
    pub(crate) steps: Vec<PathStep>,
}

/// One attribute step of a [`FragmentPath`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PathStep {
    /// The member name, a lexer-restricted identifier.
    pub(crate) member: String,
    /// Whether the attribute is a container, whose elements the step names.
    pub(crate) multi: bool,
    /// The filter conditions on the step, all of which must hold.
    pub(crate) filter: Vec<Condition>,
}

/// One filter condition: `@.<path> <op> <value>`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Condition {
    /// The member path from the filtered item.
    pub(crate) path: Vec<String>,
    /// The comparison.
    pub(crate) op: CompOp,
    /// The compared JSON scalar.
    pub(crate) value: Value,
}

impl FragmentPath {
    /// The SQL/JSON path text and its `vars` object: every compared value
    /// is a variable (`$p0`, `$p1`, …), never path text.
    #[must_use]
    pub fn sql_text(&self) -> (String, serde_json::Map<String, Value>) {
        let mut jp = String::from("$");
        let mut vars = serde_json::Map::new();
        for step in &self.steps {
            let _ = write!(jp, ".{}", step.member);
            if step.multi {
                jp.push_str("[*]");
            }
            if step.filter.is_empty() {
                continue;
            }
            let mut conds = Vec::with_capacity(step.filter.len());
            for cond in &step.filter {
                let name = format!("p{}", vars.len());
                vars.insert(name.clone(), cond.value.clone());
                let mut lhs = String::from("@");
                for part in &cond.path {
                    let _ = write!(lhs, ".{part}");
                }
                conds.push(format!("{lhs} {} ${name}", op_text(cond.op)));
            }
            let _ = write!(jp, " ? ({})", conds.join(" && "));
        }
        (jp, vars)
    }

    /// Every item the path selects in `root`, in document order.
    #[must_use]
    pub fn all<'a>(&self, root: &'a Value) -> Vec<&'a Value> {
        let mut items = vec![root];
        for step in &self.steps {
            let mut next = Vec::new();
            for item in items {
                member(item, &step.member, &mut next);
            }
            if step.multi {
                next = next.into_iter().flat_map(elements).collect();
            }
            if !step.filter.is_empty() {
                next = next
                    .into_iter()
                    .flat_map(elements)
                    .filter(|item| step.filter.iter().all(|cond| cond.holds(item)))
                    .collect();
            }
            items = next;
            if items.is_empty() {
                break;
            }
        }
        items
    }

    /// The first item the path selects in `root`, as
    /// `jsonb_path_query_first` answers it.
    #[must_use]
    pub fn first<'a>(&self, root: &'a Value) -> Option<&'a Value> {
        self.all(root).into_iter().next()
    }
}

impl Condition {
    /// Whether some item of the operand compares true with the value.
    fn holds(&self, item: &Value) -> bool {
        let mut operand = vec![item];
        for part in &self.path {
            let mut next = Vec::new();
            for item in operand {
                member(item, part, &mut next);
            }
            operand = next;
        }
        operand
            .into_iter()
            .flat_map(elements)
            .any(|left| compare(left, &self.value, self.op))
    }
}

/// Lax member access: an object's member, or each array element's.
fn member<'a>(item: &'a Value, name: &str, out: &mut Vec<&'a Value>) {
    match item {
        Value::Object(map) => out.extend(map.get(name)),
        Value::Array(array) => out.extend(array.iter().filter_map(|e| e.as_object()?.get(name))),
        _ => {}
    }
}

/// Lax `[*]`: an array's elements, or the item itself.
fn elements(item: &Value) -> Vec<&Value> {
    match item {
        Value::Array(array) => array.iter().collect(),
        other => vec![other],
    }
}

/// A SQL/JSON path comparison: `true` only when the two items are of one
/// JSON type and compare so, or one is `null` and the other not under `!=`.
fn compare(left: &Value, right: &Value, op: CompOp) -> bool {
    let ordering = match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) | (_, Value::Null) => return op == CompOp::Ne,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => match number_order(a, b) {
            Some(ordering) => ordering,
            None => return false,
        },
        (Value::String(a), Value::String(b)) => a.as_bytes().cmp(b.as_bytes()),
        _ => return false,
    };
    match op {
        CompOp::Eq => ordering.is_eq(),
        CompOp::Ne => ordering.is_ne(),
        CompOp::Lt => ordering.is_lt(),
        CompOp::Le => ordering.is_le(),
        CompOp::Gt => ordering.is_gt(),
        CompOp::Ge => ordering.is_ge(),
    }
}

/// The numeric order of two JSON numbers: exact for two integers, by their
/// nearest doubles otherwise.
fn number_order(a: &serde_json::Number, b: &serde_json::Number) -> Option<Ordering> {
    if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
        return Some(a.cmp(&b));
    }
    if let (Some(a), Some(b)) = (a.as_u64(), b.as_u64()) {
        return Some(a.cmp(&b));
    }
    a.as_f64()?.partial_cmp(&b.as_f64()?)
}

/// The SQL/JSON path comparison operator of an AQL comparison operator
/// (PostgreSQL docs §9.16.2).
fn op_text(op: CompOp) -> &'static str {
    match op {
        CompOp::Eq => "==",
        CompOp::Ne => "!=",
        CompOp::Lt => "<",
        CompOp::Le => "<=",
        CompOp::Gt => ">",
        CompOp::Ge => ">=",
    }
}

/// Writes `value` as the JSON text a jsonb column of it was served as.
///
/// Every object's keys come in jsonb order (by length, then bytewise), and
/// numbers and strings as `serde_json` writes them.
pub fn write_jsonb(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            out.push('{');
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if !entries.is_sorted_by(|a, b| jsonb_key_order(a.0, b.0).is_le()) {
                entries.sort_by(|a, b| jsonb_key_order(a.0, b.0));
            }
            for (i, (key, member)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_scalar(&Value::String(key.clone()), out);
                out.push(':');
                write_jsonb(member, out);
            }
            out.push('}');
        }
        Value::Array(array) => {
            out.push('[');
            for (i, element) in array.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_jsonb(element, out);
            }
            out.push(']');
        }
        scalar => write_scalar(scalar, out),
    }
}

/// Writes a JSON scalar as `serde_json` does.
fn write_scalar(value: &Value, out: &mut String) {
    // NOTE: serde_json::to_string of a scalar cannot fail, so an empty write is unreachable.
    out.push_str(&serde_json::to_string(value).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn path(steps: &[(&str, bool)]) -> FragmentPath {
        FragmentPath {
            steps: steps
                .iter()
                .map(|(member, multi)| PathStep {
                    member: (*member).to_owned(),
                    multi: *multi,
                    filter: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn member_access_unwraps_one_array_level() {
        let doc = json!({"a": [{"b": 1}, {"b": 2}, [{"b": 3}], 4]});
        let p = path(&[("a", false), ("b", false)]);
        assert_eq!(p.all(&doc), vec![&json!(1), &json!(2)]);
    }

    #[test]
    fn any_array_wraps_a_scalar() {
        let doc = json!({"a": 5});
        assert_eq!(path(&[("a", true)]).all(&doc), vec![&json!(5)]);
    }

    #[test]
    fn a_filter_keeps_only_true_items() {
        let doc = json!({"links": [
            {"type": {"value": "x"}, "target": {"value": "1"}},
            {"type": {"value": "y"}, "target": {"value": "2"}},
            {"type": {"value": 7}, "target": {"value": "3"}}
        ]});
        let mut p = path(&[("links", true), ("target", false), ("value", false)]);
        p.steps[0].filter.push(Condition {
            path: vec!["type".to_owned(), "value".to_owned()],
            op: CompOp::Ne,
            value: json!("x"),
        });
        assert_eq!(p.all(&doc), vec![&json!("2")]);
        let (text, vars) = p.sql_text();
        assert_eq!(text, "$.links[*] ? (@.type.value != $p0).target.value");
        assert_eq!(vars.get("p0"), Some(&json!("x")));
    }

    #[test]
    fn null_compares_unequal_to_everything_else() {
        assert!(compare(&json!(null), &json!("a"), CompOp::Ne));
        assert!(!compare(&json!(null), &json!("a"), CompOp::Eq));
        assert!(!compare(&json!({"a": 1}), &json!("a"), CompOp::Ne));
        assert!(compare(&json!(1.5), &json!(1), CompOp::Gt));
    }

    #[test]
    fn objects_are_written_in_jsonb_key_order() {
        let mut out = String::new();
        write_jsonb(
            &json!({"value": 1, "_type": "X", "a": [{"bb": 1, "c": 2}]}),
            &mut out,
        );
        assert_eq!(out, r#"{"a":[{"c":2,"bb":1}],"_type":"X","value":1}"#);
    }
}
