// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Typed `sea-query` building blocks shared across the SQL submodules, plus the
//! two lexical translations the AQL language pins exactly — the `LIKE` pattern
//! conversion (QUERY master03 §Operators/LIKE) and the `archetype_node_id`
//! predicate (QUERY master03 §Archetype predicate, re-grounded on BASE/AM
//! subsumption — see [`archetype_predicate`]).
//!
//! No openEHR spec governs the lowering itself — this is our own design: every reference is a typed [`Expr::col`], every
//! literal binds through [`Expr::val`], and the PostgreSQL-specific pieces use
//! only the sanctioned typed escape hatches ([`Func::cust`] for functions
//! sea-query does not model; [`PgFunc::any`] for `= ANY(ARRAY[…])`;
//! [`BinOper::Custom`] for `#>>` / `->>` which have no
//! typed variant). Runtime functions resolve unqualified (the domain's own
//! `search_path`, which ends in `ext, public`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, BinOper, Expr, ExprTrait as _, Func, IntoIden, Value};

use openehr_query::lexer::CompOp;

use crate::aql::ir::{Coercion, EhrField, LeafPath, PathTarget, TypeSet, TypedLit, VersionField};
use crate::aql::lineage::{ArchetypeLineage, decompose_hrid};
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

// ── archetype_node_id predicate ──────────────────────────────────────

/// Build the `archetype_node_id` predicate condition for a node alias.
///
/// When `value` parses as a full archetype id (BASE `base_types` master05
/// §Archetype Identifiers, form `qualified_rm_entity.domain_concept.vN`), the
/// predicate implements query subsumption: "the data created with any
/// specialised archetype will always be matched by queries based on the parent
/// archetype" (BASE `architecture_overview` master10 §Design-time
/// Relationships), with AM master07 §Supporting Archetype-based Querying making
/// the matching set for a query naming X be X, its older minor and patch
/// variants, its specialisation parents and their older variants, bounded to one
/// major version. That set comes from two sources.
///
/// The first is the ADL 1.4 naming convention, and applies to an ADL 1.4-form id
/// only (a major-only `.vN` version): a specialisation child extends the
/// parent's `domain_concept` with a `-`-separated segment, so a query naming a
/// parent matches its own node plus every child whose `arch_concept` begins with
/// `concept-`, scoped to the same `qualified_rm_entity` and major. All parts
/// compare lowercased (master05 §"Composite Identifiers and Case"), served by
/// `idx_node_arch_subsume`.
///
/// The second is `lineage`, the stored specialisation graph
/// ([`ArchetypeLineage`] resolved from the `specialize` clauses of the ADL2 and
/// OPT2 family before the SQL is built), since "for specialised archetypes, the
/// specialisation lineage can only be obtained from the operational form of the
/// archetype, found in the template used to create the data" (AM
/// `Identification` master07). Every stored descendant joins the matching set on
/// its own `(arch_entity, arch_major, arch_concept)` triple, a declared parent
/// reference being free to name another major. A queried identifier with no
/// stored family resolves to itself.
///
/// For at- and id-codes, arbitrary strings and params resolving to non-HRIDs the
/// predicate is the case-folded equality on `archetype`, served by the
/// `idx_node_archetype_lower` functional index.
//
// The `-`-prefix rule is exact for an ADL 1.4-form id (major-only `.vN`,
// lineage encoded directly in the concept) and is applied ONLY to that form: AM
// `Identification` master03 §Legacy ADL 1.4 Semantics strips the separator of
// all meaning for AOM2-era identifiers ("the '-' character is still be allowed,
// but no longer has any semantic significance").
// NOTE: QUERY master03 §Archetype predicate equates the predicate to
// `archetype_node_id` string equality; we implement BASE/AM subsumption instead,
// since a parent archetype must retrieve its children (master10).
pub(super) fn archetype_predicate(node: &str, value: &str, lineage: &ArchetypeLineage) -> Expr {
    // Both eras' identifier forms are read here, through the SAME decomposition
    // the lineage index is built with — a form one side accepts and the other
    // declines would silently narrow the answer instead of erroring.
    let Some(decomposed) = decompose_hrid(value) else {
        return archetype_equality(node, value);
    };
    let queried = decomposed.key;
    let legacy_form = decomposed.legacy_form;

    // The matching set, grouped by the (entity, major) interface boundary each
    // member is scoped to. `BTreeMap`/`BTreeSet` keep the emitted SQL
    // byte-deterministic for a given input.
    let mut groups: BTreeMap<(String, i32), BTreeSet<String>> = BTreeMap::new();
    groups
        .entry((queried.entity.clone(), queried.major))
        .or_default()
        .insert(queried.concept.clone());
    for descendant in lineage.descendants(&queried) {
        groups
            .entry((descendant.entity, descendant.major))
            .or_default()
            .insert(descendant.concept);
    }

    let mut cond: Option<Expr> = None;
    for ((entity, group_major), concepts) in groups {
        let own_group = entity == queried.entity && group_major == queried.major;
        let mut concept_cond = concept_match(node, &concepts);
        if legacy_form && own_group {
            let child_prefix = format!("{}-%", like_escape(&queried.concept));
            concept_cond = concept_cond.or(col(node, Node::ArchConcept).like(child_prefix));
        }
        let group_cond = col(node, Node::ArchEntity)
            .eq(Expr::val(entity))
            .and(col(node, Node::ArchMajor).eq(Expr::val(group_major)))
            .and(concept_cond);
        cond = Some(match cond {
            None => group_cond,
            Some(previous) => previous.or(group_cond),
        });
    }
    // `groups` always holds the queried identity, so the fallback is
    // unreachable; it keeps the function total without a panic.
    cond.unwrap_or_else(|| archetype_equality(node, value))
}

