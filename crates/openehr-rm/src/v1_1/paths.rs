// @generated-from-template templates/openehr-rm/paths.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! openEHR path navigation over the canonical-JSON tree.
//!
//! The `PATHABLE` functions (RM `org.openehr.rm.common.pathable.adoc`), the
//! `LOCATABLE` node-id predicates, and the [`EhrUri`] parser, over the BASE
//! `master11-paths.adoc` grammar and the `ehr:` scheme (RM
//! `data_types/master10-uri_package` §"DV_EHR_URI Syntax").
//!
//! `parent` is the root-anchored [`parent_of`]. General comparison predicates
//! are [`Comparison`] conjuncts with XPath existential semantics; other
//! predicate text is [`PathError::UnsupportedPredicate`].
//!
//! NOTE: `//` and `[n]` belong to the master11 grammar, not to AQL (QUERY
//! `master03` §"Predicates").

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::Value;
use std::fmt;
use std::str::FromStr;

use openehr_base::v1_2::prelude::{ArchetypeId, ObjectVersionId, Uid};
use uuid::Uuid;

pub mod write;

/// Error raised when parsing an openEHR path expression.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// The path string was empty.
    #[error("empty path")]
    Empty,
    /// A path segment had no attribute name (e.g. `//`, or a leading `[`).
    #[error("path segment {0} has no attribute name")]
    MissingAttribute(usize),
    /// A `[` predicate was not terminated by `]`.
    #[error("unterminated predicate in segment {0}")]
    UnterminatedPredicate(usize),
    /// A predicate outside the supported archetype-id, name, uid, positional and
    /// comparison forms.
    #[error("unsupported predicate expression: {0:?}")]
    UnsupportedPredicate(String),
    /// A positional predicate `[n]` that is not a positive integer (master11
    /// §"Using Positional Parameters").
    #[error("invalid positional predicate {0:?} (positions are 1-based integers)")]
    InvalidPosition(String),
    /// A dangling `//` with no following attribute segment.
    #[error("path pattern ends with '//' (no following attribute)")]
    DanglingDescendant,
}

/// The predicate of one path segment, a conjunction of the master11
/// §"Predicate Expressions" shortcuts: archetype node id, name, uid, the
/// 1-based position `[n]` on the container, and general comparisons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Predicate {
    /// `archetype_node_id` (or, at archetype roots, archetype id).
    pub archetype_node_id: Option<String>,
    /// `name/value`.
    pub name_value: Option<String>,
    /// `LOCATABLE.uid.value` (a `UID_BASED_ID`).
    pub uid: Option<String>,
    /// 1-based positional index into the container attribute.
    pub position: Option<usize>,
    /// General attribute comparisons (§"Other Predicates"), ANDed with the
    /// conjuncts above.
    pub comparisons: Vec<Comparison>,
}

/// A comparison operator in a general predicate conjunct (BASE
/// `master11-paths` §"Other Predicates").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl CmpOp {
    /// The operator's path-syntax token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    /// Apply the operator to a total ordering.
    fn holds(self, ord: std::cmp::Ordering) -> bool {
        match self {
            CmpOp::Eq => ord.is_eq(),
            CmpOp::Ne => ord.is_ne(),
            CmpOp::Lt => ord.is_lt(),
            CmpOp::Le => ord.is_le(),
            CmpOp::Gt => ord.is_gt(),
            CmpOp::Ge => ord.is_ge(),
        }
    }
}

/// The literal of a comparison: a quoted string, compared byte-lexically, or
/// a number, compared numerically. master11 defines only the syntax: our own
/// design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmpLiteral {
    /// A `'…'` string literal.
    Str(String),
    /// An unquoted numeric literal (stored as written; compared as `f64`).
    Num(String),
}

/// One comparison conjunct (master11 §"Other Predicates"), true when any node
/// the relative path selects satisfies it; a scalar wrapper compares by its
/// `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// The relative attribute path (plain attribute names, no predicates).
    pub path: Vec<String>,
    /// The comparison operator.
    pub op: CmpOp,
    /// The right-hand literal.
    pub value: CmpLiteral,
}

impl Comparison {
    /// Whether any node selected by `path` from `node` satisfies the
    /// comparison (XPath existential semantics).
    fn matches(&self, node: &Value) -> bool {
        let mut leaves = Vec::new();
        collect_leaves(node, &self.path, &mut leaves);
        leaves.iter().any(|leaf| self.satisfied_by(leaf))
    }

    /// Whether one leaf satisfies the comparison, an object by its `value`.
    fn satisfied_by(&self, leaf: &Value) -> bool {
        let leaf = match leaf {
            Value::Object(o) => match o.get("value") {
                Some(v) => v,
                None => return false,
            },
            other => other,
        };
        self.satisfied_by_scalar(leaf.as_str(), leaf.as_f64())
    }

    /// Whether a scalar leaf satisfies the comparison, as string or number by
    /// the literal's kind.
    #[must_use]
    pub fn satisfied_by_scalar(&self, as_str: Option<&str>, as_f64: Option<f64>) -> bool {
        match &self.value {
            CmpLiteral::Str(want) => {
                as_str.is_some_and(|have| self.op.holds(have.cmp(want.as_str())))
            }
            CmpLiteral::Num(raw) => match (as_f64, raw.parse::<f64>()) {
                (Some(have), Ok(want)) => have
                    .partial_cmp(&want)
                    .is_some_and(|ord| self.op.holds(ord)),
                _ => false,
            },
        }
    }
}

/// Collects every node the relative path selects, through containers.
fn collect_leaves<'a>(node: &'a Value, path: &[String], out: &mut Vec<&'a Value>) {
    let Some((first, rest)) = path.split_first() else {
        out.push(node);
        return;
    };
    let Some(child) = node.get(first) else {
        return;
    };
    match child {
        Value::Array(items) => {
            for item in items {
                collect_leaves(item, rest, out);
            }
        }
        other => collect_leaves(other, rest, out),
    }
}

impl Predicate {
    /// Whether this predicate constrains nothing (matches every node).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.archetype_node_id.is_none()
            && self.name_value.is_none()
            && self.uid.is_none()
            && self.position.is_none()
            && self.comparisons.is_empty()
    }

    /// Whether a node satisfies the attribute conjuncts (`archetype_node_id`,
    /// `name/value`, `uid`), exact and ANDed; the position is applied in `step`.
    #[must_use]
    pub fn matches(&self, node: &Value) -> bool {
        if let Some(id) = self.archetype_node_id.as_deref()
            && node.get("archetype_node_id").and_then(Value::as_str) != Some(id)
        {
            return false;
        }
        if let Some(name) = self.name_value.as_deref()
            && node
                .get("name")
                .and_then(|n| n.get("value"))
                .and_then(Value::as_str)
                != Some(name)
        {
            return false;
        }
        if let Some(uid) = self.uid.as_deref()
            && node_uid(node) != Some(uid)
        {
            return false;
        }
        self.comparisons.iter().all(|c| c.matches(node))
    }
}

/// A node's `uid.value`, from the object form or a bare string.
fn node_uid(node: &Value) -> Option<&str> {
    match node.get("uid") {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(u) => u.get("value").and_then(Value::as_str),
        None => None,
    }
}

/// One `attribute[predicate]` segment of an openEHR path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSegment {
    /// The RM attribute name (`content`, `data`, `events`, `items`, …).
    pub attribute: String,
    /// The optional `[...]` predicate.
    pub predicate: Predicate,
    /// Whether this segment follows `//` and matches at any depth (master11
    /// §"Basic Syntax").
    pub descendant: bool,
}

/// A parsed openEHR path: `/`-separated [`PathSegment`]s, absolute (leading
/// `/`) or relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmPath {
    /// Whether the path is absolute; navigation from a root treats both alike.
    pub absolute: bool,
    /// The segments in order.
    pub segments: Vec<PathSegment>,
}

