// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Typed `sea-query` building blocks shared across the SQL submodules, plus the
//! `LIKE` pattern conversion the AQL language pins exactly (QUERY master03
//! §Operators/LIKE). The `archetype_node_id` predicate is the `archetype`
//! module.
//!
//! No openEHR spec governs the lowering itself — this is our own design: every reference is a typed [`Expr::col`], every
//! literal binds through [`Expr::val`], and the PostgreSQL-specific pieces use
//! only the sanctioned typed escape hatches ([`Func::cust`] for functions
//! sea-query does not model; `PgFunc::any` for `= ANY(ARRAY[…])`;
//! [`BinOper::Custom`] for `#>>` / `->>` which have no
//! typed variant). Runtime functions resolve unqualified (the domain's own
//! `search_path`, which ends in `ext, public`).

use std::fmt::Write as _;

use sea_query::{Alias, BinOper, Expr, ExprTrait as _, Func, IntoIden, Value};

use openehr_query::lexer::CompOp;

use crate::aql::ir::{Coercion, EhrField, LeafPath, PathTarget, TypeSet, TypedLit, VersionField};
use crate::db::iden::Node;

/// A typed `"alias"."column"` reference, the column named through the schema
/// catalog ([`crate::db::iden`]) rather than as a string.
pub(super) fn col(alias: &str, column: impl IntoIden) -> Expr {
    Expr::col((Alias::new(alias), column.into_iden()))
}

/// A typed `"alias"."name"` reference to a DERIVED column — the output of a
/// subquery or of a set-returning function, which no schema relation declares
/// and the catalog therefore cannot name.
pub(super) fn derived_col(alias: &str, name: &str) -> Expr {
    Expr::col((Alias::new(alias), Alias::new(name)))
}

/// The hot-tier predicate on a partitioned relation, written as a LITERAL.
///
/// `version`, `node` and `vo_attestation` are partitioned by tier. A literal
/// lets PostgreSQL prune the cold partition at PLAN time rather than at
/// execution (PostgreSQL 18, "Partition Pruning",
/// <https://www.postgresql.org/docs/18/ddl-partitioning.html>), and AQL wants
/// exactly the hot tier: archived content leaves the queryable store until it
/// is restored. No openEHR spec governs storage tiering — our own design.
pub(super) fn hot(alias: &str, tier: impl IntoIden) -> Expr {
    col(alias, tier).eq(Expr::cust(HOT_TIER_LITERAL))
}

/// The hot-tier predicate on an UNALIASED relation inside a subquery.
pub(super) fn hot_unaliased() -> Expr {
    Expr::col(Node::Tier).eq(Expr::cust(HOT_TIER_LITERAL))
}

/// The tier AQL queries, as a SQL LITERAL. It is a constant of this module,
/// never a caller-supplied byte, which is what makes writing it into the
/// statement rather than binding it safe — and a bound parameter would move
/// pruning from plan time to execution time.
const HOT_TIER_LITERAL: &str = "'hot'";

/// A typed custom-function call `name(args...)`.
pub(super) fn call(name: &str, args: Vec<Expr>) -> Expr {
    let mut f = Func::cust(Alias::new(name));
    for a in args {
        f = f.arg(a);
    }
    Expr::from(f)
}

/// `to_jsonb(x)` — normalizes any scalar into a canonical-JSON cell.
pub(super) fn to_jsonb(e: Expr) -> Expr {
    call("to_jsonb", vec![e])
}

/// The member a fragment jsonpath reads first (`$.value.magnitude` →
/// `value`), `None` for the bare `$`.
fn first_member(jp: &str) -> Option<&str> {
    let rest = jp.strip_prefix("$.")?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    rest.get(..end).filter(|member| !member.is_empty())
}

