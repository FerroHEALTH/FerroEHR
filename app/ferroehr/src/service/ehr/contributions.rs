// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_EHR_CONTRIBUTION` (`i_ehr_contribution.adoc`) — the explicit
//! CONTRIBUTION-level surface: has/get/commit/list/count, plus the ITS-REST
//! raw-wire commit seam (`create_ehr_contribution`).
//!
//! Spec: RM common `master06-change_control_package.adoc` §Contributions /
//! §Committal and Audits. The change-set engine itself (classify, atomic
//! multi-version commit, retrieval assembly) is versioning law and lives in
//! [`crate::versioning`] (`commit_version_set` / `get_contribution` /
//! `list_contributions` / `count_contributions`); this file keeps only the
//! `I_EHR_CONTRIBUTION` surface and the wire glue. The `Pre_has_ehr` guard
//! is enforced inside `commit_version_set` via the
//! [`crate::versioning::CommitEnv`] `ensure_ehr_exists` hook.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::EhrId;
use crate::service::list::Page;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::SmError;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use openehr_its::rest::generated::ehr::Versionable;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::versioning::contribution::{
    commit_version_set, count_contributions, get_contribution, list_contributions,
};

use super::parse_time_range;

impl FerroEhrService {
    /// `POST /ehr/{ehr_id}/contribution` — commit a raw wire CONTRIBUTION body
    /// atomically and return the stored `CONTRIBUTION` with its resource
    /// metadata (the `contribution_uid` for the `201` `ETag`/`Location`).
    ///
    /// The body is the raw wire CONTRIBUTION, read member by member in
    /// `crate::versioning::contribution::commit_version_set`, where all RM
    /// `change_control` semantics stay.
    ///
    /// # Errors
    /// [`SmError`] if the CONTRIBUTION fails classification, content
    /// validation, the optimistic lock, or its storage commit.
    pub async fn create_ehr_contribution(
        &self,
        ehr_id: EhrId,
        body: Value,
    ) -> Result<ServiceResponse, SmError> {
        // TODO(#3595): decode the body through the generated `NewContribution`;
        // the raw read still accepts the SM `Terminology_code` and plain-string
        // spellings, an omitted member `change_type` and an `ATTESTATION` member
        // `_type`, all of which the generated envelope refuses.
        let committed = commit_version_set(self, Some(ehr_id), &body, false).await?;
        let body = self.ehr_contribution(ehr_id, committed.id, false).await?;
        let meta = ResourceMeta::new(ehr_id.to_string(), committed.id.to_string())
            .with_last_modified(committed.time_committed)
            .with_content(written_content(&committed.versions));
        Ok(ServiceResponse::new(body, meta))
    }

    /// Retrieve a CONTRIBUTION by id (scoped to the EHR), its `versions` as
    /// `OBJECT_REF`s, or — with `resolve_refs` — as the resolved VERSION
    /// objects (`ORIGINAL_VERSION`, or `IMPORTED_VERSION` for a version this
    /// repository received from another system) (ITS-REST `Prefer: resolve_refs`,
    /// `Requests_and_responses` §Representation details negotiation).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the CONTRIBUTION does not exist in this
    /// EHR; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn ehr_contribution(
        &self,
        ehr_id: EhrId,
        contribution_id: Uuid,
        resolve_refs: bool,
    ) -> Result<Value, ServiceError> {
        get_contribution(
            &self.pool,
            self.signer(),
            self.spec_profile,
            ehr_id,
            contribution_id,
            resolve_refs,
        )
        .await
    }
}

// ── The SM I_EHR_CONTRIBUTION call surface ────────────────────────────────────

