// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `EHR_ACCESS` — the EHR-wide access-control top-level structure
//! (arch-overview `master06-design_of_the_ehr.adoc` §`EHR_ACCESS`; RM ehr
//! `org.openehr.rm.ehr.ehr_access.adoc`). This file owns the object's default
//! (created under the EHR-creation CONTRIBUTION — there is no direct ITS-REST
//! `EHR_ACCESS` write), the settings read the protocol adapter consumes, and
//! the per-EHR scheme cache; the commit validator lives in
//! [`validation`](super::validation).
//!
//! The `EhrAccessCache` mechanics are spec-silent — no openEHR spec governs a
//! per-request cache of `EHR_ACCESS` settings (our own design/extension). The
//! cache rides along here (cross-register ruling) but the access-control
//! *gate* that consults it (RBAC/ABAC) is a Stage-2 enterprise concern
//! (CLAUDE.md), not designed in this chapter.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use std::sync::Arc;

use crate::ids::EhrId;
use crate::service::ehr::access_types::{EhrAccessIndex, EhrAccessSettings};
use crate::service::error::{ServiceError, internal_fault};
use crate::service::status::SmError;
use moka::future::Cache;
use openehr_rm::prelude::{DvText, DvTextData, EhrAccess};
use serde_json::Value;
use sqlx::PgConnection;

use crate::service::FerroEhrService;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, commit_contribution};
use crate::versioning::read::read_current;

