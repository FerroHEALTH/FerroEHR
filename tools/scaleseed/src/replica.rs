// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What one replica holds of the caller facts at scale (#5590): patients under
//! the care of their teams, and the professionals of those teams with their
//! clearances, seeded into the facts side table, then read into a replica's
//! copy as a request reads it.
//!
//! The seed writes the rows the commit path derives from a patient's
//! `under_care_of` relationship and a professional's `member_of` relationship
//! and clearance straight into `party.binding_caller_fact`, as the caller
//! facts' integration test does, stamps each party with the facts revision and
//! moves both revisions, so the copy reads every row. The measurement starts
//! a [`LiveGrants`] over the party pool and reads it, then reads the care
//! relationships of as many subjects as the care copy holds and the patients
//! of every team, each twice, a miss and a hit. It reports the resident
//! memory the copy and the filled caches added to this process, the time of
//! the read, and the median time of each lookup. Run one population per
//! process, so the resident figures are that population's alone. No openEHR
//! specification governs the copy: our own design.

use std::time::Instant;

use anyhow::{Context as _, bail};
use ferroehr::party_binding::grants::live::LiveGrants;
use sqlx::PgPool;
use uuid::Uuid;

/// What one `replica` run seeds.
#[derive(Debug, Clone, Copy, clap::Args)]
pub(crate) struct Target {
    /// The patients under care.
    #[arg(long, default_value_t = 100_000)]
    pub(crate) patients: i64,
    /// The care teams they are spread over.
    #[arg(long, default_value_t = 500)]
    pub(crate) teams: i64,
    /// The professionals, each a member of one team with one clearance.
    #[arg(long, default_value_t = 5_000)]
    pub(crate) staff: i64,
}

/// Seeds the facts side table, unless it already holds this seed.
///
/// # Errors
/// When a statement fails, or the table already holds facts.
pub(crate) async fn seed(pool: &PgPool, target: Target) -> anyhow::Result<()> {
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.binding_caller_fact WHERE kind = 'under_care_of'",
    )
    .fetch_one(pool)
    .await
    .context("count the seeded patients")?;
    if held == target.patients {
        eprintln!("scaleseed: the facts table already holds {held} patients under care");
        return Ok(());
    }
    if held != 0 {
        bail!(
            "the facts table holds {held} patients under care, not {}; seed an empty store",
            target.patients
        );
    }
    let mut tx = pool.begin().await.context("begin the seed")?;
    // One care relationship to a team for each patient, and for each
    // professional; the label and the kind are literals of this function.
    for (label, kind, count) in [
        ("patient", "under_care_of", target.patients),
        ("staff", "member_of", target.staff),
    ] {
        sqlx::query(
            "INSERT INTO party.binding_caller_fact \
                 (holder_id, ordinal, kind, party_id, target_id, lifecycle_state, role_validity, \
                  validity) \
             SELECT md5($3::text || '-' || n)::uuid, 0, $4, md5($3::text || '-' || n)::uuid, \
                    md5('team-' || (n % $2))::uuid, '532', daterange(NULL, NULL), \
                    daterange(date '2026-01-01', NULL) \
             FROM generate_series(1, $1::bigint) AS n",
        )
        .bind(count)
        .bind(target.teams)
        .bind(label)
        .bind(kind)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("seed the {kind} relationships"))?;
    }
    sqlx::query(
        "INSERT INTO party.binding_caller_fact \
             (holder_id, ordinal, kind, party_id, terminology_id, code_string, \
              lifecycle_state, role_validity, validity) \
         SELECT md5('role-' || n)::uuid, 0, 'sensitivity_clearance', md5('staff-' || n)::uuid, \
                '2.16.840.1.113883.5.25', 'HIV', '532', daterange(NULL, NULL), \
                daterange(NULL, NULL) \
         FROM generate_series(1, $1::bigint) AS n",
    )
    .bind(target.staff)
    .execute(&mut *tx)
    .await
    .context("seed the clearances")?;
    sqlx::query(
        "INSERT INTO party.binding_caller_fact_party (party_id, revision) \
         SELECT DISTINCT party_id, 1 FROM party.binding_caller_fact",
    )
    .execute(&mut *tx)
    .await
    .context("stamp the parties")?;
    sqlx::query(
        "UPDATE party.binding_grant_revision \
         SET revision = revision + 1, facts_revision = facts_revision + 1",
    )
    .execute(&mut *tx)
    .await
    .context("move the revisions")?;
    tx.commit().await.context("commit the seed")?;
    for table in [
        "party.binding_caller_fact",
        "party.binding_caller_fact_party",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("VACUUM (ANALYZE) {table}")))
            .execute(pool)
            .await
            .with_context(|| format!("vacuum {table}"))?;
    }
    eprintln!(
        "scaleseed: seeded {} patients under the care of {} teams and {} professionals",
        target.patients, target.teams, target.staff
    );
    Ok(())
}

/// The resident set of this process, in bytes, as `ps` reports it.
fn resident_bytes() -> anyhow::Result<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .context("run ps")?;
    let kib: u64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .context("read the resident set ps printed")?;
    Ok(kib.saturating_mul(1024))
}

