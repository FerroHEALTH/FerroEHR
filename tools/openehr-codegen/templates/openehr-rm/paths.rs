// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! openEHR path machinery (hand-written spec behaviour).
//!
//! Implements the RM `PATHABLE` pathing functions — `item_at_path` /
//! `items_at_path` / `path_exists` / `path_unique` / `path_of_item` / `parent` —
//! over the canonical-JSON value tree, which is this workspace's uniform RM
//! representation. It also carries the two `LOCATABLE` node-id form predicates
//! ([`archetype_node_id_is_term_code`], [`is_archetype_root_node_id`]) and the
//! [`EhrUri`] structural parser for the `ehr:` URI scheme.
//!
//! Spec: RM
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.pathable.adoc`
//! for the function signatures and parent semantics, and BASE
//! `docs/specs/openehr/BASE/docs/architecture_overview/master11-paths.adoc`
//! §Basic Syntax / §Predicate Expressions / §Using a Uid-based Predicate /
//! §Using Positional Parameters for the path grammar; the `ehr:` scheme is BASE
//! `master11-paths` §"EHR URIs" and RM `data_types/master10-uri_package`
//! §"DV_EHR_URI Syntax".
//!
//! Four shape decisions:
//!
//! - `LOCATABLE.concept(): DV_TEXT` is absent, because it is not realisable
//!   from an instance: `common/master03-archetyped_package.adoc` §"The
//!   LOCATABLE Class" derives the meaning of a node from the archetype's
//!   `ontology` section, which the instance does not carry. What the instance
//!   does answer — whether a node id names an archetype root — is
//!   [`is_archetype_root_node_id`].
//! - `PATHABLE.parent()` is a back-reference, realised as the root-anchored
//!   lookup [`parent_of`] rather than a stored pointer.
//! - The typed RM tree is not walked directly: every consumer already holds the
//!   canonical JSON form, so a second typed visitor would duplicate this logic
//!   for no wire gain.
//! - General comparison predicates (§"Other Predicates") are [`Comparison`]
//!   conjuncts evaluated with XPath existential node-set semantics; predicate
//!   text outside the grammar fails loud as
//!   [`PathError::UnsupportedPredicate`].
//!
//! NOTE: the `//` pattern and the positional predicate `[n]` belong to the
//! master11 PATH grammar realised here, not to the AQL 1.1 path grammar (QUERY
//! `master03` §"Predicates" enumerates only the standard/archetype/node
//! predicates).

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::Value;
use std::fmt;
use std::str::FromStr;

use openehr_base::v1_3::prelude::{ArchetypeId, ObjectVersionId, Uid};
use uuid::Uuid;

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
    /// A predicate uses a construct outside the supported
    /// archetype-id / name / uid / positional subset (general comparisons like
    /// `time >= '...'` are not part of this navigation primitive).
    #[error("unsupported predicate expression: {0:?}")]
    UnsupportedPredicate(String),
    /// A positional predicate `[n]` had a value that is not a positive integer
    /// (XPath positions are 1-based — BASE `master11-paths` §"Using Positional
    /// Parameters").
    #[error("invalid positional predicate {0:?} (positions are 1-based integers)")]
    InvalidPosition(String),
    /// A dangling `//` with no following attribute segment.
    #[error("path pattern ends with '//' (no following attribute)")]
    DanglingDescendant,
}

/// The predicate of one path segment (a conjunction of the BASE `master11-paths`
/// §"Predicate Expressions" shortcuts):
/// - `archetype_node_id` — `[at0003]` / `[openEHR-EHR-OBSERVATION.x.v1]` /
///   `[archetype_id=…]`, the shortcut for `[@archetype_node_id='…']` (at
///   archetype chaining points the node id carries the archetype id);
/// - `name_value` — `[at0003,'name']` / `[at0003 and name/value='name']`;
/// - `uid` — `[uid='…']` / `[at0003 and uid='…']` (§"Using a Uid-based
///   Predicate");
/// - `position` — the 1-based XPath positional predicate `[n]` (§"Using
///   Positional Parameters"), applied to the *container*, not a node attribute;
/// - `comparisons` — general attribute comparisons (§"Other Predicates"), e.g.
///   `[at0007 and time >= '2005-06-24T09:30:00']` or
///   `[at0002.1 and value/defining_code/code_string = 'A04']`.
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

