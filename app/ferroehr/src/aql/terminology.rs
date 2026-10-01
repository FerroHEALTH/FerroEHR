// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `TERMINOLOGY('expand', …)` `matches`-operand expansion (B4 stage (a)).
//!
//! master03 §TERMINOLOGY (`docs/specs/openehr/QUERY/docs/AQL/master03-syntax.adoc`
//! lines 748–767) allows a `TERMINOLOGY()` call as — or *inside* — the
//! right-hand operand of `matches`, "for merging explicit codes with the
//! function results (in which case the AQL interpreter is responsible for
//! generating a valid list of codes during semantic analysis)":
//!
//! ```text
//! WHERE …/code_string matches TERMINOLOGY('expand', 'hl7.org/fhir/4.0', '<vs-url>')
//! WHERE …/code_string matches {'http://snomed.info/id/442031002',
//!                              TERMINOLOGY('expand', 'hl7.org/fhir/4.0', '<vs-url>')}
//! ```
//!
//! This module is the semantic-analysis pre-pass that realises the merge: it
//! resolves each `expand` call through a [`TerminologyExpander`] (the SM
//! `I_TERMINOLOGY_SERVICE` seam, the in-process `openehr-term` bundle by default
//! and a remote FHIR TS when configured) and rewrites the AST value list in
//! place to explicit string codes before planning and SQL generation, so the
//! planner (`super::lower`) sees an ordinary `matches { … }` value list.
//!
//! All three master03 §TERMINOLOGY usage forms are realised here:
//!
//! * `expand` as, or inside, a `matches` operand, merged into the value list;
//! * the Boolean value expression (`TERMINOLOGY('validate', …) = true`),
//!   evaluated once over its constant string arguments and replaced with its
//!   truth value;
//! * the terminology-URI operand (`matches { terminology://… }`, master03
//!   §matches/URI), expanded through the same seam into an explicit value list.
//!
//! Operations with no defined comparison semantics in AQL (`lookup`, `map`)
//! remain typed rejects at the seam implementation.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use openehr_query::ast::{
    CompareOperand, FunctionCall, IdentifiedExpr, MatchesOperand, Primitive, SelectQuery, Terminal,
    TerminologyFunction, ValueListItem, WhereExpr,
};
use openehr_query::lexer::CompOp;

use super::error::AqlError;

/// The only `TERMINOLOGY()` operation supported inside a `matches` operand
/// (case-insensitive): "expand: … retrieve all the codes contained in a value
/// set as an explicit set" (master03 §TERMINOLOGY).
const EXPAND: &str = "expand";

/// The resolver seam for `TERMINOLOGY('expand', …)`: turns a `(service_api,
/// params_uri)` pair into the explicit list of codes of the referenced value
/// set.
///
/// Implemented by the application service over the SM `I_TERMINOLOGY_SERVICE`
/// providers.
#[async_trait]
pub trait TerminologyExpander: Send + Sync {
    /// Expand the value set identified by `params_uri` via the terminology
    /// service selected by `service_api`, returning its codes.
    ///
    /// # Errors
    ///
    /// * [`AqlError::Feature`] with `UnknownTerminologyService` — `service_api`
    ///   names no configured service; or `TerminologyValueSetNotFound` —
    ///   `params_uri` names no known value set (both → 400).
    /// * [`AqlError::Exec`] with `ExecError::Terminology` — the upstream
    ///   terminology server call failed (→ 500).
    async fn expand(&self, service_api: &str, params_uri: &str) -> Result<Vec<String>, AqlError>;

    /// Evaluate a Boolean `TERMINOLOGY()` operation (`validate`, `subsumes`)
    /// via the terminology service selected by `service_api`.
    ///
    /// # Errors
    ///
    /// As [`TerminologyExpander::expand`]; an operation with no boolean
    /// semantics (`lookup`, `map`) is a typed 400.
    async fn boolean_operation(
        &self,
        operation: &str,
        service_api: &str,
        params_uri: &str,
    ) -> Result<bool, AqlError>;

    /// Expand a terminology URI operand (`matches { terminology://… }` —
    /// master03 §matches/URI) into the set of codes it identifies.
    ///
    /// # Errors
    ///
    /// As [`TerminologyExpander::expand`].
    async fn expand_uri(&self, uri: &str) -> Result<Vec<String>, AqlError>;
}

