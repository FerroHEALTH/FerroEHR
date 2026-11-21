// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller's care for the subject of an EHR in a query's one SQL
//! statement: whether it holds a live care relationship at all, and whether
//! it holds one that meets the care roles and path a rule names
//! ([`crate::access_eval::care_tie::CareNeed`]), read from the census the
//! statement binds once, from the ties of the EHRs a query is pinned to, or
//! from the clinical care projection. No openEHR spec governs this: our own
//! design (#5645, #5699, #6075).

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, CommonTableExpression, Expr, ExprTrait, Query, WithClause};
use uuid::Uuid;

use super::access::CARE;
use super::expr::col;
use crate::access_eval::care_tie::CareNeed;
use crate::access_eval::condition::care::{Care, CaredEhr, CaredSubject, ProjectedCare};
use crate::access_eval::condition::{AccessCondition, ConditionGroups};
use crate::db::iden::Ehr;
use crate::storage::access_projection::EhrSubject;

/// The common table that holds the subjects the caller cares for.
const CARED: &str = "access_cared";
/// The one column of [`CARED`]: a `jsonb` object keyed by subject id.
const SUBJECTS: &str = "subjects";

/// The `WITH` clause that binds the subjects the caller cares for once per
/// statement, which every [`cared`] probe reads; `None` when no verdict of
/// the condition reads care.
///
/// The subjects are one `jsonb` object keyed by subject id: the planner
/// estimates `jsonb ? text` at a fixed selectivity whatever the census, where
/// it walks a bound array element by element, and a probe is one search of
/// the object's sorted keys (PostgreSQL 18 §9.16, `jsonb ? text`). No openEHR
/// spec governs this: our own design (#5645).
pub(super) fn census(condition: &AccessCondition) -> Option<WithClause> {
    if condition.pinned().is_some() {
        return None;
    }
    let condition = condition.groups()?;
    let pairs = std::iter::once(&condition.closed)
        .chain(condition.groups.iter().map(|(_, pair)| pair))
        .chain(condition.default.iter());
    if pairs.into_iter().all(|pair| pair.cared == pair.uncared) {
        return None;
    }
    let Care::Census(cared) = &condition.cared else {
        return None;
    };
    let mut row = Query::select();
    row.expr_as(
        Expr::val(census_text(cared)).cast_as(Alias::new("jsonb")),
        Alias::new(SUBJECTS),
    );
    let mut table = CommonTableExpression::new();
    table.query(row).table_name(Alias::new(CARED));
    let mut with = WithClause::new();
    with.cte(table);
    Some(with)
}

/// The [`census`] of the subjects `cared`: a `jsonb` object text keyed by
/// [`census_key`], each value the array of the tokens of the ties by which
/// the caller holds the subject ([`crate::access_eval::care_tie::Tie::tokens`]),
/// which a [`care_need`] probe reads.
pub(super) fn census_text(cared: &[CaredSubject]) -> String {
    let mut keys: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for cared in cared {
        let tokens = keys.entry(census_key(&cared.subject)).or_default();
        tokens.extend(
            cared
                .ties
                .iter()
                .flat_map(crate::access_eval::care_tie::Tie::tokens),
        );
    }
    serde_json::Value::Object(
        keys.into_iter()
            .map(|(key, tokens)| {
                (
                    key,
                    tokens.into_iter().map(serde_json::Value::String).collect(),
                )
            })
            .collect(),
    )
    .to_string()
}

/// The ties by which the caller holds each EHR of `ehrs`: a `jsonb` object
/// text keyed by EHR id, each value the array of the tokens of its ties,
/// which a [`care_need`] probe of a query pinned to its EHRs reads.
pub(super) fn ehr_ties_text(ehrs: &[CaredEhr]) -> String {
    serde_json::Value::Object(
        ehrs.iter()
            .map(|cared| {
                let tokens: std::collections::BTreeSet<String> = cared
                    .ties
                    .iter()
                    .flat_map(crate::access_eval::care_tie::Tie::tokens)
                    .collect();
                (
                    cared.ehr.to_string(),
                    tokens.into_iter().map(serde_json::Value::String).collect(),
                )
            })
            .collect(),
    )
    .to_string()
}

/// The key of `subject` in the [`census`]: its namespace and its id joined by
/// U+001F, which no `OBJECT_REF.namespace` holds (BASE `OBJECT_REF`), as a
/// [`cared`] probe builds it from the `ehr` row.
fn census_key(subject: &EhrSubject) -> String {
    format!("{}\u{1f}{}", subject.namespace, subject.id)
}

