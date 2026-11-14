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
use sea_query::{Alias, Expr, ExprTrait, Func, Query, SelectStatement};
use uuid::Uuid;

use super::expr::{archetype_node_id, col};
use crate::access_eval::class::Keepers;
use crate::access_eval::condition::{AccessCondition, CallerKeys, ConditionGroups, VerdictPair};
use crate::access_eval::scope::{
    CategoryMatch, DataMatch, Edge, EhrFact, Held, Polarity, Pred, Recorded,
};
use crate::db::iden::{AccessSettings, Ehr, Node, VersionRow, VoHead};

/// The alias of the root node an archetype probe reads.
const ROOT: &str = "access_root";
/// The alias of the trunk head version an origin probe reads.
const TRUNK: &str = "access_trunk";
/// The alias of the `ehr` row a care probe reads.
const CARE: &str = "access_ehr";
/// The alias of the projected settings row of the EHR.
const SETTINGS: &str = "access_settings_row";

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
    Some(by_settings(target, or_all(branches), default))
}

/// The predicate on the EHR id `target` of an EHR a query reads: an EHR no
/// item of which the caller may read is left out; whether an item is read is
/// [`on_head`]'s.
pub(super) fn on_ehr_id(condition: &AccessCondition, target: &Expr) -> Option<Expr> {
    let condition = condition.groups()?;
    let reaches = |pair: &VerdictPair, facts: Facts<'_>| -> Option<Expr> {
        let (cared, uncared) = (pair.cared.reaches(), pair.uncared.reaches());
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
    Some(by_settings(target.clone(), or_all(branches), default))
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
        .map(|(ids, named)| in_classes(ids).and(pred(named, h, facts)))
        .collect();
    let mut probe = settings_row(col(h, VoHead::EhrId));
    probe.and_where(or_all(branches));
    Expr::exists(probe)
}

/// Whether the EHR `target` carries `EHR_ACCESS` settings.
pub(super) fn settled(target: Expr) -> Expr {
    Expr::exists(settings_row(target))
}

/// For the EHR `target`: with a settings row, `with` over that row; with
/// none, `default`, or every row when `default` is `None`.
fn by_settings(target: Expr, with: Expr, default: Option<Expr>) -> Expr {
    match default {
        // NOTE: our own design (#5588), one probe of the settings row when an EHR with
        // none admits every row: the row is absent or its class admits.
        None => {
            let mut held = settings_row(target);
            held.and_where(Expr::cust_with_exprs("NOT COALESCE($1, false)", [with]));
            Expr::exists(held).not()
        }
        Some(default) => {
            let mut held = settings_row(target.clone());
            held.and_where(with);
            Expr::exists(settings_row(target))
                .not()
                .and(default)
                .or(Expr::exists(held))
        }
    }
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

/// The rows of an EHR of `pair` admitted, on head row `h` of EHR `target`.
fn admitted(
    condition: &ConditionGroups,
    pair: &VerdictPair,
    h: &str,
    target: Expr,
    facts: Facts<'_>,
) -> Expr {
    let (with, without) = (pair.cared.admitted(), pair.uncared.admitted());
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

/// Whether the subject of the EHR `target` is under the care of the caller:
/// a `CASE` condition, since an EHR names one subject.
fn cared(condition: &ConditionGroups, target: Expr) -> Expr {
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
