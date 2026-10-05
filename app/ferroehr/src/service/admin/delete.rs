// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Physical deletion (SM `I_ADMIN_SERVICE.physical_ehr_delete` /
//! `physical_party_delete` + the `admin_ehr_delete_all` bulk extension).
//!
//! Spec: `docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`
//! (`physical_ehr_delete`, precondition `has_ehr`, error `ehr_id_does_not_exist`;
//! `physical_party_delete`, "along with related Party relationships", error
//! `party_id_does_not_exist`). Behaviour is anchored by the CNF Robot prior art
//! (`I_ADMIN_SERVICE/001-EHR.robot` + `admin_keywords.robot`): a physical,
//! cascading delete after which every backing table returns to its pre-EHR
//! baseline count. No openEHR spec governs the cascade SQL / FK graph — our own
//! design over the greenfield schema.

#![cfg_attr(
    feature = "multimedia",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/TDD/dump-load compose \
                  over verbatim stored content (RM common master06 §Copying); the Value sites \
                  are multimedia-gated, so the expectation exists only where it is fulfilled"
    )
)]

use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::{CallStatusType, SmError};

impl FerroEhrService {
    /// SM `physical_ehr_delete`: physically delete one EHR and every trace of
    /// it.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `ehr_id` is not a well-formed UUID.
    /// - `versioned_object_does_not_exist` (`404`) — no EHR with that id
    ///   (`has_ehr` false → `ehr_id_does_not_exist`).
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn admin_ehr_delete(&self, ehr_id: String) -> Result<(), SmError> {
        Ok(self
            .delete_ehr(EhrId(super::parse_uuid(&ehr_id, "EHR")?))
            .await?)
    }

    /// The `admin_ehr_delete_all` extension: physically delete a set of EHRs
    /// (each with the full [`Self::admin_ehr_delete`] cascade), returning the
    /// count actually deleted. An **empty** list means "delete ALL EHRs" (the
    /// ITS-REST admin `DELETE /admin/ehr` types `ehr_id` as an **optional**
    /// subset selector, so an absent/empty list denotes the full set).
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — any id in the list is malformed
    ///   (the whole bulk request is rejected before any deletion runs).
    /// - `exception` — a database fault while deleting.
    pub async fn admin_ehr_delete_all(&self, ehr_ids: Vec<String>) -> Result<u64, SmError> {
        let ids: Vec<EhrId> = super::parse_uuid_list(&ehr_ids, "EHR")?
            .into_iter()
            .map(EhrId)
            .collect();
        Ok(self.delete_ehr_set(&ids).await?)
    }

    /// Admin extension `DELETE /admin/template/{template_id}`: physically delete
    /// one operational template by its wire `template_id` (case-insensitive,
    /// §Composite Identifiers and Case), evicting its `WebTemplate` cache entry.
    ///
    /// NOTE: no openEHR spec governs this operation, the ITS-REST Admin API
    /// defining only EHR deletes — our own design/extension, mirroring the SM
    /// UUID-keyed delete ([`Self::delete_opt`]) but addressed by the wire id.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — no template with that id.
    /// - `409` (`ServiceError::Conflict`) — a `version` row still references
    ///   the template (`version.template_id` FK, `clinical/0006_definitions.sql`);
    ///   a physical delete must never orphan the compositions built on it.
    /// - `exception` — a database fault.
    pub async fn admin_template_delete(&self, template_id: String) -> Result<(), SmError> {
        Ok(self.delete_template_by_id(&template_id).await?)
    }

    /// Admin extension `DELETE /admin/query/{qualified_name}/{version}`:
    /// physically delete exactly one stored query (a single version row),
    /// case-insensitive on the qualified name (matching the PUT store path),
    /// exact on the version.
    ///
    /// NOTE: no openEHR spec governs this operation, the ITS-REST Admin API
    /// defining only EHR deletes — our own design/extension; the SM
    /// `I_DEFINITION_QUERY.delete_query` deletes every version by name while
    /// this surface targets a single `(name, version)` row.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — no stored query at that
    ///   name + version.
    /// - `exception` — a database fault.
    pub async fn admin_query_delete(
        &self,
        qualified_name: String,
        version: String,
    ) -> Result<(), SmError> {
        Ok(self
            .delete_stored_query_version(&qualified_name, &version)
            .await?)
    }