/// The right-hand literal of a general comparison conjunct: a single-quoted
/// string (compared byte-lexically) or an unquoted number (compared
/// numerically).
///
/// BASE `master11-paths.adoc` §Predicate Expressions defines the predicate
/// SYNTAX only, so the evaluation rule is our own design. A lexical compare
/// coincides with temporal order only for identically-formatted, same-offset,
/// fully-specified ISO-8601 values — master11's own example literal is
/// day-first, where it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmpLiteral {
    /// A `'…'` string literal.
    Str(String),
    /// An unquoted numeric literal (stored as written; compared as `f64`).
    Num(String),
}

/// One general comparison conjunct of a predicate.
///
/// BASE `master11-paths` §"Other Predicates": a relative attribute path, an
/// operator, and a literal — e.g. `time >= '2005-06-24T09:30:00'` or
/// `value/defining_code/code_string = 'A04'`.
///
/// Evaluation follows XPath 1 existential node-set semantics: the conjunct
/// holds if ANY node the relative path selects from the candidate satisfies
/// the comparison. An RM scalar wrapper object (e.g. a `DV_DATE_TIME`)
/// compares by its `value` member, matching the string-value XPath would see.
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

    /// Whether one resolved leaf satisfies the comparison. An object leaf
    /// drops to its `value` member (the RM scalar-wrapper convention, as in
    /// [`node_uid`]).
    fn satisfied_by(&self, leaf: &Value) -> bool {
        let leaf = match leaf {
            Value::Object(o) => match o.get("value") {
                Some(v) => v,
                None => return false,
            },
            other => other,
        };
        match &self.value {
            CmpLiteral::Str(want) => leaf
                .as_str()
                .is_some_and(|have| self.op.holds(have.cmp(want.as_str()))),
            CmpLiteral::Num(raw) => match (leaf.as_f64(), raw.parse::<f64>()) {
                (Some(have), Ok(want)) => have
                    .partial_cmp(&want)
                    .is_some_and(|ord| self.op.holds(ord)),
                _ => false,
            },
        }
    }
}

/// Collect every node the relative attribute path selects from `node`,
/// descending into container attributes element-wise (XPath node-set
/// traversal).
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

    /// Whether a canonical-JSON RM node satisfies this predicate's
    /// *attribute-based* conjuncts (`archetype_node_id`, `name/value`, `uid`).
    /// The positional conjunct is orthogonal — it selects by container index
    /// and is applied in `step`, not here.
    ///
    /// `archetype_node_id` matches `LOCATABLE.archetype_node_id`; `name_value`
    /// matches `LOCATABLE.name.value` (a `DV_TEXT`); `uid` matches
    /// `LOCATABLE.uid.value` (BASE `master11-paths` §"Name-based Predicate" /
    /// §"Using a Uid-based Predicate"). All are exact, case-sensitive, and ANDed.
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

/// The `LOCATABLE.uid.value` of a canonical-JSON node, tolerating both the
/// object form (`{"_type":"HIER_OBJECT_ID","value":"…"}`, the canonical
/// encoding) and a bare-string `uid`.
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
    /// `true` when this segment is reached via a `//` path pattern, i.e. it
    /// matches at *any depth* below the current context rather than as a direct
    /// child (BASE `master11-paths` §"Basic Syntax": the `//` pattern "can match
    /// any number of path segments").
    pub descendant: bool,
}

