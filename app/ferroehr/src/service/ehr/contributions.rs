// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The explicit CONTRIBUTION-level surface: get, commit and the paged list,
//! plus the ITS-REST raw-wire commit seam (`create_ehr_contribution`).
//!
//! Spec: RM common `master06-change_control_package.adoc` §Contributions /
//! §Committal and Audits. The change-set engine itself (classify, atomic
//! multi-version commit, retrieval assembly) is versioning law and lives in
//! [`crate::versioning`] (`commit_version_set` / `get_contribution` /
//! `count_contributions`); this file keeps only the
//! CONTRIBUTION surface and the wire glue. The EHR-existence guard
//! is enforced inside `commit_version_set` via the
//! [`crate::versioning::CommitEnv`] `ensure_ehr_exists` hook.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::EhrId;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::StatusError;
use openehr_its::rest::generated::ehr::NewContribution;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::versioning::contribution::{
    commit_version_set, count_contributions, decode_new_contribution, get_contribution,
};

impl FerroEhrService {
    /// `POST /ehr/{ehr_id}/contribution` — commit a raw wire CONTRIBUTION body
    /// atomically and return the stored `CONTRIBUTION` with its resource
    /// metadata (the `contribution_uid` for the `201` `ETag`/`Location`).
    ///
    /// The body is the wire CONTRIBUTION, decoded through the generated
    /// `NewContribution` (ITS-REST `schemas/ehr/NewContribution.yaml`) and then
    /// committed by `crate::versioning::contribution::commit_version_set`,
    /// where all RM `change_control` semantics stay.
    ///
    /// # Errors
    /// [`StatusError`] if the body is not a `NewContribution` (400), or the
    /// CONTRIBUTION fails classification, content validation, the optimistic
    /// lock, or its storage commit.
    pub async fn create_ehr_contribution(
        &self,
        ehr_id: EhrId,
        body: Value,
    ) -> Result<ServiceResponse, StatusError> {
        let committed = self.commit_wire_contribution(ehr_id, &body).await?;
        let body = self.ehr_contribution(ehr_id, committed.id, false).await?;
        let meta = ResourceMeta::new(ehr_id.to_string(), committed.id.to_string())
            .with_last_modified(committed.time_committed)
            .with_content(self.written_content(&committed.versions).await);
        Ok(ServiceResponse::new(body, meta))
    }

    /// Decode a wire CONTRIBUTION body through the generated `NewContribution`
    /// (ITS-REST `schemas/ehr/NewContribution.yaml`) and commit it.
    ///
    /// # Errors
    /// [`ServiceError::BadRequest`] when the body is not a `NewContribution`;
    /// the `commit_version_set` refusals.
    async fn commit_wire_contribution(
        &self,
        ehr_id: EhrId,
        body: &Value,
    ) -> Result<crate::versioning::change::CommittedContribution, ServiceError> {
        // NOTE: ITS-REST `schemas/ehr/NewContribution.yaml` leaves `additionalProperties` open
        // (`ItemTag.yaml` closes its object with `false`), so an undeclared member is ignored.
        let NewContribution {
            uid,
            versions,
            audit,
            additional_properties: _,
        } = decode_new_contribution::<NewContribution<Value>>(body)?;
        commit_version_set(self, Some(ehr_id), uid.as_ref(), versions, &audit, false).await
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

// ── The CONTRIBUTION call surface ─────────────────────────────────────────────

impl FerroEhrService {
    /// The stored `CONTRIBUTION`
    /// (its `versions` as `OBJECT_REF`s).
    ///
    /// # Errors
    /// [`StatusError`] when the CONTRIBUTION does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn get_contribution(
        &self,
        an_ehr_id: EhrId,
        a_contrib_id: Uuid,
    ) -> Result<Value, StatusError> {
        Ok(self
            .ehr_contribution(an_ehr_id, a_contrib_id, false)
            .await?)
    }

    /// The stored `CONTRIBUTION` with its `versions` resolved to the full
    /// VERSION objects — `ORIGINAL_VERSION`, or `IMPORTED_VERSION` for a
    /// version received from another system (ITS-REST `Prefer: resolve_refs`).
    ///
    /// # Errors
    /// [`StatusError`] when the CONTRIBUTION does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn get_contribution_resolved(
        &self,
        an_ehr_id: EhrId,
        a_contrib_id: Uuid,
    ) -> Result<Value, StatusError> {
        Ok(self.ehr_contribution(an_ehr_id, a_contrib_id, true).await?)
    }

