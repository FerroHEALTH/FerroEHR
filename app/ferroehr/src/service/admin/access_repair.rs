// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's commit and read of an EHR's `EHR_ACCESS` settings, and the
//! repair of an EHR held closed by settings that do not parse.
//!
//! The controller commits the person's instructions, the EHDS Art. 8
//! restriction and the Art. 10 opt-out among them, as a new `EHR_ACCESS`
//! version over the one it read (`docs/law/eu/ehds/text.html`): the settings
//! hold them as `restriction` and `opt_out` directives, and the commit
//! projects them onto the marks the read paths filter on
//! (`service::ehr::access_marks`). Its read returns every version, so
//! the history of the person's choices is the version history (BASE
//! Architecture Overview §Security and Confidentiality, "Audit trailing").
//!
//! An EHR whose `access_directives` settings cannot be read is held closed:
//! every path route refuses it and every query leaves it out
//! ([`crate::service::ehr::access_settings::EhrAccessSettings::HeldClosed`]). The
//! CONTRIBUTION that would replace the settings names the EHR and is refused
//! too, so the repair is the one way back. Each act commits a modification
//! with its own commit audit (RM common `master06-change_control_package.adoc`
//! §Committal and Audits) and writes an access record. No openEHR
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
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::ehr::access_settings::{EhrAccessSettings, check_written_settings};
use crate::service::response::ResourceMeta;
use crate::service::write::Mode;
use crate::storage::version_repo::read::VersionAt;
use crate::system_log::event::EventActionCode;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, Committed, Write, WriteEnvelope};
use crate::versioning::object_version_id::{TreeId, object_version_id};
use crate::versioning::read::VersionRead;

/// The name the access record gives the act.
const ACT: &str = "ehr-access-repair";

/// The name the access record gives the controller's commit and read of the
/// settings.
const SETTINGS: &str = "ehr-access";

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
        let (vo_id, tip, current) = self.current_ehr_access(ehr_id).await?;
        if EhrAccessSettings::from_ehr_access(&current.canonical).is_ok() {
            return Err(ServiceError::conflict(format!(
                "the EHR_ACCESS settings of EHR {ehr_id} parse, so the EHR is not held; \
                 change them with a CONTRIBUTION"
            )));
        }
        let committed = self
            .replace_ehr_access_settings(
                ehr_id,
                (vo_id, tip, current.canonical),
                settings,
                "EHR_ACCESS settings repair",
            )
            .await?;
        self.emit_mark_access(EventActionCode::Update, ehr_id, Some(vo_id), ACT)?;
        Ok(self.version_meta(
            ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        ))
    }

    /// Commit the controller's change of an EHR's `EHR_ACCESS` settings, on the
    /// person's instruction, as a new version over `preceding_version_uid`.
    ///
    /// This is the controller's route for the EHDS Art. 8 restriction and the
    /// Art. 10 opt-out, which are `restriction` and `opt_out` directives of
    /// the settings; the commit projects them onto the marks the read paths
    /// filter on (`service::ehr::access_marks`). `settings` is the
    /// whole new `EHR_ACCESS.settings` object, or `null` for none.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown or has no
    ///   `EHR_ACCESS`.
    /// - `bad_request` (`400`) — `settings` are not settings of the
    ///   `access_directives` scheme whose every invariant holds, or hold a
    ///   `restriction` or `opt_out` directive the projection refuses
    ///   (`service::ehr::access_marks::refused`).
    /// - `version_conflict` (`412`) — `preceding_version_uid` is not the
    ///   current version, or the `EHR_ACCESS` moved on while the commit ran.
    /// - `exception` — a database fault (rolled back).
    /// - `service_overloaded` — the access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn commit_ehr_access_settings(
        &self,
        ehr_id: EhrId,
        preceding_version_uid: &str,
        settings: Value,
    ) -> Result<ResourceMeta, ServiceError> {
        let (vo_id, tip, current) = self.current_ehr_access(ehr_id).await?;
        let current_uid = object_version_id(vo_id, &current.creating_system_id, tip);
        if preceding_version_uid != current_uid {
            return Err(ServiceError::version_conflict(format!(
                "the current EHR_ACCESS version of EHR {ehr_id} is {current_uid}, not \
                 {preceding_version_uid}"
            )));
        }
        let committed = self
            .replace_ehr_access_settings(
                ehr_id,
                (vo_id, tip, current.canonical),
                settings,
                "EHR_ACCESS settings committed on the person's instruction",
            )
            .await?;
        self.emit_mark_access(EventActionCode::Update, ehr_id, None, SETTINGS)?;
        Ok(self.version_meta(
            ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        ))
    }

    /// Every version of an EHR's `EHR_ACCESS`, newest first, each as its
    /// version uid and its settings (`null` for none or a logical deletion):
    /// the controller's read of the person's choices and their history.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown or has no
    ///   `EHR_ACCESS`.
    /// - `exception` — a database fault.
    /// - `service_overloaded` — the access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn ehr_access_versions(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<(String, Value)>, ServiceError> {
        let (vo_id, _, _) = self.current_ehr_access(ehr_id).await?;
        let mut versions = Vec::new();
        for ordinal in 1.. {
            let Some(read) = crate::versioning::read::read(
                &self.pool,
                self.spec_profile,
                VersionAt::Ordinal(vo_id, ordinal),
            )
            .await?
            else {
                break;
            };
            let settings = read
                .canonical
                .get("settings")
                .cloned()
                .unwrap_or(Value::Null);
            versions.push((
                object_version_id(vo_id, &read.creating_system_id, read.tree),
                settings,
            ));
        }
        versions.reverse();
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, SETTINGS)?;
        Ok(versions)
    }

    /// The `EHR_ACCESS` of `ehr_id`: its object, its current version and that
    /// version read.
    pub(in crate::service) async fn current_ehr_access(
        &self,
        ehr_id: EhrId,
    ) -> Result<(VoId, TreeId, VersionRead), ServiceError> {
        let missing = || {
            ServiceError::not_found(
                Missing::Ehr,
                format!("no EHR with id {ehr_id} holds an EHR_ACCESS"),
            )
        };
        let (vo_id, tip) = self
            .current_vo(ehr_id, Kind::EhrAccess)
            .await?
            .ok_or_else(missing)?;
        let current =
            crate::versioning::read::read(&self.pool, self.spec_profile, VersionAt::Current(vo_id))
                .await?
                .ok_or_else(missing)?;
        Ok((vo_id, tip, current))
    }

    /// Commit `settings` in place of the current settings of the `EHR_ACCESS`
    /// `current`, as a modification over its version `tip`, and drop the
    /// cached settings.
    async fn replace_ehr_access_settings(
        &self,
        ehr_id: EhrId,
        (vo_id, tip, mut access): (VoId, TreeId, Value),
        settings: Value,
        description: &str,
    ) -> Result<Committed, ServiceError> {
        if !settings.is_null() {
            check_written_settings(&settings)?;
        }
        let Some(object) = access.as_object_mut() else {
            return Err(ServiceError::exception(format!(
                "the stored EHR_ACCESS of EHR {ehr_id} is not a JSON object"
            )));
        };
        if settings.is_null() {
            object.remove("settings");
        } else {
            object.insert("settings".to_owned(), settings);
        }
        self.validate_for_commit(Kind::EhrAccess, &access, false)
            .await?;
        let audit = self.audit(change_type::MODIFICATION, description);
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
        Ok(committed)
    }
}