/// A parsed openEHR path: `/`-separated [`PathSegment`]s, absolute (leading
/// `/`) or relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmPath {
    /// `true` for an absolute path (leading `/`): the starting point is the
    /// top of the structure. Navigation from a given root treats both forms
    /// identically (the root *is* the starting point).
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
        // Split on top-level '/' only — slashes inside [...] predicates (e.g.
        // `name/value=...`) are not separators. An *empty* raw segment marks a
        // `//` path pattern: the following real segment is a descendant match
        // (BASE `master11-paths` §"Basic Syntax").
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

/// Split a path body on top-level `/` (slashes inside `[...]` predicates are not
/// separators), returning the raw segment strings — including empty strings for
/// `//` markers. On an unterminated `[` returns `Err(index)` with the 0-based
/// index of the offending segment.
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
    /// Render this predicate in its canonical bracketed form (a normalised
    /// re-emission of the parsed shortcuts — see the module NOTE on
    /// round-trip stability). Emits nothing when empty.
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
            // Preferred openEHR shortcut `[id,'name']` when combined with an
            // archetype id, else the explicit `name/value='name'` form (BASE
            // `master11-paths` §"Name-based Predicate").
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

/// Parse the inside of a `[...]` predicate: the archetype-id shortcut, the
/// `,'name'` shortcut, explicit `name/value='...'` conjuncts, and the
/// explicit `@archetype_node_id='...'` form.
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
    // Explicit archetype-id predicate at a chaining point:
    // `[archetype_id=openEHR-EHR-…]` / `[@archetype_id='…']` — the long form of
    // the bare `[openEHR-EHR-…]` shortcut (§"Paths within Top-level
    // Structures"). Both resolve against the node's `archetype_node_id`, which
    // at an archetype root carries the archetype id. The value may be unquoted
    // (it contains no `'`).
    if let Some(value) = strip_eq(c, "@archetype_id").or_else(|| strip_eq(c, "archetype_id")) {
        let value = value
            .strip_prefix('\'')
            .and_then(|v| v.strip_suffix('\''))
            .unwrap_or(value);
        predicate.archetype_node_id = Some(value.to_owned());
        return Ok(());
    }
    // A general comparison conjunct (§"Other Predicates"), e.g.
    // `time >= '2005-06-24T09:30:00'` or
    // `value/defining_code/code_string = 'A04'`.
    if c.contains(['=', '<', '>']) {
        predicate.comparisons.push(parse_comparison(c)?);
        return Ok(());
    }
    // The bare shortcut: an at-code / id-code or an archetype id. Anything
    // else containing literal syntax is not a valid predicate conjunct.
    if c.contains('\'') {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    }
    // NOTE: the name-based bare-bracket form with a parenthesised
    // uniqueness modifier that RM common `master05-directory_package.adoc` §Paths
    // states has no production in BASE `master11-paths`, so it is refused, not bound.
    if c.contains('(') || c.contains(')') {
        return Err(PathError::UnsupportedPredicate(c.to_owned()));
    }
    predicate.archetype_node_id = Some(c.to_owned());
    Ok(())
}

/// Parse a general comparison conjunct `<relative-path> <op> <literal>`
/// (BASE `master11-paths` §"Other Predicates"). The left side is a relative
/// attribute path (plain names, `/`-separated, optional XPath-style leading
/// `@`); the literal is a single-quoted string or an unquoted number.
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

/// One navigation step: from `node`, follow `segment` and append every
/// matching child to `out`.
///
/// The positional conjunct `[n]` (1-based) selects the nth container element
/// (or, on a single-valued attribute, requires `n == 1`) — BASE
/// `master11-paths` §"Using Positional Parameters"; the attribute-based
/// conjuncts are then still ANDed via [`Predicate::matches`].
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

/// Descendant-or-self navigation for a `//`-pattern segment: apply [`step`] at
/// `node` and at every descendant object, so the segment matches at any depth
/// (BASE `master11-paths` §"Basic Syntax"). Each candidate node is the value of
/// exactly one (attribute, parent) pair, so a match is produced once.
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