/// `arch_concept` against a matching set: plain equality for the single-member
/// set (the overwhelming case — no stored family), else `= ANY(ARRAY[…])`, the
/// form `PostgreSQL` normalizes an `OR`/`IN` list to anyway
/// (<https://www.postgresql.org/docs/18/functions-comparisons.html>). Both
/// shapes are index-servable by `idx_node_arch_subsume`.
fn concept_match(node: &str, concepts: &BTreeSet<String>) -> Expr {
    let members: Vec<String> = concepts.iter().cloned().collect();
    match members.as_slice() {
        [only] => col(node, Node::ArchConcept).eq(Expr::val(only.clone())),
        _ => col(node, Node::ArchConcept).eq(Expr::from(PgFunc::any(Expr::val(members)))),
    }
}

/// Case-insensitive equality on the `archetype` column (the non-HRID
/// fallback: at/id-codes and arbitrary strings). The column is case-folded
/// at write (`storage::codec` — BASE `base_types` master05 §"Composite
/// Identifiers and Case"), so folding the BIND VALUE alone yields plain
/// indexed column equality with honest planner statistics.
fn archetype_equality(node: &str, value: &str) -> Expr {
    col(node, Node::Archetype).eq(Expr::val(value.to_ascii_lowercase()))
}

/// Escape the SQL `LIKE` metacharacters (`%`, `_`, `\`) in a literal prefix.
/// `domain_concept` segments are alphanumeric + `-` per the master05 grammar, so
/// this is defensive; the default `PostgreSQL` `LIKE` escape character `\` matches.
fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// ── test-only inline renderers ─────────────────────────────────────────────

/// Render the `archetype_node_id` predicate for `value` to inline `PostgreSQL`
/// text (values folded in, not parameterized) so tests can assert the emitted
/// condition.
#[cfg(test)]
pub(super) fn archetype_predicate_sql(value: &str, lineage: &ArchetypeLineage) -> String {
    use sea_query::{PostgresQueryBuilder, Query};

    let cond = archetype_predicate("n", value, lineage);
    let mut q = Query::select();
    q.expr(Expr::val(1)).and_where(cond);
    q.to_string(PostgresQueryBuilder)
}

