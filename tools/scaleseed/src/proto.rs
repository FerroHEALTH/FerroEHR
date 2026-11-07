// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The prototypes: compositions committed through the service, each one the
//! source of every clone that reuses its content.
//!
//! Two of the repository's own CKM templates carry the content: the generic
//! lab test result, which the measured lab-results query reads, and vital
//! signs, so a population query meets more than one archetype. Each variant
//! has its own start and event time and, for a lab result, its own number of
//! analytes and their values. A share of the variants is updated one to three
//! times, so a cloned store holds about two versions per composition.

use anyhow::{Context as _, anyhow};
use serde_json::json;
use sqlx::PgPool;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use openehr_rm::prelude::Composition;

/// The lab-results template the measured query reads.
const LAB_TEMPLATE: &str =
    include_str!("../../../corpus/templates/ckm/generic-lab-test-result.opt");
/// Its example composition, the base of every lab variant.
const LAB_EXAMPLE: &str =
    include_str!("../../../corpus/templates/ckm/generic-lab-test-result.example.json");
/// The vital signs template, a second archetype set for the population query.
const VITALS_TEMPLATE: &str = include_str!("../../../corpus/templates/ckm/vital-signs.opt");
/// Its example composition, the base of every vital signs variant.
const VITALS_EXAMPLE: &str = include_str!("../../../corpus/templates/ckm/vital-signs.example.json");

/// 2020-01-01T00:00:00Z, the first variant's start time; each next one is an
/// hour and a minute later.
const START_BASE: i64 = 1_577_836_800;

/// The updates a variant receives, by its index modulo the list: three in
/// seven are never updated, so a store averages two versions per composition.
const UPDATES: [usize; 7] = [0, 0, 0, 1, 1, 2, 3];

/// The schema the seeder keeps its own bookkeeping in, beside the product's.
pub(crate) const BOOKKEEPING: &str = "
CREATE SCHEMA IF NOT EXISTS scaleseed;
CREATE TABLE IF NOT EXISTS scaleseed.proto_ehr (ehr_id uuid PRIMARY KEY);
CREATE TABLE IF NOT EXISTS scaleseed.prototype (
    vo_id uuid PRIMARY KEY, variant int NOT NULL UNIQUE, versions int NOT NULL);
CREATE TABLE IF NOT EXISTS scaleseed.chunk (
    id bigint PRIMARY KEY, ehrs int NOT NULL, compositions bigint NOT NULL,
    versions bigint NOT NULL, done_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE IF NOT EXISTS scaleseed.sample (
    vo_id uuid PRIMARY KEY, proto uuid NOT NULL, ehr_id uuid NOT NULL);
";

/// The content of variant `index` at its `revision` (0 for the first
/// version).
#[expect(
    clippy::disallowed_types,
    reason = "the CKM example compositions are edited as canonical JSON before they decode (#1694)"
)]
fn body(index: usize, revision: usize) -> anyhow::Result<serde_json::Value> {
    let lab = index % 10 < 7;
    let mut doc: serde_json::Value =
        serde_json::from_str(if lab { LAB_EXAMPLE } else { VITALS_EXAMPLE })?;
    let offset = i64::try_from(index)?.saturating_mul(3_660);
    let start = jiff::Timestamp::from_second(START_BASE.saturating_add(offset))?.to_string();
    *field(&mut doc, &["context", "start_time", "value"])? = json!(start);
    if lab {
        lab_variant(&mut doc, index, revision, &start)?;
    }
    Ok(doc)
}