/// Resolve and merge every `TERMINOLOGY('expand', …)` used as (or inside) a
/// `matches` operand in `query`'s `WHERE` clause, rewriting the value lists
/// in place to explicit string codes.
///
/// A no-op when the query has no `WHERE` clause or no `expand` operand.
///
/// Each distinct `(service_api, params_uri)` is resolved exactly once. Non-
/// `expand` `TERMINOLOGY()` operands are left untouched (the planner rejects
/// them).
///
/// Returns `true` when at least one terminology operand was resolved and
/// spliced into the AST — i.e. the lowered plan now embeds a terminology
/// resolution that may differ on a later execution, so the query service must
/// **not** cache it (`crate::service::query` plan cache). Returns `false` for a
/// query with no terminology operand (a pure function of the query text, safe
/// to cache).
///
/// # Errors
///
/// Propagates the [`TerminologyExpander::expand`] error for the first operand
/// that fails to resolve.
pub async fn expand_matches(
    query: &mut SelectQuery,
    expander: &(impl TerminologyExpander + ?Sized),
) -> Result<bool, AqlError> {
    let Some(where_) = query.where_.as_ref() else {
        return Ok(false);
    };

    // Pass 1 (sync): collect the distinct terminology requests.
    let mut requests: BTreeSet<Request> = BTreeSet::new();
    collect(where_, &mut requests);
    if requests.is_empty() {
        return Ok(false);
    }

    // Resolve each request exactly once (async, off the sync borrow).
    let mut resolved: BTreeMap<Request, Resolution> = BTreeMap::new();
    for request in requests {
        let resolution = match &request {
            Request::Expand(api, uri) => Resolution::Codes(expander.expand(api, uri).await?),
            Request::Boolean(op, api, uri) => {
                Resolution::Bool(expander.boolean_operation(op, api, uri).await?)
            }
            Request::Uri(uri) => Resolution::Codes(expander.expand_uri(uri).await?),
        };
        resolved.insert(request, resolution);
    }

    // Pass 2 (sync): splice the resolutions into the WHERE tree.
    if let Some(where_) = query.where_.as_mut() {
        rewrite(where_, &resolved);
    }
    Ok(true)
}

/// One distinct terminology-service request found in the `WHERE` clause.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Request {
    /// `TERMINOLOGY('expand', api, uri)` as/inside a `matches` operand.
    Expand(String, String),
    /// `TERMINOLOGY(op, api, uri) = <bool>` — the Boolean value expression.
    Boolean(String, String, String),
    /// `matches { <terminology uri> }`.
    Uri(String),
}

/// A resolved request.
enum Resolution {
    Codes(Vec<String>),
    Bool(bool),
}

/// Whether a `TERMINOLOGY()` call is an `expand` operation (case-insensitive).
fn is_expand(tf: &TerminologyFunction) -> bool {
    tf.operation.eq_ignore_ascii_case(EXPAND)
}

/// The resolution key for an `expand` call: `(service_api, params_uri)`.
fn key(tf: &TerminologyFunction) -> (String, String) {
    (tf.arg2.clone(), tf.arg3.clone())
}

/// Collect the distinct terminology requests reachable from a `WHERE` sub-tree.
fn collect(expr: &WhereExpr, out: &mut BTreeSet<Request>) {
    match expr {
        WhereExpr::Identified(IdentifiedExpr::Matches { operand, .. }, _) => match operand {
            MatchesOperand::Terminology(tf) if is_expand(tf) => {
                let (api, uri) = key(tf);
                out.insert(Request::Expand(api, uri));
            }
            MatchesOperand::ValueList(items) => {
                for tf in items.iter().filter_map(expand_item) {
                    let (api, uri) = key(tf);
                    out.insert(Request::Expand(api, uri));
                }
            }
            MatchesOperand::Uri(uri) => {
                out.insert(Request::Uri(uri.clone()));
            }
            MatchesOperand::Terminology(_) => {}
        },
        WhereExpr::Identified(ie, _) => {
            if let Some((tf, _, _)) = boolean_form(ie) {
                let (api, uri) = key(tf);
                out.insert(Request::Boolean(tf.operation.clone(), api, uri));
            }
        }
        WhereExpr::Not(w) => collect(w, out),
        WhereExpr::And(a, b) | WhereExpr::Or(a, b) => {
            collect(a, out);
            collect(b, out);
        }
    }
}

/// The Boolean value-expression form `TERMINOLOGY(op, api, uri) <=|!=> <bool>`
/// (master03 §TERMINOLOGY, third usage): the call, the operator, and the
/// boolean literal.
fn boolean_form(ie: &IdentifiedExpr) -> Option<(&TerminologyFunction, CompOp, bool)> {
    let IdentifiedExpr::Compare {
        lhs: CompareOperand::Function(FunctionCall::Terminology(tf)),
        op: op @ (CompOp::Eq | CompOp::Ne),
        rhs: Terminal::Primitive(Primitive::Boolean(b)),
    } = ie
    else {
        return None;
    };
    (!is_expand(tf)).then_some((tf, *op, *b))
}

