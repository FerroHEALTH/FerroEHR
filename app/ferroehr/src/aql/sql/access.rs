// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator's condition in a query's one SQL statement.
//!
//! The evaluator judges every settings class, an EHR held closed and the EHR
//! with none in memory ([`crate::access_eval::condition::RequestAccess::condition`]);
//! this module renders the verdicts as predicates on the head row of each
//! versioned object a query reads, and on the id of each EHR it reads, so
//! `COUNT`, `LIMIT` and paging see only the rows the caller may read and
//! nothing else leaves the database. The EHR's projected settings row is
//! joined by its id, its class selects the verdict, and each fact the
//! verdict leaves to the EHR is read from the row's facts; an EHR of a class
//! the condition does not know contributes no row. Each [`Pred`] reads the
//! same row attributes the in-memory check reads: the object id, the
//! template, the root archetype, the origins of the trunk head and its commit
//! time. Every atom is null-safe, so a missing attribute decides by the
//! polarity of the fact and never by SQL's unknown. No openEHR spec governs
//! this: our own design (#5513, #5588).

use sea_query::extension::postgres::PgFunc;
use sea_query::{
    Alias, CommonTableExpression, Expr, ExprTrait, Func, Query, SelectStatement, WithClause,
};
use uuid::Uuid;

use super::expr::{archetype_node_id, col};
use crate::access_eval::class::Keepers;
use crate::access_eval::condition::{AccessCondition, CallerKeys, ConditionGroups, VerdictPair};
use crate::access_eval::scope::{
    CategoryMatch, DataMatch, Edge, EhrFact, Held, Polarity, Pred, Recorded,
};
use crate::db::iden::{AccessSettings, Ehr, Node, VersionRow, VoHead};
use crate::storage::access_projection::EhrSubject;

/// The alias of the root node an archetype probe reads.
const ROOT: &str = "access_root";
/// The alias of the trunk head version an origin probe reads.
const TRUNK: &str = "access_trunk";
/// The alias of the `ehr` row a care probe reads.
const CARE: &str = "access_ehr";
/// The alias of the projected settings row of the EHR.
const SETTINGS: &str = "access_settings_row";
/// The common table that holds the subjects the caller cares for.
const CARED: &str = "access_cared";
/// The one column of [`CARED`]: a `jsonb` object keyed by subject id.
const SUBJECTS: &str = "subjects";

/// What a predicate is read against: the caller's facts, which a class's
/// facts are compared with; `None` outside a class.
#[derive(Clone, Copy)]
struct Facts<'a> {
    caller: Option<&'a CallerKeys>,
}

/// The predicate on the head row `h` of a versioned object; `None` when the
/// condition admits every row.
pub(super) fn on_head(condition: &AccessCondition, h: &str) -> Option<Expr> {
    let condition = condition.groups()?;
    let target = col(h, VoHead::EhrId);
    let admits = |pair: &VerdictPair, facts: Facts<'_>| -> Option<Expr> {
        if pair.admits_all() {
            Some(Expr::val(true))
        } else if pair.admits_none() {
            None
        } else {
            Some(admitted(condition, pair, h, target.clone(), facts))
        }
    };
    let caller = Facts {
        caller: Some(&condition.facts),
    };
    let none = Facts { caller: None };
    let mut branches = Vec::new();
    if let Some(admitted) = admits(&condition.closed, none) {
        branches.push(col(SETTINGS, AccessSettings::HeldClosed).and(admitted));
    }
    for (ids, pair) in &condition.groups {
        if let Some(admitted) = admits(pair, caller) {
            branches.push(in_classes(ids).and(admitted));
        }
    }
    let default = condition
        .default
        .as_deref()
        .map(|pair| admits(pair, none).unwrap_or_else(|| Expr::val(false)));
    let held = by_settings(target.clone(), or_all(branches), default);
    Some(confine(condition, &target, held))
}