impl FerroEhrService {
    /// Drop the cached `EHR_ACCESS` settings for `ehr_id` — the
    /// [`crate::versioning::CommitEnv`] `invalidate_ehr_access` hook, called
    /// after any `EHR_ACCESS` commit so the next access decision reflects the
    /// new version (the settings are change-controlled — RM ehr master04 §EHR
    /// Access).
    #[expect(
        clippy::same_name_method,
        reason = "the `CommitEnv` seam (service/commit_env.rs) deliberately \
                  mirrors these chapter method names so the versioning layer \
                  calls them by their own vocabulary; that impl disambiguates \
                  explicitly with `FerroEhrService::<name>(self, …)`"
    )]
    pub(in crate::service) async fn invalidate_ehr_access(&self, ehr_id: EhrId) {
        self.ehr_access.invalidate(ehr_id).await;
    }

    /// Pre-warm the `EHR_ACCESS` cache for a just-created EHR as default-open
    /// (`None`). Every EHR is created with the settings-less
    /// `super::initial_ehr_access` (there is no direct `EHR_ACCESS` write —
    /// RM ehr master04 §EHR Access), so a fresh EHR is unconditionally
    /// default-open; seeding that entry saves the first-access DB miss. A
    /// later `EHR_ACCESS` commit evicts it through
    /// [`Self::invalidate_ehr_access`].
    pub(in crate::service) async fn prewarm_ehr_access_open(&self, ehr_id: EhrId) {
        self.ehr_access.insert(ehr_id, None).await;
    }

    /// Commit the default [`initial_ehr_access`] for an EHR that has none,
    /// inside the caller's transaction — the EHR-Extract import bootstrap
    /// ([`crate::service::message`]).
    ///
    /// `EHR.ehr_access` is 1..1 (RM ehr `ehr.adoc` invariant
    /// `Ehr_access_valid`) and RM ehr master04 §EHR Creation requires that
    /// creating an EHR yields "a root EHR object, an EHR Status object, and an
    /// EHR Access object … created and committed in a Contribution". A clone
    /// whose source extract carried no `EHR_ACCESS` would otherwise violate
    /// that invariant permanently and serve an `EHR` body without the mandatory
    /// reference, so the missing object is created locally.
    ///
    /// It is server-created content rather than replayed extract content, so it
    /// commits as a normal first `ORIGINAL_VERSION` (`249|creation|`,
    /// server-signed when signing is enabled) under its own CONTRIBUTION; the
    /// import CONTRIBUTION records the local act of committal for received
    /// originals, which "are never modified" (RM common master06 §Copying). Both
    /// CONTRIBUTIONs commit in the caller's single transaction, so the created
    /// EHR is complete atomically.
    ///
    /// # Errors
    /// The [`crate::versioning::change::commit_contribution`] write errors.
    pub(in crate::service) async fn commit_default_ehr_access(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        description: &str,
    ) -> Result<(), ServiceError> {
        let audit = self.audit(change_type::CREATION, description);
        commit_contribution(
            tx,
            Some(ehr_id),
            None,
            &audit,
            vec![(
                audit.clone(),
                Change::Create {
                    kind: Kind::EhrAccess,
                    canonical: initial_ehr_access(),
                    template_id: None,
                    signature: None,
                    lifecycle_state: None,
                    attestations: Vec::new(),
                },
            )],
            Vec::new(),
            &self.signing_ctx(),
        )
        .await?;
        Ok(())
    }

    /// Read + parse the EHR's current `EHR_ACCESS` scheme settings from
    /// storage (the cache-miss path). `None` when the EHR has no `EHR_ACCESS`,
    /// its settings are absent, or they belong to another scheme — all
    /// default-open.
    async fn load_ehr_access_settings(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<EhrAccessSettings>, SmError> {
        let Some((vo_id, _)) = self.current_vo(ehr_id, Kind::EhrAccess).await? else {
            return Ok(None);
        };
        let Some(read) = read_current(&self.pool, self.spec_profile, vo_id).await? else {
            return Ok(None);
        };
        Ok(EhrAccessSettings::from_ehr_access(&read.canonical))
    }

    /// The EHR's current `EHR_ACCESS` scheme settings, cached per EHR and
    /// returned as the cache's own [`std::sync::Arc`] (the per-request gate
    /// never clones an access list) — the `EhrAccessAdapter` native-API
    /// extension: the protocol adapter (`ferroehr-rest`) — the out-of-band
    /// access-decision point (SM `openehr_platform/master02-overview.adoc`)
    /// — reads the settings through this seam. The SM defines no
    /// `I_EHR_ACCESS` interface — no openEHR spec governs this adapter, our
    /// own extension. `None` = default-open (no settings, or a scheme this
    /// server does not understand).
    ///
    /// # Errors
    /// [`SmError`] when the cache-miss load (the `current_vo` + `read_current`
    /// storage reads) fails; the error is shared across concurrent callers of
    /// the single-flight load.
    pub async fn current_ehr_access_settings(
        &self,
        ehr_id: EhrId,
    ) -> Result<Arc<Option<EhrAccessSettings>>, SmError> {
        // Clone the (cheap, Arc-backed) service into an owned, `'static` load
        // future so `moka`'s single-flight `try_get_with` can drive it.
        let svc = self.clone();
        self.ehr_access
            .get_or_load(
                ehr_id,
                async move { svc.load_ehr_access_settings(ehr_id).await },
            )
            .await
            .map_err(|e| (*e).clone())
    }

    /// Every EHR whose current `EHR_ACCESS` carries settings this server
    /// understands, with those settings: the input of the per-EHR decision a
    /// query applies to every EHR it could reach, where the path routes decide
    /// one EHR at a time ([`Self::current_ehr_access_settings`]). An EHR absent
    /// from the list has no settings and falls to the server-wide default.
    ///
    /// Cached as one entry and dropped on every `EHR_ACCESS` commit, like the
    /// per-EHR entries. No openEHR spec governs this index — our own design.
    ///
    /// # Errors
    /// [`SmError`] when the storage read fails or a stored body is not JSON.
    pub async fn ehr_access_settings_index(&self) -> Result<EhrAccessIndex, SmError> {
        let svc = self.clone();
        self.ehr_access
            .index_or_load(async move { svc.load_ehr_access_index().await })
            .await
            .map_err(|e| (*e).clone())
    }

    /// The cache-miss path of [`Self::ehr_access_settings_index`].
    async fn load_ehr_access_index(&self) -> Result<Vec<(EhrId, EhrAccessSettings)>, SmError> {
        let bodies =
            crate::storage::version_repo::meta::ehr_access_bodies_with_settings(&self.pool)
                .await
                .map_err(ServiceError::from)?;
        let mut index = Vec::with_capacity(bodies.len());
        for (ehr_id, body) in bodies {
            let value: Value = serde_json::from_str(&body)
                .map_err(|e| internal_fault("read a stored EHR_ACCESS body", &e))?;
            if let Some(settings) = EhrAccessSettings::from_ehr_access(&value) {
                index.push((ehr_id, settings));
            }
        }
        Ok(index)
    }
}