/// The jsonb a fragment jsonpath `jp` reads off the node row `alias`: the
/// stored `data` with the member `jp` starts at put back where the row's
/// columns hold it (`crate::storage::slim`).
///
/// `value` comes from the typed value columns when the row has them, `name`,
/// `archetype_node_id`, `archetype_details`, `language` and `encoding` from
/// their columns when `data` lacks them, `_type` from `rm_type`, and every
/// other member from `data` as stored. Without a fragment path the whole
/// rebuilt fragment is read (`node_fragment`). No openEHR spec governs
/// storage: our own design.
pub(super) fn fragment_source(alias: &str, jp: Option<&str>) -> Expr {
    let data = col(alias, Node::Data);
    let value = || {
        call(
            "node_value",
            vec![
                col(alias, Node::ValueType),
                col(alias, Node::ValueNum),
                col(alias, Node::ValueText),
                col(alias, Node::ValueCode),
                col(alias, Node::ValueTerminology),
            ],
        )
    };
    let Some(jp) = jp else {
        return call(
            "node_fragment",
            vec![
                data,
                col(alias, Node::RmType),
                col(alias, Node::Archetype),
                col(alias, Node::Name),
                col(alias, Node::NameCode),
                col(alias, Node::NameTerminology),
                col(alias, Node::ValueType),
                col(alias, Node::ValueNum),
                col(alias, Node::ValueText),
                col(alias, Node::ValueCode),
                col(alias, Node::ValueTerminology),
                col(alias, Node::RmVersion),
                col(alias, Node::Language),
                col(alias, Node::Encoding),
            ],
        );
    };
    match first_member(jp) {
        Some("value") => Expr::cust_with_exprs(
            "CASE WHEN $1 IS NULL THEN $2 ELSE jsonb_build_object('value', $3) END",
            [col(alias, Node::ValueType), data, value()],
        ),
        Some("name") => Expr::cust_with_exprs(
            "CASE WHEN $1 IS NULL OR ($2 -> 'name') IS NOT NULL THEN $3 \
             ELSE jsonb_build_object('name', node_name($4, $5, $6)) END",
            [
                col(alias, Node::Name),
                data.clone(),
                data,
                col(alias, Node::Name),
                col(alias, Node::NameCode),
                col(alias, Node::NameTerminology),
            ],
        ),
        Some("archetype_node_id") => Expr::cust_with_exprs(
            "CASE WHEN $1 IS NULL OR ($2 -> 'archetype_node_id') IS NOT NULL THEN $3 \
             ELSE jsonb_build_object('archetype_node_id', $4) END",
            [
                col(alias, Node::Archetype),
                data.clone(),
                data,
                archetype_node_id(alias),
            ],
        ),
        Some("archetype_details") => Expr::cust_with_exprs(
            "CASE WHEN $1 IS NULL OR ($2 -> 'archetype_details') IS NOT NULL THEN $3 \
             ELSE jsonb_build_object('archetype_details', node_archetype_details($4, $5)) END",
            [
                col(alias, Node::RmVersion),
                data.clone(),
                data,
                col(alias, Node::Archetype),
                col(alias, Node::RmVersion),
            ],
        ),
        Some(member @ ("language" | "encoding")) => {
            let (column, terminology) = if member == "language" {
                (Node::Language, crate::storage::header::LANGUAGE_TERMINOLOGY)
            } else {
                (Node::Encoding, crate::storage::header::ENCODING_TERMINOLOGY)
            };
            Expr::cust_with_exprs(
                "CASE WHEN $1 IS NULL OR ($2 -> $3) IS NOT NULL THEN $4 \
                 ELSE jsonb_build_object($5, node_code_phrase($6, $7)) END",
                [
                    col(alias, column),
                    data.clone(),
                    Expr::val(member),
                    data,
                    Expr::val(member),
                    Expr::val(terminology),
                    col(alias, column),
                ],
            )
        }
        Some("_type") => Expr::cust_with_exprs(
            "jsonb_build_object('_type', $1)",
            [col(alias, Node::RmType)],
        ),
        _ => data,
    }
}