/// The predicate on the EHR id `target` of an EHR a query reads: an EHR no
/// item of which the caller may read is left out; whether an item is read is
/// [`on_head`]'s.
pub(super) fn on_ehr_id(condition: &AccessCondition, target: &Expr) -> Option<Expr> {
    let condition = condition.groups()?;
    let reaches = |pair: &VerdictPair, facts: Facts<'_>| -> Option<Expr> {
        let (cared, uncared) = (
            simplified(&pair.cared.reaches()),
            simplified(&pair.uncared.reaches()),
        );
        if cared == Pred::False && uncared == Pred::False {
            return None;
        }
        if cared == uncared {
            return Some(pred(&cared, CARE, facts));
        }
        Some(Expr::cust_with_exprs(
            "CASE WHEN $1 THEN $2 ELSE $3 END",
            [
                self::cared(condition, target.clone()),
                pred(&cared, CARE, facts),
                pred(&uncared, CARE, facts),
            ],
        ))
    };
    let caller = Facts {
        caller: Some(&condition.facts),
    };
    let none = Facts { caller: None };
    let mut branches = Vec::new();
    if let Some(reach) = reaches(&condition.closed, none) {
        branches.push(col(SETTINGS, AccessSettings::HeldClosed).and(reach));
    }
    for (ids, pair) in &condition.groups {
        if let Some(reach) = reaches(pair, caller) {
            branches.push(in_classes(ids).and(reach));
        }
    }
    let default = condition
        .default
        .as_deref()
        .map(|pair| reaches(pair, none).unwrap_or_else(|| Expr::val(false)));
    let held = by_settings(target.clone(), or_all(branches), default);
    Some(confine(condition, target, held))
}

/// `held` on the EHR `target`, and for a subject caller only in the EHRs the
/// condition confines it to (#5586).
fn confine(condition: &ConditionGroups, target: &Expr, held: Expr) -> Expr {
    match &condition.confined {
        Some(ehrs) => target
            .clone()
            .eq(Expr::from(PgFunc::any(Expr::val(ehrs.to_vec()))))
            .and(held),
        None => held,
    }
}

/// The predicate on the head row `h` that only a gate-keeper `keepers` names
/// reads an `EHR_ACCESS` object.
pub(super) fn keeper_reads(keepers: &Keepers, h: &str) -> Expr {
    let facts = CallerKeys {
        now: jiff::Timestamp::UNIX_EPOCH,
        parties: Vec::new().into(),
        keeper: keepers.party.clone(),
    };
    let facts = Facts {
        caller: Some(&facts),
    };
    let branches = keepers
        .groups
        .iter()
        .map(|(ids, named)| in_classes(ids).and(pred(&simplified(named), h, facts)))
        .collect();
    let mut probe = settings_row(col(h, VoHead::EhrId));
    probe.and_where(or_all(branches));
    Expr::exists(probe)
}

/// Whether the EHR `target` carries `EHR_ACCESS` settings: one probe of the
/// row by its key, as a scalar subquery, which PostgreSQL runs per row and
/// never turns into a hashed scan of every settings row (#5701).
pub(super) fn settled(target: Expr) -> Expr {
    let mut row = settings_row(target);
    row.clear_selects();
    row.expr(Expr::val(true));
    Expr::cust_with_exprs(
        "COALESCE($1, false)",
        [Expr::SubQuery(None, Box::new(row.into()))],
    )
}

/// For the EHR `target`: with a settings row, `with` over that row; with
/// none, `default`, or every row when `default` is `None`.
///
/// One scalar probe of the settings row by its key decides both: it answers
/// `with` (an unknown `with` is false) when the row exists and NULL when it
/// does not, which `default` then replaces. An `EXISTS` beside a `NOT
/// EXISTS` under an `OR` was planned as two hashed `SubPlans` over every
/// settings row before the first row came back (#5701). No openEHR spec
/// governs this: our own design.
fn by_settings(target: Expr, with: Expr, default: Option<Expr>) -> Expr {
    let mut row = settings_row(target);
    row.clear_selects();
    row.expr(Expr::cust_with_exprs("COALESCE($1, false)", [with]));
    Expr::cust_with_exprs(
        "COALESCE($1, $2)",
        [
            Expr::SubQuery(None, Box::new(row.into())),
            default.unwrap_or_else(|| Expr::val(true)),
        ],
    )
}

