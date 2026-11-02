// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 10 opt-out from access through the access services: the
//! record, the reversal, the register and the override record.
//!
//! Regulation (EU) 2025/327 Art. 10(1) lets a Member State's law give a person
//! the right to opt out from access to their data "through the electronic
//! health data access services referred to in Articles 4 and 12", reversibly,
//! and Art. 10(2) lets a healthcare provider or health professional reach the
//! data where processing is necessary to protect vital interests
//! (`docs/law/eu/ehds/text.html`). The software part is a whole-EHR mark the
//! callers a deployment declares as access-service callers read as absent;
//! the Member State's law, its rules and its safeguards are the deployer's.
//! Recording, reversing, reading and overriding the opt-out are each an access
//! record. No openEHR spec governs an opt-out: our own design/extension.

use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::storage::opt_out;
use crate::system_log::event::EventActionCode;

/// The register name the opt-out and its reversal carry in the access record.
const REGISTER: &str = "access-opt-out";

impl FerroEhrService {
    /// Record the person's EHDS Art. 10(1) opt-out for one EHR, on their
    /// instruction.
    ///
    /// Recording a second opt-out while one stands adds a register row and
    /// leaves the mark as it was.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed EHR id.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    /// - `service_overloaded` — the access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn record_access_opt_out(
        &self,
        ehr_id: &str,
        note: Option<&str>,
    ) -> Result<(), ServiceError> {
        let ehr_id = self.opt_out_target(ehr_id).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        opt_out::record(&mut tx, ehr_id, note)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Create, ehr_id, None, REGISTER)?;
        Ok(())
    }

    /// Reverse every opt-out of one EHR that stands (EHDS Art. 10(1): the
    /// exercise of the right is reversible); reversing nothing succeeds.
    ///
    /// # Errors
    /// The [`Self::record_access_opt_out`] rejections.
    pub async fn reverse_access_opt_out(&self, ehr_id: &str) -> Result<(), ServiceError> {
        let ehr_id = self.opt_out_target(ehr_id).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        opt_out::reverse(&mut tx, ehr_id)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Delete, ehr_id, None, REGISTER)?;
        Ok(())
    }

    /// The opt-out register of one EHR, newest first.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed EHR id.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn access_opt_out_register(
        &self,
        ehr_id: &str,
    ) -> Result<Vec<opt_out::OptOutRow>, ServiceError> {
        let ehr_id = self.opt_out_target(ehr_id).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, REGISTER)?;
        opt_out::register(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// The EHR an opt-out act names, parsed and existence-checked.
    async fn opt_out_target(&self, ehr_id: &str) -> Result<EhrId, ServiceError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        self.check_mark_target(ehr_id, None).await?;
        Ok(ehr_id)
    }

    /// Whether the EHR `ehr_id` carries the EHDS Art. 10 opt-out mark; `false`
    /// for an unknown EHR.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn is_ehr_opted_out(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        opt_out::is_opted_out(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// The EHR that owns `vo_id` when that EHR carries the EHDS Art. 10
    /// opt-out mark; `None` for an unknown object, a party object, or an EHR
    /// with no opt-out.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn opted_out_owner(&self, vo_id: VoId) -> Result<Option<EhrId>, ServiceError> {
        let owner = crate::storage::version_repo::meta::vo_owner(&self.pool, vo_id)
            .await
            .map_err(ServiceError::Storage)?
            .flatten();
        match owner {
            Some(ehr_id) if self.is_ehr_opted_out(ehr_id).await? => Ok(Some(ehr_id)),
            _ => Ok(None),
        }
    }

    /// Whether the EHR `ehr_id` reads as absent to the current request, under
    /// the EHDS Art. 8 restriction or the Art. 10 opt-out it enforces; `false`
    /// for an unknown EHR.
    ///
    /// # Errors
    /// [`ServiceError::Storage`] or a pool fault when the marks cannot be read.
    pub(crate) async fn ehr_hidden_from_request(
        &self,
        ehr_id: EhrId,
    ) -> Result<bool, ServiceError> {
        use crate::system_log::access_context::{
            access_opt_out_enforced, access_restriction_enforced,
        };
        let (restricted, opted_out) = (access_restriction_enforced(), access_opt_out_enforced());
        if !restricted && !opted_out {
            return Ok(false);
        }
        let mut conn = acquire(&self.pool).await?;
        Ok(opt_out::ehr_hidden(&mut conn, ehr_id, restricted, opted_out).await?)
    }

    /// Record that an access-service caller reached the opted-out EHR `ehr_id`
    /// under the EHDS Art. 10(2) override.
    ///
    /// The record carries the declared purpose, which gives it the emergency
    /// mark, beside the access record of the request itself.
    ///
    /// # Errors
    /// `service_overloaded` — the access record was refused under
    /// `[audit] fail_mode = "closed"`.
    pub fn record_access_opt_out_override(&self, ehr_id: EhrId) -> Result<(), ServiceError> {
        let register = format!("{REGISTER}-override");
        self.emit_mark_access(EventActionCode::Execute, ehr_id, None, &register)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    #[test]
    fn the_access_service_roles_are_off_by_default_and_map_from_the_environment() {
        let defaults = crate::config::FerroEhrConfig::default();
        assert_eq!(
            defaults.authz.rbac.access_service_roles,
            Vec::<String>::new()
        );
        let env: HashMap<String, String> = [(
            "FERROEHR__AUTHZ__RBAC__ACCESS_SERVICE_ROLES",
            "EHDS-PORTAL,EHDS-PROFESSIONAL",
        )]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let config = crate::config::assemble(None, &env, &[]).expect("the config assembles");
        assert_eq!(
            config.authz.rbac.access_service_roles,
            vec!["EHDS-PORTAL".to_owned(), "EHDS-PROFESSIONAL".to_owned()]
        );
    }
}
