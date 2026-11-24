// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request values a statement shape binds from slots, each read as the
//! SQL emitter binds it.

use sea_query::Value;
use uuid::Uuid;

use super::super::{SqlCtx, care};
use crate::access_eval::condition::care::Care;
use crate::aql::ir::{ParamValue, Params};

/// A value a statement binds that varies between the requests of one shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Live {
    /// The `ehr_ids` scope, as one array.
    EhrIds,
    /// The row limit.
    Limit,
    /// The row offset.
    Offset,
    /// The moment of the request, which a class's time facts are read
    /// against.
    Now,
    /// The parties the caller is or acts for.
    Parties,
    /// The subjects the caller cares for, as the census `jsonb` text.
    Census,
    /// The EHRs of a pinned query whose subject the caller cares for.
    CareEhrs,
    /// The ties by which the caller holds each of them, as `jsonb` text.
    CareTies,
    /// The caller's principals.
    CarePrincipals,
    /// Those of the caller's principals a relationship reaches directly.
    CareDirect,
    /// The issuers of the providers the caller acts for.
    CareIssuers,
    /// The ids of the providers the caller acts for.
    CareProviders,
    /// The day care relationships are judged on.
    CareDay,
    /// The EHRs a subject caller is confined to.
    Confined,
    /// The EHR a pinned query reads.
    PinnedEhr,
    /// The revision its verdict was judged on.
    PinnedRevision,
    /// The text of the EHR-id parameter at this index of
    /// [`super::Shapes::live_params`].
    ParamText(usize),
    /// That parameter's id, in its canonical form.
    ParamId(usize),
}

impl Live {
    /// Every source of a plan with `params` EHR-id parameters.
    pub(super) fn every(params: usize) -> Vec<Self> {
        let mut every = vec![
            Self::EhrIds,
            Self::Limit,
            Self::Offset,
            Self::Now,
            Self::Parties,
            Self::Census,
            Self::CareEhrs,
            Self::CareTies,
            Self::CarePrincipals,
            Self::CareDirect,
            Self::CareIssuers,
            Self::CareProviders,
            Self::CareDay,
            Self::Confined,
            Self::PinnedEhr,
            Self::PinnedRevision,
        ];
        every.extend((0..params).flat_map(|at| [Self::ParamText(at), Self::ParamId(at)]));
        every
    }

    /// The value this source binds in a request of `params` and `ctx`;
    /// `None` when the request has no such value.
    pub(super) fn eval(
        self,
        live_params: &[String],
        params: &Params,
        ctx: &SqlCtx,
    ) -> Option<Value> {
        let groups = ctx.ehr_access.groups();
        let pinned = ctx.ehr_access.pinned();
        match self {
            Self::EhrIds => (!ctx.ehr_ids.is_empty())
                .then(|| Value::from(ctx.ehr_ids.iter().map(|id| id.0).collect::<Vec<Uuid>>())),
            Self::Limit => ctx
                .limit
                .filter(|limit| *limit >= 0)
                .map(|limit| Value::from(u64::try_from(limit).unwrap_or(u64::MAX))),
            Self::Offset => ctx
                .offset
                .filter(|offset| *offset > 0)
                .map(|offset| Value::from(u64::try_from(offset).unwrap_or(0))),
            Self::Now => groups.map(|groups| Value::from(groups.facts.now.to_string())),
            Self::Parties => groups.map(|groups| Value::from(groups.facts.parties.to_vec())),
            Self::Census => match groups.map(|groups| &groups.cared) {
                Some(Care::Census(cared)) => Some(Value::from(care::census_text(
                    cared,
                    groups.is_some_and(|groups| groups.episodic),
                ))),
                _ => None,
            },
            Self::CareEhrs => match groups.map(|groups| &groups.cared) {
                Some(Care::Ehrs(ehrs)) => Some(Value::from(
                    ehrs.iter().map(|cared| cared.ehr).collect::<Vec<Uuid>>(),
                )),
                _ => None,
            },
            Self::CareTies => match groups.map(|groups| &groups.cared) {
                Some(Care::Ehrs(ehrs)) => Some(Value::from(care::ehr_ties_text(
                    ehrs,
                    groups.is_some_and(|groups| groups.episodic),
                ))),
                _ => None,
            },
            Self::CarePrincipals => match groups.map(|groups| &groups.cared) {
                Some(Care::Projected(care)) => Some(Value::from(care.principals.to_vec())),
                _ => None,
            },
            Self::CareDirect => match groups.map(|groups| &groups.cared) {
                Some(Care::Projected(care)) => Some(Value::from(care.direct.to_vec())),
                _ => None,
            },
            Self::CareIssuers => match groups.map(|groups| &groups.cared) {
                Some(Care::Projected(care)) => Some(Value::from(care.provider_issuers.to_vec())),
                _ => None,
            },
            Self::CareProviders => match groups.map(|groups| &groups.cared) {
                Some(Care::Projected(care)) => Some(Value::from(care.provider_ids.to_vec())),
                _ => None,
            },
            Self::CareDay => match groups.map(|groups| &groups.cared) {
                Some(Care::Projected(care)) => Some(Value::from(care.day.to_string())),
                _ => None,
            },
            Self::Confined => groups
                .and_then(|groups| groups.confined.as_deref())
                .map(|ehrs| Value::from(ehrs.to_vec())),
            Self::PinnedEhr => pinned.map(|pinned| Value::from(pinned.ehr)),
            Self::PinnedRevision => pinned.map(|pinned| Value::from(pinned.revision_json())),
            Self::ParamText(at) => {
                text(live_params, at, params).map(|text| Value::from(text.to_owned()))
            }
            // NOTE: our own design (#5783), a value that is no id carries no canonical form.
            Self::ParamId(at) => text(live_params, at, params)
                .and_then(|text| text.parse::<Uuid>().ok())
                .map(|id| Value::from(id.to_string())),
        }
    }
}

/// The text of the EHR-id parameter at `at`, when the request binds it as a
/// string.
fn text<'a>(live_params: &[String], at: usize, params: &'a Params) -> Option<&'a str> {
    match params.get(live_params.get(at)?)? {
        ParamValue::Str(text) => Some(text),
        _ => None,
    }
}
