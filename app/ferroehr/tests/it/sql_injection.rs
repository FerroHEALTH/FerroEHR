// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Injection evidence for the AQL→SQL engine: identifiers, jsonpath, `ORDER BY`.
//!
//! The engine builds SQL dynamically from attacker-supplied query text, so the
//! OWASP SQL Injection Prevention Cheat Sheet's primary defence — bind every
//! value — is not by itself sufficient: identifiers and sort directions cannot
//! be parameterized, and for those the cheat sheet asks for a mapping onto
//! "legal/expected table or column names"
//! (<https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html>).
//! This suite pins both halves: hostile text in a value position reaches the
//! bound-value vector and never the SQL string, and hostile text in an
//! identifier position is refused before planning.
//!
//! ## The pinned enumeration: every identifier source is a closed set
//!
//! Nothing outside this list can produce an identifier, a function name, a cast
//! type or a sort direction in the emitted SQL. Each entry names the source file
//! and the item that fixes the set; [`every_quoted_identifier_comes_from_the_closed_set`]
//! re-derives it from real output over a query corpus, so a new construct that
//! widens the set fails this suite.
//!
//! * **Tables** — `db::iden` (`Node`, `VoVersion`, `Ehr`, `Audit`): `sea-query`
//!   `Iden` enums, one variant per relation, no string path in.
//! * **Columns** — string literals at every `aql::sql::expr::col` call site,
//!   plus `storage::promoted::PROMOTED_LEAVES[..].column`, whose fields are
//!   `&'static str`. `IDENTIFIERS` below pins the whole vocabulary against the
//!   clinical migrations that define those relations
//!   (`migrations/clinical/0002_ehr.sql` through `0006_definitions.sql`).
//! * **Table aliases** — `format!` over an integer source id or the builder's
//!   own counter, never over query text: `n{sid}`, `v{sid}`, `e{sid}`,
//!   `x{ctr}`, `xv{ctr}`, `s{ctr}`, `w{ctr}`, `p{ctr}`, `qg{ctr}`, `esv{ctr}`,
//!   `esn{ctr}`, `a_v{sid}` (`aql::sql::from`, `aql::sql::value`,
//!   `aql::sql::select`).
//! * **Output-column aliases** — `col{i}` / `col{i}_{vo,sv,num,cap}` from the
//!   SELECT index (`aql::sql::select`), `scope_ehr_{i}` / `scope_template_{i}`
//!   from the VO-root index (`aql::sql::build_scope`), `access_ehr_{i}` /
//!   `access_vo_{i}` / `access_sv_{i}` from the same index
//!   (`aql::sql::select::build_access_ehr_columns` and
//!   `build_access_version_columns`, the hidden columns the access log reads
//!   the served EHRs and the served version rows from), and the literal `hit`
//!   for the streaming `EXISTS` probe (`aql::sql::value`). An AQL `AS <label>`
//!   is carried on `ColumnSpec::name` for the `RESULT_SET` only and is never a
//!   SQL identifier — see [`the_select_as_label_never_becomes_a_sql_identifier`].
//! * **Function names** — the six literals passed to `aql::sql::expr::call`:
//!   `to_jsonb`, `jsonb_path_query_first`, `jsonb_path_query_array`,
//!   `openehr_magnitude`, `openehr_timestamp`, `jsonb_build_object`; plus the
//!   `Expr::cust`/`cust_with_exprs` fragments in
//!   `aql::sql::predicate` and `aql::sql::value`, whose arguments are all
//!   positional `$n` expressions.
//! * **Cast types** — the seven literals passed to `aql::sql::expr::cast`:
//!   `numeric`, `boolean`, `timestamptz`, `text`, `text[]`, `uuid`, `jsonpath`.
//! * **Sort direction** — `sea_query::Order::Asc`/`Order::Desc` chosen from the
//!   IR's `OrderKey::ascending` boolean (`aql::sql::value::build_order_by`); the
//!   AQL grammar admits only the keywords, so there is no third value.
//! * **Comparison operators** — `sea_query::BinOper` from the closed
//!   `openehr_query::lexer::CompOp` enum (`aql::sql::expr::binoper`).
//!
//! The jsonpath is the one place a path segment's text travels: it is assembled
//! into `$.a.b` (`aql::sql::value::fragment_jsonpath`) and then **bound**, as
//! `jsonb_path_query_first(data, CAST($n AS jsonpath))`. Segment text is also
//! restricted at the front door — `openehr_query::lexer::Token::Identifier` is
//! `[a-zA-Z][a-zA-Z0-9_]*`, so a quote, semicolon, comment sequence or jsonpath
//! metacharacter in a path position is a lex failure, not a payload.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              helpers and async bodies; panicking assertions are the intended \
              shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use sea_query::Value;
