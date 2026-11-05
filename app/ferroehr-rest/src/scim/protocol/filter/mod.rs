// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM filter (RFC 7644 §3.4.2.2): the whole grammar of Figure 1, its
//! evaluation over a resource, and the subset a provider supports.
//!
//! [`parse`] reads every operator of Table 3, the logical operators of Table
//! 4, grouping and value paths of Table 5, with the precedence the section
//! orders (grouping, then `not`, `and`, `or`). Attribute names and operators
//! are case-insensitive. An unrecognised operator is refused with
//! `invalidFilter` and the message the section suggests. A provider that
//! supports less declares it with [`FilterSupport`], whose
//! [`FilterSupport::check`] refuses the rest the same way.
//!
//! A chain of one logical operator is one flat [`Logic::And`] or
//! [`Logic::Or`], so the depth of a filter is its nesting of groups and value
//! paths, which the parser bounds.

pub(crate) mod eval;
mod lexer;
mod parser;
mod support;

use std::fmt;

use serde_json::Number;

use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::path::{AttrName, AttrPath};

/// A logical combination of leaf expressions (RFC 7644 Table 4).
#[derive(Debug, Clone, PartialEq)]
pub enum Logic<T> {
    /// One expression.
    Leaf(T),
    /// Every expression holds.
    And(Vec<Logic<T>>),
    /// At least one expression holds.
    Or(Vec<Logic<T>>),
    /// The expression does not hold.
    Not(Box<Logic<T>>),
}

/// An attribute expression, `attrPath SP "pr"` or `attrPath SP compareOp SP
/// compValue` (RFC 7644 Figure 1).
#[derive(Debug, Clone, PartialEq)]
pub struct AttrExp<P> {
    /// The attribute, an [`AttrPath`] in a filter and an [`AttrName`] inside
    /// a value path.
    pub path: P,
    /// What the attribute is tested for.
    pub test: Test,
}

/// The test an attribute expression applies.
#[derive(Debug, Clone, PartialEq)]
pub enum Test {
    /// `pr`: the attribute has a value.
    Present,
    /// A comparison with a value.
    Compare(CompareOp, CompValue),
}

/// A comparison operator (RFC 7644 Table 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    /// `eq`, equal.
    Eq,
    /// `ne`, not equal.
    Ne,
    /// `co`, contains.
    Co,
    /// `sw`, starts with.
    Sw,
    /// `ew`, ends with.
    Ew,
    /// `gt`, greater than.
    Gt,
    /// `lt`, less than.
    Lt,
    /// `ge`, greater than or equal to.
    Ge,
    /// `le`, less than or equal to.
    Le,
}

impl CompareOp {
    /// Every operator, in the order of Table 3.
    pub const ALL: [Self; 9] = [
        Self::Eq,
        Self::Ne,
        Self::Co,
        Self::Sw,
        Self::Ew,
        Self::Gt,
        Self::Lt,
        Self::Ge,
        Self::Le,
    ];

    /// Returns the operator as Table 3 spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Ne => "ne",
            Self::Co => "co",
            Self::Sw => "sw",
            Self::Ew => "ew",
            Self::Gt => "gt",
            Self::Lt => "lt",
            Self::Ge => "ge",
            Self::Le => "le",
        }
    }

    /// Returns the operator `word` names in any case (RFC 7644 §3.4.2.2).
    fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|op| op.as_str().eq_ignore_ascii_case(word))
    }
}

/// A comparison value, `false / null / true / number / string` (RFC 7644
/// Figure 1, "rules from JSON").
#[derive(Debug, Clone, PartialEq)]
pub enum CompValue {
    /// `false`.
    False,
    /// `null`.
    Null,
    /// `true`.
    True,
    /// A JSON number.
    Number(Number),
    /// A JSON string, unescaped.
    String(String),
}

