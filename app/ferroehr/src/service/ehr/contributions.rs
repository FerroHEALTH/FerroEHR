// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The explicit CONTRIBUTION-level surface: get, commit and the paged list,
//! plus the ITS-REST raw-wire commit seam (`create_ehr_contribution`).
//!
//! Spec: RM common `master06-change_control_package.adoc` §Contributions /
//! §Committal and Audits. The version set is parsed in
//! [`crate::versioning::contribution`], which also assembles a stored one
//! (`get_contribution`), and committed by [`crate::service::contribution`]
//! (`commit_version_set`); this file keeps only the
//! CONTRIBUTION surface and the wire glue. The EHR-existence guard
//! is enforced inside `commit_version_set`, which calls
//! `storage::ehr_repo::require_ehr`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::EhrId;
use crate::service::ehr::access_view;
use crate::service::response::{ContributionRead, ResourceMeta, ServiceResponse};
use openehr_its::rest::generated::ehr::NewContribution;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::service::FerroEhrService;
use crate::service::contribution::commit_version_set;
use crate::versioning::contribution::get_contribution;
use crate::versioning::envelope::{self, ContributionText, Parts};

impl FerroEhrService {
    /// `POST /ehr/{ehr_id}/contribution` — commit a raw wire CONTRIBUTION body
    /// atomically and return the stored `CONTRIBUTION` with its resource
    /// metadata (the `contribution_uid` for the `201` `ETag`/`Location`).
    ///
    /// The body is the wire CONTRIBUTION, decoded through the generated
    /// `NewContribution` (ITS-REST `schemas/ehr/NewContribution.yaml`) and then
    /// committed by `crate::service::contribution::commit_version_set`,
    /// where all RM `change_control` semantics stay.
    ///
    /// # Errors
    /// [`ServiceError`] if the body is not a `NewContribution` (400), or the
    /// CONTRIBUTION fails classification, content validation, the optimistic
    /// lock, or its storage commit.
    pub async fn create_ehr_contribution(
        &self,
        ehr_id: EhrId,
        body: impl Into<ContributionText>,
    ) -> Result<ServiceResponse, ServiceError> {
        let committed = self
            .commit_wire_contribution(ehr_id, &body.into())
            .await
            .map_err(ServiceError::flattened)?;
        let body = self
            .ehr_contribution(ehr_id, committed.id, false)
            .await?
            .body;
        let meta = ResourceMeta::new(ehr_id.to_string(), committed.id.to_string())
            .with_last_modified(committed.time_committed)
            .with_content(self.written_content(None, &committed.versions).await);
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
        body: &ContributionText,
    ) -> Result<crate::versioning::change::CommittedContribution, ServiceError> {
        let Parts {
            uid,
            versions,
            audit,
        } = envelope::read::<NewContribution<Box<RawValue>>, NewContribution<Value>>(body)?;
        commit_version_set(self, Some(ehr_id), uid.as_ref(), versions, &audit, false).await
    }

    /// The stored `CONTRIBUTION`
    /// (its `versions` as `OBJECT_REF`s).
    ///
    /// With `resolve_refs` the version references are resolved into the
    /// versions they name.
    ///
    /// A CONTRIBUTION that commits an `EHR_ACCESS` version is answered as an
    /// unknown one to a caller who may not read the EHR's settings
    /// ([`Self::ehr_access_view`]), since a restriction must not be visible
    /// to a provider (EHDS Art. 8); our own design. To a gate-keeper who is
    /// not the subject, a resolved `EHR_ACCESS` version is served without
    /// its marking and its signature (#6165). Resolved for anyone but the
    /// controller, a CONTRIBUTION one of whose `EHR_ACCESS` versions holds
    /// the controller's exclusions is answered as unknown (#6077).
    ///
    /// # Errors
    /// [`ServiceError`] when the CONTRIBUTION does not exist in this EHR
    /// (404-equivalent) or a read fails.
    pub async fn ehr_contribution(
        &self,
        ehr_id: EhrId,
        contribution_id: Uuid,
        resolve_refs: bool,
    ) -> Result<ContributionRead, ServiceError> {
        let mut contribution = get_contribution(
            &self.pool,
            self.signer(),
            self.spec_profile,
            ehr_id,
            contribution_id,
            resolve_refs,
        )
        .await?;
        if !commits_ehr_access(&contribution) {
            return Ok(ContributionRead {
                body: contribution,
                redacted: false,
            });
        }
        // NOTE: draft access control decision procedure, Reading and Writing the Settings, the
        // signed versions of a CONTRIBUTION are never served altered, so exclusions hide it.
        let excluded = resolve_refs
            && !self.settings_caller().administrator()
            && carries_exclusions(&contribution);
        let view = Box::pin(self.ehr_access_view(ehr_id)).await?;
        if excluded || !view.reads() {
            return Err(ServiceError::not_found(
                Missing::Contribution,
                format!("CONTRIBUTION {contribution_id}"),
            ));
        }
        let redacted = view.redacts() && redact_access_versions(&mut contribution);
        Ok(ContributionRead {
            body: contribution,
            redacted,
        })
    }
}