#[cfg(test)]
mod tests {
    use super::{ArchetypeLineage, aql_like_to_sql, archetype_predicate_sql};

    /// The predicate as rendered with no stored ADL2/OPT2 family — the
    /// overwhelming case, and the one the pre-lineage behaviour is pinned on.
    fn predicate_sql(value: &str) -> String {
        archetype_predicate_sql(value, &ArchetypeLineage::default())
    }

    /// `\*` / `\?` in a LIKE pattern are the literal characters (§Operators/LIKE).
    #[test]
    fn like_escapes_are_literal() {
        assert_eq!(aql_like_to_sql("a*b"), "a%b");
        assert_eq!(aql_like_to_sql("a?b"), "a_b");
        assert_eq!(aql_like_to_sql(r"a\*b"), "a*b");
        assert_eq!(aql_like_to_sql(r"a\?b"), "a?b");
        assert_eq!(aql_like_to_sql("100%"), r"100\%");
    }

    /// The `archetype_node_id` predicate: a full archetype HRID lowers to the
    /// subsumption condition (parent + specialisation children, same entity +
    /// major); an at-code keeps the case-folded equality. BASE
    /// `architecture_overview` master10 §Design-time Relationships; AM master07
    /// §Querying.
    #[test]
    fn archetype_predicate_subsumption_sql() {
        // Parent archetype: matches its own concept OR any `laboratory-` child,
        // scoped to the same qualified RM entity + major.
        let parent = predicate_sql("openEHR-EHR-OBSERVATION.laboratory.v1");
        assert!(
            parent.contains(r#""n"."arch_entity" = 'openehr-ehr-observation'"#),
            "entity match (lowercased): {parent}"
        );
        assert!(
            parent.contains(r#""n"."arch_major" = 1"#),
            "major bound: {parent}"
        );
        assert!(
            parent.contains(r#""n"."arch_concept" = 'laboratory'"#),
            "exact concept match: {parent}"
        );
        assert!(
            parent.contains(r#""n"."arch_concept" LIKE 'laboratory-%'"#),
            "specialisation-child prefix match: {parent}"
        );
        assert!(
            !parent.contains("LOWER"),
            "the HRID path does not fall back to case-folded equality: {parent}"
        );

        // A specialisation child carries its full concept, so it prefix-matches its
        // own further children too.
        let child = predicate_sql("openEHR-EHR-OBSERVATION.laboratory-glucose.v1");
        assert!(
            child.contains(r#""n"."arch_concept" = 'laboratory-glucose'"#),
            "exact specialised concept: {child}"
        );
        assert!(
            child.contains(r#""n"."arch_concept" LIKE 'laboratory-glucose-%'"#),
            "further-specialisation prefix: {child}"
        );

        // An at-code is not a full HRID → plain equality on the write-folded
        // `archetype` column (no LOWER() on either side — the column is
        // lowercased at write, the bind value here), no subsumption columns.
        let at_code = predicate_sql("AT0001");
        assert!(
            !at_code.contains("LOWER") && at_code.contains(r#""n"."archetype""#),
            "at-code compares as plain equality on the write-folded column: {at_code}"
        );
        assert!(
            at_code.contains("'at0001'"),
            "the bind value is case-folded on the way in: {at_code}"
        );
        assert!(
            !at_code.contains("arch_entity"),
            "at-code does not use the subsumption columns: {at_code}"
        );
    }

    /// An AOM2-era physical identifier (`.vMAJOR.MINOR.PATCH`) gets NO
    /// `-`-prefix subsumption: AM `Identification` master03 §Legacy ADL 1.4
    /// Semantics removes the separator's meaning, so lineage may come only
    /// from the stored family.
    #[test]
    fn aom2_era_identifier_drops_the_concept_prefix_heuristic() {
        let sql = predicate_sql("openEHR-EHR-OBSERVATION.laboratory.v1.4.2");
        assert!(
            sql.contains(r#""n"."arch_concept" = 'laboratory'"#),
            "the exact concept still matches: {sql}"
        );
        assert!(
            sql.contains(r#""n"."arch_major" = 1"#),
            "the major is the matching boundary, minor/patch are not: {sql}"
        );
        assert!(
            !sql.contains("LIKE"),
            "no `-`-prefix heuristic for an AOM2-era identifier: {sql}"
        );
    }

    /// A value that is no archetype identifier at all falls back to plain
    /// equality on the whole `archetype` column.
    ///
    /// Asserted so the fallback stays REACHABLE and honest: an `at`-code or a
    /// free string names no interface, so there is nothing to subsume over —
    /// but if the decomposition ever started accepting these, the predicate
    /// would silently widen instead of matching what was asked for.
    #[test]
    fn a_non_identifier_falls_back_to_equality() {
        for value in [
            "at0001",
            "id3",
            "not an archetype",
            "openEHR-EHR-OBSERVATION",
        ] {
            let sql = archetype_predicate_sql(value, &ArchetypeLineage::default());
            assert!(
                sql.contains(r#""n"."archetype""#),
                "{value:?} names no interface, so it matches the whole id: {sql}"
            );
            assert!(
                !sql.contains("arch_concept"),
                "{value:?} must not reach the subsumption columns: {sql}"
            );
        }
    }

    /// A stored specialisation family widens the matching set to `= ANY(…)`
    /// over the queried concept plus its stored descendants, still scoped to
    /// the entity + major each member carries (AM master07 §Supporting
    /// Archetype-based Querying).
    #[test]
    fn stored_lineage_widens_the_matching_set() {
        let lineage = ArchetypeLineage::from_parent_edges([
            (
                "openEHR-EHR-OBSERVATION.hdl_result.v1.0.0",
                "openEHR-EHR-OBSERVATION.lipid_panel.v1",
            ),
            (
                "openEHR-EHR-OBSERVATION.ldl_result.v1.0.0",
                "openEHR-EHR-OBSERVATION.lipid_panel.v1",
            ),
        ]);
        let sql = archetype_predicate_sql("openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0", &lineage);
        assert!(
            sql.contains(
                r#""n"."arch_concept" = ANY(ARRAY ['hdl_result','ldl_result','lipid_panel']"#
            ),
            "the queried concept and its stored descendants form one ANY set: {sql}"
        );
        assert!(
            !sql.contains("LIKE"),
            "the widening is lineage-derived, never a string heuristic: {sql}"
        );

        // The ADL 1.4-form of the same identity keeps its concept-prefix rule
        // ON TOP of the stored lineage — the two sources are additive.
        let legacy = archetype_predicate_sql("openEHR-EHR-OBSERVATION.lipid_panel.v1", &lineage);
        // `_` is a LIKE metacharacter, so the concept's underscore is escaped
        // (`like_escape`) — the pattern is still exactly `concept-%`.
        assert!(
            legacy.contains(r#""n"."arch_concept" LIKE E'lipid\\_panel-%'"#),
            "the 1.4-form prefix rule is unchanged: {legacy}"
        );
    }

    /// A descendant that declares a parent in another major line is matched on
    /// its OWN (entity, major) boundary — never folded into the query's.
    #[test]
    fn a_cross_major_descendant_gets_its_own_boundary() {
        let lineage = ArchetypeLineage::from_parent_edges([(
            "openEHR-EHR-EVALUATION.genetic_diagnosis.v2.0.0",
            "openEHR-EHR-EVALUATION.diagnosis.v1",
        )]);
        let sql = archetype_predicate_sql("openEHR-EHR-EVALUATION.diagnosis.v1.29.0", &lineage);
        assert!(
            sql.contains(r#""n"."arch_major" = 1"#)
                && sql.contains(r#""n"."arch_concept" = 'diagnosis'"#),
            "the queried identity keeps its own major: {sql}"
        );
        assert!(
            sql.contains(r#""n"."arch_major" = 2"#)
                && sql.contains(r#""n"."arch_concept" = 'genetic_diagnosis'"#),
            "the descendant is matched at its own major: {sql}"
        );
    }
}