/// Whether the subject of the EHR `target` is under the care of the caller:
/// a `CASE` condition, since an EHR names one subject. The subjects are the
/// [`census`] the statement binds once, each keyed by [`census_key`]: its
/// reference matches a cared one by namespace and id, the id of a reference
/// in this CDR's own namespace by its container's UUID in any case, as
/// [`crate::party_binding::grants::live::LiveGrants::person_of`] reads it.
pub(super) fn cared(condition: &ConditionGroups, target: Expr) -> Expr {
    match &condition.cared {
        Care::Census(_) => {}
        // NOTE: our own design (#5699), a query pinned to EHRs binds the ones found cared for.
        Care::Ehrs(ehrs) => {
            let ids: Vec<Uuid> = ehrs.iter().map(|cared| cared.ehr).collect();
            return target.eq(Expr::from(PgFunc::any(Expr::val(ids))));
        }
        Care::Projected(care) => return projected(care, target),
        Care::Unresolved => return Expr::val(false),
    }
    census_probe(condition, target, |subjects, key| {
        Expr::cust_with_exprs("$1 ? $2", [subjects, key])
    })
}

/// An `EXISTS` over the `ehr` row of `target` whose subject the [`census`]
/// holds as `test` reads it, given the census and the row's [`census_key`].
fn census_probe(
    condition: &ConditionGroups,
    target: Expr,
    test: impl FnOnce(Expr, Expr) -> Expr,
) -> Expr {
    let mut subjects = Query::select();
    subjects
        .column(Alias::new(SUBJECTS))
        .from(Alias::new(CARED));
    let key = Expr::cust_with_exprs(
        "($1 || chr(31) || CASE WHEN $1 = $3 THEN lower($2) ELSE $2 END)",
        [
            col(CARE, Ehr::SubjectNamespace),
            col(CARE, Ehr::SubjectId),
            Expr::val(condition.subject_namespace.to_string()),
        ],
    );
    let mut probe = Query::select();
    probe
        .expr(Expr::val(1))
        .from_as(Ehr::Table, Alias::new(CARE))
        .and_where(col(CARE, Ehr::Id).eq(target))
        .and_where(test(Expr::SubQuery(None, Box::new(subjects.into())), key));
    Expr::exists(probe)
}

/// Whether the caller holds the subject of the EHR `target` by a tie that
/// meets `need`, read as `condition` reads care: from the census or the ties
/// of a pinned query, whose tokens it asks for ([`CareNeed::tokens`]), or
/// from the clinical care projection. Null-safe, so the fold may decide it
/// beside its negation. No openEHR spec governs this: our own design (#6075).
pub(super) fn care_need(need: &CareNeed, condition: &ConditionGroups, target: Expr) -> Expr {
    let tokens = need.tokens();
    if tokens.is_empty() {
        return Expr::val(false);
    }
    match &condition.cared {
        Care::Unresolved => Expr::val(false),
        Care::Census(_) => census_probe(condition, target, |subjects, key| {
            Expr::cust_with_exprs(
                "COALESCE(($1 -> $2) ?| CAST($3 AS text[]), false)",
                [subjects, key, Expr::val(tokens)],
            )
        }),
        Care::Ehrs(ehrs) => Expr::cust_with_exprs(
            "COALESCE((CAST($1 AS jsonb) -> CAST($2 AS text)) ?| CAST($3 AS text[]), false)",
            [Expr::val(ehr_ties_text(ehrs)), target, Expr::val(tokens)],
        ),
        Care::Projected(care) => projected_need(care, need, target),
    }
}

/// The rows of the clinical care projection, as `access_care_row`, that the
/// caller's principals (`$2`) hold in the care of a provider it acts for
/// (`$5`, `$6`), live on the day `$3` or within the grace `$4` after,
/// with the dates read as
/// [`crate::party_binding::grants::caller::care_of::CareOf::live_on`] reads
/// them.
const HELD: &str = "access_care_row.carer_id = ANY($2) \
     AND (access_care_row.provider_issuer, access_care_row.provider_id) IN \
         (SELECT * FROM unnest(CAST($5 AS text[]), CAST($6 AS text[]))) \
     AND (access_care_row.care_from IS NULL OR access_care_row.care_from <= CAST($3 AS date)) \
     AND (access_care_row.care_until IS NULL \
          OR CAST($3 AS date) < access_care_row.care_until \
          OR CAST($3 AS date) < access_care_row.care_until + CAST($4 AS interval))";

/// Of [`HELD`], a row that admits the caller as
/// [`crate::party_binding::grants::caller::care_of::CareOf::admits_writer`]
/// does: a self-declared one only where the policy turns that path on
/// (`$7`), any other one only when no principal of the caller wrote it.
const ADMITS: &str = "CASE WHEN access_care_row.self_declared THEN CAST($7 AS boolean) \
     ELSE access_care_row.writer_id IS NULL OR access_care_row.writer_id <> ALL($2) END";

/// The values [`HELD`] and [`ADMITS`] bind as `$2` to `$7`, after the EHR
/// `target` as `$1`.
fn held_values(care: &ProjectedCare, target: Expr) -> Vec<Expr> {
    vec![
        target,
        Expr::val(care.principals.to_vec()),
        Expr::val(care.day.to_string()),
        Expr::val(care.grace.clone()),
        Expr::val(care.provider_issuers.to_vec()),
        Expr::val(care.provider_ids.to_vec()),
        Expr::val(care.self_declared),
    ]
}