/// Builds the `EHR_ACCESS` committed with every new EHR (RM ehr master04 §EHR
/// Creation).
///
/// `EHR_ACCESS` is a LOCATABLE with only the
/// optional `settings`; with no access-control scheme configured (Stage 1 has
/// no RBAC), it is committed with none.
///
/// `archetype_details` is mandatory: `EHR_ACCESS` carries the same
/// unconditional `Is_archetype_root` invariant as `EHR_STATUS`
/// (RM ehr `ehr_access.adoc`), so `Archetyped_valid` (RM common
/// `locatable.adoc`) requires the `ARCHETYPED` block on the root.
pub(in crate::service) fn initial_ehr_access() -> Value {
    const ARCHETYPE: &str = "openEHR-EHR-EHR_ACCESS.generic.v1";
    let access = EhrAccess {
        name: DvText::DvText(DvTextData {
            value: "EHR Access".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        }),
        archetype_node_id: ARCHETYPE.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(crate::service::ehr::service::archetyped(ARCHETYPE)),
        feeder_audit: None,
        // `EHR_ACCESS.settings` (0..1) — see the doc comment above: with no
        // access-control scheme in force the default carries none.
        settings: None,
    };
    openehr_its::json::to_canonical_value(&access)
}

/// A shared, cloneable per-EHR cache of the current `EHR_ACCESS` scheme
/// settings.
///
/// The `EHR_ACCESS` gateway clause ("All access decisions to data in the EHR
/// must be made in accordance with the policies and rules in this object" —
/// RM ehr `ehr_access.adoc`) is consulted on **every** EHR-scoped request, so
/// the current settings are cached rather than re-read + re-decomposed per
/// request. Invalidated on every `EHR_ACCESS` commit (the settings are
/// change-controlled — RM ehr master04 §EHR Access).
///
/// No openEHR spec governs this cache — our own design/extension. `moka`'s
/// `Cache` is `Arc`-backed, so every clone of the owning service shares one
/// cache (mirroring `openehr_sdt::flat::cache::WebTemplateCache`).
#[derive(Debug, Clone)]
pub(in crate::service) struct EhrAccessCache {
    inner: Cache<EhrId, Arc<Option<EhrAccessSettings>>>,
    index: Cache<(), EhrAccessIndex>,
}

impl EhrAccessCache {
    /// A cache holding up to `capacity` EHRs' settings.
    fn new(capacity: u64) -> Self {
        Self {
            inner: Cache::builder().max_capacity(capacity).build(),
            index: Cache::builder().max_capacity(1).build(),
        }
    }

    /// The cached settings for `ehr_id`, or load them via `init` (run at most
    /// once per key under contention) and cache the result.
    ///
    /// # Errors
    /// Propagates the `init` error (shared across concurrent callers as an
    /// `Arc<SmError>`).
    pub(in crate::service) async fn get_or_load<Fut>(
        &self,
        ehr_id: EhrId,
        init: Fut,
    ) -> Result<Arc<Option<EhrAccessSettings>>, Arc<SmError>>
    where
        Fut: Future<Output = Result<Option<EhrAccessSettings>, SmError>>,
    {
        self.inner
            .try_get_with(ehr_id, async move { init.await.map(Arc::new) })
            .await
    }

    /// Seed the cache with a value for `ehr_id` without a DB read — the
    /// pre-warm path. `try_get_with` already negative-caches a default-open
    /// (`None`) result on the first miss, but the *first* access to a
    /// freshly-created EHR would still pay one DB round trip (the
    /// `current_vo` + `read_current` lookup) to discover it is default-open.
    /// A workload that creates EHRs constantly (a hospital day) pays that miss
    /// per new EHR; seeding the known-default-open entry at creation turns the
    /// first access into a hit. Any later `EHR_ACCESS` commit evicts this
    /// entry via [`Self::invalidate`], so a subsequently-restricted EHR is
    /// re-read.
    pub(in crate::service) async fn insert(
        &self,
        ehr_id: EhrId,
        settings: Option<EhrAccessSettings>,
    ) {
        self.inner.insert(ehr_id, Arc::new(settings)).await;
    }

    /// Drop the cached settings for `ehr_id` — called on every `EHR_ACCESS`
    /// commit so the next read reflects the new version (evicts a positive OR
    /// a pre-warmed default-open negative entry alike).
    pub(in crate::service) async fn invalidate(&self, ehr_id: EhrId) {
        self.inner.invalidate(&ehr_id).await;
        self.index.invalidate(&()).await;
    }

    /// The cached settings index, or load it via `init` (run at most once
    /// under contention) and cache the result.
    ///
    /// # Errors
    /// Propagates the `init` error (shared across concurrent callers as an
    /// `Arc<SmError>`).
    pub(in crate::service) async fn index_or_load<Fut>(
        &self,
        init: Fut,
    ) -> Result<EhrAccessIndex, Arc<SmError>>
    where
        Fut: Future<Output = Result<Vec<(EhrId, EhrAccessSettings)>, SmError>>,
    {
        self.index
            .try_get_with((), async move { init.await.map(Arc::new) })
            .await
    }
}

impl Default for EhrAccessCache {
    fn default() -> Self {
        Self::new(4096)
    }
}
