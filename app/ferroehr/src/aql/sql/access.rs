// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator's condition in a query's one SQL statement.
//!
//! The evaluator judges every EHR with settings, and the EHR with none, in
//! memory ([`crate::access_eval::condition::RequestAccess::condition`]); this
//! module renders the verdicts as predicates on the head row of each
//! versioned object a query reads, and on the id of each EHR it reads, so
//! `COUNT`, `LIMIT` and paging see only the rows the caller may read and
//! nothing else leaves the database. Each [`Pred`] reads the same row
//! attributes the in-memory check reads: the object id, the template, the
//! root archetype, the origins of the trunk head and its commit time. Every
//! atom is null-safe, so a missing attribute decides by the polarity of the
//! fact and never by SQL's unknown. No openEHR spec governs this: our own
//! design (#5513).

use sea_query::extension::postgres::PgFunc;
use sea_query::{Alias, Expr, ExprTrait, Func, Query};
use uuid::Uuid;

use super::expr::{archetype_node_id, col};
use crate::access_eval::condition::{AccessCondition, VerdictPair};
use crate::access_eval::scope::{CategoryMatch, DataMatch, Polarity, Pred, Recorded};
use crate::db::iden::{Ehr, Node, VersionRow, VoHead};

/// The alias of the root node an archetype probe reads.
const ROOT: &str = "access_root";
/// The alias of the trunk head version an origin probe reads.
const TRUNK: &str = "access_trunk";
/// The alias of the `ehr` row a care probe reads.
const CARE: &str = "access_ehr";

/// The predicate on the head row `h` of a versioned object; `None` when the
/// condition admits every row.
pub(super) fn on_head(condition: &AccessCondition, h: &str) -> Option<Expr> {
    if condition.is_unrestricted() {
        return None;
    }
    let target = col(h, VoHead::EhrId);
    let mut branches = Vec::new();
    for (ids, pair) in &condition.groups {
        if pair.admits_all() && condition.default.is_none() {
            continue;
        }
        if pair.admits_none() {
            continue;
        }
        let admitted = admitted(condition, pair, h, target.clone());
        branches.push(in_ids(target.clone(), ids).and(admitted));
    }
    let others = match &condition.default {
        None => {
            let held: Vec<Uuid> = condition
                .groups
                .iter()
                .filter(|(_, pair)| !pair.admits_all())
                .flat_map(|(ids, _)| ids.iter().copied())
                .collect();
            Some(not_in_ids(target.clone(), &held))
        }
        Some(pair) if pair.admits_none() => None,
        Some(pair) => Some(not_in_ids(target.clone(), &condition.known).and(admitted(
            condition,
            pair,
            h,
            target.clone(),
        ))),
    };
    branches.extend(others);
    Some(or_all(branches))
}

/// The predicate on the EHR id `target` of an EHR a query reads: an EHR no
/// item of which the caller may read is left out; whether an item is read is
/// [`on_head`]'s.
pub(super) fn on_ehr_id(condition: &AccessCondition, target: &Expr) -> Option<Expr> {
    if condition.is_unrestricted() {
        return None;
    }
    let reaches = |pair: &VerdictPair| -> Option<Expr> {
        match (pair.cared.admits_any(), pair.uncared.admits_any()) {
            (true, true) => Some(Expr::val(true)),
            (false, false) => None,
            (true, false) => Some(cared(condition, target.clone())),
            (false, true) => Some(cared(condition, target.clone()).not()),
        }
    };
    let mut branches = Vec::new();
    for (ids, pair) in &condition.groups {
        if let Some(reach) = reaches(pair) {
            branches.push(in_ids(target.clone(), ids).and(reach));
        }
    }
    let default = condition.default.as_deref();
    match default.map_or(Some(Expr::val(true)), reaches) {
        Some(reach) => branches.push(not_in_ids(target.clone(), &condition.known).and(reach)),
        None => {}
    }
    Some(or_all(branches))
}

