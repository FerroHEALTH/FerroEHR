// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The recursive descent over the filter tokens (RFC 7644 §3.4.2.2).
//!
//! `logic` reads `or` over `and` over a unary term, the precedence the
//! section orders; a unary term is `not (...)`, a group `(...)` or a leaf. A
//! word in the position of a leaf is always an attribute, so an attribute
//! named `and` or `not` stays readable. Groups, `not` and value paths count
//! towards [`MAX_DEPTH`], which keeps the recursion bounded on any input.

use std::fmt;

use serde_json::Number;

use super::lexer::{Kind, Token, lex};
use super::{AttrExp, CompValue, CompareOp, Logic, Term, Test};
use crate::scim::protocol::deviation;
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::path::{AttrName, AttrPath, PatchPath};

/// The deepest nesting of groups, `not` and value paths a filter may hold;
/// no specification bounds it, so the bound is our own design.
const MAX_DEPTH: usize = 32;

/// Why a token sequence is not a filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SyntaxError {
    /// A word stands where an operator does and names none of Table 3.
    UnknownOperator(String),
    /// Something else stands where `what` does; `at` is the byte offset, or
    /// `None` at the end of the filter.
    Expected {
        what: &'static str,
        at: Option<usize>,
    },
    /// The filter nests deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A word stands where an attribute path does and is not one.
    NotAPath(String),
    /// A word inside a value path is not a sub-attribute name.
    NotASubAttribute(String),
    /// A value path holds another, which a complex attribute's
    /// sub-attributes cannot (RFC 7643 §2.3.8).
    NestedValuePath,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // NOTE: RFC 7644 §3.4.2.2 suggests this message: "The operator 'regex' is not supported."
            Self::UnknownOperator(operator) => {
                write!(f, "The operator '{operator}' is not supported.")
            }
            Self::Expected { what, at: Some(at) } => {
                write!(f, "Invalid filter: expected {what} at byte {at}.")
            }
            Self::Expected { what, at: None } => {
                write!(f, "Invalid filter: expected {what}, but the filter ended.")
            }
            Self::TooDeep => write!(
                f,
                "Invalid filter: groups nest deeper than {MAX_DEPTH} levels."
            ),
            Self::NotAPath(word) => write!(f, "Invalid filter: '{word}' is not an attribute path."),
            Self::NotASubAttribute(word) => {
                write!(
                    f,
                    "Invalid filter: '{word}' is not a sub-attribute name inside a value path."
                )
            }
            Self::NestedValuePath => {
                f.write_str("Invalid filter: a value path cannot hold another value path.")
            }
        }
    }
}

/// A kind of leaf the logical operators combine.
pub(super) trait Leaf: Sized {
    /// Reads one leaf at the parser's position.
    fn leaf(parser: &mut Parser<'_>) -> Result<Self, SyntaxError>;
}

/// The state of one descent: the tokens, the position and the nesting depth.
#[derive(Debug)]
pub(super) struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    at: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    /// Starts a descent over `tokens` already `depth` levels deep.
    pub(super) const fn new(tokens: Vec<Token<'a>>, depth: usize) -> Self {
        Self {
            tokens,
            at: 0,
            depth,
        }
    }

    /// Reads `or` terms (the loosest binding).
    pub(super) fn logic<T: Leaf>(&mut self) -> Result<Logic<T>, SyntaxError> {
        let first = self.conjunction::<T>()?;
        if !self.at_keyword("or") {
            return Ok(first);
        }
        let mut terms = vec![first];
        while self.eat_keyword("or") {
            terms.push(self.conjunction::<T>()?);
        }
        Ok(Logic::Or(terms))
    }

    /// Refuses any token left after the filter.
    pub(super) fn finish(&self) -> Result<(), SyntaxError> {
        match self.tokens.get(self.at) {
            None => Ok(()),
            Some(token) => Err(SyntaxError::Expected {
                what: "the end of the filter",
                at: Some(token.at),
            }),
        }
    }

