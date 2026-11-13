// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The repair of an EHR held closed by `EHR_ACCESS` settings that do not
//! parse.
//!
//! An EHR whose `access_directives` settings cannot be read is held closed:
//! every path route refuses it and every query leaves it out
//! ([`crate::service::ehr::access_settings::EhrAccessSettings::HeldClosed`]). The
//! CONTRIBUTION that would replace the settings names the EHR and is refused
//! too, so this administrator's act is the one way back: it commits a new
//! `EHR_ACCESS` version carrying settings that parse, as a modification with
//! its own commit audit (RM common `master06-change_control_package.adoc`
//! §Committal and Audits), and writes an access record. No openEHR
//! specification governs FerroEHR's access-control scheme: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use serde_json::Value;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr::access_settings::{EhrAccessSettings, check_written_settings};
use crate::service::response::ResourceMeta;
use crate::service::write::Mode;
use crate::storage::version_repo::read::VersionAt;
use crate::system_log::event::EventActionCode;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, Write, WriteEnvelope};

/// The name the access record gives the act.
const ACT: &str = "ehr-access-repair";

impl FerroEhrService {
    /// Replace the `EHR_ACCESS` settings of an EHR held closed by settings that
    /// do not parse, committing the new `EHR_ACCESS` version as a modification.
    ///
    /// `settings` is the whole new `EHR_ACCESS.settings` object; every other
    /// attribute of the current `EHR_ACCESS` is kept. The commit runs the
    /// `EHR_ACCESS` commit validation, writes its contribution and commit
    /// audit, drops the cached settings, and records the act as an access.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown or has no
    ///   `EHR_ACCESS`.
    /// - `conflict` (`409`) — the EHR's settings parse, so it is not held; the
    ///   ordinary CONTRIBUTION changes them.
    /// - `bad_request` (`400`) — `settings` are not settings of the
    ///   `access_directives` scheme whose every invariant holds, naming the
    ///   first defect.
    /// - `version_conflict` — the `EHR_ACCESS` moved on while the repair ran.
    /// - `exception` — a database fault (rolled back).
    /// - `service_overloaded` — the access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn repair_ehr_access_settings(
        &self,
        ehr_id: EhrId,
        settings: Value,
    ) -> Result<ResourceMeta, ServiceError> {
        let Some((vo_id, tip)) = self.current_vo(ehr_id, Kind::EhrAccess).await? else {
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("no EHR with id {ehr_id} holds an EHR_ACCESS"),
            ));
        };
        let Some(current) =
            crate::versioning::read::read(&self.pool, self.spec_profile, VersionAt::Current(vo_id))
                .await?
        else {
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("no EHR with id {ehr_id} holds an EHR_ACCESS"),
            ));
        };
        if EhrAccessSettings::from_ehr_access(&current.canonical).is_ok() {
            return Err(ServiceError::conflict(format!(
                "the EHR_ACCESS settings of EHR {ehr_id} parse, so the EHR is not held; \
                 change them with a CONTRIBUTION"
            )));
        }
        check_written_settings(&settings)?;
        let mut access = current.canonical;
        let Some(object) = access.as_object_mut() else {
            return Err(ServiceError::exception(format!(
                "the stored EHR_ACCESS of EHR {ehr_id} is not a JSON object"
            )));
        };
        object.insert("settings".to_owned(), settings);
        self.validate_for_commit(Kind::EhrAccess, &access, false)
            .await?;
        let audit = self.audit(change_type::MODIFICATION, "EHR_ACCESS settings repair");
        let committed = self
            .commit_one(&self.pool, Mode::Transaction, async move |_tx| {
                Ok(Write::new(
                    Some(ehr_id),
                    Change::modify(
                        vo_id,
                        Kind::EhrAccess,
                        access,
                        Some(tip),
                        None,
                        WriteEnvelope::default(),
                    ),
                    audit,
                ))
            })
            .await
            .map_err(ServiceError::flattened)?;
        self.invalidate_ehr_access(ehr_id).await;
        self.emit_mark_access(EventActionCode::Update, ehr_id, Some(vo_id), ACT)?;
        Ok(self.version_meta(
            ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        ))
    }
}