/// The median of `samples` in nanoseconds, sorted in place.
fn median(samples: &mut [u128]) -> u64 {
    samples.sort_unstable();
    let at = samples.len().div_euclid(2);
    samples
        .get(at)
        .copied()
        .and_then(|ns| u64::try_from(ns).ok())
        .unwrap_or_default()
}

/// The median of the times `run` takes over `items`, in nanoseconds.
async fn timed<T, F, Fut>(items: &[T], mut run: F) -> anyhow::Result<u64>
where
    F: FnMut(&T) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mut samples = Vec::with_capacity(items.len());
    for item in items {
        let started = Instant::now();
        run(item).await?;
        samples.push(started.elapsed().as_nanos());
    }
    Ok(median(&mut samples))
}

/// Reads the copy over `party`, fills its care caches to their bounds, and
/// prints one JSON row of what it holds and costs, a row of the `replica`
/// rows of the access-path record.
///
/// # Errors
/// When the copy cannot be read, or `ps` cannot be run.
#[expect(
    clippy::print_stdout,
    reason = "the row is the record's input; progress goes to stderr"
)]
pub(crate) async fn measure(party: PgPool, target: Target) -> anyhow::Result<()> {
    let subjects: Vec<Uuid> = sqlx::query_scalar(
        "SELECT party_id FROM party.binding_caller_fact WHERE kind = 'under_care_of' \
         ORDER BY holder_id LIMIT $1",
    )
    .bind(i64::try_from(
        ferroehr::party_binding::grants::care::SUBJECTS,
    )?)
    .fetch_all(&party)
    .await
    .context("read the sampled subjects")?;
    let teams: Vec<Uuid> = sqlx::query_scalar(
        "SELECT md5('team-' || n)::uuid FROM generate_series(0, $1::bigint - 1) AS n",
    )
    .bind(target.teams)
    .fetch_all(&party)
    .await
    .context("name the teams")?;
    let today = jiff::Zoned::now().date();
    let before = resident_bytes()?;
    let grants = LiveGrants::new(party);
    let started = Instant::now();
    grants.refresh().await.context("read the copy")?;
    let reload_ms = u64::try_from(started.elapsed().as_millis())?;
    let copy = resident_bytes()?;
    let care_of = |id: &Uuid| {
        let (grants, id) = (&grants, *id);
        async move {
            grants
                .under_care_of(id)
                .await
                .context("read a subject's care")?;
            Ok(())
        }
    };
    let census = |team: &Uuid| {
        let (grants, team) = (&grants, *team);
        async move {
            grants
                .cared_for_by(&[team], today, jiff::Span::new())
                .await
                .context("read a team's patients")?;
            Ok(())
        }
    };
    let care_miss_ns = timed(&subjects, care_of).await?;
    let care_hit_ns = timed(&subjects, care_of).await?;
    let census_miss_ns = timed(&teams, census).await?;
    let census_hit_ns = timed(&teams, census).await?;
    let held = resident_bytes()?;
    let cared = grants
        .cared_for_by(
            teams.first().map_or(&[][..], std::slice::from_ref),
            today,
            jiff::Span::new(),
        )
        .await
        .context("read one team's patients")?
        .len();
    if cared == 0 {
        bail!("the first team holds none of the seeded patients");
    }
    let (subjects_held, teams_held, census_held) = grants.care_held();
    let row = Row {
        patients: target.patients,
        teams: target.teams,
        staff: target.staff,
        parties_held: grants.parties_held(),
        copy_bytes: copy.saturating_sub(before),
        reload_ms,
        care_bytes: held.saturating_sub(copy),
        subjects_held,
        teams_held,
        census_held,
        care_miss_ns,
        care_hit_ns,
        census_miss_ns,
        census_hit_ns,
        cared_by_one_team: cared,
    };
    println!("{}", serde_json::to_string(&row).context("render the row")?);
    Ok(())
}

/// One `replica` row of the access-path record.
#[derive(Debug, serde::Serialize)]
struct Row {
    patients: i64,
    teams: i64,
    staff: i64,
    parties_held: usize,
    copy_bytes: u64,
    reload_ms: u64,
    care_bytes: u64,
    subjects_held: u64,
    teams_held: u64,
    census_held: u64,
    care_miss_ns: u64,
    care_hit_ns: u64,
    census_miss_ns: u64,
    census_hit_ns: u64,
    cared_by_one_team: usize,
}

/// Seeds `target` through the clinical pool `pool` and, unless `seed_only`,
/// measures a copy over the party domain of `config`.
///
/// # Errors
/// As [`seed`] and [`measure`], or when the party domain cannot be reached.
pub(crate) async fn run(
    config: &ferroehr::db::DbConfig,
    pool: &PgPool,
    target: Target,
    seed_only: bool,
) -> anyhow::Result<()> {
    seed(pool, target).await?;
    if seed_only {
        return Ok(());
    }
    let party = ferroehr::db::connect_domain(config, ferroehr::db::domain::Domain::Party)
        .await
        .context("connect to the party domain")?;
    measure(party, target).await
}
