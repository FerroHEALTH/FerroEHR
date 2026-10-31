// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legal marks an administrator sets and lifts: restriction of processing,
//! the research objection, and the retention register.
//!
//! **Our own extension.** The SM declares no admin interface for any of this
//! (`docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc` is the two
//! physical deletes and the four statistics calls), and no openEHR
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

use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::{ServiceStatus, StatusError};
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
    /// - `precondition_violation` (`400`) — a malformed id, or a ground outside
    ///   [`RESTRICTION_GROUNDS`].
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `versioned_object_does_not_exist` (`404`) — the object is unknown, or
    ///   is not in that EHR.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn restrict_processing(
        &self,
        ehr_id: &str,
        vo_id: Option<&str>,
        ground: &str,
        note: Option<&str>,
    ) -> Result<(), StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        let vo_id = vo_id
            .map(|raw| super::parse_uuid(raw, "versioned object").map(VoId))
            .transpose()?;
        if !RESTRICTION_GROUNDS.contains(&ground) {
            return Err(StatusError::precondition(format!(
                "restriction ground {ground:?} is not one of {}",
                RESTRICTION_GROUNDS.join(", ")
            )));
        }
        self.check_mark_target(ehr_id, vo_id).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::record_restriction(&mut tx, ehr_id, vo_id, ground, note)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Create, ehr_id, vo_id, "restriction")?;
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
        ehr_id: &str,
        vo_id: Option<&str>,
    ) -> Result<(), StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        let vo_id = vo_id
            .map(|raw| super::parse_uuid(raw, "versioned object").map(VoId))
            .transpose()?;
        self.check_mark_target(ehr_id, vo_id).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::lift_restriction(&mut tx, ehr_id, vo_id)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Delete, ehr_id, vo_id, "restriction")?;
        Ok(())
    }

    /// The restriction register of one EHR, newest request first.
    ///
    /// The read the controller answers an Art. 15 access request from, and what
    /// the Art. 18(3) notification is prepared against.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed EHR id.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn restriction_register(
        &self,
        ehr_id: &str,
    ) -> Result<Vec<marks::RestrictionRow>, StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, "restriction")?;
        Ok(marks::restrictions(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// Record the subject's objection to research processing, the controller's
    /// public-interest override of one, or the withdrawal of the objection.
    ///
    /// `objected = false` withdraws; `ground` records the Art. 21(6) override
    /// and is meaningful only while the objection stands.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed EHR id, or a ground
    ///   given together with a withdrawal (the two contradict each other).
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn set_research_objection(
        &self,
        ehr_id: &str,
        objected: bool,
        ground: Option<&str>,
    ) -> Result<(), StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        if !objected && ground.is_some() {
            return Err(StatusError::precondition(
                "a withdrawal of the objection carries no override ground: there is nothing \
                 left to override",
            ));
        }
        self.check_mark_target(ehr_id, None).await?;
        let mut conn = acquire(&self.pool).await?;
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
        tx.commit().await.map_err(ServiceError::Database)?;
        let action = if objected {
            EventActionCode::Create
        } else {
            EventActionCode::Delete
        };
        self.emit_mark_access(action, ehr_id, None, "research-objection")?;
        Ok(())
    }

    /// Declare the retention period of one content kind in one jurisdiction,
    /// optionally keyed on one EHDS category and one registered origin, with
    /// the legal citation it rests on.
    ///
    /// GDPR Art. 30(1)(f) asks the record of processing activities for "the
    /// envisaged time limits for erasure of the different categories of data"
    /// and DSG Art. 25 Abs. 2 lit. d asks that the subject learn "die
    /// Aufbewahrungsdauer der Personendaten oder ... die Kriterien zur
    /// Festlegung dieser Dauer" (`docs/law/eu/gdpr/text.html`,
    /// `docs/law/ch/fadp/text-de.html`). EHDS Annex II 3.4 asks the storing
    /// components to support retention periods that "take into account the
    /// origins and categories of electronic health data"
    /// (`docs/law/eu/ehds/text.html`); `category` and `origin` are those
    /// dimensions. A category is named as `[audit.categories]` names one and
    /// stored as its `<system>|<code>` token; an origin is the id of a
    /// registered origin (`[origins]`). `None` keys the period on every object
    /// of the kind, or of every origin. The register is where those answers
    /// come from.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a kind, category, origin, anchor
    ///   rule or period the register refuses.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn put_retention_policy(
        &self,
        period: marks::RetentionPeriod<'_>,
    ) -> Result<(), StatusError> {
        if let Some(origin) = period.origin
            && !self.origins().is_registered(origin)
        {
            return Err(StatusError::precondition(format!(
                "{origin:?} is not a registered origin: a retention period is keyed only on an \
                 origin [[origins.registered]] declares, by its id"
            )));
        }
        let category = period
            .category
            .map(|reference| {
                let fallback = crate::system_log::categories::Classifier::default();
                let map = self.audit.as_ref().map_or(
                    &fallback,
                    crate::system_log::sender::AuditSender::categories,
                );
                if reference == crate::system_log::categories::NONE {
                    return Ok(reference.to_owned());
                }
                map.category_token(reference).ok_or_else(|| {
                    StatusError::precondition(format!(
                        "{reference:?} is not an audit category: a priority category of EHDS \
                         Art. 14(1) by its code (Patient-Summaries, Electronic-Prescriptions, \
                         Electronic-Dispensations, Medical-Imaging, Laboratory-Reports, \
                         Discharge-Reports) or as <system>|<code>, or a national category \
                         [audit.categories] national declares, or none"
                    ))
                })
            })
            .transpose()?;
        let mut conn = acquire(&self.pool).await?;
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
            StatusError::precondition(format!("the retention register refused the period: {e}"))
        })?;
        tx.commit().await.map_err(ServiceError::Database)?;
        Ok(())
    }

    /// Mirror the `[audit.categories]` map into the retention register's
    /// `category_map`, replacing the previous mirror whole, so the
    /// `retention_due` view keys category periods on the map in force.
    ///
    /// Called once at boot. No openEHR spec governs the map — our own
    /// design/extension.
    ///
    /// # Errors
    /// `exception` — a database fault (rolled back).
    pub async fn mirror_category_map(
        &self,
        map: &crate::system_log::categories::Classifier,
    ) -> Result<(), StatusError> {
        Ok(marks::replace_category_map(&self.pool, map.rows())
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// The retention register as the book page and the access answer render it.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn retention_policies(&self) -> Result<Vec<marks::RetentionPolicyRow>, StatusError> {
        Ok(marks::retention_policies(&self.pool)
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// Record one EHR's jurisdiction, its anchor instant once known, and any
    /// EHR-wide hold that suspends disposal.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed EHR id, or a hold
    ///   instant with no ground.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn put_retention_anchor(
        &self,
        ehr_id: &str,
        jurisdiction: &str,
        anchored_at: Option<&str>,
        hold: Option<(&str, &str)>,
    ) -> Result<(), StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        self.check_mark_target(ehr_id, None).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::put_retention_anchor(&mut tx, ehr_id, jurisdiction, anchored_at, hold)
            .await
            .map_err(|e| {
                StatusError::precondition(format!("the retention register refused the anchor: {e}"))
            })?;
        tx.commit().await.map_err(ServiceError::Database)?;
        Ok(())
    }

    /// Place or release the per-object retention hold.
    ///
    /// EPDV Art. 10 Abs. 2 lit. b lets the patient ask that named data be
    /// exempted from the twenty-year destruction of Abs. 1 lit. d
    /// (`docs/law/ch/epdv/text-de.html`); this is the exemption at object
    /// grain, and `retention_due` counts what it covers.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed id.
    /// - `versioned_object_does_not_exist` (`404`) — the object is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn set_retention_hold(&self, vo_id: &str, held: bool) -> Result<(), StatusError> {
        let vo_id = VoId(super::parse_uuid(vo_id, "versioned object")?);
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        let found = marks::set_retention_hold(&mut tx, vo_id, held)
            .await
            .map_err(ServiceError::Storage)?;
        if !found {
            return Err(StatusError::new(
                ServiceStatus::VersionedObjectDoesNotExist,
                format!("no versioned object {vo_id}"),
            ));
        }
        tx.commit().await.map_err(ServiceError::Database)?;
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
    ) -> Result<Vec<marks::RetentionDueRow>, StatusError> {
        if limit <= 0 {
            return Err(StatusError::precondition(
                "the retention-due limit must be a positive number of rows",
            ));
        }
        Ok(marks::retention_due(&self.pool, limit)
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// Record an EHDS Art. 8 restriction of access by health professionals on
    /// one EHR, or on one versioned object of it, on the person's instruction.
    ///
    /// The restricted data then reads as absent to every professional route,
    /// query and export, unless an emergency access in the vital interest of
    /// the person overrides it (EHDS Art. 11(5)). It limits access only:
    /// nothing is deleted, and storage and the controller's own processing go
    /// on (`docs/law/eu/ehds/text.html` Art. 8). No openEHR spec governs
    /// restriction of access — our own design/extension.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed id.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `versioned_object_does_not_exist` (`404`) — the object is unknown, or
    ///   is not in that EHR.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn restrict_access(
        &self,
        ehr_id: &str,
        vo_id: Option<&str>,
        note: Option<&str>,
    ) -> Result<(), StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        let vo_id = vo_id
            .map(|raw| super::parse_uuid(raw, "versioned object").map(VoId))
            .transpose()?;
        self.check_mark_target(ehr_id, vo_id).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::record_access_restriction(&mut tx, ehr_id, vo_id, note)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Create, ehr_id, vo_id, "access-restriction")?;
        Ok(())
    }

    /// Lift every in-force EHDS Art. 8 access restriction of one EHR at the
    /// given grain; a lift with nothing to lift succeeds.
    ///
    /// # Errors
    /// The [`Self::restrict_access`] rejections.
    pub async fn lift_access_restriction(
        &self,
        ehr_id: &str,
        vo_id: Option<&str>,
    ) -> Result<(), StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        let vo_id = vo_id
            .map(|raw| super::parse_uuid(raw, "versioned object").map(VoId))
            .transpose()?;
        self.check_mark_target(ehr_id, vo_id).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        marks::lift_access_restriction(&mut tx, ehr_id, vo_id)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Delete, ehr_id, vo_id, "access-restriction")?;
        Ok(())
    }

    /// The EHDS Art. 8 access-restriction register of one EHR, newest request
    /// first.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed id.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    pub async fn access_restriction_register(
        &self,
        ehr_id: &str,
    ) -> Result<Vec<marks::AccessRestrictionRow>, StatusError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, "access-restriction")?;
        Ok(marks::access_restrictions(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// Whether the EHR `ehr_id` carries a whole-EHR EHDS Art. 8 access
    /// restriction; `false` for an unknown EHR.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn is_ehr_access_restricted(&self, ehr_id: EhrId) -> Result<bool, StatusError> {
        Ok(marks::is_ehr_access_restricted(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// Whether the versioned object `vo_id` carries an EHDS Art. 8 access
    /// restriction, its own or its EHR's; `false` for an unknown object.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn is_object_access_restricted(&self, vo_id: VoId) -> Result<bool, StatusError> {
        let mut conn = acquire(&self.pool).await?;
        Ok(marks::is_access_restricted(&mut conn, vo_id)
            .await
            .map_err(ServiceError::Storage)?)
    }

    /// Existence-check the EHR a mark names, and the object when one is named.
    ///
    /// All-or-nothing like every other admin route: an unknown id is refused
    /// before anything is written, so a typo cannot leave a register row
    /// pointing at nothing.
    pub(super) async fn check_mark_target(
        &self,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
    ) -> Result<(), ServiceError> {
        if !crate::storage::version_repo::meta::ehr_exists(&self.pool, ehr_id).await? {
            return Err(ServiceError::new(
                ServiceStatus::EhrIdDoesNotExist,
                format!("no EHR with id {ehr_id}"),
            ));
        }
        if let Some(vo_id) = vo_id {
            let owner = crate::storage::version_repo::meta::vo_owner(&self.pool, vo_id).await?;
            if owner.flatten() != Some(ehr_id) {
                return Err(ServiceError::new(
                    ServiceStatus::VersionedObjectDoesNotExist,
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
    pub(super) fn emit_mark_access(
        &self,
        action: EventActionCode,
        ehr_id: EhrId,
        vo_id: Option<VoId>,
        register: &str,
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(action, ObjectClass::Ehr, EventOutcome::Success);
        event.domain = AccessDomain::Ehr;
        event.object_id = Some(match vo_id {
            Some(vo_id) => format!("{register}:{vo_id}"),
            None => format!("{register}:ehr-wide"),
        });
        event.ehr_id = Some(ehr_id.to_string());
        event.result_count = Some(1);
        event.purpose = crate::system_log::access_context::current_purpose();
        event.permit = crate::system_log::access_context::current_permit();
        if let Some(committer) = crate::service::committer::current_committer() {
            event.user_id = committer.subject;
        }
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event).map_err(ServiceError::Unrecorded)
    }
}
