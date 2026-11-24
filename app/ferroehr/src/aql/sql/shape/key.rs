// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What decides the text of a plan's statement: every input of the SQL
//! build but the values a [`super::Live`] slot carries.

use std::sync::Arc;
use uuid::Uuid;

use super::super::own_records::OwnRecords;
use super::super::readers::{EhrAccessReaders, PolicyParties};
use super::super::{EhrScopeKind, SqlCtx};
use crate::access_eval::caseload::Caseload;
use crate::access_eval::condition::care::Care;
use crate::access_eval::condition::{AccessCondition, ConditionGroups};
use crate::aql::ir::{ParamValue, Params, QueryIr};
use crate::aql::lineage::ArchetypeLineage;

/// What the running request's task decides of the text: whether the EHDS
/// Art. 3(3) delay withholds held data, and whether a statement of its ride
/// reads the party store's revisions (#6320).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ambient {
    person_withheld: bool,
    ride: bool,
}

impl Ambient {
    /// The running request's.
    pub(super) fn now() -> Self {
        Self {
            person_withheld: crate::system_log::access_context::access_delay_enforced(),
            ride: crate::party_binding::request::reads_door(),
        }
    }
}

/// The inputs of one statement shape.
///
/// Each list of a [`super::Live`] source counts only by whether it is there,
/// and each bound by whether it applies. The archetype lineage is the one
/// the key holds, by identity: the key keeps it alive, so its address names
/// no other.
#[derive(Debug, Clone)]
pub(super) struct Key {
    params: Vec<Keyed>,
    system_id: String,
    ehr_scope: Option<EhrScopeKind>,
    limit: Option<bool>,
    offset: bool,
    lineage: Arc<ArchetypeLineage>,
    policy_parties: PolicyParties,
    access_settings: EhrAccessReaders,
    condition: AccessCondition,
    ambient: Ambient,
    own_records: Option<OwnRecords>,
    caseload: Option<Arc<Caseload>>,
}

/// One parameter of the plan, in [`QueryIr::params`] order.
#[derive(Debug, Clone, PartialEq)]
enum Keyed {
    /// An EHR-id parameter bound to an id, which a slot carries.
    EhrId,
    /// Any other, by value.
    Value(Option<ParamValue>),
}

impl Key {
    /// The key of a request of `ir`.
    pub(super) fn of(
        ir: &QueryIr,
        live_params: &[String],
        params: &Params,
        ctx: &SqlCtx,
        ambient: Ambient,
    ) -> Self {
        Self {
            params: ir
                .params
                .iter()
                .map(|name| keyed(live_params, name, params))
                .collect(),
            system_id: ctx.system_id.clone(),
            ehr_scope: ehr_scope(ctx),
            limit: limit(ctx),
            offset: offset(ctx),
            lineage: Arc::clone(&ctx.archetype_lineage),
            policy_parties: ctx.policy_parties,
            access_settings: ctx.access_settings.clone(),
            condition: ctx.ehr_access.clone(),
            ambient,
            own_records: ctx.own_records.clone(),
            caseload: ctx.caseload.clone(),
        }
    }

    /// Whether a request of `ir` has this key, compared field by field with
    /// no allocation.
    pub(super) fn admits(
        &self,
        ir: &QueryIr,
        live_params: &[String],
        params: &Params,
        ctx: &SqlCtx,
        ambient: Ambient,
    ) -> bool {
        self.ambient == ambient
            && self.ehr_scope == ehr_scope(ctx)
            && self.limit == limit(ctx)
            && self.offset == offset(ctx)
            && self.policy_parties == ctx.policy_parties
            && Arc::ptr_eq(&self.lineage, &ctx.archetype_lineage)
            && self.system_id == ctx.system_id
            && self.params.len() == ir.params.len()
            && self
                .params
                .iter()
                .zip(&ir.params)
                .all(|(held, name)| held.admits(live_params, name, params))
            && self.access_settings == ctx.access_settings
            && same_condition(&self.condition, &ctx.ehr_access)
            && self.own_records == ctx.own_records
            && self.caseload == ctx.caseload
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.ambient == other.ambient
            && self.ehr_scope == other.ehr_scope
            && self.limit == other.limit
            && self.offset == other.offset
            && self.policy_parties == other.policy_parties
            && Arc::ptr_eq(&self.lineage, &other.lineage)
            && self.system_id == other.system_id
            && self.params == other.params
            && self.access_settings == other.access_settings
            && same_condition(&self.condition, &other.condition)
            && self.own_records == other.own_records
            && self.caseload == other.caseload
    }
}

impl Keyed {
    /// Whether the request's `name` keys as this.
    fn admits(&self, live_params: &[String], name: &str, params: &Params) -> bool {
        match self {
            Self::EhrId => is_ehr_id(live_params, name, params),
            Self::Value(value) => params.get(name) == value.as_ref(),
        }
    }
}

/// How the request keys its parameter `name`.
fn keyed(live_params: &[String], name: &str, params: &Params) -> Keyed {
    if is_ehr_id(live_params, name, params) {
        Keyed::EhrId
    } else {
        Keyed::Value(params.get(name).cloned())
    }
}

/// Whether `name` is an EHR-id parameter bound to the text of an id.
pub(super) fn is_ehr_id(live_params: &[String], name: &str, params: &Params) -> bool {
    live_params.iter().any(|live| live == name)
        && matches!(params.get(name), Some(ParamValue::Str(text)) if text.parse::<Uuid>().is_ok())
}

/// The kind of a non-empty `ehr_ids` scope.
fn ehr_scope(ctx: &SqlCtx) -> Option<EhrScopeKind> {
    (!ctx.ehr_ids.is_empty()).then_some(ctx.ehr_scope_kind)
}

/// Whether the statement carries a `LIMIT`, by the bound's presence.
fn limit(ctx: &SqlCtx) -> Option<bool> {
    ctx.limit.map(|limit| limit >= 0)
}

/// Whether the statement carries an `OFFSET`.
fn offset(ctx: &SqlCtx) -> bool {
    ctx.offset.is_some_and(|offset| offset > 0)
}

/// Whether two access conditions render the same text: equal but for the
/// values a [`super::Live`] slot carries.
fn same_condition(a: &AccessCondition, b: &AccessCondition) -> bool {
    let groups = match (a.groups(), b.groups()) {
        (None, None) => true,
        (Some(a), Some(b)) => same_groups(a, b),
        _ => false,
    };
    let pinned = match (a.pinned(), b.pinned()) {
        (None, None) => true,
        (Some(a), Some(b)) => a.admitted == b.admitted && a.settled == b.settled,
        _ => false,
    };
    groups && pinned
}

/// [`same_condition`] of two sets of verdicts.
fn same_groups(a: &ConditionGroups, b: &ConditionGroups) -> bool {
    std::ptr::eq(a, b)
        || (same_care(&a.cared, &b.cared)
            && a.confined.is_some() == b.confined.is_some()
            && a.facts.keeper == b.facts.keeper
            && a.subject_namespace == b.subject_namespace
            && a.episodic == b.episodic
            && a.closed == b.closed
            && a.default == b.default
            && a.groups == b.groups)
}

/// Whether two reads of care render the same text.
fn same_care(a: &Care, b: &Care) -> bool {
    match (a, b) {
        (Care::Unresolved, Care::Unresolved)
        | (Care::Census(_), Care::Census(_))
        | (Care::Ehrs(_), Care::Ehrs(_)) => true,
        (Care::Projected(a), Care::Projected(b)) => {
            a.grace == b.grace && a.self_declared == b.self_declared
        }
        _ => false,
    }
}
