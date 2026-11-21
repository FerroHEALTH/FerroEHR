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

use openehr_rm::draft::access_control::terminology::{AccessDefault, Refusal};

use crate::access_eval::condition::care::{Care, CareUnsettled};
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

    /// Whether the store holds the EHR `ehr_id`, read only when a verdict
    /// refuses, so an admitted request reads nothing more: an EHR that does
    /// not exist answers `404` before any step of the procedure, as a
    /// restricted one does (EHDS Art. 8; our own design, #5623).
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when no connection is free, and the
    /// storage error of the read.
    pub async fn ehr_held(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        let mut conn = crate::db::acquire::acquire(&self.pool).await?;
        Ok(crate::storage::ehr_repo::ehr_exists(&mut *conn, ehr_id).await?)
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
        Ok(access.judge_ehr(ehr_id, &entry).await?)
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
        seen: &Revision,
    ) -> Result<Arc<Verdict>, ServiceError> {
        access.read_in_store(ehr_id, seen);
        match access.judgement(ehr_id) {
            Some(judged) if judged.revision == *seen => Ok(judged.verdict),
            Some(_) => {
                self.ehr_access.forget(ehr_id).await;
                let entry = Box::pin(self.ehr_access_entry(ehr_id)).await?;
                Ok(access.judge_ehr(ehr_id, &entry).await?)
            }
            None => self.ehr_verdict(access, ehr_id).await,
        }
    }

    /// Reads ahead for `access` whether the caller facts copy holds the
    /// store's revisions, when the caller's care decides its verdict on
    /// `ehr_id` ([`RequestAccess::care_ahead`]), so a read the request makes
    /// alongside carries the exact check (owner decision on #5891). No openEHR
    /// spec governs this: our own design.
    pub(crate) async fn care_ahead(&self, access: &RequestAccess, ehr_id: EhrId) {
        if access.verdict(ehr_id).is_some() {
            return;
        }
        // NOTE: our own design (#5891), settings that do not read here read ahead nothing, and
        // the verdict reads them again and answers that read's error.
        if let Ok(entry) = Box::pin(self.ehr_access_entry(ehr_id)).await {
            access.care_ahead(ehr_id, &entry).await;
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
    /// 10, `docs/law/eu/ehds/text.html`), noting the refusal for the
    /// request's access record; `false` outside a request.
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
        let hidden = matches!(verdict.outcome(&Item::Ehr), Outcome::Absent(_));
        // NOTE: EHDS Art. 9(1) and Annex II 3.2, the refusal that hides the EHR is noted for the
        // request's access record, so a query or a lookup it answers as absent is logged (#5611).
        if hidden {
            access.note(ehr_id, &verdict, &Item::Ehr);
        }
        Ok(hidden)
    }

    /// Whether the EHR `ehr_id`, found by its subject, reads as absent to the
    /// running request's lookup: as [`Self::ehr_hidden_from_request`], and
    /// for a confined caller every EHR but its own, so a lookup of someone
    /// else's subject answers as one no EHR names and tells nothing of it.
    ///
    /// No openEHR spec governs this: our own design (#6106).
    ///
    /// # Errors
    /// As [`Self::ehr_verdict`].
    pub(crate) async fn ehr_hidden_from_lookup(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        let Some(access) = current() else {
            return Ok(false);
        };
        let verdict = self.ehr_verdict(&access, ehr_id).await?;
        let caller = access.caller();
        let hidden = match verdict.outcome(&Item::Ehr) {
            // NOTE: draft access control, The Decision Procedure, Refusals: a look-up of an EHR
            // by its subject always leaves a record refused for its confidentiality out.
            Outcome::Absent(_) | Outcome::Refused(Refusal::Confidentiality) => true,
            Outcome::Refused(refusal) => caller.confined() && refusal == caller.not_own(),
            Outcome::Admit => false,
        };
        if hidden {
            access.note(ehr_id, &verdict, &Item::Ehr);
        }
        Ok(hidden)
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
    /// the default; unrestricted outside a request. Its care is
    /// [`Care::Unresolved`] until [`Self::resolve_care`] knows which EHRs the
    /// query reads.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the classes cannot be read.
    pub async fn request_access_condition(&self) -> Result<AccessCondition, ServiceError> {
        let Some(access) = current() else {
            return Ok(AccessCondition::default());
        };
        let classes = self.ehr_access_classes().await?;
        Ok(access.condition(&classes, Care::Unresolved))
    }

    /// `condition` with the caller's care read for a query whose rows all
    /// belong to `pinned` when it names them, or to any EHR otherwise.
    ///
    /// Care that admits any EHR is read again when the caller facts copy is
    /// older than the store ([`RequestAccess::care_in_force`]), so a removal
    /// another replica committed admits nothing (#5806).
    ///
    /// A condition no verdict of which reads care, or one already resolved,
    /// stays as it is. For pinned EHRs the care of each is read from the
    /// evaluator's copies and only the cared ones are bound; otherwise the
    /// subjects of every patient under the caller's care teams are (#5699).
    /// The store's revisions are read ahead beside the clinical read of the
    /// pinned EHRs' entries, never beside a read of the party store (#5967).
    /// No openEHR spec governs this: our own design.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the settings copy or the care copy
    /// cannot be read, and `access-refreshing` (`503`) when the caller facts
    /// copy stays older than the store.
    pub async fn resolve_care(
        &self,
        condition: &AccessCondition,
        pinned: Option<&[EhrId]>,
    ) -> Result<AccessCondition, ServiceError> {
        let unresolved = condition
            .groups()
            .is_some_and(|groups| groups.cared == Care::Unresolved);
        if !unresolved || !condition.reads_care() {
            return Ok(condition.clone());
        }
        let Some(access) = current() else {
            return Ok(condition.with_care(Care::Census(Vec::new().into())));
        };
        let mut care = match pinned {
            // NOTE: our own design (#5891, #5967), the read ahead runs beside the clinical read of
            // the entries only, so the request never holds two party connections at once.
            Some(ehrs) => {
                let (entries, ()) =
                    tokio::join!(self.ehr_access_entries(ehrs), access.care_read_ahead());
                Care::Ehrs(access.cared_ehrs(&entries?).await?.into())
            }
            None => self.care_of(&access, None).await?,
        };
        if binds_any(&care) && !access.care_in_force(std::future::ready(())).await? {
            care = self.care_of(&access, pinned).await?;
        }
        Ok(condition.with_care(care))
    }

    /// The care of the caller of `access` a query reads, for its EHRs
    /// `pinned` when it names them, as [`Self::resolve_care`] reads it.
    async fn care_of(
        &self,
        access: &RequestAccess,
        pinned: Option<&[EhrId]>,
    ) -> Result<Care, ServiceError> {
        Ok(match pinned {
            Some(ehrs) => {
                let entries = self.ehr_access_entries(ehrs).await?;
                Care::Ehrs(access.cared_ehrs(&entries).await?.into())
            }
            None => match (access.attested_care(), projected_care(access)) {
                (Some(attested), _) => attested,
                (None, Some(care)) if self.care_projection_current() => Care::Projected(care),
                (None, _) => Care::Census(access.cared_subjects().await?.into()),
            },
        })
    }

    /// The caller's own records by the party binding, on which the EHDS
    /// Art. 3(3) delay withholds held data from a query (#6108): the subject
    /// references of the `PERSON` a caller whose delay holds per record is
    /// bound to, read on the party pool. `None` when the delay holds on
    /// every record the query reads or on none. No openEHR spec governs this:
    /// our own design.
    ///
    /// # Errors
    /// [`ServiceError`] when the `PERSON`'s references cannot be read.
    pub(crate) async fn delayed_own_records(
        &self,
    ) -> Result<Option<crate::aql::sql::own_records::OwnRecords>, ServiceError> {
        use crate::system_log::access_context::{AccessDelay, access_delay, access_delay_enforced};
        if access_delay() != AccessDelay::Records || access_delay_enforced() {
            return Ok(None);
        }
        let Some(access) = current() else {
            return Ok(None);
        };
        // NOTE: the local source names each party by its container's UUID, so a bound party
        // id that is no UUID names no PERSON of this store and no record.
        let Some(person) = access
            .caller()
            .party
            .as_deref()
            .and_then(|party| party.parse::<uuid::Uuid>().ok())
        else {
            return Ok(None);
        };
        let namespace = self.grants.namespace();
        let references = self.grants.subjects_of(person).await?;
        let count = u64::try_from(references.len()).unwrap_or(u64::MAX);
        let mut subjects = Vec::new();
        for subject in references {
            // NOTE: our own design (#6108), this CDR's own references name a PERSON by its
            // UUID, read in either case, as the evaluator reads it (`LiveGrants::person_of`).
            if *subject.namespace == *namespace {
                subjects.push((subject.id.to_uppercase(), subject.namespace.clone()));
                subjects.push((subject.id.to_lowercase(), subject.namespace));
            } else {
                subjects.push((subject.id, subject.namespace));
            }
        }
        // NOTE: our own design (#6108), the read of the caller's own subject references is
        // recorded as every subject-reference read is, by its namespace and never its ids.
        self.emit_resolution(&namespace, None, count).await?;
        Ok(Some(crate::aql::sql::own_records::OwnRecords {
            subjects: subjects.into(),
        }))
    }

    /// The condition of a query whose rows all belong to `pinned` when it
    /// names them: its care read as [`Self::resolve_care`] reads it, and the
    /// one EHR of a query pinned to one decided before the statement, from
    /// this replica's copy ([`RequestAccess::pin`]), so the statement carries
    /// only what the verdict leaves to each object (#5746). No openEHR spec
    /// governs this: our own design.
    ///
    /// # Errors
    /// As [`Self::resolve_care`], and as [`Self::ehr_access_entry`].
    pub async fn query_access_condition(
        &self,
        condition: &AccessCondition,
        pinned: Option<&[EhrId]>,
    ) -> Result<AccessCondition, ServiceError> {
        let resolved = Box::pin(self.resolve_care(condition, pinned)).await?;
        let (Some(access), Some([ehr_id])) = (current(), pinned) else {
            return Ok(resolved);
        };
        if resolved.groups().is_none() {
            return Ok(resolved);
        }
        let entry = Box::pin(self.ehr_access_entry(*ehr_id)).await?;
        access
            .bind_subject(*ehr_id, &entry)
            .await
            .map_err(CareUnsettled::from)?;
        Ok(access.pin(&resolved, *ehr_id, &entry))
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
            access.judge_ehr(ehr_id, &entry).await?;
        }
        Ok(())
    }
}