impl FromStr for RmPath {
    type Err = PathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(PathError::Empty);
        }
        let (absolute, body) = match s.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        // Split on top-level `/`; an empty segment marks `//`.
        let raw = split_top_level(body).map_err(PathError::UnterminatedPredicate)?;
        let mut segments: Vec<PathSegment> = Vec::new();
        let mut pending_descendant = false;
        let mut index = 0usize;
        for seg in raw {
            if seg.is_empty() {
                // `//` (or a leading `/` already consumed by `absolute`):
                // the next real segment matches at any depth.
                pending_descendant = true;
                continue;
            }
            let mut parsed = parse_segment(seg, index)?;
            parsed.descendant = pending_descendant;
            pending_descendant = false;
            segments.push(parsed);
            index += 1;
        }
        if pending_descendant {
            return Err(PathError::DanglingDescendant);
        }
        if segments.is_empty() && !absolute {
            return Err(PathError::MissingAttribute(0));
        }
        Ok(Self { absolute, segments })
    }
}

/// Splits a path body on top-level `/`, `//` as empty segments; an
/// unterminated `[` gives `Err(index)` of its segment.
fn split_top_level(body: &str) -> Result<Vec<&str>, usize> {
    let bytes = body.as_bytes();
    let (mut i, n) = (0, bytes.len());
    let mut out = Vec::new();
    while i <= n {
        let start = i;
        let mut depth = 0usize;
        while let Some(&b) = bytes.get(i) {
            match b {
                b'[' => depth += 1,
                b']' => depth = depth.saturating_sub(1),
                b'/' if depth == 0 => break,
                _ => {}
            }
            i += 1;
        }
        if depth > 0 {
            return Err(out.len());
        }
        // `start..i` walks whole ASCII bytes of `body`, so both ends are UTF-8
        // boundaries; `get` keeps that a fact rather than an assumption.
        out.push(body.get(start..i).unwrap_or_default());
        if i == n {
            break;
        }
        i += 1; // skip the '/'
    }
    Ok(out)
}

impl fmt::Display for RmPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.absolute && self.segments.is_empty() {
            return f.write_str("/");
        }
        for (i, seg) in self.segments.iter().enumerate() {
            // A descendant segment is prefixed with `//`; the first segment of
            // an absolute path with `//` already carries it, so no extra `/`.
            if seg.descendant {
                f.write_str("//")?;
            } else if self.absolute || i > 0 {
                f.write_str("/")?;
            }
            f.write_str(&seg.attribute)?;
            seg.predicate.render(f)?;
        }
        Ok(())
    }
}

impl Predicate {
    /// Renders this predicate in its normalised bracketed form, nothing when
    /// empty.
    fn render(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return Ok(());
        }
        // A positional predicate is the guaranteed-unique form and stands alone
        // (BASE `master11-paths` §"Using Positional Parameters").
        if let Some(pos) = self.position {
            return write!(f, "[{pos}]");
        }
        f.write_str("[")?;
        let mut wrote = false;
        if let Some(id) = &self.archetype_node_id {
            f.write_str(id)?;
            wrote = true;
        }
        if let Some(name) = &self.name_value {
            // `[id,'name']` beside an archetype id, else `name/value='name'`.
            if wrote {
                write!(f, ",'{name}'")?;
            } else {
                write!(f, "name/value='{name}'")?;
            }
            wrote = true;
        }
        if let Some(uid) = &self.uid {
            if wrote {
                f.write_str(" and ")?;
            }
            write!(f, "uid='{uid}'")?;
            wrote = true;
        }
        for cmp in &self.comparisons {
            if wrote {
                f.write_str(" and ")?;
            }
            cmp.render(f)?;
            wrote = true;
        }
        f.write_str("]")
    }
}

impl Comparison {
    /// Render one comparison term of a predicate (`path op literal`).
    fn render(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.join("/"))?;
        match &self.value {
            CmpLiteral::Str(s) => write!(f, " {} '{s}'", self.op.token()),
            CmpLiteral::Num(n) => write!(f, " {} {n}", self.op.token()),
        }
    }
}

/// Parse one `attribute[predicate]` segment.
fn parse_segment(seg: &str, index: usize) -> Result<PathSegment, PathError> {
    let Some(open) = seg.find('[') else {
        return Ok(PathSegment {
            attribute: seg.to_owned(),
            predicate: Predicate::default(),
            descendant: false,
        });
    };
    let Some((attribute, bracketed)) = seg.split_at_checked(open) else {
        return Err(PathError::UnterminatedPredicate(index));
    };
    if attribute.is_empty() {
        return Err(PathError::MissingAttribute(index));
    }
    let attribute = attribute.to_owned();
    let Some(inner) = bracketed
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
    else {
        return Err(PathError::UnterminatedPredicate(index));
    };
    Ok(PathSegment {
        attribute,
        predicate: parse_predicate(inner)?,
        descendant: false,
    })
}

/// Parses the inside of a `[...]` predicate.
fn parse_predicate(inner: &str) -> Result<Predicate, PathError> {
    let mut predicate = Predicate::default();
    // `[at0001, 'standing']` — the comma shortcut binds the whole remainder.
    let (head, comma_name) = match inner.split_once(',') {
        Some((h, rest)) => (h.trim(), Some(rest.trim())),
        None => (inner.trim(), None),
    };
    if let Some(name) = comma_name {
        let name = name
            .strip_prefix('\'')
            .and_then(|n| n.strip_suffix('\''))
            .ok_or_else(|| PathError::UnsupportedPredicate(inner.to_owned()))?;
        predicate.name_value = Some(name.to_owned());
        apply_conjunct(head, &mut predicate)?;
        return Ok(predicate);
    }
    // Split on ` and ` / ` AND ` conjunctions.
    let mut rest = head;
    while let Some((left, right)) = rest
        .split_once(" and ")
        .or_else(|| rest.split_once(" AND "))
    {
        apply_conjunct(left.trim(), &mut predicate)?;
        rest = right.trim();
    }
    apply_conjunct(rest.trim(), &mut predicate)?;
    Ok(predicate)
}

/// Interpret one predicate conjunct.
fn apply_conjunct(c: &str, predicate: &mut Predicate) -> Result<(), PathError> {
    if c.is_empty() {
        return Ok(());
    }
    // A bare positive integer is the XPath positional predicate `[n]` (1-based;
    // BASE `master11-paths` §"Using Positional Parameters").
    if c.bytes().all(|b| b.is_ascii_digit()) {
        let Ok(n) = c.parse::<usize>() else {
            return Err(PathError::InvalidPosition(c.to_owned()));
        };
        if n == 0 {
            return Err(PathError::InvalidPosition(c.to_owned()));
        }
        predicate.position = Some(n);
        return Ok(());
    }
    // Explicit name/value='...'.
    if let Some(value) = strip_eq(c, "name/value") {
        predicate.name_value = Some(unquote(value, c)?.to_owned());
        return Ok(());
    }
    // Uid-based predicate `uid='...'` / `@uid='...'` (§"Using a Uid-based
    // Predicate").
    if let Some(value) = strip_eq(c, "@uid").or_else(|| strip_eq(c, "uid")) {
        predicate.uid = Some(unquote(value, c)?.to_owned());
        return Ok(());
    }
    // Explicit @archetype_node_id='...'.
    if let Some(value) = strip_eq(c, "@archetype_node_id") {
        predicate.archetype_node_id = Some(unquote(value, c)?.to_owned());
        return Ok(());
    }
    // `[archetype_id=…]` or `[@archetype_id='…']`, matched against
    // `archetype_node_id` (§"Paths within Top-level Structures").
    if let Some(value) = strip_eq(c, "@archetype_id").or_else(|| strip_eq(c, "archetype_id")) {
        let value = value
            .strip_prefix('\'')
            .and_then(|v| v.strip_suffix('\''))
            .unwrap_or(value);
        predicate.archetype_node_id = Some(value.to_owned());
        return Ok(());
    }
    // A comparison conjunct (§"Other Predicates").
    if c.contains(['=', '<', '>']) {
        predicate.comparisons.push(parse_comparison(c)?);
        return Ok(());
    }
    // The bare shortcut: an at-code / id-code or an archetype id. Anything
    // else containing literal syntax is not a valid predicate conjunct.
    if c.contains('\'') {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    }
    // NOTE: RM `master05-directory_package.adoc` §Paths' parenthesised modifier has
    // no master11 production, so it is refused.
    if c.contains('(') || c.contains(')') {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    }
    predicate.archetype_node_id = Some(c.to_owned());
    Ok(())
}