/// One RM path step over the canonical-JSON tree: every value under
/// `segment.attribute` on `node` that satisfies `segment.predicate`, in
/// document order.
///
/// A single-valued attribute yields at most one value (kept only if the
/// predicate matches — a predicate constrains a node regardless of the
/// attribute's cardinality, BASE `master11-paths` §"Predicate Expressions");
/// a container attribute yields each matching element. This is the primitive
/// [`items_at_path`] iterates, exposed so path-walking consumers (the FLAT
/// converters, the composition validator) compose it instead of re-deriving a
/// walker.
#[must_use]
pub fn select_children<'a>(node: &'a Value, segment: &PathSegment) -> Vec<&'a Value> {
    let mut out = Vec::new();
    step(node, segment, &mut out);
    out
}

/// RM `PATHABLE.items_at_path`: every item the path resolves to, relative to
/// `root` (document order preserved). An unresolvable path yields an empty
/// list.
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

/// RM `PATHABLE.item_at_path`: the item at a path.
///
/// The spec precondition is `path_unique(a_path)`; for a non-unique path the
/// *first* item in document order is returned (use [`items_at_path`] to see
/// all), and `None` when the path does not exist.
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

/// RM `PATHABLE.path_of_item`: the path of `item` relative to `root`, located
/// by pointer identity (`item` must be a node *inside* `root`'s tree).
///
/// Container steps carry the `[archetype_node_id,'name']` predicates needed
/// to make the returned path as selective as the data allows.
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

/// RM `PATHABLE.parent`: the parent RM node (nearest enclosing JSON object,
/// skipping the container array) of `item` inside `root`'s tree, or `None`
/// for the root itself / a node not in the tree.
///
/// Realised as a root-anchored search — no owning back-references are stored
/// (repo convention).
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

/// The RM parent reached through one member of `owner`: `owner` itself when
/// the member IS the item (or holds it directly in an array), else whatever
/// the member's own subtree reports.
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

// ── typed entry points ───────────────────────────────────────────────────────
//
// The navigation above is over the canonical-JSON value tree, which is what
// every consumer already holds. A TYPED RM value reaches it by encoding
// itself with the crate's own canonical-JSON impls first — one door, so a
// typed path answer and a wire path answer cannot differ.

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

/// The canonical-JSON form of a typed RM value, as the navigation primitives
/// above consume it.
///
/// # Errors
/// Returns [`TypedPathError::Encode`] when the value cannot be encoded — a
/// non-finite `Real` is the only shape the RM types admit that JSON does not.
pub fn canonical_value<T: serde::Serialize>(value: &T) -> Result<Value, TypedPathError> {
    serde_json::to_value(value).map_err(TypedPathError::Encode)
}

// ── LOCATABLE node-id forms ──────────────────────────────────────────────────

/// Whether `node_id` is an ADL **term code** — an at-code or id-code such as
/// `at0005`, `at0002.1` or `id3`.
///
/// This is the interior-node form of `LOCATABLE.archetype_node_id`:
/// `UML/classes/org.openehr.rm.common.locatable.adoc` §Attributes states it is
/// "Always in the form of an at-code, e.g. `at0005`", and AM `ADL2`
/// `master02-overview.adoc` §"ADL 2.4" adds the id-code alternative ("ADL 2.4
/// introduces an option to use the **at-code coding system** of ADL1, as an
/// alternative to the **id-code coding system** introduced in ADL2"), whose
/// node codes carry the `id` leader (`master01-preface.adoc`: "'id-codes' are
/// used for that purpose").
///
/// A term code is recognised by its leader followed by `.`-separated numeric
/// segments; the specialisation suffix (`at0002.1`) is the depth notation of
/// AM `ADL2` `master09.02-spec_concepts.adoc` §"Specialisation Depth".
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