/// A value-list item that is an `expand` `TERMINOLOGY()` call, if it is one.
fn expand_item(item: &ValueListItem) -> Option<&TerminologyFunction> {
    match item {
        ValueListItem::Terminology(tf) if is_expand(tf) => Some(tf),
        _ => None,
    }
}

/// Rewrite a `WHERE` sub-tree, replacing resolved terminology operands: value
/// lists gain explicit codes; a resolved Boolean value expression becomes its
/// constant truth value.
fn rewrite(expr: &mut WhereExpr, resolved: &BTreeMap<Request, Resolution>) {
    match expr {
        WhereExpr::Identified(IdentifiedExpr::Matches { operand, .. }, _) => {
            rewrite_operand(operand, resolved);
        }
        WhereExpr::Identified(ie, _) => {
            if let Some((tf, op, rhs)) = boolean_form(ie) {
                let (api, uri) = key(tf);
                let request = Request::Boolean(tf.operation.clone(), api, uri);
                if let Some(Resolution::Bool(value)) = resolved.get(&request) {
                    let truth = match op {
                        CompOp::Eq => *value == rhs,
                        _ => *value != rhs,
                    };
                    *ie = IdentifiedExpr::Resolved(truth);
                }
            }
        }
        WhereExpr::Not(w) => rewrite(w, resolved),
        WhereExpr::And(a, b) | WhereExpr::Or(a, b) => {
            rewrite(a, resolved);
            rewrite(b, resolved);
        }
    }
}

/// Rewrite a single `matches` operand: a standalone `expand` call becomes a
/// value list of its codes; a value list has each `expand` item spliced into
/// its codes (explicit codes and non-`expand` items preserved in order).
fn rewrite_operand(operand: &mut MatchesOperand, resolved: &BTreeMap<Request, Resolution>) {
    let codes_of = |request: &Request| match resolved.get(request) {
        Some(Resolution::Codes(codes)) => Some(codes),
        _ => None,
    };
    match operand {
        MatchesOperand::Terminology(tf) if is_expand(tf) => {
            let (api, uri) = key(tf);
            *operand = MatchesOperand::ValueList(code_items(codes_of(&Request::Expand(api, uri))));
        }
        MatchesOperand::ValueList(items) => {
            let taken = std::mem::take(items);
            let mut rebuilt = Vec::with_capacity(taken.len());
            for item in taken {
                match expand_item(&item) {
                    Some(tf) => {
                        let (api, uri) = key(tf);
                        rebuilt.extend(code_items(codes_of(&Request::Expand(api, uri))));
                    }
                    None => rebuilt.push(item),
                }
            }
            *items = rebuilt;
        }
        MatchesOperand::Uri(uri) => {
            let request = Request::Uri(uri.clone());
            *operand = MatchesOperand::ValueList(code_items(codes_of(&request)));
        }
        MatchesOperand::Terminology(_) => {}
    }
}