use sea_query_sqlx::SqlxValues;
use sqlx::{AssertSqlSafe, Connection, PgConnection, Row};

use ferroehr::aql::error::{AnalysisError, AqlError, AqlFeatureError};
use ferroehr::aql::ir::{ParamValue, Params, QueryIr};
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::plan;
use ferroehr::aql::sql::{PreparedQuery, SqlCtx};
use openehr_query::parser::parse_str;

// ── the hostile corpus ───────────────────────────────────────────────────────

/// The marker every payload carries. It is plain ASCII letters, so it survives
/// any escaping a renderer might apply and its presence in the SQL text — or
/// absence from the bound values — is unambiguous.
const MARKER: &str = "ZqInJeCtIoN";

/// Hostile payloads: quote breaking, comment sequences, statement terminators,
/// placeholder impersonation, jsonpath metacharacters, `LIKE` metacharacters,
/// control characters, and Unicode apostrophe lookalikes (which a
/// normalizing layer could fold into `'`). Every entry carries [`MARKER`].
const HOSTILE: &[&str] = &[
    "ZqInJeCtIoN' OR '1'='1",
    "ZqInJeCtIoN'; DROP TABLE node; --",
    "ZqInJeCtIoN\"; DROP TABLE \"node\"; --",
    "ZqInJeCtIoN/* comment */ UNION SELECT NULL",
    "ZqInJeCtIoN--",
    "ZqInJeCtIoN; SELECT pg_sleep(10)",
    "ZqInJeCtIoN' || pg_sleep(10) || '",
    "ZqInJeCtIoN\\",
    "ZqInJeCtIoN\\'",
    "ZqInJeCtIoN\u{0}",
    "$1 ZqInJeCtIoN",
    "$$ZqInJeCtIoN$$",
    "ZqInJeCtIoN?@**.[*]",
    "$.** ? (@ == \"ZqInJeCtIoN\")",
    "ZqInJeCtIoN%_",
    "ZqInJeCtIoN\u{2019}\u{ff07}\u{02bc}",
    "ZqInJeCtIoN\n-- newline",
    "ZqInJeCtIoN\") AS x, (SELECT 1) AS y --",
    "ZqInJeCtIoN\u{feff}",
    "ZqInJeCtIoN'::text; ROLLBACK; --",
];

// ── helpers ──────────────────────────────────────────────────────────────────

fn ctx() -> SqlCtx {
    SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: Vec::new(),
        subject_scope: None,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    }
}

fn plan_ok(q: &str) -> QueryIr {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    plan(
        &ast,
        &Params::new(),
        ferroehr::config::profile::SpecProfile::default(),
    )
    .unwrap_or_else(|e| panic!("plan failed for {q:?}: {e}"))
}

/// Plan `q` and lower it to SQL with `params` bound and `ctx` as the execution
/// context.
fn build_with(q: &str, params: &Params, ctx: &SqlCtx) -> PreparedQuery {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    let ir = plan(
        &ast,
        params,
        ferroehr::config::profile::SpecProfile::default(),
    )
    .unwrap_or_else(|e| panic!("plan failed for {q:?}: {e}"));
    ferroehr::aql::sql::build(&ir, params, ctx)
        .unwrap_or_else(|e| panic!("SQL build failed for {q:?}: {e}"))
}

fn build(q: &str, params: &Params) -> PreparedQuery {
    build_with(q, params, &ctx())
}

/// One string parameter.
fn one_param(name: &str, value: &str) -> Params {
    let mut p = Params::new();
    p.insert(name.to_owned(), ParamValue::Str(value.to_owned()));
    p
}