/// The settings row of the EHR `target`.
fn settings_row(target: Expr) -> SelectStatement {
    let mut row = Query::select();
    row.expr(Expr::val(1))
        .from_as(AccessSettings::Table, Alias::new(SETTINGS))
        .and_where(col(SETTINGS, AccessSettings::EhrId).eq(target));
    row
}

/// The settings row is of one of the classes `ids`.
fn in_classes(ids: &[i64]) -> Expr {
    col(SETTINGS, AccessSettings::ClassId).eq(Expr::from(PgFunc::any(Expr::val(ids.to_vec()))))
}

/// The rows of an EHR of `pair` of `condition` admitted, on head row `h` of
/// EHR `target`.
fn admitted(
    condition: &ConditionGroups,
    pair: &VerdictPair,
    h: &str,
    target: Expr,
    facts: Facts<'_>,
) -> Expr {
    let (with, without) = (
        simplified(&pair.cared.admitted()),
        simplified(&pair.uncared.admitted()),
    );
    if with == without {
        return pred(&with, h, facts);
    }
    Expr::cust_with_exprs(
        "CASE WHEN $1 THEN $2 ELSE $3 END",
        [
            cared(condition, target),
            pred(&with, h, facts),
            pred(&without, h, facts),
        ],
    )
}

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
    let condition = condition.groups()?;
    let pairs = std::iter::once(&condition.closed)
        .chain(condition.groups.iter().map(|(_, pair)| pair))
        .chain(condition.default.iter());
    if pairs.into_iter().all(|pair| pair.cared == pair.uncared) {
        return None;
    }
    // NOTE: our own design (#5667), each key is escaped by serde_json as it is written, with
    // no map to build: jsonb keeps one of a repeated key, so `?` answers as before.
    let keys: Vec<String> = condition
        .cared
        .iter()
        .map(|subject| format!("{}:true", serde_json::Value::String(census_key(subject))))
        .collect();
    let mut row = Query::select();
    row.expr_as(
        Expr::val(format!("{{{}}}", keys.join(","))).cast_as(Alias::new("jsonb")),
        Alias::new(SUBJECTS),
    );
    let mut table = CommonTableExpression::new();
    table.query(row).table_name(Alias::new(CARED));
    let mut with = WithClause::new();
    with.cte(table);
    Some(with)
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
fn cared(condition: &ConditionGroups, target: Expr) -> Expr {
    let mut subjects = Query::select();
    subjects
        .column(Alias::new(SUBJECTS))
        .from(Alias::new(CARED));
    let mut probe = Query::select();
    probe
        .expr(Expr::val(1))
        .from_as(Ehr::Table, Alias::new(CARE))
        .and_where(col(CARE, Ehr::Id).eq(target))
        .and_where(Expr::cust_with_exprs(
            "$1 ? ($2 || chr(31) || CASE WHEN $2 = $4 THEN lower($3) ELSE $3 END)",
            [
                Expr::SubQuery(None, Box::new(subjects.into())),
                col(CARE, Ehr::SubjectNamespace),
                col(CARE, Ehr::SubjectId),
                Expr::val(condition.subject_namespace.to_string()),
            ],
        ));
    Expr::exists(probe)
}