/// RM `LOCATABLE.is_archetype_root()` decided from `archetype_node_id` alone.
///
/// The function answers "True if this node is the root of an archetyped
/// structure" (`UML/classes/org.openehr.rm.common.locatable.adoc` §Functions).
///
/// The derivation is the one the RM itself states for the attribute: "At an
/// archetype root point, the value of this attribute is always the stringified
/// form of the `archetype_id` found in the `archetype_details` object"
/// (same file, §Attributes), restated in
/// `common/master03-archetyped_package.adoc` §"The LOCATABLE Class" ("The only
/// exception is at archetype root points in data, where `archetype_node_id`
/// carries the archetype identifier in string form rather than an interior node
/// id from an archetype"). So a node id in `ARCHETYPE_ID` lexical form —
/// `rm_originator '-' rm_name '-' rm_entity '.' concept_name { '-'
/// specialisation }* '.v' version_id` (BASE
/// `UML/classes/org.openehr.base.base_types.archetype_id.adoc` §Description) —
/// is an archetype root, and an interior term code
/// ([`archetype_node_id_is_term_code`]) never is. The two forms are disjoint:
/// a term code carries neither the three-part RM qualifier nor a `.v` segment.
///
/// NOTE: the spec leaves `is_archetype_root()` itself undefined — §Functions
/// gives only the Meaning sentence above, with no postcondition or derivation
/// expression — so this node-id reading is one of two readings the text
/// admits, the other being derivation from `archetype_details` presence (which
/// would make `LOCATABLE.Archetyped_valid`, `is_archetype_root xor
/// archetype_details = Void`, a tautology). Callers that need the
/// `archetype_details` reading must test that attribute themselves; this
/// function answers only the node-id question.
#[must_use]
pub fn is_archetype_root_node_id(node_id: &str) -> bool {
    !archetype_node_id_is_term_code(node_id) && node_id.parse::<ArchetypeId>().is_ok()
}

// ── EHR URIs (the `ehr:` scheme) ─────────────────────────────────────────────

/// The literal `ehr` URI scheme (RM `data_types/master10-uri_package`
/// §Definitions: `Ehr_scheme: String = "ehr"`).
pub const EHR_SCHEME: &str = "ehr";

/// The `EHR`-class attribute names usable as a `top_level_structure_locator`
/// (BASE `master11-paths` §"Top-level Structure Locator": "The possible values
/// … come from attribute names of the class `EHR` … namely `compositions`,
/// `directory` etc."; RM `ehr` `EHR` class). Only the attributes that reference
/// a top-level `VERSIONED_OBJECT` are listed (`tags` is an `EHR` attribute but
/// not a versioned top-level structure).
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

/// How a `top_level_structure_locator` identifies a version of a top-level
/// object (BASE `master11-paths` §"Top-level Structure Locator").
// `Eq` is omitted because `ObjectVersionId` is only `PartialEq` (BASE `openehr-base`).
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

/// A parsed `top_level_structure_locator`: an `EHR` attribute name and an
/// optional versioned-object reference.
///
/// Master11 writes locators as `compositions/<uid-or-OVID>` or `directory`.
/// The attribute is not optional: BASE `master11-paths` §"EHR Reference URIs"
/// enumerates the locator's values — "The possible values for
/// `top_level_structure_locator` come from attribute names of the class `EHR`
/// … namely `compositions`, `directory` etc." — so a versioned-object id
/// standing where the attribute belongs does not name a top-level structure.
#[derive(Debug, Clone, PartialEq)]
pub struct TopLevelLocator {
    /// The `EHR` attribute name (`compositions`, `directory`, …).
    pub attribute: String,
    /// The versioned-object reference, if present (absent for e.g. `directory`).
    pub object: Option<VersionLocator>,
}

/// A structurally parsed `ehr:` URI (BASE `master11-paths` §"EHR URIs"; RM
/// `data_types/master10-uri_package` §"DV_EHR_URI Syntax").
///
/// Covers the four absolute forms plus the relative forms
/// (`ehr:compositions/…`, `ehr:directory`).
///
/// This is the *structural* grammar on top of the `DV_EHR_URI` scheme
/// invariant (`Scheme_valid`, which lives in
/// `data_types/uri/dv_ehr_uri_impl.rs` and is not duplicated here).
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
    #[error("invalid path inside top-level structure: {0}")]
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
        // The three leading forms are distinguished by what follows `ehr:`:
        //   `//` → authority form (`//system_id/ehr_id/…`);
        //   `/`  → absolute form  (`/ehr_id/…`);
        //   else → relative form  (`compositions/…`, `directory`).
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

