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

use super::expr::col;
use super::fold::simplified;
use crate::access_eval::class::Keepers;
use crate::access_eval::condition::{AccessCondition, CallerKeys, ConditionGroups, VerdictPair};
use crate::access_eval::scope::{
    CategoryMatch, DataMatch, Edge, EhrFact, Held, Polarity, Pred, Recorded,
};
use crate::db::iden::{AccessSettings, VersionRow, VoHead};

/// The alias of the trunk head version an origin probe reads.
const TRUNK: &str = "access_trunk";
/// The alias of the `ehr` row a care probe reads.
pub(super) const CARE: &str = "access_ehr";
/// The alias of the projected settings row of the EHR.
const SETTINGS: &str = "access_settings_row";

/// What a predicate is read against: the caller's facts, which a class's
/// facts are compared with, `None` outside a class; and the caller's care
/// with the EHR id it is probed by, `None` where no care is read.
#[derive(Clone, Copy)]
struct Facts<'a> {
    caller: Option<&'a CallerKeys>,
    care: Option<(&'a ConditionGroups, &'a Expr)>,
}

/// The predicate on the head row `h` of a versioned object; `None` when the
/// condition admits every row.
pub(super) fn on_head(condition: &AccessCondition, h: &str) -> Option<Expr> {
    if let Some(pinned) = condition.pinned() {
        return Some(super::pinned::on_head(pinned, h));
    }
    let condition = condition.groups()?;
    let target = col(h, VoHead::EhrId);
    if admits_every_known(condition) {
        return Some(confine(
            condition,
            &target,
            class_known(condition, target.clone()),
        ));
    }
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
        care: None,
    };
    let none = Facts {
        caller: None,
        care: None,
    };
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
    let held = by_settings(condition, target.clone(), or_all(branches), default);
    Some(confine(condition, &target, held))
}

/// The predicate on the EHR id `target` of an EHR a query reads: an EHR no
/// item of which the caller may read is left out; whether an item is read is
/// [`on_head`]'s.
pub(super) fn on_ehr_id(condition: &AccessCondition, target: &Expr) -> Option<Expr> {
    if let Some(pinned) = condition.pinned() {
        return Some(super::pinned::on_ehr_id(pinned, target));
    }
    let condition = condition.groups()?;
    if admits_every_known(condition) {
        return Some(confine(
            condition,
            target,
            class_known(condition, target.clone()),
        ));
    }
    let reaches = |pair: &VerdictPair, facts: Facts<'_>| -> Option<Expr> {
        let (cared, uncared) = (
            simplified(&pair.cared.reaches()),
            simplified(&pair.uncared.reaches()),
        );
        if cared == Pred::False && uncared == Pred::False {
            return None;
        }
        let with_care = Facts {
            care: Some((condition, target)),
            ..facts
        };
        if cared == uncared {
            return Some(pred(&cared, CARE, with_care));
        }
        Some(Expr::cust_with_exprs(
            "CASE WHEN $1 THEN $2 ELSE $3 END",
            [
                super::care::cared(condition, target.clone()),
                pred(&cared, CARE, with_care),
                pred(&uncared, CARE, with_care),
            ],
        ))
    };
    let caller = Facts {
        caller: Some(&condition.facts),
        care: None,
    };
    let none = Facts {
        caller: None,
        care: None,
    };
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
    let held = by_settings(condition, target.clone(), or_all(branches), default);
    Some(confine(condition, target, held))
}

/// Whether every class `condition` knows, an EHR held closed and an EHR with
/// no settings each admit every item: then only an EHR of a class it does
/// not know is withheld ([`class_known`]). One class, the closed verdict or
/// the default that refuses anything leaves the condition in full.
fn admits_every_known(condition: &ConditionGroups) -> bool {
    condition.default.is_none()
        && condition.closed.admits_all()
        && condition.groups.iter().all(|(_, pair)| pair.admits_all())
}

/// The EHR `target` carries no settings row of a class `condition` does not
/// know: the condition [`by_settings`] renders when every known class, the
/// closed verdict and the default admit every item, rewritten exactly.
///
/// The EHRs of an unknown class are read by one uncorrelated subquery,
/// which PostgreSQL hashes once per execution, where `by_settings` probed
/// the settings row once per candidate row. A row of such an EHR fails the
/// statement as `by_settings` does ([`unknown_class`]). A row with no EHR, a
/// party's head, has no settings row and is admitted, as the default admits
/// it. No openEHR spec governs this: our own design (#5782).
fn class_known(condition: &ConditionGroups, target: Expr) -> Expr {
    Expr::cust_with_exprs(
        "CASE WHEN COALESCE($1 NOT IN (SELECT access_unknown.ehr_id \
         FROM access_settings access_unknown \
         WHERE NOT access_unknown.held_closed AND access_unknown.class_id <> ALL($2)), true) \
         THEN true ELSE access_class_unknown($1) END",
        [target, Expr::val(known_classes(condition))],
    )
}