/// The member and the read of the `node` columns that hold the scalar a
/// fragment path addresses, for the paths into a key the row stores as a
/// column when the fragment leaves it out (`crate::storage::slim`): the
/// name's value and code, the archetype node id, the archetype header's RM
/// version, and the language and encoding codes.
pub(super) fn column_read(alias: &str, path: &[&str]) -> Option<(&'static str, Expr)> {
    let column = |member, column| Some((member, col(alias, column)));
    match path {
        ["name", "value"] => column("name", Node::Name),
        ["name", "defining_code", "code_string"] => column("name", Node::NameCode),
        ["name", "defining_code", "terminology_id", "value"] => {
            column("name", Node::NameTerminology)
        }
        ["archetype_node_id"] => Some(("archetype_node_id", archetype_node_id(alias))),
        ["archetype_details", "rm_version"] => column("archetype_details", Node::RmVersion),
        ["language", "code_string"] => column("language", Node::Language),
        ["encoding", "code_string"] => column("encoding", Node::Encoding),
        _ => None,
    }
}

/// The node row `alias`'s `archetype_node_id` as its columns hold it: the
/// lowercase `archetype`, recased when the row keeps its archetype header in
/// `rm_version` (`ext.node_archetype_node_id`).
pub(super) fn archetype_node_id(alias: &str) -> Expr {
    call(
        "node_archetype_node_id",
        vec![col(alias, Node::Archetype), col(alias, Node::RmVersion)],
    )
}

/// `jsonb_path_query_first(data, '<jp>'::jsonpath[, vars])` — `vars` carries
/// the filter-expression variables of a predicated fragment path, always as a
/// bound jsonb parameter (never spliced into the path text).
pub(super) fn jsonb_path(data: Expr, jp: &str, vars: Option<Expr>) -> Expr {
    let mut args = vec![data, cast(Expr::val(jp.to_owned()), "jsonpath")];
    if let Some(vars) = vars {
        args.push(vars);
    }
    call("jsonb_path_query_first", args)
}

/// `NULLIF(jsonb_path_query_array(data, '<jp>'::jsonpath), '[]'::jsonb)` —
/// every fragment match as ONE jsonb array cell, SQL `NULL` when the path
/// matches nothing (so absence keeps reading as a NULL cell / a false
/// comparison, exactly as the scalar extraction reads it).
pub(super) fn jsonb_path_array(data: Expr, jp: &str, vars: Option<Expr>) -> Expr {
    let mut args = vec![data, cast(Expr::val(jp.to_owned()), "jsonpath")];
    if let Some(vars) = vars {
        args.push(vars);
    }
    call(
        "nullif",
        vec![
            call("jsonb_path_query_array", args),
            cast(Expr::val("[]"), "jsonb"),
        ],
    )
}

/// `<jsonb> #>> '{}'` — the scalar's text at the empty path.
pub(super) fn as_text(e: Expr) -> Expr {
    e.binary(BinOper::Custom("#>>"), cast(Expr::val("{}"), "text[]"))
}

/// A typed cast `<e>::<ty>`.
pub(super) fn cast(e: Expr, ty: &str) -> Expr {
    e.cast_as(Alias::new(ty))
}

/// The jsonb extraction base: `jsonb_path_query_first(<data>, jp)` when a
/// fragment path is present, else the raw `<data>` expression.
pub(super) fn extract_base(data: Expr, jp: Option<&str>, vars: Option<Expr>) -> Expr {
    match jp {
        Some(jp) => jsonb_path(data, jp, vars),
        None => data,
    }
}