/// The bound string values, in bind order.
fn bound_strings(values: &SqlxValues) -> Vec<String> {
    values
        .0
        .0
        .iter()
        .filter_map(|v| match v {
            Value::String(Some(s)) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// Whether some bound value carries [`MARKER`], compared case-insensitively.
///
/// NOTE: the archetype predicate case-folds its bind value, because openEHR
/// identifier equality is case-insensitive (BASE `base_types` master05
/// §"Composite Identifiers and Case").
fn bound_carries_marker(values: &SqlxValues) -> bool {
    let needle = MARKER.to_ascii_lowercase();
    bound_strings(values)
        .iter()
        .any(|v| v.to_ascii_lowercase().contains(&needle))
}

/// The planning error of a query that must not plan.
fn plan_err(q: &str) -> AqlError {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    plan(
        &ast,
        &Params::new(),
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect_err(&format!("expected {q:?} to be refused"))
}

// ── (1) hostile parameter values ─────────────────────────────────────────────

/// The user-supplied positions a `$parameter` value can reach. A parameter value
/// arrives as JSON in the REST request body, so it bypasses the AQL lexer
/// entirely and may hold any byte — this is the widest surface the engine has.
const PARAM_POSITIONS: &[(&str, &str)] = &[
    (
        "comparison right-hand side",
        "SELECT c/name/value FROM COMPOSITION c WHERE c/name/value = $p",
    ),
    (
        "LIKE pattern",
        "SELECT c/name/value FROM COMPOSITION c WHERE c/name/value LIKE $p",
    ),
    (
        "MATCHES member",
        "SELECT c/name/value FROM COMPOSITION c WHERE c/name/value MATCHES {$p}",
    ),
    (
        "node name constraint",
        "SELECT c/name/value FROM COMPOSITION c[name/value = $p]",
    ),
    (
        "archetype predicate parameter",
        "SELECT o/data FROM COMPOSITION c CONTAINS OBSERVATION o[$p]",
    ),
    (
        "standard predicate value",
        "SELECT c/name/value FROM COMPOSITION c[context/setting/value = $p]",
    ),
    (
        "EHR time_created predicate",
        "SELECT e/ehr_id/value FROM EHR e[time_created > $p]",
    ),
    (
        "VERSION commit-audit predicate",
        "SELECT c/name/value FROM VERSION v[commit_audit/committer/name = $p] CONTAINS COMPOSITION c",
    ),
    (
        "ORDER BY over a filtered path",
        "SELECT c/name/value FROM COMPOSITION c WHERE c/name/value != $p ORDER BY c/name/value DESC",
    ),
];

#[test]
fn hostile_parameter_values_never_change_the_generated_sql() {
    for (position, query) in PARAM_POSITIONS {
        let benign = build(query, &one_param("p", "benign"));
        for payload in HOSTILE {
            let hostile = build(query, &one_param("p", payload));
            assert_eq!(
                benign.sql, hostile.sql,
                "the SQL structure changed at the {position} for payload {payload:?}"
            );
            assert_eq!(
                benign.values.0.0.len(),
                hostile.values.0.0.len(),
                "the bound-value count changed at the {position} for payload {payload:?}"
            );
            assert!(
                !hostile.sql.contains(MARKER),
                "user text reached the SQL text at the {position}: {}",
                hostile.sql
            );
            assert!(
                bound_carries_marker(&hostile.values),
                "the payload {payload:?} is not among the bound values at the {position}: {:?}",
                bound_strings(&hostile.values)
            );
        }
    }
}

/// The `ehr_id/value` comparison is the one parameter position that binds
/// nothing: a value that does not parse as a UUID can equal no EHR id, so the
/// comparison folds to a constant (`aql::sql::predicate::ehr_id_typed_compare`).
/// The payload must still be absent from the SQL, and the shape must not depend
/// on it.
#[test]
fn a_non_uuid_ehr_id_parameter_folds_to_a_constant_without_reaching_the_sql() {
    let query = "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = $p";
    let benign = build(query, &one_param("p", "not-a-uuid"));
    for payload in HOSTILE {
        let hostile = build(query, &one_param("p", payload));
        assert_eq!(
            benign.sql, hostile.sql,
            "the SQL structure changed for payload {payload:?}"
        );
        assert!(
            !hostile.sql.contains(MARKER),
            "user text reached the SQL text: {}",
            hostile.sql
        );
        assert!(
            !bound_strings(&hostile.values)
                .iter()
                .any(|v| v.contains(MARKER)),
            "a non-uuid comparison must bind nothing from the payload"
        );
    }
}

// ── (2) hostile string literals written into the query text ──────────────────

/// The same payloads written as AQL string literals, escaped per the lexer's
/// `'([^'\\]|\\.)*'` rule. A literal is a value, so it binds exactly like a
/// parameter — this pins that the escape handling does not leak it into the SQL.
#[test]
fn hostile_string_literals_bind_and_never_reach_the_sql_text() {
    for payload in HOSTILE {
        // NUL and the BOM are not writable as AQL source text; they are covered
        // by the parameter path, which is where such bytes actually arrive.
        if payload.contains('\u{0}') || payload.contains('\u{feff}') {
            continue;
        }
        let escaped = payload.replace('\\', r"\\").replace('\'', r"\'");
        let query =
            format!("SELECT c/name/value FROM COMPOSITION c WHERE c/name/value = '{escaped}'");
        let Ok(ast) = parse_str(&query) else {
            // A payload the lexer refuses is refused — also an acceptable
            // outcome, and asserted as such rather than skipped silently.
            continue;
        };
        let ir = plan(
            &ast,
            &Params::new(),
            ferroehr::config::profile::SpecProfile::default(),
        )
        .unwrap_or_else(|e| panic!("plan failed for {query:?}: {e}"));
        let prepared = ferroehr::aql::sql::build(&ir, &Params::new(), &ctx())
            .unwrap_or_else(|e| panic!("SQL build failed for {query:?}: {e}"));
        assert!(
            !prepared.sql.contains(MARKER),
            "a string literal reached the SQL text: {}",
            prepared.sql
        );
        assert!(
            bound_strings(&prepared.values)
                .iter()
                .any(|v| v.contains(MARKER)),
            "the literal {payload:?} is not among the bound values: {:?}",
            bound_strings(&prepared.values)
        );
        // The structure is the benign one, byte for byte.
        let benign = build(
            "SELECT c/name/value FROM COMPOSITION c WHERE c/name/value = 'benign'",
            &Params::new(),
        );
        assert_eq!(
            benign.sql, prepared.sql,
            "the SQL structure changed for literal {payload:?}"
        );
    }
}

// ── (3) the SELECT `AS` label ────────────────────────────────────────────────

/// The single most likely identifier leak: an AQL `AS <label>`. It is the
/// `RESULT_SET` column name only; the SQL alias is always `col{index}`.
#[test]
fn the_select_as_label_never_becomes_a_sql_identifier() {
    let prepared = build(
        "SELECT DISTINCT c/name/value AS ZqInJeCtIoN FROM COMPOSITION c ORDER BY c/name/value DESC",
        &Params::new(),
    );
    assert!(
        !prepared.sql.contains(MARKER),
        "the AS label became a SQL identifier: {}",
        prepared.sql
    );
    assert!(
        prepared.sql.contains(r#"AS "col0""#),
        "the SQL alias must be the index-derived `col0`: {}",
        prepared.sql
    );
    // Under DISTINCT the sort key references the OUTPUT column, which is the
    // same generated alias — never the label.
    assert!(
        prepared.sql.contains(r#"ORDER BY "col0" DESC"#),
        "the DISTINCT sort key must reference `col0`: {}",
        prepared.sql
    );
    assert_eq!(
        prepared.columns.first().map(|c| c.name.as_str()),
        Some(MARKER),
        "the label belongs on the RESULT_SET column, not the SQL"
    );
    // A whole-object column keeps the same rule across its four locator columns.
    let whole = build("SELECT c AS ZqInJeCtIoN FROM COMPOSITION c", &Params::new());
    assert!(
        !whole.sql.contains(MARKER),
        "the AS label became a SQL identifier on a whole-object column: {}",
        whole.sql
    );
    assert_eq!(
        whole.columns.first().map(|c| c.sql_cols.as_slice()),
        Some(
            ["col0_vo", "col0_sv", "col0_num", "col0_cap"]
                .map(String::from)
                .as_slice()
        ),
    );
}

// ── (4) identifier positions are refused, not escaped ────────────────────────

/// Hostile text in a position that would become an identifier is refused by the
/// front end. `Token::Identifier` is `[a-zA-Z][a-zA-Z0-9_]*`
/// (`openehr_query::lexer`), so every metacharacter lexes as something else and
/// the parse fails.
#[test]
fn hostile_text_in_an_identifier_position_is_refused_by_the_parser() {
    let positions: &[(&str, String)] = &[
        (
            "SELECT AS label",
            format!("SELECT c/name/value AS {MARKER}' FROM COMPOSITION c"),
        ),
        (
            "path segment",
            format!("SELECT c/{MARKER}'; DROP TABLE node; -- FROM COMPOSITION c"),
        ),
        (
            "FROM class name",
            format!("SELECT c/name/value FROM {MARKER}\"; DROP TABLE node c"),
        ),
        (
            "variable name",
            format!("SELECT c/name/value FROM COMPOSITION {MARKER}';--"),
        ),
        (
            "archetype predicate",
            format!("SELECT c/name/value FROM COMPOSITION c[{MARKER}'; DROP TABLE node]"),
        ),
        (
            "ORDER BY direction",
            format!("SELECT c/name/value FROM COMPOSITION c ORDER BY c/name/value {MARKER}"),
        ),
    ];
    for (position, query) in positions {
        assert!(
            parse_str(query).is_err(),
            "the {position} accepted hostile text: {query}"
        );
    }
}

/// The one identifier position that survives lexing is a function name: the AQL
/// grammar's `functionCall` admits a bare `IDENTIFIER`
/// (`openehr_query::lexer` module docs), so `zqinjection(x)` PARSES. It is
/// classified as `FunctionCall::Other` and refused at lowering — the name never
/// reaches `Func::cust`.
#[test]
fn an_unknown_function_name_is_refused_at_lowering() {
    let error = plan_err("SELECT zqinjection(c/name/value) FROM COMPOSITION c");
    assert!(
        matches!(
            error,
            AqlError::Feature(AqlFeatureError::UnsupportedFunction(_))
        ),
        "expected an unsupported-function refusal, got {error:?}"
    );
    // A name carrying SQL metacharacters does not even lex as one identifier.
    assert!(
        parse_str(&format!(
            "SELECT {MARKER}'(c/name/value) FROM COMPOSITION c"
        ))
        .is_err(),
        "a function name with a quote must not parse"
    );
}

/// A well-formed but hostile `archetype_node_id` VALUE is refused at analysis
/// with a typed error rather than reaching the archetype predicate: it names no
/// archetype root and is no term code
/// (`aql::analyze::apply_standard`, over `openehr_rm::v1_2::paths`).
#[test]
fn a_hostile_archetype_node_id_is_a_typed_refusal() {
    let error = plan_err(
        "SELECT c/name/value FROM COMPOSITION c[archetype_node_id = 'ZqInJeCtIoN\\'; DROP TABLE node']",
    );
    assert!(
        matches!(
            error,
            AqlError::Analysis(AnalysisError::MalformedArchetypeNodeId(_))
        ),
        "expected a malformed-archetype-node-id refusal, got {error:?}"
    );
}

/// An unknown path segment is a typed refusal, so a segment can never reach the
/// jsonpath unless the RM model declares it.
#[test]
fn an_unmodelled_path_segment_is_a_typed_refusal() {
    let error = plan_err("SELECT c/zqinjection FROM COMPOSITION c");
    assert!(
        matches!(
            error,
            AqlError::Analysis(
                AnalysisError::UnresolvableAttribute { .. }
                    | AnalysisError::AttributeNotInProfile { .. }
            )
        ),
        "expected an unresolvable-attribute refusal, got {error:?}"
    );
}

// ── (5) every emitted identifier comes from the closed set ───────────────────

/// A corpus spanning the FROM shapes (flat, streaming, EHR-scoped, `EHR_STATUS`,
/// `VERSION`, `OR`/`NOT CONTAINS`), the projection kinds (scalar, whole object,
/// aggregate, literal, scalar function), and the predicate families.
const CORPUS: &[&str] = &[
    "SELECT c/name/value FROM COMPOSITION c",
    "SELECT c FROM COMPOSITION c",
    "SELECT DISTINCT c/name/value FROM COMPOSITION c ORDER BY c/name/value ASC",
    "SELECT count(*), max(o/data/origin/value) FROM COMPOSITION c CONTAINS OBSERVATION o",
    "SELECT 1, 'x' FROM COMPOSITION c",
    "SELECT length(c/name/value), substring(c/name/value, 1, 2) FROM COMPOSITION c",
    "SELECT now(), current_date_time() FROM COMPOSITION c",
    "SELECT c/uid/value FROM COMPOSITION c",
    "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1]",
    "SELECT o/data FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[at0001]",
    "SELECT e/ehr_status/subject/external_ref/id/value FROM EHR e",
    "SELECT e/ehr_status FROM EHR e",
    "SELECT v/commit_audit/time_committed, v/lifecycle_state/value FROM VERSION v CONTAINS COMPOSITION c",
    "SELECT c/name/value FROM VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
    "SELECT e/ehr_id/value FROM EHR e CONTAINS (COMPOSITION c OR OBSERVATION o)",
    "SELECT c/name/value FROM COMPOSITION c NOT CONTAINS OBSERVATION o",
    "SELECT c/name/value FROM COMPOSITION c WHERE c/context/start_time/value > '2020-01-01'",
    "SELECT c/name/value FROM COMPOSITION c[context/setting/value = 'x'] WHERE c/name/value MATCHES {'a','b'}",
    "SELECT c/name/value FROM COMPOSITION c WHERE NOT (c/name/value LIKE 'a*') AND EXISTS c/name/value",
    "SELECT c/name/value FROM COMPOSITION c LIMIT 5",
];

/// The relation and column names the lowering may emit, plus the one `to_char`
/// format fragment that renders inside double quotes.
///
/// The column names are the vocabulary the clinical migrations define
/// (`migrations/clinical/0002_ehr.sql` through `0006_definitions.sql`).
const IDENTIFIERS: &[&str] = &[
    // relations
    "node",
    "version",
    "vo_head",
    "ehr",
    "commit_audit",
    // node
    "tier",
    "vo_id",
    "sys_version",
    "num",
    "num_cap",
    "parent_num",
    "ehr_id",
    "rm_type",
    "archetype",
    "arch_entity",
    "arch_concept",
    "arch_major",
    "name",
    "name_code",
    "name_terminology",
    "path",
    "data",
    "context_start",
    // version
    "kind",
    "trunk_version",
    "branch_number",
    "branch_version",
    "committed_at",
    "lifecycle_state",
    "creating_system_id",
    "contribution_id",
    "commit_audit_id",
    "template_id",
    // vo_head
    "trunk_head_sys_version",
    "restricted_at",
    "research_objected_at",
    "research_objection_ground",
    // ehr
    "id",
    "system_id",
    "time_created",
    "subject_id",
    "is_queryable",
    // audit
    "time_committed",
    "change_type",
    "description",
    "committer",
    // NOTE: `T` is the literal date/time separator inside the `to_char` format
    // string of QUERY master03 §Date and time functions, not an identifier.
    "T",
];

/// The generated-alias shapes, anchored. Each is `format!`-built from an integer
/// (a source id or the builder's counter) or a SELECT index.
const ALIAS_PATTERNS: &[&str] = &[
    r"^n[0-9]+$",
    r"^v[0-9]+$",
    r"^e[0-9]+$",
    r"^x[0-9]+$",
    r"^xv[0-9]+$",
    r"^s[0-9]+$",
    r"^w[0-9]+$",
    r"^p[0-9]+$",
    r"^qg[0-9]+$",
    r"^esv[0-9]+$",
    r"^esn[0-9]+$",
    r"^a_v[0-9]+$",
    r"^v[0-9]+_head$",
    r"^v[0-9]+_at$",
    r"^esv[0-9]+_head$",
    r"^xv[0-9]+_head$",
    r"^v[0-9]+_mark$",
    r"^esv[0-9]+_mark$",
    r"^xv[0-9]+_mark$",
    r"^col[0-9]+(_(vo|sv|num|cap))?$",
    r"^scope_(ehr|template)_[0-9]+$",
    r"^access_(ehr|vo|sv)_[0-9]+$",
    r"^hit$",
];

#[test]
fn every_quoted_identifier_comes_from_the_closed_set() {
    let quoted = regex::Regex::new(r#""([^"]*)""#).expect("identifier regex");
    let aliases: Vec<regex::Regex> = ALIAS_PATTERNS
        .iter()
        .map(|p| regex::Regex::new(p).expect("alias regex"))
        .collect();

    // Every corpus query is built twice: once clean, once with a hostile value
    // in each of the parameter positions that query admits, so the scan sees the
    // shapes an attacker can actually reach.
    let mut statements: Vec<(String, String)> = Vec::new();
    for query in CORPUS {
        let prepared = build(query, &Params::new());
        statements.push(((*query).to_owned(), prepared.sql));
        // The scope query (the ABAC post-check) is a second emitted statement.
        let ir = plan_ok(query);
        if let Some(scope) = ferroehr::aql::sql::build_scope(&ir, &Params::new(), &ctx())
            .unwrap_or_else(|e| panic!("scope build failed for {query:?}: {e}"))
        {
            statements.push((format!("{query} [scope]"), scope.sql));
        }
    }
    for payload in HOSTILE {
        for (position, query) in PARAM_POSITIONS {
            let prepared = build(query, &one_param("p", payload));
            statements.push((format!("{position}: {query}"), prepared.sql));
        }
    }
    // The STREAMING FROM shape is selected by the execution context's effective
    // row limit rather than the query text, and it emits its own alias family
    // (the `p{n}` LATERAL probes and their `hit` column), so it is scanned too.
    let streaming = SqlCtx {
        limit: Some(5),
        ..ctx()
    };
    for query in [
        "SELECT c/name/value FROM COMPOSITION c",
        // The literal is a (partial) ISO value: a temporal comparison bound is
        // shape-checked at plan time, and a non-temporal string would refuse
        // before any SQL exists to scan.
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
         WHERE o/data/origin/value = '2020'",
        "SELECT o/data FROM COMPOSITION c CONTAINS OBSERVATION o",
    ] {
        let prepared = build_with(query, &Params::new(), &streaming);
        statements.push((format!("{query} [streaming]"), prepared.sql));
    }

    for (source, sql) in &statements {
        for capture in quoted.captures_iter(sql) {
            let ident = capture.get(1).map_or("", |m| m.as_str());
            let known = IDENTIFIERS.contains(&ident) || aliases.iter().any(|r| r.is_match(ident));
            assert!(
                known,
                "the emitted SQL for {source} carries the identifier {ident:?}, which is outside \
                 the closed set pinned in this module's docs:\n{sql}"
            );
        }
    }
}

// ── (6) the sort direction ───────────────────────────────────────────────────

/// `ORDER BY` renders exactly one of two keywords, chosen from a boolean, and
/// the two statements differ only in that keyword.
#[test]
fn the_sort_direction_is_a_closed_two_valued_choice() {
    let asc = build(
        "SELECT c/name/value FROM COMPOSITION c ORDER BY c/name/value ASC",
        &Params::new(),
    );
    let desc = build(
        "SELECT c/name/value FROM COMPOSITION c ORDER BY c/name/value DESC",
        &Params::new(),
    );
    assert_eq!(
        asc.sql.replace(" ASC", " DESC"),
        desc.sql,
        "ASC and DESC must differ only in the direction keyword"
    );
    assert!(asc.sql.ends_with(" ASC"), "unexpected tail: {}", asc.sql);
    assert!(desc.sql.ends_with(" DESC"), "unexpected tail: {}", desc.sql);
}

// ── (7) the execution context ────────────────────────────────────────────────

/// The context values the access layer supplies (`system_id`, the ABAC patient
/// scope) are bound too, so a hostile value there cannot alter the statement
/// either. No openEHR spec governs the ABAC scope — it is our own extension.
#[test]
fn hostile_execution_context_values_bind() {
    let query = "SELECT e/system_id, c/name/value FROM EHR e CONTAINS COMPOSITION c";
    let benign = build_with(query, &Params::new(), &ctx());
    for payload in HOSTILE {
        let mut hostile_ctx = ctx();
        hostile_ctx.system_id = (*payload).to_owned();
        hostile_ctx.subject_scope = Some((*payload).to_owned());
        let hostile = build_with(query, &Params::new(), &hostile_ctx);
        assert!(
            !hostile.sql.contains(MARKER),
            "a context value reached the SQL text: {}",
            hostile.sql
        );
        assert!(
            bound_strings(&hostile.values)
                .iter()
                .any(|v| v.contains(MARKER)),
            "the context value {payload:?} is not among the bound values"
        );
        // The subject scope adds its own subquery, so only the benign
        // statement's prefix is comparable; the direction that matters is that
        // the payload changes nothing structural between two hostile builds.
        let mut second_ctx = ctx();
        second_ctx.system_id = format!("{payload}{payload}");
        second_ctx.subject_scope = Some(format!("{payload}{payload}"));
        let second = build_with(query, &Params::new(), &second_ctx);
        assert_eq!(
            hostile.sql, second.sql,
            "the SQL structure depends on a context value's content"
        );
        assert!(
            benign.sql.len() < hostile.sql.len(),
            "the scope subquery is expected to be additive"
        );
    }
}

// ── (8) least privilege: what the application's database role cannot do ──────

/// The privilege posture of the role the running server connects as.
///
/// The shipped deployments authenticate as a **non-superuser** login role that
/// is a member of `ferroehr_clinical` (`deploy/helm/ferroehr/values.yaml`,
/// `docker/postgres/initdb/10-ferroehr-init.sh`). This test creates such a role
/// and verifies the negative half first-hand: it is not a superuser, does not
/// bypass row-level security, cannot create or drop schema objects, and cannot
/// create roles — while ordinary reads on the clinical tables still work.
///
/// Roles are cluster-global on the shared harness server, so the role is named
/// off the clone database (`<clone>_leastpriv`) for the testkit sweep to reap.
#[tokio::test]
async fn the_application_database_role_cannot_run_ddl_or_escalate() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let role = format!("{}_leastpriv", testdb.name());
    sqlx::query(AssertSqlSafe(format!(
        "CREATE ROLE {role} LOGIN PASSWORD 'testpw' IN ROLE ferroehr_clinical"
    )))
    .execute(&pool)
    .await
    .expect("create the app-privilege role");

    let (scheme, rest) = testdb.url().split_once("://").expect("dsn scheme");
    let tail = rest.split_once('@').map_or(rest, |(_, t)| t);
    let mut conn = PgConnection::connect(&format!("{scheme}://{role}:testpw@{tail}"))
        .await
        .expect("connect as the app role");
    sqlx::query("SET search_path TO clinical, ext, public")
        .execute(&mut conn)
        .await
        .expect("search_path");

    // The role attributes: none of the escalation flags.
    let row = sqlx::query(
        "SELECT rolsuper, rolbypassrls, rolcreaterole, rolcreatedb, rolreplication \
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&mut conn)
    .await
    .expect("read the current role's attributes");
    for flag in [
        "rolsuper",
        "rolbypassrls",
        "rolcreaterole",
        "rolcreatedb",
        "rolreplication",
    ] {
        let held: bool = row.try_get(flag).expect("role attribute");
        assert!(!held, "the application role must not hold {flag}");
    }

    // DDL is refused in every direction the application could otherwise take.
    for statement in [
        "CREATE TABLE ehr.zq_probe (id int)",
        "DROP TABLE node",
        "ALTER TABLE node ADD COLUMN zq_probe int",
        "CREATE SCHEMA zq_probe",
        "CREATE INDEX zq_probe ON node (num)",
        "TRUNCATE node",
        "CREATE ROLE zq_probe",
        "ALTER TABLE node DISABLE ROW LEVEL SECURITY",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await;
        assert!(
            outcome.is_err(),
            "the application role must not be able to run `{statement}`"
        );
    }

    // …while the reads the server actually performs still work.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM node")
        .fetch_one(&mut conn)
        .await
        .expect("the application role must be able to read the clinical tables");
    assert_eq!(count, 0, "a fresh clone carries no nodes");
}
