// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Physical deletion (`physical_ehr_delete` / `physical_party_delete` + the
//! `admin_ehr_delete_all` bulk extension).
//!
//! Spec: ITS-REST `operations/admin_ehr_delete.yaml`; an unknown EHR is
//! `ehr_id_does_not_exist`, and a party goes with its relationships, an unknown
//! one `party_id_does_not_exist`. Behaviour is anchored by the CNF Robot prior art
//! (`I_ADMIN_SERVICE/001-EHR.robot` + `admin_keywords.robot`): a physical,
//! cascading delete after which every backing table returns to its pre-EHR
//! baseline count. No openEHR spec governs the cascade SQL / FK graph — our own
//! design over the greenfield schema.

#![cfg_attr(
    feature = "multimedia",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/dump-load compose \
                  over verbatim stored content (RM common master06 §Copying); the Value sites \
                  are multimedia-gated, so the expectation exists only where it is fulfilled"
    )
)]

use sqlx::Connection as _;
use sqlx::PgConnection;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::storage::{erasure, identifier_repo, template_repo, version_repo};

impl FerroEhrService {
    /// Admin extension `DELETE /admin/template/{template_id}`: physically delete
    /// one operational template by its wire `template_id` (case-insensitive,
    /// §Composite Identifiers and Case), evicting its `WebTemplate` cache entry.
    ///
    /// NOTE: no openEHR spec governs this operation, the ITS-REST Admin API
    /// defining only EHR deletes — our own design/extension, addressed by the
    /// wire id.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — no template with that id.
    /// - `409` (`ServiceError::Conflict`) — the party mapping names the
    ///   template, or a `version` row still references the template (`version.template_id` FK, `clinical/0006_definitions.sql`);
    ///   a physical delete must never orphan the compositions built on it.
    /// - `exception` — a database fault.
    pub async fn admin_template_delete(&self, template_id: &str) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        // Resolve the stored (case-preserved) id; absent → 404 (§Composite
        // Identifiers and Case: compare case-insensitively).
        let Some(stored) = template_repo::stored_template_id(&mut tx, template_id).await? else {
            return Err(ServiceError::not_found(
                Missing::Template,
                format!("template {template_id}"),
            ));
        };
        // Counted over BOTH storage tiers: `version` is one relation
        // partitioned by tier, so this reaches an archived composition's
        // reference as well as a live one.
        if self.binding.names_template(&stored) {
            return Err(ServiceError::conflict(format!(
                "template '{stored}' is named by [authz.party_binding]; remove it from the party \
                 mapping before deleting it"
            )));
        }
        let refs = template_repo::template_use_count(&mut tx, &stored).await?;
        if refs > 0 {
            return Err(ServiceError::conflict(format!(
                "template '{stored}' is still referenced by {refs} committed version(s); \
                 delete those compositions before deleting the template"
            )));
        }
        template_repo::delete_template(&mut tx, &stored).await?;
        // Deregister the wire address unless a template-kind ADL2 artefact
        // also claims it (`template_ref` is the union of both dialects'
        // addresses; the FK blocks the deregistration if a concurrent commit
        // referenced it after the count above).
        template_repo::release_template_ref(&mut tx, &stored).await?;
        tx.commit().await?;
        // Evict the derived-runtime cache for the deleted id (case-canonical
        // key). No openEHR spec governs the cache.
        self.web_templates
            .invalidate(&crate::templates::identity::canonical_key(&stored))
            .await;
        Ok(())
    }

    /// Physically delete one EHR and every trace of it.
    ///
    /// The sequence, in order, is the erasure reach: one clinical transaction,
    /// then the object store.
    ///
    /// 1. `DELETE FROM ehr` cascades through `vo_head`, `version` (both tier
    ///    partitions), `node`, `vo_attestation`, `contribution`,
    ///    `commit_audit`, `item_tag`, `ehr_folder`, `restriction`,
    ///    `retention_anchor`, `blob_ref` and the EHR's `event_outbox` rows.
    ///    The `commit_audit` rows are the exception: nothing references `ehr`
    ///    from them, so their ids are captured before the cascade removes the
    ///    version and contribution rows that name them, and they are deleted
    ///    after it.
    /// 2. An erasure tombstone is appended to the outbox before the
    ///    transaction commits, so every registered reader is told what to
    ///    delete downstream (Art. 19: the controller communicates an erasure
    ///    "to each recipient to whom the personal data have been disclosed").
    /// 3. The externalized blobs this EHR referenced and no surviving version
    ///    still does are removed from the object store.
    ///
    /// The access records naming the EHR stay: Art. 17(3)(b) withholds erasure
    /// where processing is necessary "for compliance with a legal obligation",
    /// and the logging periods are that obligation.
    ///
    /// Row-count 0 on the EHR delete means the EHR did not exist →
    /// [`ServiceError::NotFound`].
    ///
    /// NOTE: ITS-REST `operations/admin_ehr_delete.yaml` answers `404` for an
    /// unknown id, so `ehr_id_does_not_exist` maps to `NotFound`.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — no EHR with that id
    ///   (`ehr_id_does_not_exist`).
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn admin_ehr_delete(&self, ehr_id: EhrId) -> Result<(), ServiceError> {
        // Read before the delete: the reference rows go with the versions that
        // carry them.
        #[cfg(feature = "multimedia")]
        let candidate_blobs = self.referenced_blob_uris(&[ehr_id]).await?;
        // Externalization is compiled out of this build, so no stored version
        // references a blob and there is nothing to collect.
        #[cfg(not(feature = "multimedia"))]
        let candidate_blobs: Vec<String> = Vec::new();

        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        if self.erase_ehrs(&mut tx, &[ehr_id]).await?.is_empty() {
            // No such EHR → `ehr_id_does_not_exist`. Rolls back
            // (nothing was written), so the audit capture is discarded.
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("EHR {ehr_id}"),
            ));
        }
        tx.commit().await?;

        // The EHR and its versions are gone; GC any blob this EHR referenced
        // that no *surviving* version still references (content-addressed dedup
        // means a blob shared with another EHR or a party must be kept).
        self.gc_unreferenced_blobs(candidate_blobs).await;
        Ok(())
    }

    /// Physically delete a set of EHRs, each with the full erasure reach of
    /// [`Self::admin_ehr_delete`]. Missing ids are skipped (idempotent bulk delete);
    /// the count of EHRs actually deleted is returned.
    ///
    /// The reach is the same, chunked: one transaction per chunk carries the
    /// cascade and the erasure tombstones.
    ///
    /// NOTE: the idempotent skip-missing semantics + returned count are
    /// our own design (no openEHR spec governs bulk-delete internals); a
    /// partial success is observable at the REST edge.
    ///
    /// # Errors
    /// - `exception` — a database fault while deleting.
    pub async fn admin_ehr_delete_all(&self, ehr_ids: &[EhrId]) -> Result<u64, ServiceError> {
        // Chunking bounds each transaction's lock/WAL footprint on a
        // full-store wipe.
        const CHUNK: usize = 128;
        // An empty selector = the full EHR set (see `admin_ehr_delete_all`); a
        // non-empty selector deletes exactly the named EHRs.
        let targets: Vec<EhrId> = if ehr_ids.is_empty() {
            erasure::all_ehr_ids(&self.pool).await?
        } else {
            ehr_ids.to_vec()
        };
        // Set statements per chunk, not a per-EHR transaction loop; a missing
        // id deletes zero rows, and `DELETE … RETURNING id` counts the EHRs
        // actually removed.
        let mut deleted = 0u64;
        for chunk in targets.chunks(CHUNK) {
            #[cfg(feature = "multimedia")]
            let candidate_blobs = self.referenced_blob_uris(chunk).await?;
            // Externalization is compiled out of this build, so no stored
            // version references a blob and there is nothing to collect.
            #[cfg(not(feature = "multimedia"))]
            let candidate_blobs: Vec<String> = Vec::new();
            let mut conn = acquire(&self.pool).await?;
            let mut tx = conn.begin().await?;
            let removed = self.erase_ehrs(&mut tx, chunk).await?;
            tx.commit().await?;
            #[expect(
                clippy::as_conversions,
                reason = "the removed-row count widens exactly: usize is at most 64 bits \
                          on every supported target"
            )]
            let removed_rows = removed.len() as u64;
            deleted += removed_rows;
            self.gc_unreferenced_blobs(candidate_blobs).await;
        }
        Ok(deleted)
    }

    /// Erase `ehr_ids` on `tx` and return the ids that existed.
    ///
    /// 1. The audit ids the EHRs' versions and contributions reference are
    ///    captured over both storage tiers, before the cascade deletes the
    ///    rows that name them.
    /// 2. `DELETE FROM ehr` cascades through the foreign-key graph and across
    ///    both tier partitions.
    /// 3. The captured audit rows, now unreferenced, are removed.
    /// 4. One erasure tombstone per erased EHR is appended.
    ///
    /// # Errors
    /// [`ServiceError::Storage`] when a statement fails.
    async fn erase_ehrs(
        &self,
        tx: &mut PgConnection,
        ehr_ids: &[EhrId],
    ) -> Result<Vec<EhrId>, ServiceError> {
        let commit_audit_ids = erasure::ehr_commit_audit_ids(&mut *tx, ehr_ids).await?;
        let removed = erasure::delete_ehrs(&mut *tx, ehr_ids).await?;
        erasure::delete_commit_audits(&mut *tx, &commit_audit_ids).await?;
        self.write_erase_tombstones(tx, &removed).await?;
        Ok(removed)
    }

    /// The externalized blob URIs the given EHRs' versions reference, read
    /// from `blob_ref` before the delete removes those rows.
    ///
    /// The index is the whole point: the question "which blobs does this EHR
    /// reference" used to pull every node body of the EHR into the process and
    /// walk it, and it is now one index read over the reference rows the write
    /// path maintains. Empty when no object store is reachable, because
    /// nothing could then be collected. Our own extension — no openEHR spec
    /// governs multimedia offload.
    ///
    /// NOTE: reachability, not `multimedia.enabled` — a deployment that stopped
    /// externalizing still has blobs to collect, and skipping them here would
    /// orphan every one of them in the bucket.
    ///
    /// # Errors
    /// [`ServiceError::Storage`] when the read fails.
    #[cfg(feature = "multimedia")]
    async fn referenced_blob_uris(&self, ehr_ids: &[EhrId]) -> Result<Vec<String>, ServiceError> {
        if self.multimedia.is_none() || ehr_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(erasure::ehr_blob_uris(&self.pool, ehr_ids).await?)
    }

    /// The externalized blob URIs the given demographic versioned objects
    /// reference.
    ///
    /// Reads inside the caller's transaction, because the rows are about to be
    /// deleted by it: a read on the pool could miss a row the transaction has
    /// already removed, and a blob whose last reference vanished unseen is a
    /// blob nothing will ever collect.
    ///
    /// Our own extension — no openEHR spec governs multimedia offload.
    ///
    /// # Errors
    /// [`ServiceError::Storage`] when the read fails.
    #[cfg(feature = "multimedia")]
    async fn party_blob_uris(
        &self,
        tx: &mut PgConnection,
        vo_ids: &[VoId],
    ) -> Result<Vec<String>, ServiceError> {
        if self.multimedia.is_none() || vo_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(erasure::object_blob_uris(tx, vo_ids).await?)
    }

    /// Delete each candidate blob no surviving version still references.
    ///
    /// The candidates are the URIs the erased versions referenced, read from
    /// `blob_ref` before they went; the survivors are the rows of that same
    /// index still standing. A blob is content-addressed, so one blob may be
    /// referenced by several versions and by both pseudonymisation domains —
    /// both are asked, because a party may be the only thing still holding a
    /// blob a clinical version also had.
    ///
    /// A blob-store failure is logged, not fatal: the delete has committed, and
    /// an orphaned blob is a storage cost rather than a correctness problem. A
    /// failed reference read keeps every candidate, so the collector never
    /// deletes on incomplete information.
    ///
    /// Our own extension — no openEHR spec governs multimedia offload.
    #[cfg(feature = "multimedia")]
    async fn gc_unreferenced_blobs(&self, candidates: Vec<String>) {
        let Some(engine) = &self.multimedia else {
            return;
        };
        if candidates.is_empty() {
            return;
        }
        let mut still_referenced: Vec<String> = Vec::new();
        for domain in [&self.pool, &self.demographic_pool] {
            match erasure::referenced_blob_uris(domain, &candidates).await {
                Ok(rows) => still_referenced.extend(rows),
                Err(e) => {
                    tracing::warn!(error = %e, "multimedia blob GC reference read failed; keeping all candidates");
                    return;
                }
            }
        }
        for uri in &candidates {
            if still_referenced.contains(uri) {
                continue;
            }
            // A URI this store did not mint names someone else's bytes; the
            // index records it (it is in a stored body) and the collector
            // leaves it alone.
            let Some(hex) = engine.store().key_from_uri(uri) else {
                continue;
            };
            if let Err(e) = engine.store().delete(hex).await {
                tracing::warn!(blob = %hex, error = %e, "multimedia blob GC delete failed");
            }
        }
    }

    /// The slim twin: externalization is compiled out, so no blob can exist to
    /// collect.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_async,
        reason = "the multimedia twin awaits; callers await unconditionally"
    )]
    async fn gc_unreferenced_blobs(&self, _candidates: Vec<String>) {}

    /// Physically delete a
    /// PARTY "along with related Party relationships", in one transaction —
    /// mirroring [`Self::admin_ehr_delete`]'s capture-then-cascade approach.
    ///
    /// The target must be a demographic PARTY root (any version, ehr-less); a
    /// `PARTY_RELATIONSHIP` or a non-party/unknown id is `party_id_does_not_exist`
    /// → [`ServiceError::NotFound`] (→ HTTP `404`). Deleted physically: the party
    /// VO + every `PARTY_RELATIONSHIP` VO whose stored canonical `source`/`target`
    /// `PARTY_REF` references the party (see `service/demographic/`), with their
    /// `version` rows (which cascade `node` + `vo_attestation` via the
    /// `(vo_id, sys_version)` FKs), the CONTRIBUTIONs/audit rows they orphan
    /// (guarded — a row shared with a survivor is kept), the head rows
    /// markers. `audit` has no FK from `version` (NO ACTION), so those rows
    /// are swept explicitly, as in the EHR delete. Every sealed national
    /// identifier row of the deleted objects, live and retired, goes in the
    /// same transaction: `national_identifier` carries no foreign key to the
    /// versions that reference it.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — the id names no
    ///   demographic PARTY root (`party_id_does_not_exist`; a
    ///   `PARTY_RELATIONSHIP` or unknown id is also this failure).
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn physical_party_delete(&self, party_id: VoId) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;

        // The target must be a demographic PARTY (ehr-less; any version exists),
        // in either storage tier — an archived party is still deletable.
        let kind = version_repo::tier::demographic_kind(&mut tx, party_id).await?;
        if !kind.as_deref().is_some_and(super::is_party_kind) {
            // `party_id_does_not_exist` → NotFound (→ 404). Rolls back cleanly.
            return Err(ServiceError::not_found(
                Missing::Party,
                format!("party {party_id}"),
            ));
        }

        // Every PARTY_RELATIONSHIP VO referencing the party as source/target, in
        // ANY version, goes with it.
        let mut vo_ids = erasure::relationships_naming(&mut tx, party_id).await?;
        vo_ids.push(party_id);

        // Capture the CONTRIBUTION + audit ids these VOs reference before the
        // version delete cascades their node/attestation rows away.
        let contribution_ids = erasure::object_contribution_ids(&mut tx, &vo_ids).await?;
        let commit_audit_ids =
            erasure::object_commit_audit_ids(&mut tx, &vo_ids, &contribution_ids).await?;

        // The blob keys these rows reference, before the delete removes the
        // rows that name them (#3180). `admin_ehr_delete` has always done this; a
        // party physically deleted without it left its blobs in the object
        // store forever, which is both a leak and a delete that did not
        // delete — a blob is content.
        #[cfg(feature = "multimedia")]
        let candidate_blobs = self.party_blob_uris(&mut tx, &vo_ids).await?;
        // Externalization is compiled out of this build, so no stored node
        // references a blob and there is nothing to collect.
        #[cfg(not(feature = "multimedia"))]
        let candidate_blobs: Vec<String> = Vec::new();

        // Delete the versioned objects — cascades node + vo_attestation, over
        // both tier partitions of the one `version` relation.
        erasure::delete_object_versions(&mut tx, &vo_ids).await?;

        // The sealed national identifiers those versions referenced, live and
        // retired: the erasure removes the values with the record that held
        // them. No openEHR spec governs identifier protection — our own design.
        identifier_repo::erase(&mut tx, &vo_ids)
            .await
            .map_err(|e| ServiceError::internal("erase the sealed national identifiers", e))?;

        // Orphaned CONTRIBUTIONs and audit rows (guarded: keep any still
        // referenced by a surviving version, attestation or contribution).
        erasure::delete_orphan_contributions(&mut tx, &contribution_ids).await?;
        erasure::delete_orphan_commit_audits(&mut tx, &commit_audit_ids).await?;

        // The reverse-relationship index rows that name the erased objects, at
        // either end (an erased party is neither a source nor a target), and
        // their head rows, which carry the archive marker.
        erasure::delete_object_index_rows(&mut tx, &vo_ids).await?;
        crate::party_binding::store::erase(&mut tx, &vo_ids).await?;

        self.write_party_erase_tombstone(&mut tx, party_id, kind.as_deref(), &vo_ids)
            .await?;

        tx.commit().await?;

        // After the commit, so a blob is only collected once nothing can
        // reference it again. The scan covers both domains, so a blob this
        // party shared with a clinical record survives.
        self.gc_unreferenced_blobs(candidate_blobs).await;
        Ok(())
    }

    /// Append one erasure tombstone per erased EHR, inside the transaction that
    /// erased it.
    ///
    /// GDPR Art. 19 (`docs/law/eu/gdpr/text.html`) makes the controller
    /// communicate an erasure "to each recipient to whom the personal data have
    /// been disclosed", and a consumer of the change-event stream is such a
    /// recipient: it holds whatever it derived from this EHR and cannot know
    /// the record is gone unless the stream says so. The row carries the erased
    /// `ehr_id` and nothing else, and it is written before the commit for the
    /// same reason every commit event is — an erasure that committed without
    /// its tombstone would leave the derived copies standing. It is written on
    /// the same gate as those events, because a deployment with no configured
    /// consumer has disclosed nothing through this channel.
    ///
    /// It carries no contribution: the EHR's contributions went with it, and
    /// their outbox rows with them. Like every outbox insert it holds the
    /// outbox lock shared to the commit's end ([`crate::extensions::outbox`]).
    /// No openEHR spec governs eventing — our own extension; ITS-REST 1.1.0
    /// defines no change notification.
    ///
    /// # Errors
    /// [`ServiceError::Storage`] when the insert fails.
    async fn write_erase_tombstones(
        &self,
        tx: &mut PgConnection,
        erased: &[EhrId],
    ) -> Result<(), ServiceError> {
        if !self.outbox_written() {
            return Ok(());
        }
        for &ehr_id in erased {
            let envelope = serde_json::json!({
                "event": "erase",
                "ehr_id": ehr_id.0,
                "versions": [{"kind": "EHR", "change_type": "erase"}],
            });
            erasure::write_erase_tombstone(&mut *tx, Some(ehr_id), &envelope).await?;
        }
        Ok(())
    }

    /// Append the erasure tombstone for one physically deleted party, inside
    /// the transaction that erased it.
    ///
    /// The clinical twin of this row is [`Self::write_erase_tombstones`], and
    /// the ground is the same: GDPR Art. 19 (`docs/law/eu/gdpr/text.html`)
    /// makes the controller communicate an erasure "to each recipient to whom
    /// the personal data have been disclosed", and a consumer of the party
    /// change-event stream holds whatever it derived from this party. Without
    /// the row it is never told the identity is gone, because every event it
    /// did receive announced a commit that no longer has a subject.
    ///
    /// One row covers the whole call: the party and every `PARTY_RELATIONSHIP`
    /// erased with it get an entry under their own RM type, so a consumer bound
    /// to relationships alone receives the erasure of the ones it tracked. The
    /// row carries no contribution, because the party's contributions went with
    /// it, and it is written before the commit for the same reason every commit
    /// event is. No openEHR spec governs eventing — our own extension.
    ///
    /// # Errors
    /// [`ServiceError::Storage`] when the insert fails.
    async fn write_party_erase_tombstone(
        &self,
        tx: &mut PgConnection,
        party_id: VoId,
        party_kind: Option<&str>,
        erased: &[VoId],
    ) -> Result<(), ServiceError> {
        if !self.outbox_written() {
            return Ok(());
        }
        let versions: Vec<serde_json::Value> = erased
            .iter()
            .map(|vo_id| {
                let kind = if *vo_id == party_id {
                    party_kind.unwrap_or("PARTY")
                } else {
                    "PARTY_RELATIONSHIP"
                };
                serde_json::json!({
                    "vo_id": vo_id.0,
                    "kind": kind,
                    "change_type": "erase",
                })
            })
            .collect();
        let envelope = serde_json::json!({
            "event": "erase",
            "ehr_id": serde_json::Value::Null,
            "vo_id": party_id.0,
            "versions": versions,
        });
        erasure::write_erase_tombstone(tx, None, &envelope).await?;
        Ok(())
    }
}