/// Parses a comparison conjunct `<relative-path> <op> <literal>` (master11
/// §"Other Predicates").
fn parse_comparison(c: &str) -> Result<Comparison, PathError> {
    // Longest operators first so `>=`/`<=`/`!=` never parse as `>`/`<`/`=`.
    let (pos, op) = ["!=", ">=", "<=", "=", ">", "<"]
        .iter()
        .filter_map(|tok| {
            c.find(tok).map(|p| {
                (
                    p,
                    match *tok {
                        "!=" => CmpOp::Ne,
                        ">=" => CmpOp::Ge,
                        "<=" => CmpOp::Le,
                        "=" => CmpOp::Eq,
                        ">" => CmpOp::Gt,
                        _ => CmpOp::Lt,
                    },
                )
            })
        })
        .min_by_key(|(p, op)| (*p, matches!(op, CmpOp::Eq | CmpOp::Gt | CmpOp::Lt)))
        .ok_or_else(|| PathError::UnsupportedPredicate(c.to_owned()))?;
    let Some((lhs, with_op)) = c.split_at_checked(pos) else {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    };
    let Some(rhs) = with_op.get(op.token().len()..) else {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    };
    let lhs = lhs.trim();
    let rhs = rhs.trim();
    let path: Vec<String> = lhs
        .strip_prefix('@')
        .unwrap_or(lhs)
        .split('/')
        .map(str::trim)
        .map(str::to_owned)
        .collect();
    if path
        .iter()
        .any(|seg| seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    }
    let value = if let Some(s) = rhs.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        CmpLiteral::Str(s.to_owned())
    } else if rhs.parse::<f64>().is_ok() {
        CmpLiteral::Num(rhs.to_owned())
    } else {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    };
    Ok(Comparison { path, op, value })
}

/// For a conjunct `c` of the form `key = <value>` (optional spaces), return the
/// trimmed `<value>` part, or `None` if `c` does not start with `key`.
fn strip_eq<'a>(c: &'a str, key: &str) -> Option<&'a str> {
    c.strip_prefix(key)
        .map(str::trim_start)?
        .strip_prefix('=')
        .map(str::trim)
}

/// Strip the surrounding single quotes from a predicate string literal.
fn unquote<'a>(value: &'a str, conjunct: &str) -> Result<&'a str, PathError> {
    value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .ok_or_else(|| PathError::UnsupportedPredicate(conjunct.to_owned()))
}

// ── navigation over the canonical-JSON RM tree ───────────────────────────────

/// One navigation step: every child of `node` matching `segment`, `[n]`
/// selecting the nth container member (master11 §"Using Positional
/// Parameters").
fn step<'a>(node: &'a Value, segment: &PathSegment, out: &mut Vec<&'a Value>) {
    let Some(child) = node.get(&segment.attribute) else {
        return;
    };
    let predicate = &segment.predicate;
    match child {
        Value::Array(items) => match predicate.position {
            Some(pos) => {
                if let Some(v) = items.get(pos - 1)
                    && predicate.matches(v)
                {
                    out.push(v);
                }
            }
            None => out.extend(items.iter().filter(|v| predicate.matches(v))),
        },
        v => {
            // A single-valued attribute has one element at position 1.
            if predicate.position.is_some_and(|p| p != 1) {
                return;
            }
            if predicate.matches(v) {
                out.push(v);
            }
        }
    }
}

/// Descendant-or-self navigation for a `//` segment; each match is produced
/// once.
fn step_descendant<'a>(node: &'a Value, segment: &PathSegment, out: &mut Vec<&'a Value>) {
    step(node, segment, out);
    if let Value::Object(map) = node {
        for v in map.values() {
            match v {
                Value::Array(items) => {
                    for e in items {
                        if e.is_object() {
                            step_descendant(e, segment, out);
                        }
                    }
                }
                Value::Object(_) => step_descendant(v, segment, out),
                _ => {}
            }
        }
    }
}

/// Every value under `segment.attribute` on `node` that satisfies the
/// predicate, in document order, for single and container attributes alike.
#[must_use]
pub fn select_children<'a>(node: &'a Value, segment: &PathSegment) -> Vec<&'a Value> {
    let mut out = Vec::new();
    step(node, segment, &mut out);
    out
}

/// RM `PATHABLE.items_at_path`: every item the path resolves to, in document
/// order.
#[must_use]
pub fn items_at_path<'a>(root: &'a Value, path: &RmPath) -> Vec<&'a Value> {
    let mut current = vec![root];
    for segment in &path.segments {
        let mut next = Vec::new();
        for node in current {
            if segment.descendant {
                step_descendant(node, segment, &mut next);
            } else {
                step(node, segment, &mut next);
            }
        }
        if next.is_empty() {
            return Vec::new();
        }
        current = next;
    }
    current
}

/// RM `PATHABLE.item_at_path`: the first item the path resolves to, or
/// `None`.
#[must_use]
pub fn item_at_path<'a>(root: &'a Value, path: &RmPath) -> Option<&'a Value> {
    items_at_path(root, path).into_iter().next()
}

/// RM `PATHABLE.path_exists`: `true` if the path resolves to at least one
/// item relative to `root`.
#[must_use]
pub fn path_exists(root: &Value, path: &RmPath) -> bool {
    !items_at_path(root, path).is_empty()
}

/// RM `PATHABLE.path_unique`: `true` if the path resolves to exactly one item
/// relative to `root`.
#[must_use]
pub fn path_unique(root: &Value, path: &RmPath) -> bool {
    items_at_path(root, path).len() == 1
}

/// RM `PATHABLE.path_of_item`: the path of `item` inside `root`, by pointer
/// identity, with the predicates that make it selective.
#[must_use]
pub fn path_of_item(root: &Value, item: &Value) -> Option<String> {
    if std::ptr::eq(root, item) {
        return Some("/".to_owned());
    }
    let mut segments = Vec::new();
    if find_path(root, item, &mut segments) {
        let path = RmPath {
            absolute: true,
            segments,
        };
        Some(path.to_string())
    } else {
        None
    }
}

/// RM `PATHABLE.parent`: the nearest enclosing object of `item` in `root`,
/// or `None` for the root or a node outside it.
#[must_use]
pub fn parent_of<'a>(root: &'a Value, item: &Value) -> Option<&'a Value> {
    if std::ptr::eq(root, item) {
        return None;
    }
    walk_for_parent(root, item)
}

/// Recursive worker for [`parent_of`]: the enclosing JSON *object* is the RM
/// parent; array elements report the object holding the array.
fn walk_for_parent<'a>(node: &'a Value, item: &Value) -> Option<&'a Value> {
    match node {
        Value::Object(map) => map
            .values()
            .find_map(|child| parent_through_member(node, child, item)),
        Value::Array(items) => items.iter().find_map(|v| walk_for_parent(v, item)),
        _ => None,
    }
}

