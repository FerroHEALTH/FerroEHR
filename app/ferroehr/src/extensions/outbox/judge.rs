// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which clinical outbox events the feed publishes: those the access
//! evaluator lets an access service of the system kind read.
//!
//! An event names the EHR and the contribution it announces, which a reader
//! downstream can act on, so it is published only where the EHR's
//! `EHR_ACCESS` settings admit a read by a caller of `CallerKind::System`
//! with no party, role, purpose or care relationship: every directive that
//! binds such a caller and denies the data is honoured, its data scope,
//! validity and exceptions included (RM ehr `ehr_access.adoc` §`EHR_ACCESS`
//! Class: "All access decisions to data in the EHR must be made in accordance
//! with the policies and rules in this object"). An event whose verdict
//! depends on the data is published only when every object its contribution
//! committed a version of is admitted, each judged by its trunk head as a read
//! judges it; an erasure, which names no contribution, is judged on the EHR
//! itself. Settings that do not read hold the EHR closed. The caller, the
//! judgement of a contribution as a whole and the paging past what is left
//! out are our own design (#5587).

use std::collections::{BTreeMap, BTreeSet};

use openehr_rm::draft::access_control::terminology::{CallerKind, OperationClass};
use uuid::Uuid;

use super::FeedEvent;
use crate::access_eval::condition::{Asking, RequestAccess};
use crate::access_eval::procedure::{Caller, Emergency, Outcome, Verdict};
use crate::access_eval::scope::Item;
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::CallerFacts;
use crate::service::FerroEhrService;
use crate::service::ehr::access_rule::SettingsCaller;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_facts::HeadFacts;
use crate::storage::access_projection::StoredSettings;

/// The message of the `503` of access settings that cannot be read.
const UNANSWERED: &str = "the EHR access settings of the outbox events could not be read";

impl FerroEhrService {
    /// The events of `events` the access evaluator lets an access service of
    /// the system kind read, in their order.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the access settings cannot be read,
    /// and the storage error of the objects' read.
    pub(super) async fn publishable(
        &self,
        events: Vec<FeedEvent>,
    ) -> Result<Vec<FeedEvent>, ServiceError> {
        let ehr_ids: Vec<EhrId> = events
            .iter()
            .filter_map(|event| event.ehr_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if ehr_ids.is_empty() {
            return Ok(events);
        }
        let access = self.outbox_access();
        let stored = crate::storage::access_projection::settings_of_many(&self.pool, &ehr_ids)
            .await
            .map_err(|e| ServiceError::overloaded(UNANSWERED, e))?;
        let verdicts: BTreeMap<EhrId, Verdict> = stored
            .into_iter()
            .map(|(ehr_id, stored)| {
                let settings = settings_of(ehr_id, stored.settings);
                (ehr_id, access.judge(settings.as_ref(), false))
            })
            .collect();
        let contributions: Vec<Uuid> = events
            .iter()
            .filter(|event| {
                event
                    .ehr_id
                    .and_then(|ehr_id| verdicts.get(&ehr_id))
                    .is_some_and(Verdict::item_dependent)
            })
            .filter_map(|event| event.contribution_id)
            .collect();
        let mut objects: BTreeMap<Uuid, Vec<HeadFacts>> = BTreeMap::new();
        if !contributions.is_empty() {
            let mut conn = acquire(&self.pool).await?;
            let facts =
                crate::storage::access_facts::of_contributions(&mut conn, &contributions).await?;
            for (contribution, facts) in facts {
                objects.entry(contribution).or_default().push(facts);
            }
        }
        Ok(events
            .into_iter()
            .filter(|event| admitted(event, &verdicts, &objects))
            .collect())
    }

    /// The evaluator of the feed: an access service of the system kind
    /// reading, with no party, role, purpose, emergency or care relationship,
    /// under the access policy, the category map and the default in force.
    fn outbox_access(&self) -> RequestAccess {
        let caller = Caller {
            kind: CallerKind::System,
            party: None,
            acts_for: Vec::new(),
            roles: Vec::new(),
            facts: CallerFacts::default(),
            administrator: false,
            settings: SettingsCaller::new(false, false, None),
        };
        let asking = Asking {
            operation: OperationClass::Read,
            purpose: None,
            emergency: Emergency::default(),
        };
        let (in_force, _) = self.access_in_force(self.ehr_access_default);
        RequestAccess::new(caller, asking, in_force)
    }
}

/// The settings `stored` holds for `ehr_id`; settings that do not read hold
/// the EHR closed.
fn settings_of(ehr_id: EhrId, stored: StoredSettings) -> Option<EhrAccessSettings> {
    match stored {
        StoredSettings::None => None,
        StoredSettings::HeldClosed => Some(EhrAccessSettings::HeldClosed),
        StoredSettings::Settings(text) => Some(
            EhrAccessSettings::from_projected(&text).unwrap_or_else(|error| {
                let cause: &(dyn std::error::Error + 'static) = &error;
                tracing::error!(%ehr_id, error = cause, "EHR_ACCESS settings do not read; the outbox withholds the EHR's events");
                EhrAccessSettings::HeldClosed
            }),
        ),
    }
}

/// Whether `event` may be published: it names no EHR, or its EHR's verdict
/// admits the objects its contribution committed, or the EHR itself when it
/// names none; an EHR that was not judged admits nothing.
fn admitted(
    event: &FeedEvent,
    verdicts: &BTreeMap<EhrId, Verdict>,
    objects: &BTreeMap<Uuid, Vec<HeadFacts>>,
) -> bool {
    let Some(ehr_id) = event.ehr_id else {
        return true;
    };
    let Some(verdict) = verdicts.get(&ehr_id) else {
        return false;
    };
    let facts = event
        .contribution_id
        .and_then(|contribution| objects.get(&contribution))
        .filter(|facts| !facts.is_empty());
    match facts {
        Some(facts) if verdict.item_dependent() => facts
            .iter()
            .all(|facts| verdict.outcome(&facts.item()) == Outcome::Admit),
        _ => verdict.outcome(&Item::Ehr) == Outcome::Admit,
    }
}