    fn conjunction<T: Leaf>(&mut self) -> Result<Logic<T>, SyntaxError> {
        let first = self.unary::<T>()?;
        if !self.at_keyword("and") {
            return Ok(first);
        }
        let mut terms = vec![first];
        while self.eat_keyword("and") {
            terms.push(self.unary::<T>()?);
        }
        Ok(Logic::And(terms))
    }

    fn unary<T: Leaf>(&mut self) -> Result<Logic<T>, SyntaxError> {
        if self.at_keyword("not") && matches!(self.kind(1), Some(Kind::Open)) {
            self.at += 2;
            return Ok(Logic::Not(Box::new(self.group::<T>()?)));
        }
        if matches!(self.kind(0), Some(Kind::Open)) {
            self.at += 1;
            return self.group::<T>();
        }
        Ok(Logic::Leaf(T::leaf(self)?))
    }

    /// Reads the inside of a group whose `(` is consumed, and its `)`.
    fn group<T: Leaf>(&mut self) -> Result<Logic<T>, SyntaxError> {
        self.enter()?;
        let inner = self.logic::<T>()?;
        self.expect(&Kind::Close, "')'")?;
        self.leave();
        Ok(inner)
    }

    fn enter(&mut self) -> Result<(), SyntaxError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(SyntaxError::TooDeep)
        } else {
            Ok(())
        }
    }

    const fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn kind(&self, ahead: usize) -> Option<&Kind<'a>> {
        self.tokens.get(self.at + ahead).map(|token| &token.kind)
    }

    fn position(&self) -> Option<usize> {
        self.tokens.get(self.at).map(|token| token.at)
    }

    fn at_keyword(&self, keyword: &str) -> bool {
        matches!(self.kind(0), Some(Kind::Word(word)) if word.eq_ignore_ascii_case(keyword))
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        let found = self.at_keyword(keyword);
        if found {
            self.at += 1;
        }
        found
    }

    fn eat(&mut self, kind: &Kind<'_>) -> bool {
        let found = self.kind(0) == Some(kind);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, kind: &Kind<'_>, what: &'static str) -> Result<(), SyntaxError> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(SyntaxError::Expected {
                what,
                at: self.position(),
            })
        }
    }

    fn word(&mut self, what: &'static str) -> Result<&'a str, SyntaxError> {
        match self.tokens.get(self.at) {
            Some(Token {
                kind: Kind::Word(word),
                ..
            }) => {
                let word = *word;
                self.at += 1;
                Ok(word)
            }
            _ => Err(SyntaxError::Expected {
                what,
                at: self.position(),
            }),
        }
    }

    /// Reads `"pr"` or `compareOp SP compValue`.
    fn test(&mut self) -> Result<Test, SyntaxError> {
        let word = self.word("an operator")?;
        if word.eq_ignore_ascii_case("pr") {
            return Ok(Test::Present);
        }
        let op =
            CompareOp::parse(word).ok_or_else(|| SyntaxError::UnknownOperator(word.to_owned()))?;
        Ok(Test::Compare(op, self.value()?))
    }

    /// Reads a `compValue`: a JSON string, literal name or number (RFC 8259 §3, §6).
    fn value(&mut self) -> Result<CompValue, SyntaxError> {
        let at = self.position();
        let expected = SyntaxError::Expected {
            what: "a comparison value",
            at,
        };
        let value = match self.tokens.get(self.at).map(|token| &token.kind) {
            Some(Kind::Str(text)) => CompValue::String(text.clone()),
            Some(Kind::Word("true")) => CompValue::True,
            Some(Kind::Word("false")) => CompValue::False,
            Some(Kind::Word("null")) => CompValue::Null,
            Some(Kind::Word(word)) => {
                // NOTE: a word that is not a JSON number is legitimately no value, refused as the grammar asks.
                CompValue::Number(
                    serde_json::from_str::<Number>(word).map_err(|_error| expected.clone())?,
                )
            }
            _ => return Err(expected),
        };
        self.at += 1;
        Ok(value)
    }
}