/// The parent reached through one member of `owner`.
fn parent_through_member<'a>(
    owner: &'a Value,
    child: &'a Value,
    item: &Value,
) -> Option<&'a Value> {
    if std::ptr::eq(child, item) {
        return Some(owner);
    }
    let Value::Array(items) = child else {
        return walk_for_parent(child, item);
    };
    if items.iter().any(|v| std::ptr::eq(v, item)) {
        return Some(owner);
    }
    items.iter().find_map(|v| walk_for_parent(v, item))
}

/// Depth-first search for `item` (pointer identity), accumulating the
/// attribute segments from `node` down to it.
fn find_path(node: &Value, item: &Value, segments: &mut Vec<PathSegment>) -> bool {
    let Value::Object(map) = node else {
        return false;
    };
    for (attr, child) in map {
        let make_segment = |v: &Value| PathSegment {
            attribute: attr.clone(),
            predicate: predicate_for(v),
            descendant: false,
        };
        match child {
            Value::Array(items) => {
                for v in items {
                    segments.push(make_segment(v));
                    if std::ptr::eq(v, item) || find_path(v, item, segments) {
                        return true;
                    }
                    segments.pop();
                }
            }
            v => {
                segments.push(PathSegment {
                    attribute: attr.clone(),
                    predicate: Predicate::default(),
                    descendant: false,
                });
                if std::ptr::eq(v, item) || find_path(v, item, segments) {
                    return true;
                }
                segments.pop();
            }
        }
    }
    false
}

/// The identifying predicate for a container element: its
/// `archetype_node_id` and `name/value`, when present.
fn predicate_for(v: &Value) -> Predicate {
    Predicate {
        archetype_node_id: v
            .get("archetype_node_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        name_value: v
            .get("name")
            .and_then(|n| n.get("value"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        uid: None,
        position: None,
        comparisons: Vec::new(),
    }
}

// Typed entry points: a typed value is encoded to canonical JSON first.

/// Failure to answer a path question asked of a TYPED RM value.
#[derive(Debug, thiserror::Error)]
pub enum TypedPathError {
    /// The path expression is not a well-formed openEHR path.
    #[error("path expression")]
    Path(#[source] PathError),
    /// The value could not be encoded as the canonical JSON the navigation
    /// runs over.
    #[error("encoding the value as canonical JSON")]
    Encode(#[source] serde_json::Error),
}

impl From<PathError> for TypedPathError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

/// The canonical-JSON form of a typed RM value.
///
/// # Errors
/// Returns [`TypedPathError::Encode`] for a non-finite `Real`.
pub fn canonical_value<T: serde::Serialize>(value: &T) -> Result<Value, TypedPathError> {
    serde_json::to_value(value).map_err(TypedPathError::Encode)
}

// ── LOCATABLE node-id forms ──────────────────────────────────────────────────

/// Whether `node_id` is an ADL term code (`at0005`, `at0002.1`, `id3`),
/// the interior form of `LOCATABLE.archetype_node_id` (`locatable.adoc`
/// §Attributes; AM `ADL2` `master02-overview.adoc` §"ADL 2.4").
#[must_use]
pub fn archetype_node_id_is_term_code(node_id: &str) -> bool {
    let Some(digits) = node_id
        .strip_prefix("at")
        .or_else(|| node_id.strip_prefix("id"))
    else {
        return false;
    };
    !digits.is_empty()
        && digits
            .split('.')
            .all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()))
}

/// RM `LOCATABLE.is_archetype_root()` from `archetype_node_id` alone.
///
/// An `ARCHETYPE_ID`-form node id is a root ("At an archetype root point, the
/// value of this attribute is always the stringified form of the
/// `archetype_id`", `locatable.adoc` §Attributes).
///
/// NOTE: `locatable.adoc` §Functions leaves the function undefined; the
/// `archetype_details` reading is the caller's to test.
#[must_use]
pub fn is_archetype_root_node_id(node_id: &str) -> bool {
    !archetype_node_id_is_term_code(node_id) && node_id.parse::<ArchetypeId>().is_ok()
}

// ── EHR URIs (the `ehr:` scheme) ─────────────────────────────────────────────

/// The literal `ehr` URI scheme (RM `data_types/master10-uri_package`
/// §Definitions: `Ehr_scheme: String = "ehr"`).
pub const EHR_SCHEME: &str = "ehr";

/// The `EHR` attributes usable as a `top_level_structure_locator` (master11
/// §"Top-level Structure Locator"), the versioned top-level ones only.
const EHR_LOCATOR_ATTRIBUTES: [&str; 6] = [
    "compositions",
    "directory",
    "ehr_status",
    "ehr_access",
    "folders",
    "contributions",
];

/// Whether `seg` names an `EHR` top-level-structure attribute.
#[must_use]
pub fn is_ehr_locator_attribute(seg: &str) -> bool {
    EHR_LOCATOR_ATTRIBUTES.contains(&seg)
}

/// How a locator identifies a version of a top-level object.
#[derive(Debug, Clone, PartialEq)]
pub enum VersionLocator {
    /// A bare `VERSIONED_OBJECT._uid_` (a GUID). "When a URI uses the object
    /// identifier, the latest trunk version is always assumed."
    VersionedObject(String),
    /// An exact `OBJECT_VERSION_ID`
    /// (`object_id '::' creating_system_id '::' version_tree_id`).
    Version(ObjectVersionId),
}

impl VersionLocator {
    /// The `VERSIONED_OBJECT._uid_` UUID this locator addresses (the `object_id`
    /// of an `OBJECT_VERSION_ID`, or the bare uid), when it is a UUID.
    #[must_use]
    pub fn object_uuid(&self) -> Option<Uuid> {
        match self {
            VersionLocator::VersionedObject(s) => Uuid::parse_str(s).ok(),
            VersionLocator::Version(ovid) => match ovid.object_id() {
                Uid::Uuid(u) => Some(*u.value()),
                Uid::InternetId(_) | Uid::IsoOid(_) => None,
            },
        }
    }
}

impl fmt::Display for VersionLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionLocator::VersionedObject(s) => f.write_str(s),
            VersionLocator::Version(ovid) => f.write_str(ovid.value()),
        }
    }
}

/// A parsed `top_level_structure_locator`: a mandatory `EHR` attribute name
/// and an optional versioned-object reference (master11 §"EHR Reference
/// URIs").
#[derive(Debug, Clone, PartialEq)]
pub struct TopLevelLocator {
    /// The `EHR` attribute name (`compositions`, `directory`, …).
    pub attribute: String,
    /// The versioned-object reference, if present (absent for e.g. `directory`).
    pub object: Option<VersionLocator>,
}

/// A structurally parsed `ehr:` URI, absolute or relative (master11 §"EHR
/// URIs"); the scheme check is `dv_ehr_uri_impl`'s.
#[derive(Debug, Clone, PartialEq)]
pub struct EhrUri {
    /// The EHR system / repository (`ehr://system_id/…`); `None` for the local
    /// system (`ehr:/…`) and relative forms.
    pub system_id: Option<String>,
    /// The `EHR._ehr_id_` (a UUID; "strongly recommended that a UUID always be
    /// used"). `None` only for the relative forms, which carry no `ehr_id`.
    pub ehr_id: Option<Uuid>,
    /// The top-level-structure locator, if the URI reaches beyond the EHR.
    pub locator: Option<TopLevelLocator>,
    /// The `path_inside_top_level_structure` (a *relative* [`RmPath`]), if any.
    pub item_path: Option<RmPath>,
}

/// Why an `ehr:` URI string was rejected structurally.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EhrUriError {
    /// The input was empty.
    #[error("empty URI")]
    Empty,
    /// No `scheme:` prefix was present.
    #[error("URI has no scheme (expected 'ehr:')")]
    MissingScheme,
    /// The scheme was not `ehr` (case-insensitive).
    #[error("URI scheme is {0:?}, not 'ehr'")]
    WrongScheme(String),
    /// The `ehr_id` segment was not a UUID.
    #[error("EHR id {0:?} is not a UUID")]
    BadEhrId(String),
    /// The first locator segment is neither an `EHR` attribute name nor a
    /// versioned-object id.
    #[error("top-level locator {0:?} is neither an EHR attribute name nor a versioned-object id")]
    UnrecognisedLocator(String),
    /// A versioned-object id in the locator was malformed.
    #[error("malformed version identifier in locator: {0:?}")]
    MalformedVersion(String),
    /// The `path_inside_top_level_structure` did not parse.
    #[error("invalid path inside top-level structure")]
    Path(#[from] PathError),
}