/// The resolved codes as string-primitive value-list items (empty when the
/// expansion produced no codes — the `matches` then matches nothing).
fn code_items(codes: Option<&Vec<String>>) -> Vec<ValueListItem> {
    codes
        .into_iter()
        .flatten()
        .map(|c| ValueListItem::Primitive(Primitive::String(c.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_query::parser::parse_str;

    /// A canned expander: every `expand` returns a fixed code list.
    struct Fixed(Vec<String>);

    #[async_trait]
    impl TerminologyExpander for Fixed {
        async fn expand(&self, _api: &str, _uri: &str) -> Result<Vec<String>, AqlError> {
            Ok(self.0.clone())
        }

        async fn boolean_operation(
            &self,
            operation: &str,
            _api: &str,
            _uri: &str,
        ) -> Result<bool, AqlError> {
            // The canned rule: `validate` is true, `subsumes` is false.
            Ok(operation.eq_ignore_ascii_case("validate"))
        }

        async fn expand_uri(&self, _uri: &str) -> Result<Vec<String>, AqlError> {
            Ok(self.0.clone())
        }
    }

    fn matches_values(q: &SelectQuery) -> Vec<String> {
        // Descend to the single WHERE matches operand and read its value list.
        let Some(WhereExpr::Identified(IdentifiedExpr::Matches { operand, .. }, _)) =
            q.where_.as_ref()
        else {
            panic!("expected a WHERE matches");
        };
        let MatchesOperand::ValueList(items) = operand else {
            panic!("expected a value list after expansion, got {operand:?}");
        };
        items
            .iter()
            .map(|i| match i {
                ValueListItem::Primitive(Primitive::String(s)) => s.clone(),
                other => panic!("expected a string primitive, got {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn standalone_expand_becomes_a_value_list() {
        let mut q = parse_str(
            "SELECT c FROM COMPOSITION c \
             WHERE c/category/defining_code/code_string \
             matches TERMINOLOGY('expand', 'openehr', 'audit_change_type')",
        )
        .expect("parse");
        let expanded = expand_matches(&mut q, &Fixed(vec!["249".into(), "250".into()]))
            .await
            .expect("expand");
        assert!(
            expanded,
            "a resolved terminology operand must report expansion (the plan is then uncacheable)"
        );
        assert_eq!(matches_values(&q), vec!["249".to_owned(), "250".to_owned()]);
    }

    #[tokio::test]
    async fn mixed_list_merges_explicit_and_expanded_codes_in_order() {
        let mut q = parse_str(
            "SELECT c FROM COMPOSITION c \
             WHERE c/category/defining_code/code_string \
             matches {'X', TERMINOLOGY('expand', 'openehr', 'g'), 'Y'}",
        )
        .expect("parse");
        expand_matches(&mut q, &Fixed(vec!["A".into(), "B".into()]))
            .await
            .expect("expand");
        assert_eq!(
            matches_values(&q),
            vec![
                "X".to_owned(),
                "A".to_owned(),
                "B".to_owned(),
                "Y".to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn no_terminology_operand_is_a_no_op() {
        let src = "SELECT c FROM COMPOSITION c \
             WHERE c/category/defining_code/code_string matches {'249', '250'}";
        let mut q = parse_str(src).expect("parse");
        let before = q.clone();
        let expanded = expand_matches(&mut q, &Fixed(vec!["ignored".into()]))
            .await
            .expect("no-op");
        assert!(
            !expanded,
            "a query with no terminology operand must report no expansion (cacheable)"
        );
        assert_eq!(
            q, before,
            "a query with no expand operand must be untouched"
        );
    }

    #[tokio::test]
    async fn empty_expansion_yields_an_empty_value_list() {
        let mut q = parse_str(
            "SELECT c FROM COMPOSITION c \
             WHERE c/category/defining_code/code_string \
             matches TERMINOLOGY('expand', 'openehr', 'g')",
        )
        .expect("parse");
        expand_matches(&mut q, &Fixed(vec![]))
            .await
            .expect("expand");
        assert!(matches_values(&q).is_empty());
    }

    #[tokio::test]
    async fn boolean_validate_form_resolves_to_a_constant() {
        // QUERY master03 §TERMINOLOGY third usage: the Boolean value
        // expression is evaluated at semantic analysis.
        let mut q = parse_str(
            "SELECT c FROM COMPOSITION c WHERE \
             TERMINOLOGY('validate', 'hl7.org/fhir/r4', \
             'system=http://snomed.info/sct&code=122298005&url=http://snomed.info/sct?fhir_vs') \
             = true",
        )
        .expect("parses");
        expand_matches(&mut q, &Fixed(vec![]))
            .await
            .expect("resolves");
        assert_eq!(
            q.where_,
            Some(WhereExpr::identified(IdentifiedExpr::Resolved(true))),
            "validate=true with a true evaluation is the constant TRUE"
        );
    }

    #[tokio::test]
    async fn boolean_form_honours_the_operator_and_literal() {
        let mut q = parse_str(
            "SELECT c FROM COMPOSITION c WHERE \
             TERMINOLOGY('subsumes', 'hl7.org/fhir/r4', 'system=s&codeA=a&codeB=b') != false",
        )
        .expect("parses");
        expand_matches(&mut q, &Fixed(vec![]))
            .await
            .expect("resolves");
        // subsumes evaluates false; `false != false` is false.
        assert_eq!(
            q.where_,
            Some(WhereExpr::identified(IdentifiedExpr::Resolved(false)))
        );
    }

    #[tokio::test]
    async fn uri_operand_expands_to_a_value_list() {
        // QUERY master03 §matches/URI: the terminology URI identifies a set;
        // matching is membership of its expansion.
        let mut q = parse_str(
            "SELECT c FROM COMPOSITION c WHERE \
             c/context/setting/defining_code/code_string matches \
             { terminology://snomed-ct/hierarchy?rootConceptId=50043002 }",
        )
        .expect("parses");
        expand_matches(&mut q, &Fixed(vec!["50043002".into(), "12345".into()]))
            .await
            .expect("resolves");
        assert_eq!(matches_values(&q), vec!["50043002", "12345"]);
    }
}