/// The analytes of a lab variant: one to eight, each with its own value.
#[expect(
    clippy::disallowed_types,
    reason = "the CKM example compositions are edited as canonical JSON before they decode (#1694)"
)]
fn lab_variant(
    doc: &mut serde_json::Value,
    index: usize,
    revision: usize,
    start: &str,
) -> anyhow::Result<()> {
    let event = field(doc, &["content", "0", "data", "events", "0"])?;
    *field(event, &["time", "value"])? = json!(start);
    let items = field(event, &["data", "items"])?
        .as_array_mut()
        .ok_or_else(|| anyhow!("the example's event items are no array"))?;
    let analyte = items
        .get(1)
        .cloned()
        .ok_or_else(|| anyhow!("the example carries no analyte cluster"))?;
    items.truncate(1);
    for n in 0..=(index % 8) {
        let mut cluster = analyte.clone();
        let magnitude = f64::from(u32::try_from((index * 7 + n * 13 + revision * 3) % 400)?) / 4.0;
        if let Ok(slot) = field(&mut cluster, &["items", "1", "value", "magnitude"]) {
            *slot = json!(magnitude);
        }
        items.push(cluster);
    }
    Ok(())
}

/// The member of `doc` at `path`, an array index written as its decimal form.
#[expect(
    clippy::disallowed_types,
    reason = "the CKM example compositions are edited as canonical JSON before they decode (#1694)"
)]
fn field<'a>(
    doc: &'a mut serde_json::Value,
    path: &[&str],
) -> anyhow::Result<&'a mut serde_json::Value> {
    let mut at = doc;
    for step in path {
        at = match step.parse::<usize>() {
            Ok(index) => at.get_mut(index),
            Err(_) => at.get_mut(*step),
        }
        .ok_or_else(|| anyhow!("the example has no {}", path.join("/")))?;
    }
    Ok(at)
}

/// Uploads the two templates, creates the prototype EHR, and commits
/// `variants` prototype compositions with their updates through the service.
/// A prototype already committed is left as it is, so a second run resumes.
///
/// # Errors
/// Any refused upload, commit or bookkeeping statement.
pub(crate) async fn commit(
    svc: &FerroEhrService,
    pool: &PgPool,
    variants: usize,
) -> anyhow::Result<()> {
    sqlx::raw_sql(BOOKKEEPING).execute(pool).await?;
    let existing = sqlx::query_scalar::<_, uuid::Uuid>("SELECT ehr_id FROM scaleseed.proto_ehr")
        .fetch_optional(pool)
        .await?;
    let ehr = if let Some(ehr) = existing {
        EhrId(ehr)
    } else {
        for template in [LAB_TEMPLATE, VITALS_TEMPLATE] {
            svc.template_adl14_upload(template)
                .await
                .context("upload a template")?;
        }
        let (ehr, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(None)))
            .await
            .context("create the prototype EHR")?;
        sqlx::query("INSERT INTO scaleseed.proto_ehr VALUES ($1)")
            .bind(ehr.0)
            .execute(pool)
            .await?;
        ehr
    };
    let done: i64 = sqlx::query_scalar("SELECT count(*) FROM scaleseed.prototype")
        .fetch_one(pool)
        .await?;
    for index in usize::try_from(done)?..variants {
        let updates = UPDATES.get(index % UPDATES.len()).copied().unwrap_or(0);
        let first = svc
            .create_composition(ehr, write(&body(index, 0)?)?)
            .await
            .with_context(|| format!("commit prototype {index}"))?;
        let vo: VoId = first.vo_id;
        for revision in 1..=updates {
            svc.update_composition(ehr, vo, write(&body(index, revision)?)?)
                .await
                .with_context(|| format!("update prototype {index}"))?;
        }
        sqlx::query("INSERT INTO scaleseed.prototype VALUES ($1, $2, $3)")
            .bind(vo.0)
            .bind(i32::try_from(index)?)
            .bind(i32::try_from(updates + 1)?)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// The write request of a canonical composition, at the server's defaults.
#[expect(
    clippy::disallowed_types,
    reason = "the CKM example compositions are edited as canonical JSON before they decode (#1694)"
)]
fn write(doc: &serde_json::Value) -> anyhow::Result<WriteRequest<Composition>> {
    let composition = openehr_its::json::from_canonical_value(doc)
        .map_err(|e| anyhow!("the variant does not decode as a COMPOSITION: {e}"))?;
    Ok(WriteRequest::new(composition))
}