/// `pred` with what its SQL would decide alike folded away: a scope that
/// names an empty list covers nothing, a repeated conjunct or disjunct counts
/// once, and a two-valued predicate beside its own negation decides the
/// conjunction (false) or the disjunction (true).
///
/// The folds keep SQL's three values: only a predicate whose SQL is never
/// unknown ([`two_valued`]) is folded against its negation. The planner
/// plans every `EXISTS` the condition names, so each one folded away is
/// planning a query no longer pays for. No openEHR spec governs this: our own
/// design (#5667).
fn simplified(pred: &Pred) -> Pred {
    match pred {
        Pred::Not(inner) => Pred::negate(simplified(inner)),
        Pred::All(preds) => folded(Pred::all(preds.iter().map(simplified).collect()), true),
        Pred::Any(preds) => folded(Pred::any(preds.iter().map(simplified).collect()), false),
        Pred::Data(data, _) if covers_nothing(data) => Pred::False,
        Pred::True | Pred::False | Pred::Data(..) | Pred::Ehr(_) => pred.clone(),
    }
}

/// `pred`, a conjunction when `all` and a disjunction otherwise, its
/// repeated members counted once and decided by a two-valued member beside
/// its negation.
fn folded(pred: Pred, all: bool) -> Pred {
    let members = match pred {
        Pred::All(members) if all => members,
        Pred::Any(members) if !all => members,
        other => return other,
    };
    let mut kept: Vec<Pred> = Vec::with_capacity(members.len());
    for member in members {
        if !kept.contains(&member) {
            kept.push(member);
        }
    }
    let decided = kept
        .iter()
        .any(|member| two_valued(member) && kept.contains(&Pred::negate(member.clone())));
    match (decided, kept.len(), all) {
        (true, _, all) => Pred::of(!all),
        (false, 1, _) => kept.pop().unwrap_or(Pred::of(all)),
        (false, _, true) => Pred::All(kept),
        (false, _, false) => Pred::Any(kept),
    }
}

/// Whether the SQL of `pred` is never unknown: every data atom is null-safe,
/// and a fact of the EHR is read from its settings row, where it may be
/// absent.
fn two_valued(pred: &Pred) -> bool {
    match pred {
        Pred::True | Pred::False | Pred::Data(..) => true,
        Pred::Ehr(_) => false,
        Pred::Not(inner) => two_valued(inner),
        Pred::All(preds) | Pred::Any(preds) => preds.iter().all(two_valued),
    }
}

/// Whether the data scope names an empty list of objects, templates,
/// archetypes or categories, which no object is among.
fn covers_nothing(data: &DataMatch) -> bool {
    let empty = |list: Option<usize>| list == Some(0);
    empty(
        data.objects
            .as_ref()
            .and_then(|objects| objects.known())
            .map(Vec::len),
    ) || empty(data.templates.as_ref().map(Vec::len))
        || empty(data.archetypes.as_ref().map(Vec::len))
        || empty(data.categories.as_ref().map(Vec::len))
}

/// `pred` on the head row `h`, its facts read with `facts`.
fn pred(pred: &Pred, h: &str, facts: Facts<'_>) -> Expr {
    match pred {
        Pred::True => Expr::val(true),
        Pred::False => Expr::val(false),
        Pred::Not(inner) => self::pred(inner, h, facts).not(),
        Pred::All(preds) => preds
            .iter()
            .map(|p| self::pred(p, h, facts))
            .reduce(ExprTrait::and)
            .unwrap_or_else(|| Expr::val(true)),
        Pred::Any(preds) => or_all(preds.iter().map(|p| self::pred(p, h, facts)).collect()),
        Pred::Data(data, polarity) => data_match(data, *polarity, h),
        Pred::Ehr(fact) => ehr_fact(*fact, facts),
    }
}

/// The text of the fact at `at` of the settings row.
fn fact_text(at: usize) -> Expr {
    Expr::cust_with_exprs(
        "($1 ->> $2)",
        [col(SETTINGS, AccessSettings::Facts), position(at)],
    )
}

/// The JSON of the fact at `at` of the settings row.
fn fact_json(at: usize) -> Expr {
    Expr::cust_with_exprs(
        "($1 -> $2)",
        [col(SETTINGS, AccessSettings::Facts), position(at)],
    )
}