    /// Delete one template by its wire id, refusing (409) while any committed
    /// version still references it. The reference count and the delete run in
    /// one transaction so the friendly 409 is consistent with the delete; the
    /// `version.template_id` → `template_ref` foreign key
    /// (`clinical/0006_definitions.sql`, NO ACTION) is the underlying guard that
    /// makes orphaning impossible even under a concurrent commit.
    async fn delete_template_by_id(&self, template_id: &str) -> Result<(), ServiceError> {
        let mut tx = self.pool.begin().await?;
        // Resolve the stored (case-preserved) id; absent → 404 (§Composite
        // Identifiers and Case: compare case-insensitively).
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT template_id FROM template_store WHERE lower(template_id) = lower($1)",
        )
        .bind(template_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(stored) = stored else {
            return Err(ServiceError::sm(
                CallStatusType::TemplateDoesNotExist,
                format!("template {template_id}"),
            ));
        };
        // Counted over BOTH storage tiers: `version` is one relation
        // partitioned by tier, so this reaches an archived composition's
        // reference as well as a live one.
        let refs: i64 = sqlx::query_scalar("SELECT count(*) FROM version WHERE template_id = $1")
            .bind(&stored)
            .fetch_one(&mut *tx)
            .await?;
        if refs > 0 {
            return Err(ServiceError::conflict(format!(
                "template '{stored}' is still referenced by {refs} committed version(s); \
                 delete those compositions before deleting the template"
            )));
        }
        sqlx::query("DELETE FROM template_store WHERE template_id = $1")
            .bind(&stored)
            .execute(&mut *tx)
            .await?;
        // Deregister the wire address unless a template-kind ADL2 artefact
        // also claims it (`template_ref` is the union of both dialects'
        // addresses; the FK blocks the deregistration if a concurrent commit
        // referenced it after the count above).
        sqlx::query(
            "DELETE FROM template_ref WHERE template_id = $1 AND NOT EXISTS \
             (SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1) \
              AND kind IN ('template', 'operational_template'))",
        )
        .bind(&stored)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        // Evict the derived-runtime cache for the deleted id (case-canonical key,
        // matching the SM delete path). No openEHR spec governs the cache.
        self.web_templates
            .invalidate(&crate::templates::identity::canonical_key(&stored))
            .await;
        Ok(())
    }

    /// SM `physical_party_delete`: physically delete a demographic PARTY "along
    /// with related Party relationships".
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `a_party_id` is not a well-formed
    ///   UUID.
    /// - `versioned_object_does_not_exist` (`404`) — the id names no
    ///   demographic PARTY root (`party_id_does_not_exist`; a
    ///   `PARTY_RELATIONSHIP` or unknown id is also this failure).
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn physical_party_delete(&self, a_party_id: String) -> Result<(), SmError> {
        Ok(self
            .physical_delete_party(VoId(super::parse_uuid(&a_party_id, "party")?))
            .await?)
    }