/// The rows of an EHR of `pair` admitted, on head row `h` of EHR `target`.
fn admitted(condition: &AccessCondition, pair: &VerdictPair, h: &str, target: Expr) -> Expr {
    let (with, without) = (pair.cared.admitted(), pair.uncared.admitted());
    if with == without {
        return pred(&with, h);
    }
    Expr::cust_with_exprs(
        "CASE WHEN $1 THEN $2 ELSE $3 END",
        [cared(condition, target), pred(&with, h), pred(&without, h)],
    )
}

/// Whether the subject of the EHR `target` is under the care of the caller:
/// a `CASE` condition, since an EHR names one subject.
fn cared(condition: &AccessCondition, target: Expr) -> Expr {
    let mut probe = Query::select();
    probe
        .expr(Expr::val(1))
        .from_as(Ehr::Table, Alias::new(CARE))
        .and_where(col(CARE, Ehr::Id).eq(target))
        .and_where(
            Expr::from(Func::lower(col(CARE, Ehr::SubjectId)))
                .eq(Expr::from(PgFunc::any(Expr::val(condition.cared.to_vec())))),
        );
    Expr::exists(probe)
}

/// `pred` on the head row `h`.
fn pred(pred: &Pred, h: &str) -> Expr {
    match pred {
        Pred::True => Expr::val(true),
        Pred::False => Expr::val(false),
        Pred::Not(inner) => self::pred(inner, h).not(),
        Pred::All(preds) => preds
            .iter()
            .map(|p| self::pred(p, h))
            .reduce(ExprTrait::and)
            .unwrap_or_else(|| Expr::val(true)),
        Pred::Any(preds) => or_all(preds.iter().map(|p| self::pred(p, h)).collect()),
        Pred::Data(data, polarity) => data_match(data, *polarity, h),
    }
}

/// The rows one data scope covers.
fn data_match(data: &DataMatch, polarity: Polarity, h: &str) -> Expr {
    let mut atoms: Vec<Expr> = Vec::new();
    if let Some(objects) = &data.objects {
        atoms.push(in_ids(col(h, VoHead::VoId), objects));
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
    Expr::from(Func::lower(col(h, VoHead::TemplateId)))
        .eq(Expr::from(PgFunc::any(Expr::val(templates.to_vec()))))
}

/// The root archetype of the trunk head of `h`, case-folded, is one of
/// `archetypes`.
fn archetype_in(h: &str, archetypes: &[String]) -> Expr {
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

/// The trunk head of `h` names one of `origins` among its originating
/// systems; a head nothing stamped is read by `polarity`.
fn origin_in(h: &str, origins: &[String], polarity: Polarity) -> Expr {
    let mut probe = Query::select();
    probe
        .expr(Expr::val(1))
        .from_as(VersionRow::Table, Alias::new(TRUNK))
        .and_where(col(TRUNK, VersionRow::Tier).eq(col(h, VoHead::Tier)))
        .and_where(col(TRUNK, VersionRow::VoId).eq(col(h, VoHead::VoId)))
        .and_where(col(TRUNK, VersionRow::SysVersion).eq(col(h, VoHead::TrunkHeadSysVersion)))
        .and_where(Expr::cust_with_exprs(
            "CASE WHEN $1 IS NULL THEN $2 ELSE EXISTS (SELECT 1 FROM \
             jsonb_array_elements_text($1) AS o(id) WHERE o.id = ANY($3)) END",
            [
                col(TRUNK, VersionRow::Origins),
                Expr::val(polarity.unreadable()),
                Expr::val(origins.to_vec()),
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
        let bound = instant(from.at);
        atoms.push(if from.included {
            at.clone().gte(bound)
        } else {
            at.clone().gt(bound)
        });
    }
    if let Some(until) = recorded.until {
        let bound = instant(until.at);
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

/// `target` is none of `ids`; every id when `ids` is empty.
fn not_in_ids(target: Expr, ids: &[Uuid]) -> Expr {
    if ids.is_empty() {
        return Expr::val(true);
    }
    target.ne(Expr::from(PgFunc::all(Expr::val(ids.to_vec()))))
}

/// The disjunction of `branches`; false when empty.
fn or_all(branches: Vec<Expr>) -> Expr {
    branches
        .into_iter()
        .reduce(ExprTrait::or)
        .unwrap_or_else(|| Expr::val(false))
}