/// The instant the fact at `at` of the settings row holds.
fn fact_instant(at: usize) -> Expr {
    fact_text(at).cast_as(Alias::new("timestamptz"))
}

/// A fact position as an `integer` operand of `->` and `->>`; a position no
/// `integer` holds reads as no fact, which is SQL's unknown.
fn position(at: usize) -> Expr {
    Expr::val(i32::try_from(at).unwrap_or(i32::MAX))
}

/// `fact`, read against the caller; no fact holds outside a class.
fn ehr_fact(fact: EhrFact, facts: Facts<'_>) -> Expr {
    let Some(caller) = facts.caller else {
        return Expr::val(false);
    };
    match fact {
        EhrFact::Within { from, until } => {
            let now = instant(caller.now);
            let mut atoms = Vec::new();
            if let Some((at, included)) = from {
                let bound = fact_instant(at);
                atoms.push(if included {
                    bound.lte(now.clone())
                } else {
                    bound.lt(now.clone())
                });
            }
            if let Some((at, included)) = until {
                let bound = fact_instant(at);
                atoms.push(if included {
                    now.lte(bound)
                } else {
                    now.lt(bound)
                });
            }
            atoms
                .into_iter()
                .reduce(ExprTrait::and)
                .unwrap_or_else(|| Expr::val(true))
        }
        EhrFact::Party(at) => {
            fact_text(at).eq(Expr::from(PgFunc::any(Expr::val(caller.parties.to_vec()))))
        }
        EhrFact::Keeper(at) => match &caller.keeper {
            Some(party) => fact_text(at).eq(Expr::val(party.clone())),
            None => Expr::val(false),
        },
    }
}

/// The rows one data scope covers.
fn data_match(data: &DataMatch, polarity: Polarity, h: &str) -> Expr {
    let mut atoms: Vec<Expr> = Vec::new();
    if let Some(objects) = &data.objects {
        atoms.push(match objects {
            Held::Known(objects) => in_ids(col(h, VoHead::VoId), objects),
            Held::Fact(at) => in_fact(
                *at,
                Expr::cust_with_exprs("($1)::text", [col(h, VoHead::VoId)]),
            ),
        });
    }
    if let Some(templates) = &data.templates {
        atoms.push(null_safe(template_in(h, templates), false));
    }
    if let Some(archetypes) = &data.archetypes {
        atoms.push(archetype_in(h, archetypes));
    }
    if let Some(categories) = &data.categories {
        atoms.push(or_all(categories.iter().map(|c| category(h, c)).collect()));
    }
    if let Some(origins) = &data.origins {
        atoms.push(origin_in(h, origins, polarity));
    }
    if let Some(recorded) = &data.recorded {
        atoms.push(null_safe(recorded_in(h, recorded), polarity.unreadable()));
    }
    atoms
        .into_iter()
        .reduce(ExprTrait::and)
        .unwrap_or_else(|| Expr::val(true))
}

/// The template of `h`, case-folded, is one of `templates`.
fn template_in(h: &str, templates: &[String]) -> Expr {
    if templates.is_empty() {
        return Expr::val(false);
    }
    Expr::from(Func::lower(col(h, VoHead::TemplateId)))
        .eq(Expr::from(PgFunc::any(Expr::val(templates.to_vec()))))
}

/// The root archetype of the trunk head of `h`, case-folded, is one of
/// `archetypes`.
fn archetype_in(h: &str, archetypes: &[String]) -> Expr {
    if archetypes.is_empty() {
        return Expr::val(false);
    }
    let mut probe = Query::select();
    let root = Expr::cust_with_exprs(
        "lower(COALESCE($1 ->> 'archetype_node_id', $2))",
        [col(ROOT, Node::Data), archetype_node_id(ROOT)],
    );
    probe
        .expr(Expr::val(1))
        .from_as(Node::Table, Alias::new(ROOT))
        .and_where(col(ROOT, Node::Tier).eq(col(h, VoHead::Tier)))
        .and_where(col(ROOT, Node::VoId).eq(col(h, VoHead::VoId)))
        .and_where(col(ROOT, Node::SysVersion).eq(col(h, VoHead::TrunkHeadContentKey)))
        .and_where(col(ROOT, Node::Num).eq(Expr::val(0)))
        .and_where(root.eq(Expr::from(PgFunc::any(Expr::val(archetypes.to_vec())))));
    Expr::exists(probe)
}

