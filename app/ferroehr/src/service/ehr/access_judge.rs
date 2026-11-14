// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator over this replica's copies: the facts a request is
//! judged under, each EHR's verdict, and a query's condition.
//!
//! Every input is held in memory: the controller's access policy
//! ([`crate::access_policy::LivePolicy`]), the category map
//! ([`crate::policy_register`]), the caller facts and care relationships
//! ([`crate::party_binding::grants::live::LiveGrants`]) and each EHR's
//! settings and subject (`service::ehr::access`), so a decision reads no table
//! while the copies are current. No openEHR spec governs this: our own
//! design (#5513).

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::AccessDefault;

use crate::access_eval::condition::{AccessCondition, InForce, RequestAccess, current};
use crate::access_eval::procedure::{Lifted, Outcome, Verdict};
use crate::access_eval::scope::{CategoryIndex, Item};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::storage::access_projection::Revision;

impl FerroEhrService {
    /// The facts a request that begins now is judged under, an EHR with no
    /// settings taking `server_default`, with the uid of the access policy
    /// version among them, which the access record names (`None` when no
    /// version is committed).
    #[must_use]
    pub fn access_in_force(&self, server_default: AccessDefault) -> (InForce, Option<String>) {
        let policy = self.access_policy().in_force();
        let in_force = InForce {
            policy: policy.policy.clone(),
            categories: CategoryIndex::new(self.policy_registers().categories()),
            server_default,
            now: jiff::Timestamp::now(),
            grants: Some(Arc::clone(self.grants())),
        };
        (in_force, policy.uid.clone())
    }

    /// The verdict of `access` on `ehr_id`, judged from this replica's copy
    /// on first use and kept for the rest of the request.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the copy cannot be read, and the
    /// fail-closed fault of settings that do not read
    /// ([`Self::ehr_access_entry`]).
    pub async fn ehr_verdict(
        &self,
        access: &RequestAccess,
        ehr_id: EhrId,
    ) -> Result<Arc<Verdict>, ServiceError> {
        if let Some(verdict) = access.verdict(ehr_id) {
            return Ok(verdict);
        }
        let entry = Box::pin(self.ehr_access_entry(ehr_id)).await?;
        Ok(access.judge_ehr(ehr_id, &entry))
    }

    /// The verdict of `access` on `ehr_id` as [`Self::ehr_verdict`] gives
    /// it, judged again from the settings row in force when the verdict was
    /// judged on a revision other than `seen`, the one a read found: a
    /// change another replica committed before this one's notification
    /// arrives decides the request. No openEHR spec governs this: our own
    /// design (#5589).
    ///
    /// # Errors
    /// As [`Self::ehr_verdict`].
    pub async fn ehr_verdict_at(
        &self,
        access: &RequestAccess,
        ehr_id: EhrId,
        seen: Revision,
    ) -> Result<Arc<Verdict>, ServiceError> {
        match access.judgement(ehr_id) {
            Some(judged) if judged.revision == seen => Ok(judged.verdict),
            Some(_) => {
                self.ehr_access.forget(ehr_id).await;
                let entry = Box::pin(self.ehr_access_entry(ehr_id)).await?;
                Ok(access.judge_ehr(ehr_id, &entry))
            }
            None => self.ehr_verdict(access, ehr_id).await,
        }
    }

    /// What the evaluator reads of each of `vo_ids` this store holds.
    ///
    /// # Errors
    /// The storage error of the read.
    pub async fn object_access_facts(
        &self,
        vo_ids: &[crate::ids::VoId],
    ) -> Result<Vec<crate::storage::access_facts::HeadFacts>, ServiceError> {
        let mut conn = crate::db::acquire::acquire(&self.pool).await?;
        Ok(crate::storage::access_facts::of_objects(&mut conn, vo_ids).await?)
    }

    /// Judges every EHR of `ehr_ids` for the running request, so the reads
    /// that follow find their verdicts; nothing outside a request.
    ///
    /// # Errors
    /// As [`Self::ehr_verdict`].
    pub async fn judge_request_ehrs(&self, ehr_ids: &[EhrId]) -> Result<(), ServiceError> {
        let Some(access) = current() else {
            return Ok(());
        };
        for ehr_id in ehr_ids {
            self.ehr_verdict(&access, *ehr_id).await?;
        }
        Ok(())
    }

