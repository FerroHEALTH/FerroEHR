// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The federation-only `FROM ENDPOINT` / `ORGANISATION` directive, separated
//! from a query before the strict AQL 1.1.0 parse.
//!
//! A federation gateway accepts `FROM ENDPOINT p ["node_1", "node_2"] CONTAINS
//! EHR e …`, or the coarser `FROM ORGANISATION ["org-a"] CONTAINS …`, to pin
//! a query to named nodes. A node never receives it, and the strict
//! [`crate::parser`] rightly refuses it, so [`parse_federated`] lifts the
//! directive out at the token level and parses the remainder as ordinary AQL:
//! a directive spelled inside a string literal is a string, never a directive.
//! [`crate::printer::to_aql`] renders the remainder alone, which is the form a
//! node may receive; [`to_federated_aql`] renders the directive back in front
//! of it.
//!
//! The directive's own grammar, as accepted here:
//!
//! ```text
//! fromClause : FROM directive CONTAINS containsExpr | FROM containsExpr
//! directive  : (ENDPOINT | ORGANISATION) IDENTIFIER? '[' STRING (',' STRING)* ']'
//! ```
//!
//! with `ENDPOINT` and `ORGANISATION` case-insensitive like every AQL keyword.
//! The directive's variable (`p`) is kept; paths through it, such as a
//! `p/id` column, stay in the remaining query for the gateway to resolve.
//!
//! # Examples
//!
//! ```
//! use openehr_query::federation::{DirectiveKind, parse_federated};
//! use openehr_query::printer::to_aql;
//!
//! let federated = parse_federated(
//!     r#"SELECT c/uid/value FROM ENDPOINT p ["node_1", "node_2"] CONTAINS EHR e CONTAINS COMPOSITION c"#,
//! )?;
//! let directive = federated.directive.as_ref().ok_or("no directive")?;
//! assert_eq!(directive.kind, DirectiveKind::Endpoint);
//! assert_eq!(directive.ids, ["node_1", "node_2"]);
//! assert_eq!(to_aql(&federated.query), "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

// NOTE: no released openEHR specification defines this directive or its
// grammar — the shape follows the federation gateways that use it; our own extension.

use std::fmt::Write as _;

use crate::ast::{SelectQuery, Span};
use crate::lexer::{SpannedTokens, Token, lex_spanned};
use crate::parser::{ParseError, SyntaxFault, parse_spanned, unquote};
use crate::printer::{escape_string, render};

/// Which node selector a directive is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKind {
    /// `ENDPOINT` — the listed identifiers name endpoints.
    Endpoint,
    /// `ORGANISATION` — the listed identifiers name organisations, each
    /// standing for the endpoints it manages.
    Organisation,
}

/// A `FROM ENDPOINT` / `ORGANISATION` directive.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    /// The selector.
    pub kind: DirectiveKind,
    /// The variable the directive binds (`p` in `ENDPOINT p [ … ]`).
    pub variable: Option<String>,
    /// The listed identifiers, decoded, in the order written.
    pub ids: Vec<String>,
    /// Where the directive was written, keyword to closing bracket.
    pub span: Span,
}

/// A query as a federation gateway receives it: the directive, when the query
/// carries one, and the strict AQL query that remains.
#[derive(Debug, Clone, PartialEq)]
pub struct Federated {
    /// The directive, or `None` for a query without one.
    pub directive: Option<Directive>,
    /// The query with the directive and its `CONTAINS` removed.
    pub query: SelectQuery,
}