/// The JSON array of strings the fact at `at` holds has `text` among them.
fn in_fact(at: usize, text: Expr) -> Expr {
    Expr::cust_with_exprs("COALESCE($1 @> to_jsonb($2), false)", [fact_json(at), text])
}

/// The trunk head of `h` names one of `origins` among its originating
/// systems; a head nothing stamped is read by `polarity`.
fn origin_in(h: &str, origins: &Held<Vec<String>>, polarity: Polarity) -> Expr {
    let named = match origins {
        Held::Known(origins) => {
            Expr::cust("o.id").eq(Expr::from(PgFunc::any(Expr::val(origins.clone()))))
        }
        Held::Fact(at) => in_fact(*at, Expr::cust("o.id")),
    };
    let mut probe = Query::select();
    probe
        .expr(Expr::val(1))
        .from_as(VersionRow::Table, Alias::new(TRUNK))
        .and_where(col(TRUNK, VersionRow::Tier).eq(col(h, VoHead::Tier)))
        .and_where(col(TRUNK, VersionRow::VoId).eq(col(h, VoHead::VoId)))
        .and_where(col(TRUNK, VersionRow::SysVersion).eq(col(h, VoHead::TrunkHeadSysVersion)))
        .and_where(Expr::cust_with_exprs(
            "CASE WHEN $1 IS NULL THEN $2 ELSE EXISTS (SELECT 1 FROM \
             jsonb_array_elements_text($1) AS o(id) WHERE $3) END",
            [
                col(TRUNK, VersionRow::Origins),
                Expr::val(polarity.unreadable()),
                named,
            ],
        ));
    Expr::exists(probe)
}

/// The object of `h` is of `category`: its template is the category's, or
/// the map names its template nowhere and its root archetype is the
/// category's.
fn category(h: &str, category: &CategoryMatch) -> Expr {
    Expr::cust_with_exprs(
        "CASE WHEN $1 THEN true WHEN $2 THEN false ELSE $3 END",
        [
            null_safe(template_in(h, &category.templates), false),
            null_safe(template_in(h, &category.mapped), false),
            archetype_in(h, &category.archetypes),
        ],
    )
}

/// The trunk head of `h` was committed inside `recorded`.
fn recorded_in(h: &str, recorded: &Recorded) -> Expr {
    let at = col(h, VoHead::CommittedAt);
    let mut atoms = Vec::new();
    if let Some(from) = recorded.from {
        let bound = edge(from);
        atoms.push(if from.included {
            at.clone().gte(bound)
        } else {
            at.clone().gt(bound)
        });
    }
    if let Some(until) = recorded.until {
        let bound = edge(until);
        atoms.push(if until.included {
            at.clone().lte(bound)
        } else {
            at.lt(bound)
        });
    }
    atoms
        .into_iter()
        .reduce(ExprTrait::and)
        .unwrap_or_else(|| Expr::val(true))
}

/// The instant of `edge`, known or a fact.
fn edge(edge: Edge) -> Expr {
    match edge.at {
        Held::Known(at) => instant(at),
        Held::Fact(at) => fact_instant(at),
    }
}

/// `at` as a `timestamptz` value.
fn instant(at: jiff::Timestamp) -> Expr {
    Expr::val(at.to_string()).cast_as(Alias::new("timestamptz"))
}

/// `e`, or `unknown` where `e` is SQL's unknown.
fn null_safe(e: Expr, unknown: bool) -> Expr {
    Expr::cust_with_exprs("COALESCE($1, $2)", [e, Expr::val(unknown)])
}

