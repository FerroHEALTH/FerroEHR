// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login lookup at scale (#5087): the binding side table seeded with bound
//! persons and groups, and the lookup every request of a person makes timed
//! against its budget.
//!
//! The seed writes the rows the commit path derives from a professional
//! `PERSON` and a `GROUP` of the mapped templates straight into
//! `party.binding_identifier` and `party.binding_membership`, as the lookup's
//! integration test does, then runs `VACUUM (ANALYZE)`. Each person's key and
//! each group's `externalId` are derived from its ordinal, so a run on a seeded
//! store asks for keys that exist. The measurement calls
//! [`LocalPartySource::read`], the lookup the server makes, with no cache: the
//! key rows, the live memberships under the group issuer and the
//! representations. No openEHR specification governs the binding: our own
//! design.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, bail};
use ferroehr::party_binding::source::LocalPartySource;
use sqlx::PgPool;

/// The identifier issuer the seeded keys sit under.
pub(crate) const KEY_ISSUER: &str = "urn:ferroehr:scaleseed:login";
/// The issuer the seeded groups' `externalId`s sit under.
pub(crate) const GROUP_ISSUER: &str = "urn:ferroehr:scaleseed:groups";
/// The type of every seeded key.
const KEY_TYPE: &str = "guid";
/// The groups each seeded person belongs to.
const GROUPS_PER_PERSON: i64 = 3;

/// What one `lookup` run seeds and measures.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    /// The bound persons to seed.
    pub(crate) persons: i64,
    /// The groups to seed.
    pub(crate) groups: i64,
    /// The lookups to time.
    pub(crate) samples: usize,
    /// The p99 budget, in milliseconds.
    pub(crate) budget_ms: f64,
}

/// Seeds the side table, unless it already holds this seed.
///
/// # Errors
/// When a statement fails, or a store already holds a seed of another size.
pub(crate) async fn seed(pool: &PgPool, target: Target) -> anyhow::Result<()> {
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.binding_identifier WHERE selector = 'key' AND issuer = $1",
    )
    .bind(KEY_ISSUER)
    .fetch_one(pool)
    .await
    .context("count the seeded persons")?;
    if held == target.persons {
        eprintln!("scaleseed: the side table already holds {held} seeded persons");
        return Ok(());
    }
    if held != 0 {
        bail!(
            "the side table holds {held} seeded persons, not {}; seed an empty store",
            target.persons
        );
    }
    let mut tx = pool.begin().await.context("begin the seed")?;
    sqlx::query(
        "INSERT INTO party.binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
              professional) \
         SELECT md5('person-' || n)::uuid, 'key', $1, $2, md5('key-' || n)::uuid::text, \
                md5('key-' || n)::uuid::text, '532', \
                datemultirange(daterange(NULL, NULL)) \
         FROM generate_series(1, $3::bigint) AS n",
    )
    .bind(KEY_ISSUER)
    .bind(KEY_TYPE)
    .bind(target.persons)
    .execute(&mut *tx)
    .await
    .context("seed the persons")?;
    sqlx::query(
        "INSERT INTO party.binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state) \
         SELECT md5('group-' || n)::uuid, 'group_external_id', $1, 'external_id', \
                'group-' || n, 'group-' || n, '532' \
         FROM generate_series(1, $2::bigint) AS n",
    )
    .bind(GROUP_ISSUER)
    .bind(target.groups)
    .execute(&mut *tx)
    .await
    .context("seed the groups")?;
    sqlx::query(
        "INSERT INTO party.binding_membership (group_id, member_id) \
         SELECT md5('group-' || (((n * $3 + k) % $2) + 1))::uuid, md5('person-' || n)::uuid \
         FROM generate_series(1, $1::bigint) AS n, generate_series(0, $3 - 1) AS k",
    )
    .bind(target.persons)
    .bind(target.groups)
    .bind(GROUPS_PER_PERSON)
    .execute(&mut *tx)
    .await
    .context("seed the memberships")?;
    tx.commit().await.context("commit the seed")?;
    for table in ["party.binding_identifier", "party.binding_membership"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("VACUUM (ANALYZE) {table}")))
            .execute(pool)
            .await
            .with_context(|| format!("vacuum {table}"))?;
    }
    eprintln!(
        "scaleseed: seeded {} persons, {} groups and {} memberships",
        target.persons,
        target.groups,
        target.persons.saturating_mul(GROUPS_PER_PERSON)
    );
    Ok(())
}