    /// Physically delete one EHR and every trace of it.
    ///
    /// The sequence, in order, is the erasure reach: one clinical transaction,
    /// then the linkage domain, then the object store.
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
    /// 3. `linkage.erase_ehr` runs once the clinical delete has committed, so
    ///    no cross-reference row outlives the record it named.
    /// 4. The externalized blobs this EHR referenced and no surviving version
    ///    still does are removed from the object store.
    ///
    /// The access records naming the EHR stay: Art. 17(3)(b) withholds erasure
    /// where processing is necessary "for compliance with a legal obligation",
    /// and the logging periods are that obligation.
    ///
    /// Row-count 0 on the EHR delete means the EHR did not exist (`has_ehr`
    /// false) → [`ServiceError::NotFound`].
    ///
    /// NOTE (already-correct — `i_admin_service.adoc` defines the failure
    /// only abstractly (`ehr_id_does_not_exist`) with no HTTP binding): we map it
    /// to `NotFound` → HTTP `404`, the natural REST reading of an operation on a
    /// non-existent resource.
    async fn delete_ehr(&self, ehr_id: EhrId) -> Result<(), ServiceError> {
        // Read before the delete: the reference rows go with the versions that
        // carry them.
        #[cfg(feature = "multimedia")]
        let candidate_blobs = self.referenced_blob_uris(&[ehr_id]).await?;
        // Externalization is compiled out of this build, so no stored version
        // references a blob and there is nothing to collect.
        #[cfg(not(feature = "multimedia"))]
        let candidate_blobs: Vec<String> = Vec::new();

        let mut tx = self.pool.begin().await?;

        // Capture the audit ids the EHR's versions and contributions reference,
        // before the cascade deletes those referencing rows. Read over BOTH
        // storage tiers: an archived version still holds its audit row.
        let commit_audit_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT commit_audit_id FROM version WHERE ehr_id = $1 \
             UNION \
             SELECT commit_audit_id FROM contribution WHERE ehr_id = $1",
        )
        .bind(ehr_id)
        .fetch_all(&mut *tx)
        .await?;

        // Delete the EHR — one statement, cascading through the FK graph and
        // across both tier partitions.
        let deleted = sqlx::query("DELETE FROM ehr WHERE id = $1")
            .bind(ehr_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() == 0 {
            // `has_ehr(ehr_id)` is false → `ehr_id_does_not_exist`. Rolls back
            // (nothing was written), so the audit capture above is discarded.
            return Err(ServiceError::sm(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR {ehr_id}"),
            ));
        }

        // The referencing version/contribution rows are gone, so the audit
        // rows are now unreferenced and can be removed.
        if !commit_audit_ids.is_empty() {
            sqlx::query("DELETE FROM commit_audit WHERE id = ANY($1)")
                .bind(&commit_audit_ids)
                .execute(&mut *tx)
                .await?;
        }

        self.write_erase_tombstones(&mut tx, &[ehr_id]).await?;

        tx.commit().await?;

        // Erasure reaches the cross-reference, on the linkage pool and after
        // the clinical delete has committed. A row there naming an erased EHR
        // is the additional information of GDPR Art. 4(5) outliving the data it
        // was additional to, and Art. 17(1) reaches it
        // (https://eur-lex.europa.eu/eli/reg/2016/679/oj).
        self.erase_ehr_linkage(ehr_id)
            .await
            .map_err(|error| ServiceError::internal("erase the EHR's linkage map", error))?;

        // The EHR and its versions are gone; GC any blob this EHR referenced
        // that no *surviving* version still references (content-addressed dedup
        // means a blob shared with another EHR or a party must be kept).
        self.gc_unreferenced_blobs(candidate_blobs).await;
        Ok(())
    }