impl Leaf for Term {
    fn leaf(parser: &mut Parser<'_>) -> Result<Self, SyntaxError> {
        let word = parser.word("an attribute path")?;
        let path =
            AttrPath::parse(word).map_err(|_error| SyntaxError::NotAPath(word.to_owned()))?;
        if !parser.eat(&Kind::OpenBracket) {
            return Ok(Self::Attr(AttrExp {
                path,
                test: parser.test()?,
            }));
        }
        parser.enter()?;
        let filter = parser.logic::<AttrExp<AttrName>>()?;
        parser.expect(&Kind::CloseBracket, "']'")?;
        parser.leave();
        let sub_attr = match parser.kind(0) {
            Some(Kind::Word(word)) if word.starts_with('.') => {
                let word = parser.word("a sub-attribute")?;
                let name = word.strip_prefix('.').and_then(AttrName::parse);
                Some(name.ok_or_else(|| SyntaxError::NotASubAttribute(word.to_owned()))?)
            }
            _ => None,
        };
        let Some(sub_attr) = sub_attr else {
            return Ok(Self::ValuePath(path, filter));
        };
        let test = parser.test()?;
        Ok(Self::ValuePath(
            path,
            deviation::value_path_comparison(
                filter,
                AttrExp {
                    path: sub_attr,
                    test,
                },
            ),
        ))
    }
}

impl Leaf for AttrExp<AttrName> {
    fn leaf(parser: &mut Parser<'_>) -> Result<Self, SyntaxError> {
        let word = parser.word("a sub-attribute name")?;
        let name =
            AttrName::parse(word).ok_or_else(|| SyntaxError::NotASubAttribute(word.to_owned()))?;
        if parser.kind(0) == Some(&Kind::OpenBracket) {
            return Err(SyntaxError::NestedValuePath);
        }
        Ok(Self {
            path: name,
            test: parser.test()?,
        })
    }
}

impl PatchPath {
    /// Parses a PATCH `path`, `attrPath / valuePath [subAttr]` (RFC 7644
    /// §3.5.2, Figure 7).
    ///
    /// # Errors
    ///
    /// Returns a `400` with `scimType` `invalidFilter` when the bracketed value
    /// filter is not a valid filter (RFC 7644 Table 9, "PATCH (Path Filter)"),
    /// and `invalidPath` for any other malformed path.
    pub fn parse(text: &str) -> Result<Self, ScimError> {
        let not_a_path = || {
            ScimError::bad_request(
                ScimType::InvalidPath,
                format!("'{text}' is not a PATCH path."),
            )
        };
        let mut tokens = lex(text).map_err(|_error| not_a_path())?.into_iter();
        let Some(Token {
            kind: Kind::Word(word),
            ..
        }) = tokens.next()
        else {
            return Err(not_a_path());
        };
        let attr = AttrPath::parse(word).map_err(|_error| not_a_path())?;
        let rest: Vec<Token<'_>> = tokens.collect();
        match rest.first() {
            None => return Ok(Self::Attribute(attr)),
            Some(token) if token.kind == Kind::OpenBracket => {}
            Some(_) => return Err(not_a_path()),
        }
        let close = rest
            .iter()
            .rposition(|token| token.kind == Kind::CloseBracket)
            .ok_or_else(not_a_path)?;
        let sub_attr = match rest.get(close + 1..) {
            Some([]) => None,
            Some(
                [
                    Token {
                        kind: Kind::Word(word),
                        ..
                    },
                ],
            ) => Some(
                word.strip_prefix('.')
                    .and_then(AttrName::parse)
                    .ok_or_else(not_a_path)?,
            ),
            _ => return Err(not_a_path()),
        };
        let inner = rest.get(1..close).ok_or_else(not_a_path)?.to_vec();
        let invalid_filter =
            |error: SyntaxError| ScimError::bad_request(ScimType::InvalidFilter, error.to_string());
        let mut parser = Parser::new(inner, 1);
        let filter = parser
            .logic::<AttrExp<AttrName>>()
            .map_err(invalid_filter)?;
        parser.finish().map_err(invalid_filter)?;
        Ok(Self::Values {
            attr,
            filter,
            sub_attr,
        })
    }
}