/// Split `s` at its first `/` into `(before, after)`; `after` is `""` when there
/// is no `/`. Used only for the leading `system_id`/`ehr_id` tokens, which carry
/// no `[...]` predicates.
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
    // The locator's first segment is an `EHR` attribute name — master11
    // §"EHR Reference URIs" enumerates the possible values, so anything else
    // (a bare versioned-object id included) names no top-level structure.
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

    /// The parenthesised uniqueness modifier of RM common
    /// `master05-directory_package.adoc` §Paths
    /// (`[hospital episodes(car accident Aug 1998)]`) has no production in the
    /// BASE `master11-paths` grammar this engine implements, so it is refused
    /// loud rather than bound whole as an `archetype_node_id` that could never
    /// match.
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

    /// The bare-bracket bindings this leaves unchanged: the master11
    /// archetype-code and archetype-id shortcuts still bind, and a bare NAME
    /// token keeps its master11 reading — it binds as an `archetype_node_id`
    /// (and so matches nothing), which is the registered handling, not a
    /// refusal.
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
        assert!(items_at_path(&obs, &p).is_empty());
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

    /// The master11 two-event blood-pressure OBSERVATION (BASE `master11-paths`
    /// §"Using a Name-based Predicate" / §"Using Positional Parameters", JSON
    /// form) — a container with two identically-archetyped events, the case the
    /// spec uses to demonstrate positional and name uniqueness.
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
        // The `[archetype_id=...]` long form (unquoted, as in the CNF DV_EHR_URI
        // fixture) resolves against `archetype_node_id` — spec §"Paths within
        // Top-level Structures".
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
        // BASE master11-paths §"Other Predicates" — the spec's own example
        // form is part of the path grammar (previously rejected).
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

    /// The CNF `DV_EHR_URI` §"validate_open" fixtures
    /// (`master17.7-content_tc_data_types-uri.adoc`), adjudicated against the
    /// released grammar.
    ///
    /// Each fixture that reaches beyond the EHR omits the `EHR` attribute name,
    /// putting the versioned-object id directly after the `ehr_id`. BASE
    /// `master11-paths.adoc` §"EHR Reference URIs" enumerates the locator's
    /// values ("come from attribute names of the class `EHR` … namely
    /// `compositions`, `directory` etc.") and every example in
    /// §"Top-level Structure Locator" and §"Item URIs" carries one, so those
    /// fixtures are refused and their attribute-bearing twins accepted. The CNF
    /// material is a stalled guide, never the oracle — the released spec wins.
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

        // The locator's attribute is mandatory (master11 §"EHR Reference URIs"
        // enumerates the values), so an OVID directly after the ehr_id names no
        // top-level structure.
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
        // The item-path form normalises `[archetype_id=…]` to the bare shortcut,
        // so it round-trips *structurally* (parse == parse∘format∘parse) even
        // when not byte-identical.
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

    /// The two node-id forms of `LOCATABLE.archetype_node_id` are disjoint and
    /// exhaustive over the shapes the RM defines: an interior term code
    /// (`locatable.adoc` §Attributes, "Always in the form of an at-code, e.g.
    /// `at0005`"; AM `ADL2` `master02-overview.adoc` for the id-code
    /// alternative) and an archetype-root `ARCHETYPE_ID` (BASE
    /// `archetype_id.adoc` §Description, the lexical form).
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

    /// Neither predicate fires on a string that is in no RM node-id form —
    /// a bare word, an at-leader with no digits, or a truncated archetype id
    /// missing the RM qualifier or the `.vN` segment.
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
