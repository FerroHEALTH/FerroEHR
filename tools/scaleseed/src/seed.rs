// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The seed: EHRs in chunks, each EHR a clone of the prototype EHR holding
//! clones of the prototype compositions.
//!
//! An EHR's composition count is spread around the mean the target sets
//! (one to twice the mean less one, by a fixed hash of the EHR's place), and
//! each composition takes the prototype a second hash picks, so an EHR holds
//! different variants and the store holds every variant equally often. Every
//! 997th composition, and the first of each chunk, is recorded in
//! `scaleseed.sample` for the check.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use futures::{StreamExt as _, TryStreamExt as _};
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::service::FerroEhrService;
use ferroehr::versioning::signature::signer::Signer;

use crate::clone::{Prototype, copy_statements};

/// How often a composition is recorded for the check.
const SAMPLE_EVERY: u64 = 997;

/// What a seed run writes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    /// The EHRs, beside the prototype EHR.
    pub(crate) ehrs: u64,
    /// The compositions over all of them.
    pub(crate) compositions: u64,
    /// The EHRs one chunk, one transaction, writes.
    pub(crate) chunk: u64,
    /// The chunks written at once, each on its own connection.
    pub(crate) jobs: usize,
}

/// Everything a chunk needs, shared by the concurrent chunks.
struct Sources {
    svc: FerroEhrService,
    pool: PgPool,
    signer: Signer,
    proto_ehr: Uuid,
    ehr: Prototype,
    compositions: Vec<Prototype>,
    mean: u64,
}

/// A fixed hash of `value`, so a chunk written again writes the same shape.
const fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Seeds the store up to `target`, resuming at the first chunk not recorded.
///
/// # Errors
/// No prototype committed, or a failed read, clone or write.
pub(crate) async fn run(svc: FerroEhrService, pool: PgPool, target: Target) -> anyhow::Result<()> {
    let proto_ehr: Uuid = sqlx::query_scalar("SELECT ehr_id FROM scaleseed.proto_ehr")
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| anyhow!("no prototype EHR: run `scaleseed prototypes` first"))?;
    let status: Uuid = sqlx::query_scalar(
        "SELECT vo_id FROM clinical.version WHERE ehr_id = $1 AND kind = 'EHR_STATUS' LIMIT 1",
    )
    .bind(proto_ehr)
    .fetch_one(&pool)
    .await
    .context("the prototype EHR's EHR_STATUS")?;
    let ehr = Prototype::read(&svc, &pool, proto_ehr, status, true).await?;
    let vos: Vec<Uuid> =
        sqlx::query_scalar("SELECT vo_id FROM scaleseed.prototype ORDER BY variant")
            .fetch_all(&pool)
            .await?;
    if vos.is_empty() {
        return Err(anyhow!(
            "no prototype composition: run `scaleseed prototypes` first"
        ));
    }
    let mut compositions = Vec::with_capacity(vos.len());
    for vo in vos {
        compositions.push(Prototype::read(&svc, &pool, proto_ehr, vo, false).await?);
    }
    let mean = target.compositions.div_ceil(target.ehrs.max(1)).max(1);
    let done: Vec<i64> = sqlx::query_scalar("SELECT id FROM scaleseed.chunk")
        .fetch_all(&pool)
        .await?;
    let chunks = target.ehrs.div_ceil(target.chunk.max(1));
    let todo: Vec<u64> = (0..chunks)
        .filter(|c| {
            !done
                .iter()
                .any(|d| u64::try_from(*d).is_ok_and(|d| d == *c))
        })
        .collect();
    let sources = Arc::new(Sources {
        svc,
        pool,
        signer: Signer::digest_default(),
        proto_ehr,
        ehr,
        compositions,
        mean,
    });
    let started = std::time::Instant::now();
    let total = todo.len();
    futures::stream::iter(todo.into_iter().map(|c| {
        let sources = Arc::clone(&sources);
        async move {
            let first = c * target.chunk;
            let last = (first + target.chunk).min(target.ehrs);
            chunk(&sources, c, first..last).await
        }
    }))
    .buffer_unordered(target.jobs.max(1))
    .enumerate()
    .map(|(n, written)| written.map(|w| (n, w)))
    .try_for_each(|(n, (compositions, versions))| async move {
        eprintln!(
            "chunk {} of {total}: {compositions} compositions, {versions} versions, {} s",
            n + 1,
            started.elapsed().as_secs()
        );
        Ok(())
    })
    .await
}

/// Writes the EHRs `places` as chunk `id`, in one transaction that records
/// it; returns the compositions and versions it wrote.
async fn chunk(
    sources: &Sources,
    id: u64,
    places: std::ops::Range<u64>,
) -> anyhow::Result<(u64, u64)> {
    let mut out: BTreeMap<usize, String> = BTreeMap::new();
    let mut samples: Vec<(Uuid, Uuid, Uuid)> = Vec::new();
    let mut compositions = 0_u64;
    let mut versions = 0_u64;
    let count = u64::try_from(sources.compositions.len())?;
    for place in places.clone() {
        let ehr = sources.svc.stamp().mint();
        sources.ehr.clone_into(
            &sources.svc,
            &sources.signer,
            sources.proto_ehr,
            ehr,
            &mut out,
        )?;
        versions += u64::try_from(sources.ehr.versions)?;
        let n = 1 + mix(place) % (2 * sources.mean - 1);
        for j in 0..n {
            let pick = usize::try_from(mix(place.wrapping_mul(1_000_003).wrapping_add(j)) % count)?;
            let proto = sources
                .compositions
                .get(pick)
                .ok_or_else(|| anyhow!("prototype {pick} out of range"))?;
            let vo = proto.clone_into(
                &sources.svc,
                &sources.signer,
                sources.proto_ehr,
                ehr,
                &mut out,
            )?;
            compositions += 1;
            versions += u64::try_from(proto.versions)?;
            if (place == places.start && j == 0)
                || mix(place ^ j.rotate_left(32)).is_multiple_of(SAMPLE_EVERY)
            {
                samples.push((vo, proto.vo, ehr));
            }
        }
    }
    let mut tx = sources.pool.begin().await?;
    sqlx::query("SET LOCAL synchronous_commit = off")
        .execute(&mut *tx)
        .await?;
    for (place, statement) in copy_statements() {
        let Some(text) = out.remove(&place) else {
            continue;
        };
        let mut copy = tx.copy_in_raw(&statement).await?;
        copy.send(text.into_bytes()).await?;
        copy.finish().await.with_context(|| statement.clone())?;
    }
    for (vo, proto, ehr) in samples {
        sqlx::query("INSERT INTO scaleseed.sample VALUES ($1, $2, $3)")
            .bind(vo)
            .bind(proto)
            .bind(ehr)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO scaleseed.chunk (id, ehrs, compositions, versions) VALUES ($1, $2, $3, $4)",
    )
    .bind(i64::try_from(id)?)
    .bind(i32::try_from(places.end - places.start)?)
    .bind(i64::try_from(compositions)?)
    .bind(i64::try_from(versions)?)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((compositions, versions))
}
