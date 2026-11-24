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
use crate::db::iden::{Ehr, VoHead};
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
    let groups = condition.groups()?;
    let pairs = std::iter::once(&groups.closed)
        .chain(groups.groups.iter().map(|(_, pair)| pair))
        .chain(groups.default.iter());
    if pairs.into_iter().all(|pair| pair.cared == pair.uncared) {
        return None;
    }
    let condition = groups;
    let Care::Census(cared) = &condition.cared else {
        return None;
    };
    let mut row = Query::select();
    row.expr_as(
        Expr::val(census_text(cared, condition.episodic)).cast_as(Alias::new("jsonb")),
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
/// the caller holds the subject ([`tie_tokens`]), which a [`care_need`] and
/// a [`covered`] probe read.
pub(super) fn census_text(cared: &[CaredSubject], episodic: bool) -> String {
    let mut keys: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for cared in cared {
        let tokens = keys.entry(census_key(&cared.subject)).or_default();
        for tie in cared.ties.iter() {
            tie_tokens(tie, episodic, tokens);
        }
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

/// The subjects `subjects` as a `jsonb` object text keyed by
/// [`census_key`], which a caseload probe asks with `?`.
pub(super) fn census_text_of(subjects: &[EhrSubject]) -> String {
    serde_json::Value::Object(
        subjects
            .iter()
            .map(|subject| (census_key(subject), serde_json::Value::Bool(true)))
            .collect(),
    )
    .to_string()
}

/// The tokens of `tie` ([`crate::access_eval::care_tie::Tie::tokens`]) into
/// `tokens`, and for an `episodic` condition each of them again joined by
/// U+001F to the hexadecimal key of the tie's episode, or to `*` for a tie
/// that names none, which a [`covered`] probe reads.
fn tie_tokens(
    tie: &crate::access_eval::care_tie::Tie,
    episodic: bool,
    tokens: &mut std::collections::BTreeSet<String>,
) {
    let own = tie.tokens();
    if episodic {
        let scope = tie
            .episode()
            .map_or_else(|| "*".to_owned(), |key| key.hex());
        tokens.extend(own.iter().map(|token| format!("{token}\u{1f}{scope}")));
    }
    tokens.extend(own);
}

/// The ties by which the caller holds each EHR of `ehrs`: a `jsonb` object
/// text keyed by EHR id, each value the array of the tokens of its ties
/// ([`tie_tokens`]), which a [`care_need`] and a [`covered`] probe of a
/// query pinned to its EHRs read.
pub(super) fn ehr_ties_text(ehrs: &[CaredEhr], episodic: bool) -> String {
    serde_json::Value::Object(
        ehrs.iter()
            .map(|cared| {
                let mut tokens = std::collections::BTreeSet::new();
                for tie in cared.ties.iter() {
                    tie_tokens(tie, episodic, &mut tokens);
                }
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
        Care::Projected(care) => return projected(care, target, !condition.episodic),
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
            [
                Expr::val(ehr_ties_text(ehrs, condition.episodic)),
                target,
                Expr::val(tokens),
            ],
        ),
        Care::Projected(care) => projected_need(care, need, target, !condition.episodic),
    }
}

/// Whether a live tie of the caller to the subject of the EHR `target`, of
/// those that meet `need` when one is named, covers the head row `h`: the
/// row is placed in no episode, or the tie names none or one the row is
/// placed in ([`crate::access_eval::scope::Pred::Covered`]). Read as
/// `condition` reads care: from the tokens the census or a pinned query
/// binds ([`tie_tokens`]), or from the episodes of the projection's rows,
/// aggregated by the EHR alone so a plan may memoize it per EHR. Null-safe.
/// No openEHR spec governs this: our own design (#6414, #6415).
pub(super) fn covered(
    need: Option<&CareNeed>,
    condition: &ConditionGroups,
    target: Expr,
    h: &str,
) -> Expr {
    let any = CareNeed::new(None, None);
    let tokens = need.unwrap_or(&any).tokens();
    if tokens.is_empty() {
        return Expr::val(false);
    }
    let placed = col(h, VoHead::Episodes);
    let scoped = |ties: Expr, placed: Expr| {
        Expr::cust_with_exprs(
            "COALESCE($1 ?| ARRAY(SELECT access_need || chr(31) || access_scope \
             FROM unnest(CAST($2 AS text[])) AS access_need \
             CROSS JOIN unnest(array_append(ARRAY(SELECT encode(access_placed, 'hex') \
                 FROM unnest($3) AS access_placed), '*')) AS access_scope), false)",
            [ties, Expr::val(tokens.clone()), placed],
        )
    };
    let by_ties = match &condition.cared {
        Care::Unresolved => return Expr::val(false),
        Care::Census(_) => census_probe(condition, target, |subjects, key| {
            scoped(
                Expr::cust_with_exprs("($1 -> $2)", [subjects, key]),
                placed.clone(),
            )
        }),
        Care::Ehrs(ehrs) => scoped(
            Expr::cust_with_exprs(
                "(CAST($1 AS jsonb) -> CAST($2 AS text))",
                [Expr::val(ehr_ties_text(ehrs, condition.episodic)), target],
            ),
            placed.clone(),
        ),
        Care::Projected(care) => Expr::cust_with_exprs(
            "COALESCE($1 && array_append($2, CAST('' AS bytea)), false)",
            [projected_episodes(care, need, target), placed.clone()],
        ),
    };
    Expr::cust_with_exprs("($1 IS NULL OR $2)", [placed, by_ties])
}

/// The rows of the clinical care projection, as `access_care_row`, that the
/// caller's principals (`$2`) hold in the care of a provider it acts for
/// (`$5`, `$6`), live on the day `$3` or within the grace `$4` after,
/// with the dates read as
/// [`crate::party_binding::grants::caller::care_of::CareOf::live_on`] reads
/// them.
const HELD_ANY: &str = "access_care_row.carer_id = ANY($2) \
     AND (access_care_row.provider_issuer, access_care_row.provider_id) IN \
         (SELECT * FROM unnest(CAST($5 AS text[]), CAST($6 AS text[]))) \
     AND (access_care_row.care_from IS NULL OR access_care_row.care_from <= CAST($3 AS date)) \
     AND (access_care_row.care_until IS NULL \
          OR CAST($3 AS date) < access_care_row.care_until \
          OR CAST($3 AS date) < access_care_row.care_until + CAST($4 AS interval))";

/// [`HELD_ANY`], of the rows scoped to no episode alone when `unscoped`: a
/// condition that leaves no coverage to the statement counts no row scoped
/// to an episode as care of the whole record, even one the projection holds
/// before the caller facts copy does.
fn held(unscoped: bool) -> String {
    if unscoped {
        format!("{HELD_ANY} AND access_care_row.episode_digest IS NULL")
    } else {
        HELD_ANY.to_owned()
    }
}

/// Of [`HELD_ANY`], a row that admits the caller as
/// [`crate::party_binding::grants::caller::care_of::CareOf::admits_writer`]
/// does: a self-declared one only where the policy turns that path on
/// (`$7`), any other one only when no principal of the caller wrote it.
const ADMITS: &str = "CASE WHEN access_care_row.self_declared THEN CAST($7 AS boolean) \
     ELSE access_care_row.writer_id IS NULL OR access_care_row.writer_id <> ALL($2) END";

/// The values [`HELD_ANY`] and [`ADMITS`] bind as `$2` to `$7`, after the EHR
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
/// and the grace ([`held`], [`ADMITS`]). No openEHR spec governs this: our
/// own design (#5699, #6055, #6073).
fn projected(care: &ProjectedCare, target: Expr, unscoped: bool) -> Expr {
    let held = held(unscoped);
    Expr::cust_with_exprs(
        format!(
            "EXISTS (SELECT 1 FROM ehr access_care_ehr \
             JOIN access_care access_care_row \
               ON access_care_row.subject_digest = access_care_ehr.subject_digest \
             WHERE access_care_ehr.id = $1 AND {held} AND {ADMITS})"
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
    let held = held(!condition.episodic);
    Some(Expr::cust_with_exprs(
        format!(
            "$1 IN (SELECT access_care_ehr.id FROM access_care access_care_row \
             JOIN ehr access_care_ehr \
               ON access_care_ehr.subject_digest = access_care_row.subject_digest \
             WHERE {held} AND {ADMITS} \
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
pub(super) fn projected_need(
    care: &ProjectedCare,
    need: &CareNeed,
    target: Expr,
    unscoped: bool,
) -> Expr {
    let Some((rows, values)) = need_rows(care, Some(need), target, unscoped) else {
        return Expr::val(false);
    };
    Expr::cust_with_exprs(format!("EXISTS (SELECT 1 {rows})"), values)
}

/// The episodes of the projection's rows by which the caller holds the
/// subject of the EHR `target`, of those that meet `need` when one is named,
/// as [`projected_need`] reads them: an empty `bytea` for a row scoped to
/// none; NULL when no row holds.
fn projected_episodes(care: &ProjectedCare, need: Option<&CareNeed>, target: Expr) -> Expr {
    let Some((rows, values)) = need_rows(care, need, target, false) else {
        return Expr::cust("CAST(NULL AS bytea[])");
    };
    Expr::cust_with_exprs(
        format!(
            "(SELECT array_agg(COALESCE(access_care_row.episode_digest, CAST('' AS bytea))) \
             {rows})"
        ),
        values,
    )
}

/// The `FROM` and `WHERE` of the projection's rows by which the caller holds
/// the subject of the EHR `$1` by a relationship that meets `need`, any one
/// it holds for `None`, with the values they bind; `None` for a need no
/// row meets.
fn need_rows(
    care: &ProjectedCare,
    need: Option<&CareNeed>,
    target: Expr,
    unscoped: bool,
) -> Option<(String, Vec<Expr>)> {
    use openehr_rm::draft::access_control::terminology::care::CarePath;
    let assigned = "NOT access_care_row.self_declared \
         AND (access_care_row.writer_id IS NULL OR access_care_row.writer_id <> ALL($2))";
    let path = match need.and_then(CareNeed::path) {
        None => ADMITS.to_owned(),
        Some(CarePath::Direct) => format!("access_care_row.carer_id = ANY($8) AND {assigned}"),
        Some(CarePath::Team) => format!("access_care_row.carer_id <> ALL($8) AND {assigned}"),
        Some(CarePath::SelfDeclared) => "access_care_row.carer_id = ANY($8) \
                 AND access_care_row.self_declared AND CAST($7 AS boolean)"
            .to_owned(),
        Some(CarePath::Attested) => return None,
    };
    let named = need.and_then(CareNeed::roles);
    let (terminologies, codes): (Vec<String>, Vec<String>) = named
        .unwrap_or_default()
        .iter()
        .map(|role| (role.terminology.clone(), role.code.clone()))
        .unzip();
    let roles = if named.is_some() {
        " AND (access_care_row.role_terminology, access_care_row.role_code) IN \
             (SELECT * FROM unnest(CAST($9 AS text[]), CAST($10 AS text[])))"
    } else {
        ""
    };
    let held = held(unscoped);
    let rows = format!(
        "FROM ehr access_care_ehr \
         JOIN access_care access_care_row \
           ON access_care_row.subject_digest = access_care_ehr.subject_digest \
         WHERE access_care_ehr.id = $1 AND {held} AND {path}{roles}"
    );
    let mut values = held_values(care, target);
    if need.is_some() {
        values.extend([
            Expr::val(care.direct.to_vec()),
            Expr::val(terminologies),
            Expr::val(codes),
        ]);
    }
    Some((rows, values))
}
