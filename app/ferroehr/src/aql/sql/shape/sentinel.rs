// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Random stand-ins for the request values a statement shape binds from
//! slots, which a derivation builds its two statements over.

use std::sync::Arc;

use uuid::Uuid;

use super::super::SqlCtx;
use crate::access_eval::condition::care::{Care, CaredEhr, CaredSubject, ProjectedCare};
use crate::access_eval::condition::pinned::PinnedEhr;
use crate::access_eval::condition::{AccessCondition, CallerKeys, ConditionGroups};
use crate::aql::ir::{ParamValue, Params};
use crate::ids::EhrId;
use crate::storage::access_projection::{EhrSubject, Revision};

/// One stand-in for every [`super::Live`] source, each list `width` long.
///
/// Every value is drawn at random, so no value a statement binds for any
/// other reason equals one, and the instants and days lie in the tenth
/// millennium before the common era, where no record or policy dates
/// anything.
#[derive(Debug)]
pub(super) struct Sentinels {
    width: usize,
    limit: i64,
    offset: i64,
    now: jiff::Timestamp,
    day: jiff::civil::Date,
    settings: i32,
}

impl Sentinels {
    /// Fresh stand-ins with lists of `width` items.
    pub(super) fn draw(width: usize) -> Self {
        let draw = |range: u64| rand::random::<u64>() % range;
        let seconds = i64::try_from(draw(31_000_000_000)).unwrap_or(0);
        Self {
            width,
            limit: 1_000_000_000 + i64::try_from(draw(1_000_000_000)).unwrap_or(0),
            offset: 1_000_000_000 + i64::try_from(draw(1_000_000_000)).unwrap_or(0),
            now: jiff::Timestamp::from_second(-377_000_000_000 + seconds)
                .unwrap_or(jiff::Timestamp::MIN),
            day: jiff::civil::Date::new(
                -9000 + i16::try_from(draw(900)).unwrap_or(0),
                1 + i8::try_from(draw(12)).unwrap_or(0),
                1 + i8::try_from(draw(28)).unwrap_or(0),
            )
            .unwrap_or(jiff::civil::Date::MIN),
            settings: i32::try_from(draw(1 << 30)).unwrap_or(0),
        }
    }

    /// `width` random ids.
    fn ids(&self) -> Vec<Uuid> {
        (0..self.width)
            .map(|_| Uuid::from_u128(rand::random()))
            .collect()
    }

    /// `width` random texts.
    fn texts(&self) -> Vec<String> {
        self.ids().iter().map(Uuid::to_string).collect()
    }

    /// `params` and `ctx` with every live source replaced by its stand-in:
    /// a source the request does not carry stays absent, so the stand-ins
    /// take the request's shape.
    pub(super) fn mask(
        &self,
        live_params: &[String],
        params: &Params,
        ctx: &SqlCtx,
    ) -> (Params, SqlCtx) {
        let mut masked_params = params.clone();
        for name in live_params {
            if super::key::is_ehr_id(live_params, name, params) {
                // NOTE: our own design (#5783), the stand-in is upper case, so its canonical form differs.
                let id = Uuid::from_u128(rand::random()).to_string().to_uppercase();
                masked_params.insert(name.clone(), ParamValue::Str(id));
            }
        }
        let mut masked = ctx.clone();
        if !masked.ehr_ids.is_empty() {
            masked.ehr_ids = self.ids().into_iter().map(EhrId).collect();
        }
        if masked.limit.is_some_and(|limit| limit >= 0) {
            masked.limit = Some(self.limit);
        }
        if masked.offset.is_some_and(|offset| offset > 0) {
            masked.offset = Some(self.offset);
        }
        masked.ehr_access = self.condition(&ctx.ehr_access);
        (masked_params, masked)
    }

    /// `condition` with its live values replaced.
    fn condition(&self, condition: &AccessCondition) -> AccessCondition {
        let groups = condition.groups().map(|groups| {
            Arc::new(ConditionGroups {
                cared: self.care(&groups.cared),
                facts: CallerKeys {
                    now: self.now,
                    parties: self.texts().into(),
                    keeper: groups.facts.keeper.clone(),
                },
                confined: groups.confined.as_ref().map(|_| self.ids().into()),
                ..groups.clone()
            })
        });
        let pinned = condition.pinned().map(|pinned| {
            Arc::new(PinnedEhr {
                ehr: Uuid::from_u128(rand::random()),
                revision: Revision {
                    settings: Some(self.settings),
                    subject: Some(EhrSubject::new(
                        Uuid::from_u128(rand::random()).to_string(),
                        Uuid::from_u128(rand::random()).to_string(),
                    )),
                },
                ..pinned.clone()
            })
        });
        AccessCondition::of_parts(groups, pinned)
    }

    /// `care` with its live values replaced.
    fn care(&self, care: &Care) -> Care {
        match care {
            Care::Unresolved => Care::Unresolved,
            Care::Census(_) => Care::Census(
                self.texts()
                    .into_iter()
                    .zip(self.texts())
                    .map(|(namespace, id)| CaredSubject {
                        subject: EhrSubject::new(namespace, id),
                        ties: Vec::new().into(),
                    })
                    .collect(),
            ),
            Care::Ehrs(_) => Care::Ehrs(
                self.ids()
                    .into_iter()
                    .map(|ehr| CaredEhr {
                        ehr,
                        ties: Vec::new().into(),
                    })
                    .collect(),
            ),
            Care::Projected(care) => Care::Projected(ProjectedCare {
                principals: self.ids().into(),
                direct: self.ids().into(),
                provider_issuers: self.texts().into(),
                provider_ids: self.texts().into(),
                day: self.day,
                grace: care.grace.clone(),
                self_declared: care.self_declared,
            }),
        }
    }
}