impl FerroEhrService {
    /// SM `I_EHR_CONTRIBUTION.has_contribution` — whether the CONTRIBUTION
    /// exists in the EHR.
    ///
    /// # Errors
    /// [`SmError`] if the retrieval fails (a missing CONTRIBUTION is
    /// `Ok(false)`).
    pub async fn has_contribution(
        &self,
        an_ehr_id: EhrId,
        a_contrib_id: Uuid,
    ) -> Result<bool, SmError> {
        match self.ehr_contribution(an_ehr_id, a_contrib_id, false).await {
            Ok(_) => Ok(true),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// SM `I_EHR_CONTRIBUTION.get_contribution` — the stored `CONTRIBUTION`
    /// (its `versions` as `OBJECT_REF`s).
    ///
    /// # Errors
    /// [`SmError`] when the CONTRIBUTION does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn get_contribution(
        &self,
        an_ehr_id: EhrId,
        a_contrib_id: Uuid,
    ) -> Result<Value, SmError> {
        Ok(self
            .ehr_contribution(an_ehr_id, a_contrib_id, false)
            .await?)
    }

    /// The stored `CONTRIBUTION` with its `versions` resolved to the full
    /// VERSION objects — `ORIGINAL_VERSION`, or `IMPORTED_VERSION` for a
    /// version received from another system (ITS-REST `Prefer: resolve_refs`).
    ///
    /// # Errors
    /// [`SmError`] when the CONTRIBUTION does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn get_contribution_resolved(
        &self,
        an_ehr_id: EhrId,
        a_contrib_id: Uuid,
    ) -> Result<Value, SmError> {
        Ok(self.ehr_contribution(an_ehr_id, a_contrib_id, true).await?)
    }

    /// SM `I_EHR_CONTRIBUTION.commit_contribution` — commit a typed change
    /// set atomically, returning the new CONTRIBUTION uid.
    ///
    /// # Errors
    /// [`SmError`] when the typed shapes fail to serialize, or the commit
    /// fails classification, content validation, the optimistic lock, or its
    /// storage write.
    pub async fn commit_contribution(
        &self,
        an_ehr_id: EhrId,
        versions: Vec<UpdateVersion<Versionable>>,
        an_audit: UpdateAudit,
    ) -> Result<String, SmError> {
        // Reassemble the wire CONTRIBUTION body `commit_version_set` parses:
        // `{ versions: [ { commit_audit, data, preceding_version_uid,
        // lifecycle_state, attestations, signature } … ], audit }`. The typed
        // shapes serialize to exactly those field names.
        //
        // NOTE: no openEHR spec governs this internal seam — our own design:
        // the typed → wire-JSON bridge routes this SM-only caller through the
        // one change-set engine (`commit_version_set`) rather than forking it.
        let versions_json = openehr_its::json::to_canonical_value(&versions);
        let audit_json = openehr_its::json::to_canonical_value(&an_audit);
        let body = json!({ "versions": versions_json, "audit": audit_json });
        let committed = commit_version_set(self, Some(an_ehr_id), &body, false).await?;
        Ok(committed.id.to_string())
    }

    /// SM `I_EHR_CONTRIBUTION.list_contributions` — the EHR's CONTRIBUTION
    /// uids, optionally bounded by `time_range`, paged.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `time_range` bound (400-equivalent), a
    /// missing EHR (`Pre_has_ehr`, enforced inside the versioning read), or a
    /// read failure.
    pub async fn list_contributions(
        &self,
        an_ehr_id: EhrId,
        time_range: crate::service::ehr::handle::TimeRange,
        page: Page,
    ) -> Result<Vec<String>, SmError> {
        let time_range = parse_time_range(time_range)?;
        // `Pre_has_ehr` (`i_ehr_contribution.adoc` §list_contributions —
        // `ehr_does_not_exist`) is enforced inside the versioning read.
        let ids = list_contributions(&self.pool, an_ehr_id, time_range, page).await?;
        Ok(ids.iter().map(Uuid::to_string).collect())
    }