impl FromStr for EhrUri {
    type Err = EhrUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(EhrUriError::Empty);
        }
        let (scheme, rest) = s.split_once(':').ok_or(EhrUriError::MissingScheme)?;
        if !scheme.eq_ignore_ascii_case(EHR_SCHEME) {
            return Err(EhrUriError::WrongScheme(scheme.to_owned()));
        }
        // `//` is the authority form, `/` the absolute form, else relative.
        let (system_id, ehr_id, tail) = if let Some(after) = rest.strip_prefix("//") {
            let (system, after) = split_first(after);
            let (ehr, tail) = split_first(after);
            (Some(system.to_owned()), parse_ehr_id(ehr)?, tail)
        } else if let Some(after) = rest.strip_prefix('/') {
            let (ehr, tail) = split_first(after);
            (None, parse_ehr_id(ehr)?, tail)
        } else {
            (None, None, rest)
        };
        let (locator, item_path) = parse_locator_and_path(tail)?;
        Ok(Self {
            system_id,
            ehr_id,
            locator,
            item_path,
        })
    }
}

impl fmt::Display for EhrUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Components after the `ehr_id`, in order.
        let mut parts: Vec<String> = Vec::new();
        if let Some(ehr) = &self.ehr_id {
            parts.push(ehr.to_string());
        }
        if let Some(loc) = &self.locator {
            parts.push(loc.attribute.clone());
            if let Some(obj) = &loc.object {
                parts.push(obj.to_string());
            }
        }
        if let Some(path) = &self.item_path {
            // A relative `RmPath` renders without a leading slash.
            parts.push(path.to_string());
        }
        f.write_str(EHR_SCHEME)?;
        f.write_str(":")?;
        match &self.system_id {
            Some(system) => {
                write!(f, "//{system}")?;
                if !parts.is_empty() {
                    write!(f, "/{}", parts.join("/"))?;
                }
            }
            None if self.ehr_id.is_some() => write!(f, "/{}", parts.join("/"))?,
            None => f.write_str(&parts.join("/"))?,
        }
        Ok(())
    }
}

/// Splits `s` at its first `/`; the second part is `""` without one.
fn split_first(s: &str) -> (&str, &str) {
    s.split_once('/').unwrap_or((s, ""))
}

/// Parse an `ehr_id` segment: `None` when empty, `Some(uuid)` when a UUID, error
/// otherwise.
fn parse_ehr_id(seg: &str) -> Result<Option<Uuid>, EhrUriError> {
    if seg.is_empty() {
        return Ok(None);
    }
    match Uuid::parse_str(seg) {
        Ok(uuid) => Ok(Some(uuid)),
        Err(_) => Err(EhrUriError::BadEhrId(seg.to_owned())),
    }
}

/// Whether a locator segment looks like a versioned-object identifier: a bare
/// UUID (`VERSIONED_OBJECT._uid_`) or a `::`-bearing `OBJECT_VERSION_ID`.
fn looks_like_version_id(seg: &str) -> bool {
    seg.contains("::") || Uuid::parse_str(seg).is_ok()
}

/// Parse one versioned-object locator segment.
fn parse_version_locator(seg: &str) -> Result<VersionLocator, EhrUriError> {
    if seg.contains("::") {
        let Ok(ovid) = ObjectVersionId::from_str(seg) else {
            return Err(EhrUriError::MalformedVersion(seg.to_owned()));
        };
        Ok(VersionLocator::Version(ovid))
    } else if Uuid::parse_str(seg).is_ok() {
        Ok(VersionLocator::VersionedObject(seg.to_owned()))
    } else {
        Err(EhrUriError::MalformedVersion(seg.to_owned()))
    }
}