    /// Whether the EHR `ehr_id` reads as absent to the running request: an
    /// opt-out or a whole-EHR restriction binds the caller (EHDS Art. 8 and
    /// 10, `docs/law/eu/ehds/text.html`); `false` outside a request.
    ///
    /// # Errors
    /// As [`Self::ehr_verdict`].
    pub(crate) async fn ehr_hidden_from_request(
        &self,
        ehr_id: EhrId,
    ) -> Result<bool, ServiceError> {
        let Some(access) = current() else {
            return Ok(false);
        };
        let verdict = self.ehr_verdict(&access, ehr_id).await?;
        Ok(matches!(verdict.outcome(&Item::Ehr), Outcome::Absent(_)))
    }

    /// The ids of `ids` that name an EHR of this store the running request
    /// may see: an EHR the evaluator answers as absent (EHDS Art. 8 and 10)
    /// is named by no query.
    ///
    /// # Errors
    /// The storage error of the existence read, and as
    /// [`Self::ehr_verdict`].
    pub(crate) async fn visible_ehr_ids(
        &self,
        ids: &[EhrId],
    ) -> Result<std::collections::HashSet<uuid::Uuid>, ServiceError> {
        let existing = crate::storage::ehr_repo::existing_ehr_ids(&self.pool, ids)
            .await
            .map_err(|e| match e {
                crate::storage::error::StorageError::Acquire(acquire) => {
                    ServiceError::Acquire(acquire)
                }
                other => ServiceError::internal("resolve the requested ehr_ids", other),
            })?;
        let mut present = std::collections::HashSet::with_capacity(existing.len());
        for id in existing {
            if !self.ehr_hidden_from_request(id).await? {
                present.insert(id.0);
            }
        }
        Ok(present)
    }

    /// The condition a query of the running request applies: every settings
    /// class judged from the copied classes, every EHR with no settings by
    /// the default; unrestricted outside a request.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the classes cannot be read.
    pub async fn request_access_condition(&self) -> Result<AccessCondition, ServiceError> {
        let Some(access) = current() else {
            return Ok(AccessCondition::default());
        };
        let classes = self.ehr_access_classes().await?;
        Ok(access.condition(&classes, access.cared_subjects()))
    }

    /// Judges every EHR the query `result` of the running request served
    /// that the request has not judged; nothing outside a request. An EHR
    /// that carried settings is judged from the per-EHR copy, the EHRs it
    /// does not hold read in one statement; any other by the default. The
    /// access record of each then names the grounds a direct read names
    /// (#5581).
    ///
    /// # Errors
    /// As [`Self::ehr_access_entries`].
    pub async fn judge_served_ehrs(
        &self,
        result: &crate::aql::exec::QueryResult,
    ) -> Result<(), ServiceError> {
        let Some(access) = current() else {
            return Ok(());
        };
        let settled: Vec<EhrId> = result
            .settled_ehrs
            .iter()
            .map(|ehr| EhrId(*ehr))
            .filter(|ehr_id| access.judgement(*ehr_id).is_none())
            .collect();
        for (ehr, _) in &result.served_ehrs {
            let ehr_id = EhrId(*ehr);
            if !result.settled_ehrs.contains(ehr) && access.judgement(ehr_id).is_none() {
                access.judge_unsettled(ehr_id);
            }
        }
        if settled.is_empty() {
            return Ok(());
        }
        for (ehr_id, entry) in self.ehr_access_entries(&settled).await? {
            access.judge_ehr(ehr_id, &entry);
        }
        Ok(())
    }
}

/// The name the access record gives an override of the opt-out.
const OPT_OUT_OVERRIDE: &str = "access-opt-out-override";

impl FerroEhrService {
    /// Record that the running request reached each EHR among `ehr_ids` under
    /// the EHDS Art. 10(2) override of its opt-out, once per EHR; an EHR whose
    /// verdict lifted no opt-out records nothing.
    ///
    /// Each record carries the declared purpose, which gives it the emergency
    /// mark, beside the access record of the request itself.
    ///
    /// # Errors
    /// As [`Self::ehr_verdict`]; `service_overloaded` when an access record
    /// was refused under `[audit] fail_mode = "closed"`.
    pub async fn record_access_opt_out_overrides(
        &self,
        ehr_ids: &[EhrId],
    ) -> Result<(), ServiceError> {
        let Some(access) = current() else {
            return Ok(());
        };
        let named: std::collections::BTreeSet<EhrId> = ehr_ids.iter().copied().collect();
        for ehr_id in named {
            let verdict = self.ehr_verdict(&access, ehr_id).await?;
            if verdict.lifted.contains(&Lifted::OptOut) {
                self.emit_mark_access(
                    crate::system_log::event::EventActionCode::Execute,
                    ehr_id,
                    None,
                    OPT_OUT_OVERRIDE,
                )?;
            }
        }
        Ok(())
    }
}