impl FerroEhrService {
    /// Adds to `served` every EHR among `named`, the EHRs the caller named as
    /// a query's scope, that its rows did not name, with no row: a count, an
    /// empty or a past-the-end page still accesses the EHR it names (EHDS
    /// Art. 9). Each is judged as a served EHR is, and a verdict that admits
    /// none of its data notes its refusal for the EHR's access record. Our
    /// own design (#5639).
    ///
    /// # Errors
    /// As [`Self::ehr_verdict`].
    pub async fn add_named_ehrs(
        &self,
        named: &[EhrId],
        served: &mut Vec<(String, u64)>,
    ) -> Result<(), ServiceError> {
        for ehr_id in named {
            let id = ehr_id.to_string();
            if served.iter().any(|(served, _)| *served == id) {
                continue;
            }
            if let Some(access) = current() {
                let verdict = self.ehr_verdict(&access, *ehr_id).await?;
                if !verdict.admits_any() {
                    access.note(*ehr_id, &verdict, &Item::Ehr);
                }
            }
            served.push((id, 0));
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
    /// could not be written.
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
                )
                .await?;
            }
        }
        Ok(())
    }
}

/// The probe of the clinical care projection for the caller of `access`, or
/// `None` when the policy's grace has a unit of time below a day, which a
/// date cannot carry as [`crate::party_binding::grants::caller::care_of::CareOf::live_on`]
/// judges it; that query binds the census.
fn projected_care(
    access: &RequestAccess,
) -> Option<crate::access_eval::condition::care::ProjectedCare> {
    let grace = access.policy().grace;
    let calendar = grace.get_hours() == 0
        && grace.get_minutes() == 0
        && grace.get_seconds() == 0
        && grace.get_milliseconds() == 0
        && grace.get_microseconds() == 0
        && grace.get_nanoseconds() == 0
        && !grace.is_negative();
    let (provider_issuers, provider_ids): (Vec<String>, Vec<String>) = access
        .caller()
        .facts
        .providers()
        .iter()
        .map(|provider| (provider.issuer.clone(), provider.id.clone()))
        .unzip();
    let day = access.today()?;
    let principals = access.principals_held();
    let facts = &access.caller().facts;
    let direct: Vec<uuid::Uuid> = principals
        .iter()
        .copied()
        .filter(|principal| {
            facts.party() == Some(*principal) || facts.performed().binary_search(principal).is_ok()
        })
        .collect();
    calendar.then(|| crate::access_eval::condition::care::ProjectedCare {
        principals: principals.into(),
        direct: direct.into(),
        provider_issuers: provider_issuers.into(),
        provider_ids: provider_ids.into(),
        day,
        grace: grace.to_string(),
        self_declared: access.policy().self_declared,
    })
}

/// Whether `care` admits any EHR, so the copy it was read from must hold the
/// store's revisions before a query binds it.
fn binds_any(care: &Care) -> bool {
    match care {
        Care::Unresolved => false,
        Care::Census(subjects) => !subjects.is_empty(),
        Care::Ehrs(ehrs) => !ehrs.is_empty(),
        Care::Projected(care) => !care.principals.is_empty() && !care.provider_ids.is_empty(),
    }
}
