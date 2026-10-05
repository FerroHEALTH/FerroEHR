// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The service-side realization of the versioning commit environment.
//!
//! The CONTRIBUTION commit orchestration
//! ([`crate::versioning::contribution::commit_version_set`]) needs cross-area
//! hooks it cannot own itself: content validation, the EHR-existence +
//! `is_modifiable` guards, the EHR-singleton lookup, and `EHR_ACCESS` cache
//! invalidation — each realized by its owning service chapter. This impl is
//! the one place those chapter seams are wired into
//! [`crate::versioning::CommitEnv`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): commit-environment fragments are the canonical \
              form the commit interior carries (stored-content class)"
)]

use serde_json::Value;
use sqlx::PgPool;

use super::FerroEhrService;
use super::ehr;
use super::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::versioning::{CommitEnv, Kind, SigningCtx};

#[async_trait::async_trait]
impl CommitEnv for FerroEhrService {
    fn pool(&self) -> &PgPool {
        &self.pool
    }

    fn demographic_pool(&self) -> &PgPool {
        &self.demographic_pool
    }

    fn effective_system_id(&self) -> String {
        FerroEhrService::effective_system_id(self)
    }

    fn default_committer(&self) -> openehr_rm::prelude::PartyProxy {
        ehr::meta::committer()
    }

    fn signing_ctx(&self) -> SigningCtx<'_> {
        FerroEhrService::signing_ctx(self)
    }

    async fn validate_for_commit(
        &self,
        kind: Kind,
        data: &Value,
        incomplete: bool,
    ) -> Result<(), ServiceError> {
        FerroEhrService::validate_for_commit(self, kind, data, incomplete).await
    }

    async fn check_folder_item_refs(
        &self,
        tx: &mut sqlx::PgConnection,
        ehr_id: EhrId,
        folder: &Value,
    ) -> Result<(), ServiceError> {
        ehr::validation::check_folder_item_refs(
            &mut *tx,
            ehr_id,
            &FerroEhrService::effective_system_id(self),
            folder,
        )
        .await
    }

    async fn ensure_ehr_exists(&self, ehr_id: EhrId) -> Result<(), ServiceError> {
        FerroEhrService::ensure_ehr_exists(self, ehr_id).await
    }

    async fn current_vo(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<(VoId, i32)>, ServiceError> {
        Ok(FerroEhrService::current_vo(self, ehr_id, kind)
            .await?
            .map(|(vo_id, tree)| (vo_id, tree.trunk)))
    }

    async fn invalidate_ehr_access(&self, ehr_id: EhrId) {
        FerroEhrService::invalidate_ehr_access(self, ehr_id).await;
    }

    async fn folder_root_exists(
        &self,
        ehr_id: EhrId,
        archetype_node_id: &str,
        name: &str,
    ) -> Result<bool, ServiceError> {
        Ok(crate::storage::ehr_repo::live_folder_root_exists(
            &self.pool,
            ehr_id,
            archetype_node_id,
            name,
        )
        .await?)
    }

    async fn pre_composition_modify(
        &self,
        tx: &mut sqlx::PgConnection,
        vo_id: VoId,
        canonical: &Value,
    ) -> Result<(), ServiceError> {
        ehr::validation::check_versioned_composition_invariants(tx, vo_id, canonical).await
    }

    async fn post_status_commit(
        &self,
        tx: &mut sqlx::PgConnection,
        ehr_id: EhrId,
        status: &Value,
    ) -> Result<(), ServiceError> {
        self.sync_ehr_subject(tx, ehr_id, status).await
    }
}
