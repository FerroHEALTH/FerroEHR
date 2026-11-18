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
        let mut out = Vec::new();
        self.walk(root, 0, &mut |found| {
            out.push(found);
            false
        });
        out
    }

    /// The first item the path selects in `root`, as
    /// `jsonb_path_query_first` answers it.
    #[must_use]
    pub fn first<'a>(&self, root: &'a Value) -> Option<&'a Value> {
        let mut first = None;
        self.walk(root, 0, &mut |found| {
            first = Some(found);
            true
        });
        first
    }

    /// Whether the path is exactly the plain member steps `names`, with no
    /// list step and no filter.
    #[must_use]
    pub fn is_exactly(&self, names: &[&str]) -> bool {
        self.steps.len() == names.len()
            && self
                .steps
                .iter()
                .zip(names)
                .all(|(step, name)| step.member == *name && !step.multi && step.filter.is_empty())
    }

    /// The member the path's first step reads, `None` for the bare path.
    #[must_use]
    pub fn first_member(&self) -> Option<&str> {
        self.steps.first().map(|step| step.member.as_str())
    }

    /// Every item the path selects in a root whose first-step member is
    /// `reached` and that has no other member, in document order: what
    /// [`Self::all`] answers when only that member can matter.
    #[must_use]
    pub fn all_below<'a>(&self, reached: &'a Value) -> Vec<&'a Value> {
        let mut out = Vec::new();
        self.walk_reached(reached, 0, &mut |found| {
            out.push(found);
            false
        });
        out
    }

    /// The first item of [`Self::all_below`].
    #[must_use]
    pub fn first_below<'a>(&self, reached: &'a Value) -> Option<&'a Value> {
        let mut first = None;
        self.walk_reached(reached, 0, &mut |found| {
            first = Some(found);
            true
        });
        first
    }

    /// Visits every item steps `at..` select from `item` in document order,
    /// depth first, which is the order PostgreSQL's sequence of items takes;
    /// stops, and returns `true`, once `found` does.
    fn walk<'a>(
        &self,
        item: &'a Value,
        at: usize,
        found: &mut dyn FnMut(&'a Value) -> bool,
    ) -> bool {
        let Some(step) = self.steps.get(at) else {
            return found(item);
        };
        member(item, &step.member).any(|reached| self.walk_reached(reached, at, found))
    }

    /// [`Self::walk`] from a value step `at`'s member access reached.
    fn walk_reached<'a>(
        &self,
        reached: &'a Value,
        at: usize,
        found: &mut dyn FnMut(&'a Value) -> bool,
    ) -> bool {
        let Some(step) = self.steps.get(at) else {
            return found(reached);
        };
        let mut visit = |candidate: &'a Value| -> bool {
            if step.filter.is_empty() {
                return self.walk(candidate, at + 1, found);
            }
            elements(candidate).any(|next| {
                step.filter.iter().all(|cond| cond.holds(next, 0)) && self.walk(next, at + 1, found)
            })
        };
        if step.multi {
            elements(reached).any(&mut visit)
        } else {
            visit(reached)
        }
    }
}

impl Condition {
    /// Whether some item the operand path selects from `item`, from part
    /// `at` on, compares true with the value.
    fn holds(&self, item: &Value, at: usize) -> bool {
        match self.path.get(at) {
            None => elements(item).any(|left| compare(left, &self.value, self.op)),
            Some(part) => member(item, part).any(|next| self.holds(next, at + 1)),
        }
    }
}

/// Lax member access: an object's member, or each array element's.
fn member<'a, 'n>(item: &'a Value, name: &'n str) -> impl Iterator<Item = &'a Value> + use<'a, 'n> {
    let (one, many): (Option<&Value>, &[Value]) = match item {
        Value::Object(map) => (map.get(name), &[]),
        Value::Array(array) => (None, array.as_slice()),
        _ => (None, &[]),
    };
    one.into_iter()
        .chain(many.iter().filter_map(move |e| e.as_object()?.get(name)))
}

/// Lax `[*]`: an array's elements, or the item itself.
fn elements(item: &Value) -> impl Iterator<Item = &Value> {
    let (one, many): (Option<&Value>, &[Value]) = match item {
        Value::Array(array) => (None, array.as_slice()),
        other => (Some(other), &[]),
    };
    one.into_iter().chain(many.iter())
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
                write_string(key, out);
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
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            let _ = write!(out, "{number}");
        }
        Value::String(text) => write_string(text, out),
        // NOTE: write_jsonb writes arrays and objects itself, so no other value reaches here.
        Value::Array(_) | Value::Object(_) => {}
    }
}

/// Writes `text` as a JSON string, escaped as `serde_json` escapes it: `"`,
/// `\\` and the control characters, the short forms where JSON has them.
pub(crate) fn write_string(text: &str, out: &mut String) {
    out.reserve(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
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
    fn scalars_are_written_as_serde_json_writes_them() {
        for value in [
            json!("plain"),
            json!(
                "quote \" slash \\ / tab \t nl \n cr \r bs \u{8} ff \u{c} nul \u{0} esc \u{1b} é ✓"
            ),
            json!(1.5),
            json!(-0.0),
            json!(1e300),
            json!(12_345_678_901_234_u64),
            json!(-7),
            json!(true),
            json!(null),
        ] {
            let mut out = String::new();
            write_jsonb(&value, &mut out);
            assert_eq!(out, serde_json::to_string(&value).unwrap(), "{value:?}");
        }
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
