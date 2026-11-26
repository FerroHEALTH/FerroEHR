// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legal marks an administrator sets and lifts: restriction of processing,
//! the research objection, and the retention register.
//!
//! **Our own extension.** No openEHR
//! specification governs restriction, objection or retention. What the RM does
//! say is that none of these may be answered with what it already has:
//! `EHR_STATUS.is_queryable` limits population queries and nothing else (RM ehr
//! `master04-ehr_package.adoc` §EHR Status), and content is indelible (RM
//! common `master06-change_control_package.adoc` §Logical Deletion), so a
//! retention period is a list for the controller to act on, never a timer.
//!
//! Every call here emits an access event. These are the acts a supervisory
//! authority asks about after the fact — who restricted this record, when was
//! it lifted, who overrode an objection — so leaving them out of the trail
//! would make the register's own history the one thing the repository could not
//! account for. GDPR Art. 18(3) (`docs/law/eu/gdpr/text.html`) makes the
//! sequence itself the obligation.

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::storage::marks;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};
/// The grounds a restriction may rest on, as the register's CHECK spells them.
///
/// The four GDPR Art. 18(1) points plus `national`, which a deployment uses for
/// a member-state rule that goes beyond them — BDSG § 35 Abs. 2 and Abs. 3 are
/// the worked example, restriction in place of erasure
/// (`docs/law/de/bdsg/BJNR209710017.xml`).
pub const RESTRICTION_GROUNDS: [&str; 5] = [
    "gdpr-18-1-a",
    "gdpr-18-1-b",
    "gdpr-18-1-c",
    "gdpr-18-1-d",
    "national",
];