/// Every settings class `condition` holds a verdict for.
fn known_classes(condition: &ConditionGroups) -> Vec<i64> {
    condition
        .groups
        .iter()
        .flat_map(|(ids, _)| ids.iter().copied())
        .collect()
}

/// The settings row is held closed or of a class `condition` holds; any
/// other row fails the statement through `access_class_unknown` (SQLSTATE
/// `ZA002`, `clinical/0049_access_stale_copy.sql`), so a class this
/// replica's copy has not yet heard of is waited for or refused, and never
/// answered as an EHR with nothing to read. The function takes the row's
/// EHR id, so the planner cannot fold it as a constant. No openEHR spec
/// governs this: our own design (#5794).
fn unknown_class(condition: &ConditionGroups, with: Expr) -> Expr {
    Expr::cust_with_exprs(
        "CASE WHEN $1 OR $2 = ANY($3) THEN COALESCE($4, false) ELSE access_class_unknown($5) END",
        [
            col(SETTINGS, AccessSettings::HeldClosed),
            col(SETTINGS, AccessSettings::ClassId),
            Expr::val(known_classes(condition)),
            with,
            col(SETTINGS, AccessSettings::EhrId),
        ],
    )
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
        care: None,
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
/// never turns into a hashed scan of every settings row (#5701); for a
/// pinned query the answer the service read.
pub(super) fn settled(condition: &AccessCondition, target: Expr) -> Expr {
    if let Some(pinned) = condition.pinned() {
        return Expr::val(pinned.settled);
    }
    let mut row = settings_row(target);
    row.clear_selects();
    row.expr(Expr::val(true));
    Expr::cust_with_exprs(
        "COALESCE($1, false)",
        [Expr::SubQuery(None, Box::new(row.into()))],
    )
}

/// For the EHR `target`: with a settings row of a class `condition` holds,
/// `with` over that row; with one of a class it does not hold, a failed
/// statement ([`unknown_class`]); with none, `default`, or every row when
/// `default` is `None`.
///
/// One scalar probe of the settings row by its key decides each: it answers
/// `with` (an unknown `with` is false) when the row exists and NULL when it
/// does not, which `default` then replaces. An `EXISTS` beside a `NOT
/// EXISTS` under an `OR` was planned as two hashed `SubPlans` over every
/// settings row before the first row came back (#5701). No openEHR spec
/// governs this: our own design.
fn by_settings(
    condition: &ConditionGroups,
    target: Expr,
    with: Expr,
    default: Option<Expr>,
) -> Expr {
    let mut row = settings_row(target);
    row.clear_selects();
    row.expr(unknown_class(condition, with));
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
    let facts = Facts {
        care: Some((condition, &target)),
        ..facts
    };
    if with == without {
        return pred(&with, h, facts);
    }
    Expr::cust_with_exprs(
        "CASE WHEN $1 THEN $2 ELSE $3 END",
        [
            super::care::cared(condition, target.clone()),
            pred(&with, h, facts),
            pred(&without, h, facts),
        ],
    )
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
        Pred::Care(need) => match facts.care {
            Some((condition, target)) => super::care::care_need(need, condition, target.clone()),
            None => Expr::val(false),
        },
    }
}

/// `pred` on the head row `h`, over the facts of the object alone.
pub(super) fn item_pred(pred: &Pred, h: &str) -> Expr {
    self::pred(
        &simplified(pred),
        h,
        Facts {
            caller: None,
            care: None,
        },
    )
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
/// `archetypes`; a head with no content is of none.
///
/// The head row carries the root archetype (#5782), so this compares a column
/// of the row the query already joined, where a probe of the root node row by
/// its key cost about four buffers per candidate row and clause. No openEHR
/// spec governs this: our own design.
fn archetype_in(h: &str, archetypes: &[String]) -> Expr {
    if archetypes.is_empty() {
        return Expr::val(false);
    }
    null_safe(
        Expr::from(Func::lower(col(h, VoHead::RootArchetype)))
            .eq(Expr::from(PgFunc::any(Expr::val(archetypes.to_vec())))),
        false,
    )
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