/// Times `target.samples` lookups of seeded keys through `source`, after a
/// warm-up of the same size, and writes `report.json` and `report.md` to `out`.
///
/// # Errors
/// When a lookup fails or finds no party, or the report cannot be written.
pub(crate) async fn measure(
    pool: &PgPool,
    source: &LocalPartySource,
    target: Target,
    out: &Path,
) -> anyhow::Result<Figures> {
    if target.persons <= 0 || target.samples == 0 {
        bail!("nothing to measure: {target:?}");
    }
    let today = jiff::Zoned::now().date().to_string();
    let samples = i64::try_from(target.samples).context("the sample count")?;
    // A stride through every ordinal, so the keys spread over the index.
    let keys: Vec<(i64, String)> = sqlx::query_as(
        "SELECT o, md5('key-' || o)::uuid::text \
         FROM (SELECT (n * 7919) % $1 + 1 AS o FROM generate_series(0, $2 - 1) AS n) AS s",
    )
    .bind(target.persons)
    .bind(samples)
    .fetch_all(pool)
    .await
    .context("derive the keys to look up")?;
    let mut timings = Vec::with_capacity(target.samples);
    for round in 0..2 {
        for (ordinal, key) in &keys {
            let started = Instant::now();
            let found = source
                .read(KEY_ISSUER, KEY_TYPE, key, Some(GROUP_ISSUER), &today)
                .await
                .with_context(|| format!("look up the key of person {ordinal}"))?;
            let elapsed = started.elapsed();
            if !matches!(&found, ferrohealth_smart::party::source::Lookup::Parties(parties) if parties.len() == 1)
            {
                bail!("the key of person {ordinal} bound no single party");
            }
            if round == 1 {
                timings.push(elapsed);
            }
        }
    }
    let mut ms: Vec<f64> = timings.iter().map(|t| t.as_secs_f64() * 1000.0).collect();
    ms.sort_unstable_by(f64::total_cmp);
    // The nearest-rank quantile: the smallest value at or above `pct` % of them.
    let at = |pct: usize| {
        let rank = (ms.len() * pct).div_ceil(100);
        ms.get(rank.saturating_sub(1)).copied().unwrap_or(0.0)
    };
    let figures = Figures {
        samples: ms.len(),
        p50_ms: at(50),
        p95_ms: at(95),
        p99_ms: at(99),
        budget_ms: target.budget_ms,
    };
    write(out, target, &figures)?;
    Ok(figures)
}

/// The figures of one measurement.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Figures {
    /// The lookups timed.
    pub(crate) samples: usize,
    /// The median, in milliseconds.
    pub(crate) p50_ms: f64,
    /// The 95th percentile, in milliseconds.
    pub(crate) p95_ms: f64,
    /// The 99th percentile, in milliseconds.
    pub(crate) p99_ms: f64,
    /// The p99 budget, in milliseconds.
    pub(crate) budget_ms: f64,
}

impl Figures {
    /// Whether the p99 is within the budget.
    pub(crate) fn within_budget(&self) -> bool {
        self.p99_ms <= self.budget_ms
    }
}

/// The JSON report of one measurement.
#[derive(Debug, serde::Serialize)]
struct Report {
    persons: i64,
    groups: i64,
    groups_per_person: i64,
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    budget_p99_ms: f64,
    within_budget: bool,
}

/// Writes the report of `figures` to `out`.
fn write(out: &Path, target: Target, figures: &Figures) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let json = Report {
        persons: target.persons,
        groups: target.groups,
        groups_per_person: GROUPS_PER_PERSON,
        samples: figures.samples,
        p50_ms: figures.p50_ms,
        p95_ms: figures.p95_ms,
        p99_ms: figures.p99_ms,
        budget_p99_ms: figures.budget_ms,
        within_budget: figures.within_budget(),
    };
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json).context("render the report")?,
    )
    .context("write report.json")?;
    let verdict = if figures.within_budget() {
        "within budget"
    } else {
        "**over budget**"
    };
    let md = [
        "### The login lookup at scale".to_owned(),
        String::new(),
        format!(
            "{} bound persons, {} groups, {GROUPS_PER_PERSON} memberships each; {} lookups \
             timed after as many warm-up lookups.",
            target.persons, target.groups, figures.samples
        ),
        String::new(),
        "| p50 (ms) | p95 (ms) | p99 (ms) | budget (ms) | verdict |".to_owned(),
        "|---|---|---|---|---|".to_owned(),
        format!(
            "| {:.3} | {:.3} | {:.3} | p99 at most {} | {verdict} |",
            figures.p50_ms, figures.p95_ms, figures.p99_ms, figures.budget_ms
        ),
        String::new(),
    ]
    .join("\n");
    std::fs::write(out.join("report.md"), md).context("write report.md")
}