    /// Physically delete a set of EHRs, each with the full erasure reach of
    /// [`Self::delete_ehr`]. Missing ids are skipped (idempotent bulk delete);
    /// the count of EHRs actually deleted is returned.
    ///
    /// The reach is the same, chunked: one transaction per chunk carries the
    /// cascade and the erasure tombstones, and the linkage erasure runs per
    /// EHR once that transaction has committed.
    ///
    /// NOTE (keep — spec-silent extension): `i_admin_service.adoc` has no
    /// bulk call, so the idempotent skip-missing semantics + returned count are
    /// our own design (no openEHR spec governs bulk-delete internals); a
    /// partial success is observable at the REST edge.
    async fn delete_ehr_set(&self, ehr_ids: &[EhrId]) -> Result<u64, ServiceError> {
        // Chunking bounds each transaction's lock/WAL footprint on a
        // full-store wipe.
        const CHUNK: usize = 128;
        // An empty selector = the full EHR set (see `admin_ehr_delete_all`); a
        // non-empty selector deletes exactly the named EHRs.
        let targets: Vec<EhrId> = if ehr_ids.is_empty() {
            sqlx::query_scalar("SELECT id FROM ehr")
                .fetch_all(&self.pool)
                .await?
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
            let mut tx = self.pool.begin().await?;
            let commit_audit_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT commit_audit_id FROM version WHERE ehr_id = ANY($1) \
                 UNION \
                 SELECT commit_audit_id FROM contribution WHERE ehr_id = ANY($1)",
            )
            .bind(chunk)
            .fetch_all(&mut *tx)
            .await?;
            let removed: Vec<Uuid> =
                sqlx::query_scalar("DELETE FROM ehr WHERE id = ANY($1) RETURNING id")
                    .bind(chunk)
                    .fetch_all(&mut *tx)
                    .await?;
            if !commit_audit_ids.is_empty() {
                sqlx::query("DELETE FROM commit_audit WHERE id = ANY($1)")
                    .bind(&commit_audit_ids)
                    .execute(&mut *tx)
                    .await?;
            }
            let removed_ids: Vec<EhrId> = removed.iter().copied().map(EhrId).collect();
            self.write_erase_tombstones(&mut tx, &removed_ids).await?;
            tx.commit().await?;
            for ehr_id in &removed_ids {
                self.erase_ehr_linkage(*ehr_id).await.map_err(|error| {
                    ServiceError::internal("erase the EHR's linkage map", error)
                })?;
            }
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
    /// [`ServiceError::Database`] when the read fails.
    #[cfg(feature = "multimedia")]
    async fn referenced_blob_uris(&self, ehr_ids: &[EhrId]) -> Result<Vec<String>, ServiceError> {
        if self.multimedia.is_none() || ehr_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT b.uri FROM blob_ref b \
             JOIN version v ON v.tier = b.tier AND v.vo_id = b.vo_id \
                 AND v.sys_version = b.sys_version \
             WHERE v.ehr_id = ANY($1)",
        )
        .bind(ehr_ids)
        .fetch_all(&self.pool)
        .await?)
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
    /// [`ServiceError::Database`] when the read fails.
    #[cfg(feature = "multimedia")]
    async fn party_blob_uris(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        vo_ids: &[VoId],
    ) -> Result<Vec<String>, ServiceError> {
        if self.multimedia.is_none() || vo_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(
            sqlx::query_scalar("SELECT DISTINCT uri FROM blob_ref WHERE vo_id = ANY($1)")
                .bind(vo_ids)
                .fetch_all(&mut **tx)
                .await?,
        )
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
            match sqlx::query_scalar("SELECT DISTINCT uri FROM blob_ref WHERE uri = ANY($1)")
                .bind(&candidates)
                .fetch_all(domain)
                .await
            {
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

    /// `physical_party_delete` (`i_admin_service.adoc`): physically delete a
    /// PARTY "along with related Party relationships", in one transaction —
    /// mirroring [`Self::delete_ehr`]'s capture-then-cascade approach.
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
    /// are swept explicitly, as in the EHR delete.
    async fn physical_delete_party(&self, party_id: VoId) -> Result<(), ServiceError> {
        let mut tx = self.demographic_pool.begin().await?;

        // The target must be a demographic PARTY (ehr-less; any version exists),
        // in either storage tier — an archived party is still deletable.
        let kind: Option<String> = sqlx::query_scalar(
            "SELECT kind FROM version WHERE vo_id = $1 AND ehr_id IS NULL LIMIT 1",
        )
        .bind(party_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !kind.as_deref().is_some_and(super::is_party_kind) {
            // `party_id_does_not_exist` → NotFound (→ 404). Rolls back cleanly.
            return Err(ServiceError::sm(
                CallStatusType::PartyIdDoesNotExist,
                format!("party {party_id}"),
            ));
        }

        // Every PARTY_RELATIONSHIP VO referencing the party as source/target, in
        // ANY version. The relationship stores `source`/`target` PARTY_REFs
        // inline in its root node's canonical fragment (they are DATA attributes,
        // not LOCATABLE children), so their `id.value` — the party's
        // versioned-object id — is matched with a jsonb path extraction.
        let party_txt = party_id.to_string();
        let rel_ids: Vec<VoId> = sqlx::query_scalar(
            "SELECT DISTINCT n.vo_id FROM node n \
             JOIN version v ON v.vo_id = n.vo_id AND v.sys_version = n.sys_version \
             WHERE v.kind = 'PARTY_RELATIONSHIP' \
               AND (n.data #>> '{source,id,value}' = $1 OR n.data #>> '{target,id,value}' = $1)",
        )
        .bind(&party_txt)
        .fetch_all(&mut *tx)
        .await?;

        let mut vo_ids = rel_ids;
        vo_ids.push(party_id);

        // Capture the CONTRIBUTION + audit ids these VOs reference before the
        // version delete cascades their node/attestation rows away.
        let contribution_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT contribution_id FROM version WHERE vo_id = ANY($1) \
             UNION \
             SELECT contribution_id FROM vo_attestation WHERE vo_id = ANY($1)",
        )
        .bind(&vo_ids)
        .fetch_all(&mut *tx)
        .await?;
        let commit_audit_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT commit_audit_id FROM version WHERE vo_id = ANY($1) \
             UNION \
             SELECT commit_audit_id FROM contribution WHERE id = ANY($2)",
        )
        .bind(&vo_ids)
        .bind(&contribution_ids)
        .fetch_all(&mut *tx)
        .await?;

        // The blob keys these rows reference, before the delete removes the
        // rows that name them (#3180). `delete_ehr` has always done this; a
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
        sqlx::query("DELETE FROM version WHERE vo_id = ANY($1)")
            .bind(&vo_ids)
            .execute(&mut *tx)
            .await?;

        // Orphaned CONTRIBUTIONs (guarded: keep any still referenced by a
        // surviving version/attestation).
        sqlx::query(
            "DELETE FROM contribution c WHERE c.id = ANY($1) \
               AND NOT EXISTS (SELECT 1 FROM version v WHERE v.contribution_id = c.id) \
               AND NOT EXISTS (SELECT 1 FROM vo_attestation a WHERE a.contribution_id = c.id)",
        )
        .bind(&contribution_ids)
        .execute(&mut *tx)
        .await?;

        // Orphaned audit rows (guarded the same way).
        sqlx::query(
            "DELETE FROM commit_audit a WHERE a.id = ANY($1) \
               AND NOT EXISTS (SELECT 1 FROM version v WHERE v.commit_audit_id = a.id) \
               AND NOT EXISTS (SELECT 1 FROM contribution c WHERE c.commit_audit_id = a.id)",
        )
        .bind(&commit_audit_ids)
        .execute(&mut *tx)
        .await?;

        // The head rows of the deleted objects, which carry the archive
        // marker among the rest of their state.
        sqlx::query("DELETE FROM vo_head WHERE vo_id = ANY($1)")
            .bind(&vo_ids)
            .execute(&mut *tx)
            .await?;

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
    /// their pending outbox rows with them. The routing key is `EHR.erase.-`,
    /// so a consumer can bind to erasures alone and a wildcard subscription
    /// receives them with everything else. No openEHR spec governs eventing —
    /// our own extension; ITS-REST 1.1.0 defines no change notification.
    ///
    /// # Errors
    /// [`ServiceError::Database`] when the insert fails.
    async fn write_erase_tombstones(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        erased: &[EhrId],
    ) -> Result<(), ServiceError> {
        if !self.outbox_enabled {
            return Ok(());
        }
        for ehr_id in erased {
            let envelope = serde_json::json!({
                "event": "erase",
                "ehr_id": ehr_id.0,
                "versions": [{"kind": "EHR", "change_type": "erase"}],
            });
            sqlx::query(
                "INSERT INTO event_outbox (ehr_id, envelope, committed_at) \
                 VALUES ($1, $2, now())",
            )
            .bind(*ehr_id)
            .bind(&envelope)
            .execute(&mut **tx)
            .await?;
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
    /// [`ServiceError::Database`] when the insert fails.
    async fn write_party_erase_tombstone(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        party_id: VoId,
        party_kind: Option<&str>,
        erased: &[VoId],
    ) -> Result<(), ServiceError> {
        if !self.outbox_enabled {
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
        sqlx::query(
            "INSERT INTO event_outbox (ehr_id, envelope, committed_at) \
             VALUES (NULL, $1, now())",
        )
        .bind(&envelope)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}