/// Whether the subject of the EHR `target` is under the care of one of the
/// caller's principals, in the care of a provider it acts for, read from the
/// clinical care projection by the EHR row's subject digest: one probe of
/// `ix_access_care_subject`, binding the principals, the providers, the day
/// and the grace ([`HELD`], [`ADMITS`]). No openEHR spec governs this: our
/// own design (#5699, #6055, #6073).
fn projected(care: &ProjectedCare, target: Expr) -> Expr {
    Expr::cust_with_exprs(
        format!(
            "EXISTS (SELECT 1 FROM ehr access_care_ehr \
             JOIN access_care access_care_row \
               ON access_care_row.subject_digest = access_care_ehr.subject_digest \
             WHERE access_care_ehr.id = $1 AND {HELD} AND {ADMITS})"
        ),
        held_values(care, target),
    )
}

/// The caseload bound on the EHR `target` of a statement whose condition
/// admits no row of an EHR outside the caller's care: the EHRs whose subject
/// the clinical care projection holds in the caller's care as [`projected`]
/// reads it, beside every EHR of a settings class the condition does not
/// know (`known`), whose rows the condition must still reach so the
/// statement fails on them ([`super::access`]). `None` unless the condition
/// reads care from the projection.
///
/// The bound only restates what the condition already requires, so it admits
/// no row the condition refuses; it is an uncorrelated `IN`, which
/// PostgreSQL turns into a semi-join, so the planner may start from the
/// caller's rows of `ix_access_care_caseload` and reach each EHR by
/// `ix_ehr_subject_digest`, or keep starting from the clinical rows and probe
/// per row, by the statistics of the caseload's size (PostgreSQL 18 §14.1).
/// No openEHR spec governs this: our own design (#6074).
pub(super) fn caseload(condition: &ConditionGroups, target: Expr, known: Vec<i64>) -> Option<Expr> {
    let Care::Projected(care) = &condition.cared else {
        return None;
    };
    let mut values = held_values(care, target);
    values.push(Expr::val(known));
    Some(Expr::cust_with_exprs(
        format!(
            "$1 IN (SELECT access_care_ehr.id FROM access_care access_care_row \
             JOIN ehr access_care_ehr \
               ON access_care_ehr.subject_digest = access_care_row.subject_digest \
             WHERE {HELD} AND {ADMITS} \
             UNION ALL SELECT access_unknown.ehr_id FROM access_settings access_unknown \
             WHERE access_unknown.class_id IN \
                 (SELECT access_known.id FROM access_class access_known \
                  WHERE access_known.id <> ALL($8)))"
        ),
        values,
    ))
}

/// Whether the subject of the EHR `target` is under the care of the caller
/// by a relationship that meets `need`, read from the clinical care
/// projection as [`projected`] reads it: its role one of the need's roles,
/// and its carer a direct principal for `direct` (assigned), a principal of
/// the caller that is no direct one for `team` (assigned), or a direct
/// principal for `self_declared` where the policy turns that path on; the
/// writer of an assigned one is never a principal of the caller. No openEHR
/// spec governs this: our own design (#6075).
fn projected_need(care: &ProjectedCare, need: &CareNeed, target: Expr) -> Expr {
    use openehr_rm::draft::access_control::terminology::care::CarePath;
    let assigned = "NOT access_care_row.self_declared \
         AND (access_care_row.writer_id IS NULL OR access_care_row.writer_id <> ALL($2))";
    let path = match need.path() {
        None => ADMITS.to_owned(),
        Some(CarePath::Direct) => format!("access_care_row.carer_id = ANY($8) AND {assigned}"),
        Some(CarePath::Team) => format!("access_care_row.carer_id <> ALL($8) AND {assigned}"),
        Some(CarePath::SelfDeclared) => "access_care_row.carer_id = ANY($8) \
                 AND access_care_row.self_declared AND CAST($7 AS boolean)"
            .to_owned(),
        Some(CarePath::Attested) => return Expr::val(false),
    };
    let (terminologies, codes): (Vec<String>, Vec<String>) = need
        .roles()
        .unwrap_or_default()
        .iter()
        .map(|role| (role.terminology.clone(), role.code.clone()))
        .unzip();
    let roles = if need.roles().is_some() {
        " AND (access_care_row.role_terminology, access_care_row.role_code) IN \
             (SELECT * FROM unnest(CAST($9 AS text[]), CAST($10 AS text[])))"
    } else {
        ""
    };
    let statement = format!(
        "EXISTS (SELECT 1 FROM ehr access_care_ehr \
         JOIN access_care access_care_row \
           ON access_care_row.subject_digest = access_care_ehr.subject_digest \
         WHERE access_care_ehr.id = $1 AND {HELD} AND {path}{roles})"
    );
    let mut values = held_values(care, target);
    values.extend([
        Expr::val(care.direct.to_vec()),
        Expr::val(terminologies),
        Expr::val(codes),
    ]);
    Expr::cust_with_exprs(statement, values)
}