/// Split the tail (everything after the `ehr_id`) into the top-level-structure
/// locator and the relative `path_inside_top_level_structure`.
fn parse_locator_and_path(
    tail: &str,
) -> Result<(Option<TopLevelLocator>, Option<RmPath>), EhrUriError> {
    if tail.is_empty() {
        return Ok((None, None));
    }
    let mut segs = split_top_level(tail)
        .map_err(|idx| EhrUriError::Path(PathError::UnterminatedPredicate(idx)))?;
    // Drop a trailing slash (`ehr:/ehr_id/` addresses the EHR, no locator).
    while segs.last() == Some(&"") {
        segs.pop();
    }
    if segs.is_empty() {
        return Ok((None, None));
    }
    let (first, rest) = segs.split_at(1);
    // The first segment must be an `EHR` attribute name.
    let first = first.first().copied().unwrap_or_default();
    if !is_ehr_locator_attribute(first) {
        return Err(EhrUriError::UnrecognisedLocator(first.to_owned()));
    }
    let attribute = first.to_owned();
    let (object, item_segs): (Option<VersionLocator>, &[&str]) = match rest.split_first() {
        Some((&head, tail)) if looks_like_version_id(head) => {
            (Some(parse_version_locator(head)?), tail)
        }
        _ => (None, rest),
    };
    let item_path = if item_segs.is_empty() {
        None
    } else {
        Some(item_segs.join("/").parse::<RmPath>()?)
    };
    Ok((Some(TopLevelLocator { attribute, object }), item_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation() -> Value {
        json!({
            "_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.blood_pressure.v2",
            "name": {"_type": "DV_TEXT", "value": "Blood pressure"},
            "data": {
                "_type": "HISTORY",
                "archetype_node_id": "at0001",
                "name": {"value": "history"},
                "events": [
                    {
                        "_type": "POINT_EVENT",
                        "archetype_node_id": "at0006",
                        "name": {"value": "any event"},
                        "time": {"_type": "DV_DATE_TIME", "value": "2005-12-03T09:22:00"},
                        "data": {
                            "_type": "ITEM_TREE",
                            "archetype_node_id": "at0003",
                            "name": {"value": "blood pressure"},
                            "items": [
                                {
                                    "_type": "ELEMENT",
                                    "archetype_node_id": "at0004",
                                    "name": {"value": "Systolic"},
                                    "value": {"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]"}
                                },
                                {
                                    "_type": "ELEMENT",
                                    "archetype_node_id": "at0005",
                                    "name": {"value": "Diastolic"},
                                    "value": {"_type": "DV_QUANTITY", "magnitude": 80.0, "units": "mm[Hg]"}
                                }
                            ]
                        }
                    }
                ]
            }
        })
    }

    fn path(s: &str) -> RmPath {
        s.parse().unwrap()
    }

    #[test]
    fn parse_forms() {
        let p = path("/data/events[at0006]/data/items[at0004,'Systolic']/value");
        assert!(p.absolute);
        assert_eq!(p.segments.len(), 5);
        assert_eq!(
            p.segments[1].predicate.archetype_node_id.as_deref(),
            Some("at0006")
        );
        assert_eq!(
            p.segments[3].predicate.name_value.as_deref(),
            Some("Systolic")
        );

        // Explicit conjunction form.
        let q = path("/data/events[at0006 and name/value='any event']");
        assert_eq!(
            q.segments[1].predicate.archetype_node_id.as_deref(),
            Some("at0006")
        );
        assert_eq!(
            q.segments[1].predicate.name_value.as_deref(),
            Some("any event")
        );

        // Relative path.
        assert!(!path("items[at0004]/value").absolute);
    }

    #[test]
    fn parse_rejections() {
        assert_eq!("".parse::<RmPath>(), Err(PathError::Empty));
        assert!(matches!(
            "/data/events[at0006".parse::<RmPath>(),
            Err(PathError::UnterminatedPredicate(1))
        ));
        // A comparison against an unquoted non-numeric literal is not in the
        // grammar (a string literal must be single-quoted).
        assert!(matches!(
            "/data/events[time >= bogus]".parse::<RmPath>(),
            Err(PathError::UnsupportedPredicate(_))
        ));
        // A comparison left side must be a plain relative attribute path.
        assert!(matches!(
            "/data/events[some path = 'x']".parse::<RmPath>(),
            Err(PathError::UnsupportedPredicate(_))
        ));
    }

    /// The parenthesised uniqueness modifier is refused.
    #[test]
    fn master05_uniqueness_modifier_is_refused() {
        assert!(matches!(
            "/folders[hospital episodes(car accident Aug 1998)]".parse::<RmPath>(),
            Err(PathError::UnsupportedPredicate(_))
        ));
        assert!(matches!(
            "/folders[at0001(x)]".parse::<RmPath>(),
            Err(PathError::UnsupportedPredicate(_))
        ));
    }

    /// The master11 shortcuts still bind, and a bare name binds as an
    /// `archetype_node_id`.
    #[test]
    fn bare_bracket_tokens_keep_their_master11_binding() {
        assert_eq!(
            path("/data/events[at0003]").segments[1]
                .predicate
                .archetype_node_id
                .as_deref(),
            Some("at0003")
        );
        assert_eq!(
            path("/content[openEHR-EHR-COMPOSITION.x.v1]").segments[0]
                .predicate
                .archetype_node_id
                .as_deref(),
            Some("openEHR-EHR-COMPOSITION.x.v1")
        );
        assert_eq!(
            path("/folders[hospital episodes]").segments[0]
                .predicate
                .archetype_node_id
                .as_deref(),
            Some("hospital episodes"),
            "master05's name form binds as an archetype_node_id under master11"
        );
    }

    #[test]
    fn general_comparison_predicates() {
        // BASE master11-paths §"Other Predicates": both normative example
        // forms parse.
        let p = path("/data/events[at0007 AND time >= '24-06-2005T09:30:00']");
        assert_eq!(
            p.segments[1].predicate.archetype_node_id.as_deref(),
            Some("at0007")
        );
        assert_eq!(p.segments[1].predicate.comparisons.len(), 1);
        let icd = path(
            "/data/items[at0002.1 AND value/defining_code/terminology_id/value = 'ICD10AM' \
             AND value/defining_code/code_string = 'A04']",
        );
        assert_eq!(icd.segments[1].predicate.comparisons.len(), 2);

        // Evaluation: string comparison drops to the RM scalar wrapper's
        // `value` member (DV_DATE_TIME), ISO strings ordering temporally.
        let obs = observation();
        let hit = path(
            "/data/events[at0006 and time >= '2005-01-01T00:00:00']/data/items[at0004]/value/magnitude",
        );
        assert_eq!(item_at_path(&obs, &hit).unwrap().as_f64(), Some(120.0));
        let miss = path("/data/events[at0006 and time >= '2006-01-01T00:00:00']");
        assert!(!path_exists(&obs, &miss));

        // Numeric comparison on a leaf.
        let sys = path("/data/events[at0006]/data/items[value/magnitude > 100]");
        let nodes = items_at_path(&obs, &sys);
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0].get("archetype_node_id").and_then(Value::as_str),
            Some("at0004")
        );

        // Existential node-set semantics: the conjunct holds if ANY selected
        // node satisfies it (one of the two items exceeds 100).
        let ev = path("/data/events[data/items/value/magnitude > 100]");
        assert!(path_exists(&obs, &ev));
        let none = path("/data/events[data/items/value/magnitude > 500]");
        assert!(!path_exists(&obs, &none));

        // Inequality.
        let ne = path("/data/events[at0006]/data/items[name/value != 'Systolic']");
        let nodes = items_at_path(&obs, &ne);
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0].get("archetype_node_id").and_then(Value::as_str),
            Some("at0005")
        );
    }

    #[test]
    fn display_round_trips() {
        for s in [
            "/data/events[at0006]/data/items[at0004,'Systolic']/value",
            "/content[openEHR-EHR-SECTION.vital_signs.v1,'Vital signs']",
            "items/value",
            "/data/events[at0006 and time >= '2005-01-01T00:00:00']",
            "/data/items[value/magnitude > 100]",
        ] {
            assert_eq!(path(s).to_string(), s);
        }
    }

    #[test]
    fn navigation_by_archetype_node_id() {
        let obs = observation();
        let p = path("/data/events[at0006]/data/items[at0004]/value/magnitude");
        let v = item_at_path(&obs, &p).unwrap();
        assert_eq!(v.as_f64(), Some(120.0));
        assert!(path_exists(&obs, &p));
        assert!(path_unique(&obs, &p));
    }

    #[test]
    fn navigation_by_name_predicate() {
        let obs = observation();
        let p = path("/data/events[at0006]/data/items[at0005,'Diastolic']/value/magnitude");
        assert_eq!(item_at_path(&obs, &p).unwrap().as_f64(), Some(80.0));
        // Wrong name → no match.
        let q = path("/data/events[at0006]/data/items[at0005,'Systolic']");
        assert!(!path_exists(&obs, &q));
    }

    #[test]
    fn unpredicated_container_selects_all() {
        let obs = observation();
        let p = path("/data/events[at0006]/data/items");
        let items = items_at_path(&obs, &p);
        assert_eq!(items.len(), 2);
        assert!(!path_unique(&obs, &p));
        assert!(path_exists(&obs, &p));
    }

    #[test]
    fn missing_path_resolves_to_nothing() {
        let obs = observation();
        let p = path("/data/events[at0099]/data");
        assert!(!path_exists(&obs, &p));
        assert!(item_at_path(&obs, &p).is_none());
        assert_eq!(items_at_path(&obs, &p), Vec::<&Value>::new());
    }

    #[test]
    fn path_of_item_reconstructs_predicated_path() {
        let obs = observation();
        let p = path("/data/events[at0006]/data/items[at0005]/value");
        let target = item_at_path(&obs, &p).unwrap();
        let found = path_of_item(&obs, target).unwrap();
        assert_eq!(
            found,
            "/data/events[at0006,'any event']/data/items[at0005,'Diastolic']/value"
        );
        // The reconstructed path resolves back to the same node.
        assert!(std::ptr::eq(
            item_at_path(&obs, &found.parse().unwrap()).unwrap(),
            target
        ));
        // The root's own path is "/".
        assert_eq!(path_of_item(&obs, &obs).as_deref(), Some("/"));
    }

    #[test]
    fn parent_of_walks_up_one_rm_node() {
        let obs = observation();
        let systolic_value =
            item_at_path(&obs, &path("/data/events[at0006]/data/items[at0004]/value")).unwrap();
        let element = parent_of(&obs, systolic_value).unwrap();
        assert_eq!(
            element.get("archetype_node_id").and_then(Value::as_str),
            Some("at0004")
        );
        // Array elements report the object holding the array.
        let event = item_at_path(&obs, &path("/data/events[at0006]")).unwrap();
        let history = parent_of(&obs, event).unwrap();
        assert_eq!(
            history.get("_type").and_then(Value::as_str),
            Some("HISTORY")
        );
        // The root has no parent.
        assert!(parent_of(&obs, &obs).is_none());
    }

    // ── `//` patterns, positional, uid, archetype_id ─────────────────────────

    /// The master11 two-event blood-pressure OBSERVATION.
    fn bp_two_events() -> Value {
        json!({
            "_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.blood_pressure.v1",
            "name": {"value": "BP measurement"},
            "data": {
                "archetype_node_id": "at0001",
                "events": [
                    {
                        "_type": "POINT_EVENT",
                        "archetype_node_id": "at0006",
                        "name": {"value": "sitting"},
                        "data": {"_type": "ITEM_LIST", "archetype_node_id": "at0003", "items": [
                            {"archetype_node_id": "at0004", "name": {"value": "systolic"}, "value": {"magnitude": 120.0}},
                            {"archetype_node_id": "at0005", "name": {"value": "diastolic"}, "value": {"magnitude": 80.0}}
                        ]}
                    },
                    {
                        "_type": "POINT_EVENT",
                        "archetype_node_id": "at0006",
                        "name": {"value": "standing"},
                        "data": {"_type": "ITEM_LIST", "archetype_node_id": "at0003", "items": [
                            {"archetype_node_id": "at0004", "name": {"value": "systolic"}, "value": {"magnitude": 105.0}},
                            {"archetype_node_id": "at0005", "name": {"value": "diastolic"}, "value": {"magnitude": 70.0}}
                        ]}
                    }
                ]
            }
        })
    }

    #[test]
    fn archetype_node_id_matches_multiple_but_name_is_unique() {
        let bp = bp_two_events();
        // The archetype path matches BOTH events (spec NOTE: "it can correspond
        // to more than one item in runtime data").
        let both = path("/data/events[at0006]/data/items[at0004]/value/magnitude");
        assert_eq!(items_at_path(&bp, &both).len(), 2);
        assert!(!path_unique(&bp, &both));
        // The name/value shortcut makes it unique (spec §"Using a Name-based
        // Predicate").
        let standing = path("/data/events[at0006, 'standing']/data/items[at0004]/value/magnitude");
        assert_eq!(item_at_path(&bp, &standing).unwrap().as_f64(), Some(105.0));
        assert!(path_unique(&bp, &standing));
    }

    #[test]
    fn positional_predicate_is_one_based_and_unique() {
        let bp = bp_two_events();
        // `[1]`/`[2]` select by container order (spec §"Using Positional
        // Parameters" — the guaranteed-unique form).
        for (events_pos, items_pos, mag) in
            [(1, 1, 120.0), (1, 2, 80.0), (2, 1, 105.0), (2, 2, 70.0)]
        {
            let p = path(&format!(
                "/data/events[{events_pos}]/data/items[{items_pos}]/value/magnitude"
            ));
            assert!(path_unique(&bp, &p));
            assert_eq!(item_at_path(&bp, &p).unwrap().as_f64(), Some(mag));
        }
        // Out-of-range position resolves to nothing.
        assert!(!path_exists(&bp, &path("/data/events[3]")));
        // Position 0 is rejected at parse time (1-based).
        assert_eq!(
            "/data/events[0]".parse::<RmPath>(),
            Err(PathError::InvalidPosition("0".to_owned()))
        );
    }

    #[test]
    fn descendant_pattern_matches_at_any_depth() {
        let bp = bp_two_events();
        // `//items[at0004]` finds both systolic ELEMENTs regardless of depth.
        let p = path("//items[at0004]");
        assert!(p.segments[0].descendant);
        assert_eq!(items_at_path(&bp, &p).len(), 2);
        // Combined with a prefix + a following segment.
        let q = path("/data//items[at0005, 'diastolic']/value/magnitude");
        let mags: Vec<f64> = items_at_path(&bp, &q)
            .iter()
            .filter_map(|v| v.as_f64())
            .collect();
        assert_eq!(mags.len(), 2);
        assert!(mags.contains(&80.0) && mags.contains(&70.0));
        // `//` round-trips through Display.
        assert_eq!(path("//items[at0004]").to_string(), "//items[at0004]");
        assert_eq!(
            path("/data//items[at0004]").to_string(),
            "/data//items[at0004]"
        );
        // A dangling `//` is rejected.
        assert_eq!(
            "/data//".parse::<RmPath>(),
            Err(PathError::DanglingDescendant)
        );
    }

    #[test]
    fn uid_predicate_matches_locatable_uid() {
        let node = json!({
            "_type": "OBSERVATION",
            "data": {"events": [
                {"archetype_node_id": "at0006", "uid": {"_type": "HIER_OBJECT_ID", "value": "25f2f224-64f0-41ec-a5c7-c31c040c77ce"}, "name": {"value": "x"}},
                {"archetype_node_id": "at0006", "uid": {"_type": "HIER_OBJECT_ID", "value": "aaaaaaaa-64f0-41ec-a5c7-c31c040c77ce"}, "name": {"value": "y"}}
            ]}
        });
        // `[uid='...']` (spec §"Using a Uid-based Predicate").
        let p = path("/data/events[uid='25f2f224-64f0-41ec-a5c7-c31c040c77ce']");
        assert!(path_unique(&node, &p));
        assert_eq!(
            item_at_path(&node, &p).unwrap()["name"]["value"].as_str(),
            Some("x")
        );
        // Combined `[at0006 and uid='...']`.
        let q = path("/data/events[at0006 and uid='aaaaaaaa-64f0-41ec-a5c7-c31c040c77ce']");
        assert_eq!(
            q.segments[1].predicate.archetype_node_id.as_deref(),
            Some("at0006")
        );
        assert_eq!(
            item_at_path(&node, &q).unwrap()["name"]["value"].as_str(),
            Some("y")
        );
    }

    #[test]
    fn archetype_id_predicate_long_form() {
        // `[archetype_id=...]` resolves against `archetype_node_id`.
        let p = path("/content[archetype_id=openEHR-EHR-SECTION.vital_signs.v1]");
        assert_eq!(
            p.segments[0].predicate.archetype_node_id.as_deref(),
            Some("openEHR-EHR-SECTION.vital_signs.v1")
        );
        // Quoted form and the bare shortcut agree.
        let q = path("/content[@archetype_id='openEHR-EHR-SECTION.vital_signs.v1']");
        assert_eq!(q.segments[0].predicate, p.segments[0].predicate);
        let bare = path("/content[openEHR-EHR-SECTION.vital_signs.v1]");
        assert_eq!(bare.segments[0].predicate, p.segments[0].predicate);
    }

    #[test]
    fn general_comparison_predicate_parses() {
        // BASE master11-paths §"Other Predicates", the spec's own example.
        let p = "/data/events[at0007 and time >= '2005-06-24T09:30:00']"
            .parse::<RmPath>()
            .unwrap();
        let pred = &p.segments[1].predicate;
        assert_eq!(pred.archetype_node_id.as_deref(), Some("at0007"));
        assert_eq!(
            pred.comparisons,
            vec![Comparison {
                path: vec!["time".to_owned()],
                op: CmpOp::Ge,
                value: CmpLiteral::Str("2005-06-24T09:30:00".to_owned()),
            }]
        );
    }

    // ── EHR URIs ─────────────────────────────────────────────────────────────

    fn ehr_uri(s: &str) -> EhrUri {
        s.parse().unwrap()
    }

    /// The CNF `DV_EHR_URI` fixtures (`master17.7-content_tc_data_types-uri.adoc`):
    /// those without an `EHR` attribute name are refused, as master11 §"EHR
    /// Reference URIs" requires.
    #[test]
    fn cnf_dv_ehr_uri_fixtures_are_adjudicated() {
        const EHR: &str = "89c0752e-0815-47d7-8b3c-b3aaea2cea7a";
        const OVID: &str = "031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1";
        const ITEM: &str = "context/other_context[at0001]/items[archetype_id=openEHR-EHR-CLUSTER.sample_symptom.v1]/items[at0034]/items[at0021]/value";

        // The EHR-only fixtures name no top-level structure and stay accepted.
        for s in [
            format!("ehr:/{EHR}"),
            format!("ehr://CLOUD_EHRSERVER/{EHR}"),
        ] {
            assert!(s.parse::<EhrUri>().is_ok(), "should parse: {s}");
        }

        // Every locator-bearing fixture, in both twins.
        for (invalid, valid) in [
            (
                format!("ehr:/{EHR}/{OVID}"),
                format!("ehr:/{EHR}/compositions/{OVID}"),
            ),
            (
                format!("ehr:/{EHR}/{OVID}/{ITEM}"),
                format!("ehr:/{EHR}/compositions/{OVID}/{ITEM}"),
            ),
            (
                format!("ehr://CLOUD_EHRSERVER/{EHR}/{OVID}"),
                format!("ehr://CLOUD_EHRSERVER/{EHR}/compositions/{OVID}"),
            ),
            (
                format!("ehr://CLOUD_EHRSERVER/{EHR}/{OVID}/{ITEM}"),
                format!("ehr://CLOUD_EHRSERVER/{EHR}/compositions/{OVID}/{ITEM}"),
            ),
        ] {
            assert!(
                matches!(
                    invalid.parse::<EhrUri>(),
                    Err(EhrUriError::UnrecognisedLocator(_))
                ),
                "the attribute-less locator names no top-level structure: {invalid}"
            );
            assert!(valid.parse::<EhrUri>().is_ok(), "should parse: {valid}");
        }

        // Non-`ehr` schemes are rejected structurally.
        assert!(matches!(
            "ftp://ftp.is.co.za/rfc/rfc1808.txt".parse::<EhrUri>(),
            Err(EhrUriError::WrongScheme(_))
        ));
        assert!(matches!(
            "xyz".parse::<EhrUri>(),
            Err(EhrUriError::MissingScheme)
        ));
    }

    #[test]
    fn ehr_uri_forms_decode() {
        const EHR: &str = "89c0752e-0815-47d7-8b3c-b3aaea2cea7a";
        // Bare EHR reference (local system).
        let u = ehr_uri(&format!("ehr:/{EHR}"));
        assert_eq!(u.system_id, None);
        assert_eq!(u.ehr_id, Some(Uuid::parse_str(EHR).unwrap()));
        assert_eq!(u.locator, None);
        assert_eq!(u.item_path, None);

        // An OVID directly after the ehr_id names no top-level structure.
        assert!(matches!(
            format!("ehr:/{EHR}/031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1")
                .parse::<EhrUri>(),
            Err(EhrUriError::UnrecognisedLocator(_))
        ));

        // The same version, located: an exact OBJECT_VERSION_ID under
        // `compositions` (master11 §"Top-level Structure Locator").
        let u = ehr_uri(&format!(
            "ehr:/{EHR}/compositions/031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1"
        ));
        let loc = u.locator.unwrap();
        assert_eq!(loc.attribute, "compositions");
        match loc.object.unwrap() {
            VersionLocator::Version(ovid) => {
                assert_eq!(
                    ovid.value(),
                    "031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1"
                );
            }
            VersionLocator::VersionedObject(_) => panic!("expected exact version"),
        }

        // master11 §"Top-level Structure Locator" attribute form with a bare uid
        // → latest trunk assumed.
        let u = ehr_uri(
            "ehr:/347a5490-55ee-4da9-b91a-9bba710f730e/compositions/87284370-2d4b-4e3d-a3f3-f303d2f4f34b",
        );
        let loc = u.locator.unwrap();
        assert_eq!(loc.attribute, "compositions");
        assert!(matches!(
            loc.object,
            Some(VersionLocator::VersionedObject(_))
        ));

        // `directory` attribute, no uid.
        let u = ehr_uri("ehr:/347a5490-55ee-4da9-b91a-9bba710f730e/directory");
        let loc = u.locator.unwrap();
        assert_eq!(loc.attribute, "directory");
        assert_eq!(loc.object, None);

        // Authority form.
        let u = ehr_uri(&format!("ehr://CLOUD_EHRSERVER/{EHR}"));
        assert_eq!(u.system_id.as_deref(), Some("CLOUD_EHRSERVER"));
        assert_eq!(u.ehr_id, Some(Uuid::parse_str(EHR).unwrap()));

        // Relative forms carry no ehr_id.
        let u = ehr_uri("ehr:directory");
        assert_eq!(u.ehr_id, None);
        assert_eq!(u.locator.unwrap().attribute, "directory");
        let u = ehr_uri(
            "ehr:compositions/87284370-2d4b-4e3d-a3f3-f303d2f4f34b/content[openEHR-EHR-SECTION.vital_signs.v1]",
        );
        assert_eq!(u.ehr_id, None);
        assert!(u.item_path.is_some());
    }

    #[test]
    fn ehr_uri_byte_round_trip_simple_forms() {
        const EHR: &str = "89c0752e-0815-47d7-8b3c-b3aaea2cea7a";
        // Forms whose predicates need no normalisation round-trip byte-exactly.
        for s in [
            format!("ehr:/{EHR}"),
            format!("ehr:/{EHR}/compositions/031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1"),
            format!("ehr://CLOUD_EHRSERVER/{EHR}"),
            format!(
                "ehr://CLOUD_EHRSERVER/{EHR}/compositions/031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1"
            ),
            "ehr:directory".to_owned(),
        ] {
            assert_eq!(ehr_uri(&s).to_string(), s, "byte round-trip: {s}");
        }
    }

    #[test]
    fn ehr_uri_structural_round_trip_with_item_path() {
        // Normalised, so it round-trips structurally.
        let s = "ehr:/89c0752e-0815-47d7-8b3c-b3aaea2cea7a/compositions/031f2513-b9ef-47b2-bbef-8db24ae68c2f::EHRSERVER::1/context/other_context[at0001]/items[archetype_id=openEHR-EHR-CLUSTER.sample_symptom.v1]/items[at0034]/items[at0021]/value";
        let u = ehr_uri(s);
        let reparsed: EhrUri = u.to_string().parse().unwrap();
        assert_eq!(u, reparsed);
    }

    #[test]
    fn ehr_uri_rejections() {
        assert_eq!("".parse::<EhrUri>(), Err(EhrUriError::Empty));
        assert!(matches!(
            "ehr:/not-a-uuid".parse::<EhrUri>(),
            Err(EhrUriError::BadEhrId(_))
        ));
        // A relative form whose first segment is neither an EHR attribute nor a
        // version id.
        assert!(matches!(
            "ehr:nonsense_attr".parse::<EhrUri>(),
            Err(EhrUriError::UnrecognisedLocator(_))
        ));
    }

    /// The term-code and archetype-root node-id forms are disjoint.
    #[test]
    fn node_id_forms_are_disjoint() {
        for term_code in ["at0000", "at0005", "at0002.1", "id3", "id1.1.4"] {
            assert!(
                archetype_node_id_is_term_code(term_code),
                "{term_code} is a term code"
            );
            assert!(
                !is_archetype_root_node_id(term_code),
                "{term_code} is not an archetype root"
            );
        }
        for hrid in [
            "openEHR-EHR-COMPOSITION.minimal.v1",
            "openEHR-EHR-OBSERVATION.blood_pressure-cuff.v2",
            "CIMI-CORE-CLUSTER.device.v1",
        ] {
            assert!(
                is_archetype_root_node_id(hrid),
                "{hrid} is an archetype root"
            );
            assert!(
                !archetype_node_id_is_term_code(hrid),
                "{hrid} is not a term code"
            );
        }
    }

    /// Neither predicate fires on a string in no node-id form.
    #[test]
    fn node_id_forms_reject_non_node_ids() {
        for other in [
            "",
            "atrial",
            "identifier",
            "at",
            "id",
            "openEHR-EHR-COMPOSITION.minimal",
            "openEHR-EHR.minimal.v1",
        ] {
            assert!(
                !archetype_node_id_is_term_code(other),
                "{other:?} is no term code"
            );
            assert!(
                !is_archetype_root_node_id(other),
                "{other:?} is no archetype root"
            );
        }
    }
}