    /// The EHR contribution-list extension (`GET /ehr/{ehr_id}/contribution`,
    /// no uid): the EHR's CONTRIBUTIONs newest-first, paged, as
    /// `{ "rows": [ { uid, time_committed, committer, inserted_by_person,
    /// change_type, change_type_rubric } ], "total" }`.
    ///
    /// NOTE: no openEHR spec governs this listing — our own design/extension;
    /// the ITS-REST contract defines only the by-uid CONTRIBUTION GET.
    ///
    /// `committer` is the audit committer `PARTY_PROXY`'s `name`, a summary
    /// string rather than the full rendering the by-uid GET returns, and
    /// `inserted_by_person` whether that committer is `PARTY_SELF`, the
    /// record's person (EHDS Art. 5);
    /// `change_type` is the stored `audit.change_type` code and
    /// `change_type_rubric` its display rubric from the `audit_change_type`
    /// group, so consumers never map codes locally. `offset` and `fetch` are
    /// already clamped by the protocol adapter.
    ///
    /// # Errors
    /// [`StatusError`] — the EHR-existence check fails (unknown EHR → 404), or a read fails.
    pub async fn ehr_contribution_list_page(
        &self,
        an_ehr_id: EhrId,
        offset: i64,
        fetch: i64,
    ) -> Result<Value, StatusError> {
        // Existence (the EHR-existence check → 404) + the unwindowed total in one call.
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
                    "inserted_by_person": r.person,
                    "change_type_rubric":
                        crate::versioning::audit::change_type_rubric(&r.change_type),
                    "change_type": r.change_type,
                })
            })
            .collect();
        Ok(json!({ "rows": rows, "total": total }))
    }

    /// The `POST /ehr/{ehr_id}/contribution` commit with the wire `Prefer`
    /// split: `return=representation` assembles the stored CONTRIBUTION body;
    /// `return=minimal` commits and returns just the metadata.
    ///
    /// # Errors
    /// [`StatusError`] if the CONTRIBUTION fails classification, content
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
    ) -> Result<ServiceResponse, StatusError> {
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
            let committed = self
                .commit_wire_contribution(an_ehr_id, &a_contribution)
                .await?;
            let meta = ResourceMeta::new(an_ehr_id.to_string(), committed.id.to_string())
                .with_last_modified(committed.time_committed)
                .with_content(self.written_content(&committed.versions).await);
            Ok(ServiceResponse::new(Value::Null, meta))
        }
    }
}

impl FerroEhrService {
    /// The identifiers of the versions a CONTRIBUTION wrote, for the access
    /// record's category classification (EHDS Annex II 3.2(c), #3621).
    ///
    /// A logical delete writes no content, so it is classified by the newest
    /// version of its object that carries a body, as the single-object delete
    /// is. The commit has landed, so a failed read leaves the deleted objects
    /// unclassified rather than failing the response.
    async fn written_content(
        &self,
        versions: &[crate::versioning::change::Committed],
    ) -> Vec<crate::system_log::categories::ContentIds> {
        let (deleted, written): (Vec<_>, Vec<_>) = versions
            .iter()
            .partition(|v| v.change_type == crate::versioning::audit::change_type::DELETED);
        let mut ids: Vec<_> = written
            .into_iter()
            .filter_map(crate::versioning::change::Committed::content_ids)
            .collect();
        if !deleted.is_empty() {
            let vo_ids: Vec<_> = deleted.iter().map(|v| v.vo_id).collect();
            match self.audit_content_of_objects(&vo_ids).await {
                Ok(read) => ids.extend(read),
                Err(e) => tracing::warn!(
                    error = %crate::service::error::ErrorChain::new(&e),
                    "reading the categories of the objects a CONTRIBUTION deleted failed; the \
                     access record leaves them unclassified"
                ),
            }
        }
        ids.sort();
        ids.dedup();
        ids
    }
}