/// Lexes `src`, lifts a leading `FROM` directive out, and parses the rest as
/// strict AQL.
///
/// A query without a directive parses exactly as [`crate::parser::parse_str`]
/// parses it. Every span, in the directive and in the remaining query, is a
/// byte range of `src` itself.
///
/// # Errors
/// [`ParseError::Lex`] when `src` does not tokenize, and [`ParseError::Syntax`]
/// when the directive is malformed (an empty or unterminated list, an
/// identifier that is not a string, no `CONTAINS` after it) or the remaining
/// query is not AQL.
pub fn parse_federated(src: &str) -> Result<Federated, ParseError> {
    let stream = lex_spanned(src)?;
    let Some(from) = stream.tokens().iter().position(|t| *t == Token::From) else {
        return parse_spanned(&stream).map(|query| Federated {
            directive: None,
            query,
        });
    };
    let keyword = from.saturating_add(1);
    let kind = match stream.tokens().get(keyword) {
        Some(Token::Identifier(word)) if word.eq_ignore_ascii_case("endpoint") => {
            DirectiveKind::Endpoint
        }
        Some(Token::Identifier(word)) if word.eq_ignore_ascii_case("organisation") => {
            DirectiveKind::Organisation
        }
        _ => {
            return parse_spanned(&stream).map(|query| Federated {
                directive: None,
                query,
            });
        }
    };
    let (directive, contains) = directive(&stream, kind, keyword)?;
    let query = parse_spanned(&stream.without(keyword..contains.saturating_add(1)))?;
    Ok(Federated {
        directive: Some(directive),
        query,
    })
}

/// Reads the directive starting at token `keyword`, returning it and the index
/// of the `CONTAINS` that ends it.
fn directive(
    stream: &SpannedTokens,
    kind: DirectiveKind,
    keyword: usize,
) -> Result<(Directive, usize), ParseError> {
    let tokens = stream.tokens();
    let mut at = keyword.saturating_add(1);
    let variable = match tokens.get(at) {
        Some(Token::Identifier(name)) => {
            at = at.saturating_add(1);
            Some(name.clone())
        }
        _ => None,
    };
    expect(stream, at, &Token::LeftBracket)?;
    let mut ids = Vec::new();
    loop {
        at = at.saturating_add(1);
        match tokens.get(at) {
            Some(Token::String(raw)) => ids.push(unquote(raw)),
            _ => return Err(fault(stream, at)),
        }
        at = at.saturating_add(1);
        match tokens.get(at) {
            Some(Token::Comma) => {}
            Some(Token::RightBracket) => break,
            _ => return Err(fault(stream, at)),
        }
    }
    let close = at;
    let contains = close.saturating_add(1);
    if !matches!(stream.tokens().get(contains), Some(Token::Contains(_))) {
        return Err(fault(stream, contains));
    }
    let directive = Directive {
        kind,
        variable,
        ids,
        span: Span::new(stream.byte_span(&(keyword..close.saturating_add(1)))),
    };
    Ok((directive, contains))
}

/// Succeeds when the token at `at` is `wanted`.
fn expect(stream: &SpannedTokens, at: usize, wanted: &Token) -> Result<(), ParseError> {
    if stream.tokens().get(at) == Some(wanted) {
        Ok(())
    } else {
        Err(fault(stream, at))
    }
}

/// The syntax error for the token at `at`, located in the source.
fn fault(stream: &SpannedTokens, at: usize) -> ParseError {
    let tokens = at..at.saturating_add(1);
    ParseError::Syntax {
        faults: vec![SyntaxFault {
            bytes: Some(stream.byte_span(&tokens)),
            found: stream.tokens().get(at).cloned(),
            tokens,
        }],
    }
}

/// Renders a federated query: the directive, when there is one, in front of
/// the remaining query's containment expression.
///
/// The output re-parses through [`parse_federated`] to an equal [`Federated`].
#[must_use]
pub fn to_federated_aql(federated: &Federated) -> String {
    let Some(directive) = &federated.directive else {
        return render(&federated.query, None);
    };
    let mut prefix = String::from(match directive.kind {
        DirectiveKind::Endpoint => "ENDPOINT",
        DirectiveKind::Organisation => "ORGANISATION",
    });
    if let Some(variable) = &directive.variable {
        let _ = write!(prefix, " {variable}");
    }
    prefix.push_str(" [");
    for (i, id) in directive.ids.iter().enumerate() {
        if i > 0 {
            prefix.push_str(", ");
        }
        let _ = write!(prefix, "'{}'", escape_string(id));
    }
    prefix.push_str("] CONTAINS ");
    render(&federated.query, Some(&prefix))
}