impl FerroEhrService {
    /// Record a restriction of processing on one EHR, or on one versioned
    /// object of it.
    ///
    /// The register row is the evidence and the marks it implies are recomputed
    /// from it, so recording the same restriction twice is two evidence rows
    /// and one mark.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a ground outside
    ///   [`RESTRICTION_GROUNDS`].
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `versioned_object_does_not_exist` (`404`) — the object is unknown, or
    ///   is not in that EHR.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn restrict_processing(
        &self,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
        ground: &str,
        note: Option<&str>,
    ) -> Result<(), ServiceError> {
        if !RESTRICTION_GROUNDS.contains(&ground) {
            return Err(ServiceError::precondition(format!(
                "restriction ground {ground:?} is not one of {}",
                RESTRICTION_GROUNDS.join(", ")
            )));
        }
        self.check_mark_target(ehr_id, vo_id).await?;
        let mut conn = self.recorded_conn(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::record_restriction(&mut tx, ehr_id, vo_id, ground, note)
            .await
            .map_err(ServiceError::Storage)?;
        let record = self.mark_record(EventActionCode::Create, ehr_id, vo_id, "restriction");
        self.commit_act_recorded(tx, vec![record]).await?;
        Ok(())
    }

    /// Lift every in-force restriction of one EHR at the given grain.
    ///
    /// Lifting writes the register rather than erasing it, so the sequence
    /// GDPR Art. 18(3) asks about survives. Returns nothing: a lift with
    /// nothing to lift is a no-op that succeeded.
    ///
    /// # Errors
    /// The [`Self::restrict_processing`] rejections, minus the ground check.
    pub async fn lift_restriction(
        &self,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
    ) -> Result<(), ServiceError> {
        self.check_mark_target(ehr_id, vo_id).await?;
        let mut conn = self.recorded_conn(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::lift_restriction(&mut tx, ehr_id, vo_id)
            .await
            .map_err(ServiceError::Storage)?;
        let record = self.mark_record(EventActionCode::Delete, ehr_id, vo_id, "restriction");
        self.commit_act_recorded(tx, vec![record]).await?;
        Ok(())
    }

    /// The restriction register of one EHR, newest request first.
    ///
    /// The read the controller answers an Art. 15 access request from, and what
    /// the Art. 18(3) notification is prepared against.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record could not be written.
    pub async fn restriction_register(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<marks::RestrictionRow>, ServiceError> {
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, "restriction")
            .await?;
        marks::restrictions(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Record the subject's objection to research processing, the controller's
    /// public-interest override of one, or the withdrawal of the objection.
    ///
    /// `objected = false` withdraws; `ground` records the Art. 21(6) override
    /// and is meaningful only while the objection stands.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a ground given
    ///   together with a withdrawal (the two contradict each other).
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn set_research_objection(
        &self,
        ehr_id: EhrId,
        objected: bool,
        ground: Option<&str>,
    ) -> Result<(), ServiceError> {
        if !objected && ground.is_some() {
            return Err(ServiceError::precondition(
                "a withdrawal of the objection carries no override ground: there is nothing \
                 left to override",
            ));
        }
        self.check_mark_target(ehr_id, None).await?;
        let mut conn = self.recorded_conn(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        if objected {
            marks::set_research_objection(&mut tx, ehr_id, ground)
                .await
                .map_err(ServiceError::Storage)?;
        } else {
            marks::clear_research_objection(&mut tx, ehr_id)
                .await
                .map_err(ServiceError::Storage)?;
        }
        let action = if objected {
            EventActionCode::Create
        } else {
            EventActionCode::Delete
        };
        let record = self.mark_record(action, ehr_id, None, "research-objection");
        self.commit_act_recorded(tx, vec![record]).await?;
        Ok(())
    }

    /// The research objection one EHR carries: when the subject objected and
    /// any Art. 21(6) override ground.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record could not be written.
    pub async fn research_objection(
        &self,
        ehr_id: EhrId,
    ) -> Result<marks::ResearchObjectionRow, ServiceError> {
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, "research-objection")
            .await?;
        marks::research_objection(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Declares the retention period of one content kind in one jurisdiction,
    /// optionally keyed on one EHDS category and one registered origin, with
    /// the legal citation it rests on.
    ///
    /// GDPR Art. 30(1)(f) and DSG Art. 25 Abs. 2 lit. d ask for the retention
    /// periods (`docs/law/eu/gdpr/text.html`, `docs/law/ch/fadp/text-de.html`),
    /// and EHDS Annex II 3.4 for periods that "take into account the origins and
    /// categories of electronic health data" (`docs/law/eu/ehds/text.html`). A
    /// category is stored as its `<system>|<code>` token; an origin is the id a
    /// `data_origin` grant in force names ([`crate::origins`]). `None` keys the
    /// period on every category or origin.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a kind, category, origin, anchor
    ///   rule or period the register refuses.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn put_retention_policy(
        &self,
        period: marks::RetentionPeriod<'_>,
    ) -> Result<(), ServiceError> {
        if let Some(origin) = period.origin
            && !self.registered_origins().await?.contains(origin)
        {
            return Err(ServiceError::precondition(format!(
                "{origin:?} is not a registered origin: a retention period is keyed only on an \
                 origin a data_origin grant in force names, by its id"
            )));
        }
        let category = period
            .category
            .map(|reference| {
                let categories = self.registers.categories();
                let map = &categories.classifier;
                if reference == crate::system_log::categories::NONE {
                    return Ok(reference.to_owned());
                }
                map.category_token(reference).ok_or_else(|| {
                    ServiceError::precondition(format!(
                        "{reference:?} is not an audit category: a priority category of EHDS \
                         Art. 14(1) by its code (Patient-Summaries, Electronic-Prescriptions, \
                         Electronic-Dispensations, Medical-Imaging, Laboratory-Reports, \
                         Discharge-Reports) or as <system>|<code>, or a national category \
                         the audit category register declares, or none"
                    ))
                })
            })
            .transpose()?;
        let mut conn = self.recorded_conn(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::put_retention_policy(
            &mut tx,
            marks::RetentionPeriod {
                category: category.as_deref(),
                ..period
            },
        )
        .await
        .map_err(|e| {
            ServiceError::precondition(format!("the retention register refused the period: {e}"))
        })?;
        self.commit_act_recorded(tx, Vec::new()).await?;
        Ok(())
    }

    /// Mirror the audit category register in force into the retention
    /// register's `category_map`, replacing the previous mirror whole, so the
    /// `retention_due` view keys category periods on the map in force.
    ///
    /// Called at boot; a write of the register mirrors its map in the same
    /// transaction. No openEHR spec governs the map — our own
    /// design/extension.
    ///
    /// # Errors
    /// `exception` — a database fault (rolled back).
    pub async fn mirror_category_map(&self) -> Result<(), ServiceError> {
        let categories = self.registers.categories();
        marks::replace_category_map(&self.pool, categories.classifier.rows())
            .await
            .map_err(ServiceError::Storage)
    }

    /// The retention register as the book page and the access answer render it.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn retention_policies(&self) -> Result<Vec<marks::RetentionPolicyRow>, ServiceError> {
        marks::retention_policies(&self.pool)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Record one EHR's jurisdiction, its anchor instant once known, and any
    /// EHR-wide hold that suspends disposal.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a hold instant with no
    ///   ground.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    /// - `service_overloaded` — the access record could not be written.
    pub async fn put_retention_anchor(
        &self,
        ehr_id: EhrId,
        jurisdiction: &str,
        anchored_at: Option<&str>,
        hold: Option<(&str, &str)>,
    ) -> Result<(), ServiceError> {
        self.check_mark_target(ehr_id, None).await?;
        let mut conn = self.recorded_conn(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::put_retention_anchor(&mut tx, ehr_id, jurisdiction, anchored_at, hold)
            .await
            .map_err(|e| {
                ServiceError::precondition(format!(
                    "the retention register refused the anchor: {e}"
                ))
            })?;
        let record = self.mark_record(EventActionCode::Update, ehr_id, None, "retention-anchor");
        self.commit_act_recorded(tx, vec![record]).await?;
        Ok(())
    }

    /// The retention facts of one EHR, `None` while none is recorded.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record could not be written.
    pub async fn retention_anchor(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<marks::RetentionAnchorRow>, ServiceError> {
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, "retention-anchor")
            .await?;
        marks::retention_anchor(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// The versioned objects of one EHR held from disposal, newest hold first.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record could not be written.
    pub async fn retention_holds(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<marks::RetentionHoldRow>, ServiceError> {
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, "retention-hold")
            .await?;
        marks::retention_holds(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Place or release the per-object retention hold.
    ///
    /// EPDV Art. 10 Abs. 2 lit. b lets the patient ask that named data be
    /// exempted from the twenty-year destruction of Abs. 1 lit. d
    /// (`docs/law/ch/epdv/text-de.html`); this is the exemption at object
    /// grain, and `retention_due` counts what it covers.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — the object is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    /// - `service_overloaded` — the access record could not be written.
    pub async fn set_retention_hold(&self, vo_id: VoId, held: bool) -> Result<(), ServiceError> {
        let mut conn = self.recorded_conn(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        let Some(owner) = marks::set_retention_hold(&mut tx, vo_id, held)
            .await
            .map_err(ServiceError::Storage)?
        else {
            return Err(ServiceError::not_found(
                Missing::VersionedObject,
                format!("no versioned object {vo_id}"),
            ));
        };
        let action = if held {
            EventActionCode::Create
        } else {
            EventActionCode::Delete
        };
        let records = owner
            .map(|ehr_id| self.mark_record(action, ehr_id, Some(vo_id), "retention-hold"))
            .into_iter()
            .collect();
        self.commit_act_recorded(tx, records).await?;
        Ok(())
    }

    /// What has fallen due, oldest first.
    ///
    /// A list, and only a list: the CDR deletes no clinical content on a timer,
    /// because the record is indelible (RM common master06 §Logical Deletion)
    /// and the national record-keeping periods are minima the controller weighs
    /// against the storage-limitation principle. Acting on a row is the
    /// controller's decision, carried out through the physical delete.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a non-positive limit.
    /// - `exception` — a database fault.
    pub async fn retention_due(
        &self,
        limit: i64,
    ) -> Result<Vec<marks::RetentionDueRow>, ServiceError> {
        if limit <= 0 {
            return Err(ServiceError::precondition(
                "the retention-due limit must be a positive number of rows",
            ));
        }
        marks::retention_due(&self.pool, limit)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Existence-check the EHR a mark names, and the object when one is named.
    pub(super) async fn check_mark_target(
        &self,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
    ) -> Result<(), ServiceError> {
        if !crate::storage::ehr_repo::ehr_exists(&mut *acquire(&self.pool).await?, ehr_id).await? {
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("no EHR with id {ehr_id}"),
            ));
        }
        if let Some(vo_id) = vo_id {
            let owner = crate::storage::version_repo::meta::vo_owner(&self.pool, vo_id).await?;
            if owner.flatten() != Some(ehr_id) {
                return Err(ServiceError::not_found(
                    Missing::VersionedObject,
                    format!("no versioned object {vo_id} in EHR {ehr_id}"),
                ));
            }
        }
        Ok(())
    }

    /// Record one administrative act on a legal mark.
    ///
    /// The record names the EHR, the object when the act was object-scoped, and
    /// which register moved — never a subject identifier, which the clinical
    /// domain does not hold in readable form anyway.
    pub(in crate::service) async fn emit_mark_access(
        &self,
        action: EventActionCode,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
        register: &str,
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        self.record_access(self.mark_record(action, ehr_id, vo_id, register))
            .await
    }

    /// The record of one administrative act on a legal mark, as
    /// [`Self::emit_mark_access`] writes it.
    pub(in crate::service) fn mark_record(
        &self,
        action: EventActionCode,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
        register: &str,
    ) -> AuditEvent {
        let mut event = AuditEvent::new(action, ObjectClass::Ehr, EventOutcome::Success);
        event.domain = AccessDomain::Ehr;
        event.object_id = Some(match vo_id {
            Some(vo_id) => format!("{register}:{vo_id}"),
            None => format!("{register}:ehr-wide"),
        });
        event.ehr_id = Some(ehr_id.to_string());
        event.result_count = Some(1);
        self.stamp_committer(&mut event);
        event
    }

    /// Commit `tx`, an act no write facts describe, with `records` reserved
    /// ([`Self::commit_recorded`]).
    pub(in crate::service) async fn commit_act_recorded(
        &self,
        tx: crate::system_log::reservation::RecordedTx<'_>,
        records: Vec<AuditEvent>,
    ) -> Result<(), ServiceError> {
        self.commit_recorded(
            tx,
            &crate::system_log::recorder::WriteFacts::default(),
            records,
        )
        .await
    }
}