/// A leaf of a filter: an attribute expression or a value path.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    /// An attribute expression.
    Attr(AttrExp<AttrPath>),
    /// `attrPath "[" valFilter "]"`: the values of a multi-valued complex
    /// attribute that a filter over their sub-attributes selects.
    ValuePath(AttrPath, ValueFilter),
}

/// A filter (`FILTER` of RFC 7644 Figure 1).
pub type Filter = Logic<Term>;

/// The filter inside a value path (`valFilter`), over sub-attribute names.
pub type ValueFilter = Logic<AttrExp<AttrName>>;

/// Parses a filter.
///
/// # Errors
///
/// Returns a `400` with `scimType` `invalidFilter` when `text` is not a
/// filter of RFC 7644 Figure 1, names an operator the grammar does not know
/// ("The operator 'regex' is not supported."), or nests groups deeper than
/// the parser allows.
pub fn parse(text: &str) -> Result<Filter, ScimError> {
    let invalid = |message: String| ScimError::bad_request(ScimType::InvalidFilter, message);
    let tokens = lexer::lex(text).map_err(|error| {
        invalid(format!(
            "Invalid filter: {} at byte {}.",
            error.message, error.at
        ))
    })?;
    let mut parser = parser::Parser::new(tokens, 0);
    let filter = parser
        .logic::<Term>()
        .map_err(|error| invalid(error.to_string()))?;
    parser
        .finish()
        .map_err(|error| invalid(error.to_string()))?;
    Ok(filter)
}

/// The filter features and attributes a provider supports.
///
/// [`FilterSupport::check`] refuses a parsed filter that uses anything else
/// (RFC 7644 §3.4.2.2: "Providers MUST decline to filter results if the
/// specified filter operation is not recognized").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSupport {
    base_schema: String,
    features: Vec<FilterFeature>,
    attributes: Vec<AttrPath>,
}

/// A filter feature a [`FilterSupport`] may add beyond equality and conjunction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterFeature {
    /// A comparison operator.
    Operator(CompareOp),
    /// The `pr` test.
    Present,
    /// The `or` operator.
    Or,
    /// The `not` operator.
    Not,
    /// Value paths, `attr[valFilter]`.
    ValuePaths,
}

impl<T: fmt::Display> fmt::Display for Logic<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Leaf(leaf) => write!(f, "{leaf}"),
            Self::And(terms) => write_joined(f, terms, " and ", |term| matches!(term, Self::Or(_))),
            Self::Or(terms) => write_joined(f, terms, " or ", |_term| false),
            Self::Not(inner) => write!(f, "not ({inner})"),
        }
    }
}

/// Writes `terms` separated by `separator`, grouping each term `grouped`
/// selects; `and` binds tighter than `or`, so only an `or` inside an `and`
/// needs its brackets.
fn write_joined<T: fmt::Display>(
    f: &mut fmt::Formatter<'_>,
    terms: &[Logic<T>],
    separator: &str,
    grouped: impl Fn(&Logic<T>) -> bool,
) -> fmt::Result {
    for (index, term) in terms.iter().enumerate() {
        if index > 0 {
            f.write_str(separator)?;
        }
        if grouped(term) {
            write!(f, "({term})")?;
        } else {
            write!(f, "{term}")?;
        }
    }
    Ok(())
}

impl<P: fmt::Display> fmt::Display for AttrExp<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.test {
            Test::Present => write!(f, "{} pr", self.path),
            Test::Compare(op, value) => write!(f, "{} {} {value}", self.path, op.as_str()),
        }
    }
}

impl fmt::Display for CompValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::False => f.write_str("false"),
            Self::Null => f.write_str("null"),
            Self::True => f.write_str("true"),
            Self::Number(number) => write!(f, "{number}"),
            Self::String(text) => {
                f.write_str(&serde_json::to_string(text).map_err(|_error| fmt::Error)?)
            }
        }
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attr(expression) => write!(f, "{expression}"),
            Self::ValuePath(attr, filter) => write!(f, "{attr}[{filter}]"),
        }
    }
}