/// The AQL comparison operators (QUERY master03 §Comparison operators) → the
/// typed `sea-query` binary operators.
pub(super) fn binoper(op: CompOp) -> BinOper {
    match op {
        CompOp::Eq => BinOper::Equal,
        CompOp::Ne => BinOper::NotEqual,
        CompOp::Lt => BinOper::SmallerThan,
        CompOp::Le => BinOper::SmallerThanOrEqual,
        CompOp::Gt => BinOper::GreaterThan,
        CompOp::Ge => BinOper::GreaterThanOrEqual,
    }
}

/// A typed AQL literal → a bound `sea-query` [`Value`] (QUERY master03
/// §Literals + §Built-in Types).
pub(super) fn literal_value(lit: &TypedLit) -> Value {
    match lit {
        TypedLit::Integer(i) => Value::from(*i),
        TypedLit::Real(r) => Value::from(*r),
        TypedLit::Boolean(b) => Value::from(*b),
        TypedLit::String(s) | TypedLit::Temporal(s) => Value::from(s.clone()),
        TypedLit::Null => Value::from(Option::<String>::None),
    }
}

/// A typed `rm_type IN (...)` condition, or `None` for an unresolved type set.
/// The concrete RM types a source/step may bind (QUERY master03 §Class
/// expressions; the abstract→concrete expansion is done in analysis).
pub(super) fn type_cond(node: &str, types: &TypeSet) -> Option<Expr> {
    if types.is_empty() {
        return None;
    }
    let members: Vec<Expr> = types.names().iter().map(|t| Expr::val(t.clone())).collect();
    Some(col(node, Node::RmType).is_in(members))
}

/// The coercion an ORDER BY key uses (QUERY master03 §ORDER BY — Ordered types
/// compare by ordered-magnitude; mirrors the analyzer's leaf typing).
pub(super) fn order_coercion(target: &PathTarget) -> Coercion {
    match target {
        PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => leaf.coercion,
        PathTarget::Version { field, .. } => {
            if *field == VersionField::TimeCommitted {
                Coercion::Temporal
            } else {
                Coercion::Text
            }
        }
        PathTarget::Ehr { field, .. } => {
            if *field == EhrField::TimeCreated {
                Coercion::Temporal
            } else {
                Coercion::Text
            }
        }
    }
}

/// A best-effort AQL path string for a `RESULT_SET` column's `path`
/// (ITS-REST 1.1.0 `RESULT_SET_COLUMN.path` — the RM path "as specified in
/// query"). `"/"` for a bare variable / whole-object leaf.
pub(super) fn leaf_path_string(leaf: &LeafPath) -> String {
    let mut s = String::new();
    for step in &leaf.anchor {
        let _ = write!(s, "/{}", step.attribute);
    }
    for step in &leaf.fragment {
        let _ = write!(s, "/{}", step.name);
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

// ── LIKE ──────────────────────────────────────────────────────────────────────

/// Translate an AQL `LIKE` pattern (`*` = any run, `?` = one char) to a SQL
/// `LIKE` pattern, escaping literal `%`/`_`/`\`. The AQL escapes `\*`/`\?` are
/// the LITERAL characters (QUERY master03 §Operators/LIKE); `\\` is a literal
/// backslash (SQL-escaped).
pub(super) fn aql_like_to_sql(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '\\' => match chars.next() {
                Some('*') => out.push('*'),
                Some('?') => out.push('?'),
                Some(other) => {
                    out.push_str("\\\\");
                    if other != '\\' {
                        out.push(other);
                    }
                }
                None => out.push_str("\\\\"),
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::aql_like_to_sql;

    /// `\*` / `\?` in a LIKE pattern are the literal characters (§Operators/LIKE).
    #[test]
    fn like_escapes_are_literal() {
        assert_eq!(aql_like_to_sql("a*b"), "a%b");
        assert_eq!(aql_like_to_sql("a?b"), "a_b");
        assert_eq!(aql_like_to_sql(r"a\*b"), "a*b");
        assert_eq!(aql_like_to_sql(r"a\?b"), "a?b");
        assert_eq!(aql_like_to_sql("100%"), r"100\%");
    }
}