/// `target` is one of `ids`.
fn in_ids(target: Expr, ids: &[Uuid]) -> Expr {
    target.eq(Expr::from(PgFunc::any(Expr::val(ids.to_vec()))))
}

/// The disjunction of `branches`; false when empty.
fn or_all(branches: Vec<Expr>) -> Expr {
    branches
        .into_iter()
        .reduce(ExprTrait::or)
        .unwrap_or_else(|| Expr::val(false))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::simplified;
    use crate::access_eval::scope::{DataMatch, EhrFact, Polarity, Pred};

    /// The objects of archetype `archetype`, read as a permit reads them.
    fn of_archetype(archetype: &str) -> Pred {
        Pred::Data(
            Arc::new(DataMatch {
                archetypes: Some(vec![archetype.to_owned()]),
                ..DataMatch::default()
            }),
            Polarity::Permit,
        )
    }

    /// The fact at position 0 names a party of the caller.
    fn party() -> Pred {
        Pred::Ehr(EhrFact::Party(0))
    }

    #[test]
    fn a_data_scope_beside_its_negation_decides_the_conjunction_and_the_disjunction() {
        let lab = of_archetype("openehr-ehr-observation.laboratory_test_result.v1");
        let both = Pred::All(vec![lab.clone(), Pred::negate(lab.clone())]);
        assert_eq!(simplified(&both), Pred::False);
        assert_eq!(simplified(&Pred::negate(both)), Pred::True);
        let either = Pred::Any(vec![Pred::negate(lab.clone()), lab]);
        assert_eq!(simplified(&either), Pred::True);
    }

    #[test]
    fn a_fact_of_the_ehr_beside_its_negation_stays_for_the_sql_to_decide() {
        let both = Pred::All(vec![party(), Pred::negate(party())]);
        assert_eq!(simplified(&both), both);
        let either = Pred::Any(vec![party(), Pred::negate(party())]);
        assert_eq!(simplified(&either), either);
    }

    #[test]
    fn a_repeated_member_counts_once_and_the_rest_keep_their_order() {
        let lab = of_archetype("openehr-ehr-observation.laboratory_test_result.v1");
        let note = of_archetype("openehr-ehr-evaluation.clinical_synopsis.v1");
        let repeated = Pred::All(vec![lab.clone(), party(), lab.clone(), note.clone()]);
        assert_eq!(
            simplified(&repeated),
            Pred::All(vec![lab.clone(), party(), note])
        );
        assert_eq!(simplified(&Pred::Any(vec![lab.clone(), lab.clone()])), lab);
    }

    #[test]
    fn a_scope_naming_an_empty_list_covers_nothing() {
        for data in [
            DataMatch {
                templates: Some(Vec::new()),
                ..DataMatch::default()
            },
            DataMatch {
                archetypes: Some(Vec::new()),
                ..DataMatch::default()
            },
            DataMatch {
                categories: Some(Vec::new()),
                ..DataMatch::default()
            },
        ] {
            let scope = Pred::Data(Arc::new(data), Polarity::Deny);
            assert_eq!(simplified(&scope), Pred::False);
            assert_eq!(simplified(&Pred::negate(scope)), Pred::True);
        }
        let unset = Pred::Data(Arc::new(DataMatch::default()), Polarity::Deny);
        assert_eq!(simplified(&unset), unset);
    }

    #[test]
    fn the_folds_reach_inside_nested_predicates() {
        let lab = of_archetype("openehr-ehr-observation.laboratory_test_result.v1");
        let nested = Pred::All(vec![
            party(),
            Pred::negate(Pred::All(vec![lab.clone(), Pred::negate(lab.clone())])),
            Pred::negate(lab.clone()),
        ]);
        assert_eq!(
            simplified(&nested),
            Pred::All(vec![party(), Pred::negate(lab)])
        );
    }
}