    /// The EHR contribution-list extension (`GET /ehr/{ehr_id}/contribution`,
    /// no uid): the EHR's CONTRIBUTIONs newest-first, paged, as
    /// `{ "rows": [ { uid, time_committed, committer, change_type,
    /// change_type_rubric } ], "total" }`.
    ///
    /// NOTE: no openEHR spec governs this listing — our own design/extension;
    /// the ITS-REST contract defines only the by-uid CONTRIBUTION GET.
    ///
    /// `committer` is the audit committer `PARTY_PROXY`'s `name`, a summary
    /// string rather than the full rendering the by-uid GET returns;
    /// `change_type` is the stored `audit.change_type` code and
    /// `change_type_rubric` its display rubric from the `audit_change_type`
    /// group, so consumers never map codes locally. `offset` and `fetch` are
    /// already clamped by the protocol adapter.
    ///
    /// # Errors
    /// [`SmError`] — `Pre_has_ehr` fails (unknown EHR → 404), or a read fails.
    pub async fn ehr_contribution_list_page(
        &self,
        an_ehr_id: EhrId,
        offset: i64,
        fetch: i64,
    ) -> Result<Value, SmError> {
        // Existence (`Pre_has_ehr` → 404) + the unwindowed total in one call.
        let total = count_contributions(&self.pool, an_ehr_id, None).await?;
        let rows = crate::storage::version_repo::contribution::list_contribution_summaries(
            &self.pool, an_ehr_id, offset, fetch,
        )
        .await?;
        let rows: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                json!({
                    "uid": r.uid.to_string(),
                    "time_committed": r.time_committed.to_string(),
                    "committer": r.committer,
                    "change_type_rubric":
                        crate::versioning::audit::change_type_rubric(&r.change_type),
                    "change_type": r.change_type,
                })
            })
            .collect();
        Ok(json!({ "rows": rows, "total": total }))
    }

    /// SM `I_EHR_CONTRIBUTION.contribution_count` — the number of
    /// CONTRIBUTIONs in the EHR, optionally bounded by `time_range`.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `time_range` bound (400-equivalent) or a
    /// read failure.
    pub async fn contribution_count(
        &self,
        an_ehr_id: EhrId,
        time_range: crate::service::ehr::handle::TimeRange,
    ) -> Result<i64, SmError> {
        let time_range = parse_time_range(time_range)?;
        Ok(count_contributions(&self.pool, an_ehr_id, time_range).await?)
    }

    /// The `POST /ehr/{ehr_id}/contribution` commit with the wire `Prefer`
    /// split: `return=representation` assembles the stored CONTRIBUTION body;
    /// `return=minimal` commits and returns just the metadata.
    ///
    /// # Errors
    /// [`SmError`] if the CONTRIBUTION fails classification, content
    /// validation, the optimistic lock, or its storage commit.
    ///
    /// Both branches carry `Last-Modified` beside the `ETag` and `Location` uid,
    /// a CONTRIBUTION being one of the resources ITS-REST
    /// `Requests_and_responses.md` §"`ETag` and `Last-Modified`" names. The value
    /// is the commit audit's `time_committed` as stored (RM common master06
    /// §Committal and Audits), carried out of the commit rather than re-read.
    pub async fn ehr_contribution_commit(
        &self,
        an_ehr_id: EhrId,
        a_contribution: Value,
        representation: bool,
    ) -> Result<ServiceResponse, SmError> {
        if representation {
            // `return=representation`: assemble the stored CONTRIBUTION body
            // (audit + version OBJECT_REFs) for the response.
            self.create_ehr_contribution(an_ehr_id, a_contribution)
                .await
        } else {
            // `return=minimal`: the response is headers-only, so commit and
            // return just the contribution uid (the `201` `ETag`/`Location`) —
            // the post-commit composite re-read the representation path pays is
            // pure waste here (RM common master06 §Committal and Audits — the
            // commit itself yields the new CONTRIBUTION id).
            let committed =
                commit_version_set(self, Some(an_ehr_id), &a_contribution, false).await?;
            let meta = ResourceMeta::new(an_ehr_id.to_string(), committed.id.to_string())
                .with_last_modified(committed.time_committed)
                .with_content(written_content(&committed.versions));
            Ok(ServiceResponse::new(Value::Null, meta))
        }
    }
}

/// The identifiers of the versions a CONTRIBUTION wrote, for the access
/// record's category classification (EHDS Annex II 3.2(c), #3621).
fn written_content(
    versions: &[crate::versioning::change::Committed],
) -> Vec<crate::system_log::categories::ContentIds> {
    let mut ids: Vec<_> = versions
        .iter()
        .filter_map(crate::versioning::change::Committed::content_ids)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}