// ── The CONTRIBUTION call surface ─────────────────────────────────────────────

impl FerroEhrService {
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
    /// `inserted_by_person` whether that committer is `PARTY_SELF` or
    /// `PARTY_RELATED`, the record's person or their representative (EHDS
    /// Art. 5);
    /// `change_type` is the stored `audit.change_type` code and
    /// `change_type_rubric` its display rubric from the `audit_change_type`
    /// group, so consumers never map codes locally. `offset` and `fetch` are
    /// already clamped by the protocol adapter.
    ///
    /// # Errors
    /// [`ServiceError`] — the EHR-existence check fails (unknown EHR → 404), or a read fails.
    pub async fn ehr_contribution_list_page(
        &self,
        an_ehr_id: EhrId,
        offset: i64,
        fetch: i64,
    ) -> Result<Value, ServiceError> {
        // Existence (the EHR-existence check → 404) + the unwindowed total in one call.
        crate::storage::ehr_repo::require_ehr(&mut *acquire(&self.pool).await?, an_ehr_id).await?;
        let total = crate::storage::version_repo::contribution::count_contributions(
            &self.pool, an_ehr_id, None, None,
        )
        .await?;
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
    /// [`ServiceError`] if the CONTRIBUTION fails classification, content
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
        a_contribution: impl Into<ContributionText>,
        representation: bool,
    ) -> Result<ServiceResponse, ServiceError> {
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
                .commit_wire_contribution(an_ehr_id, &a_contribution.into())
                .await
                .map_err(ServiceError::flattened)?;
            let meta = ResourceMeta::new(an_ehr_id.to_string(), committed.id.to_string())
                .with_last_modified(committed.time_committed)
                .with_content(self.written_content(None, &committed.versions).await);
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
    /// is. `tx` is the write's own transaction while it is open: the read
    /// runs on it, so the request asks the pool for no second connection
    /// while it holds one (#5879), and a failed read there fails the commit.
    /// After the commit (`None`), a failed read leaves the deleted objects
    /// unclassified rather than failing the response.
    pub(crate) async fn written_content(
        &self,
        tx: Option<&mut sqlx::PgConnection>,
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
            let read = match tx {
                Some(tx) => self.audit_content_of_objects_on(tx, &vo_ids).await,
                None => self.audit_content_of_objects(&vo_ids).await,
            };
            match read {
                Ok(read) => ids.extend(read),
                Err(e) => tracing::warn!(
                    error = %crate::error::ErrorChain::new(&e),
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

/// Whether a stored CONTRIBUTION lists an `EHR_ACCESS` version, as an
/// `OBJECT_REF` of that type or as a resolved version of that data.
fn commits_ehr_access(contribution: &Value) -> bool {
    contribution
        .get("versions")
        .and_then(Value::as_array)
        .is_some_and(|versions| {
            versions.iter().any(|version| {
                version.get("type").and_then(Value::as_str) == Some("EHR_ACCESS")
                    || version.pointer("/data/_type").and_then(Value::as_str) == Some("EHR_ACCESS")
            })
        })
}

/// Whether a resolved version of the CONTRIBUTION `contribution` is an
/// `EHR_ACCESS` whose settings carry the controller's exclusions.
fn carries_exclusions(contribution: &Value) -> bool {
    contribution
        .get("versions")
        .and_then(Value::as_array)
        .is_some_and(|versions| {
            versions.iter().any(|version| {
                version
                    .get("data")
                    .and_then(super::access_exclusions::held)
                    .is_some()
            })
        })
}

/// Redacts every resolved `EHR_ACCESS` version of `contribution`
/// ([`access_view::redact_version`]); whether it redacted one.
fn redact_access_versions(contribution: &mut Value) -> bool {
    let mut redacted = false;
    let versions = contribution
        .get_mut("versions")
        .and_then(Value::as_array_mut)
        .map_or(&mut [][..], Vec::as_mut_slice);
    for version in versions {
        if version.pointer("/data/_type").and_then(Value::as_str) == Some("EHR_ACCESS") {
            access_view::redact_version(version);
            redacted = true;
        }
    }
    redacted
}
