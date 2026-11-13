// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The measurement: the queries of the scale test against one seeded EHR,
//! the sizes of the store, and what `ANALYZE` costs at that size.
//!
//! Each AQL query is lowered by the engine the server runs
//! (`ferroehr::aql::plan` and `ferroehr::aql::sql::build`) and run on the
//! pool. The first run is the first execution on a fresh connection, which
//! pays the planning a new connection pays; the warm figure is the median of
//! ten runs after it on that connection. Planning and execution come from
//! `EXPLAIN (ANALYZE, BUFFERS)` on a fresh connection and again after the warm
//! runs; the plans are written beside the report.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, anyhow};
use sqlx::{Connection as _, PgConnection, PgPool};
use uuid::Uuid;

use ferroehr::aql::ir::{ParamValue, Params};
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::{EhrAccessScope, EhrScopeKind, SqlCtx};
use ferroehr::config::profile::SpecProfile;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;

/// The lab-results query of Cadasto/FerroHEALTH-backlog#25, pinned to `$ehr`.
const LAB: &str = "SELECT o/data[at0001]/events[at0002]/time/value, \
    o/data[at0001]/events[at0002]/data[at0003]/items[at0005]/value/value, c/composer/name, \
    a/items[at0024]/value, a/items[at0001]/value, a/items[at0004]/value/value, \
    a/items[at0005]/value/defining_code/code_string, a/items[at0003]/value/value \
    FROM EHR e CONTAINS COMPOSITION c \
    CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] \
    CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1] \
    WHERE e/ehr_id/value = $ehr AND c/archetype_details/template_id/value = $template";

/// The same query with the EHR left to the request's `ehr_id` scope.
const LAB_UNPINNED: &str = "SELECT o/data[at0001]/events[at0002]/time/value, \
    a/items[at0001]/value FROM EHR e CONTAINS COMPOSITION c \
    CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] \
    CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1] \
    WHERE c/archetype_details/template_id/value = $template";

/// The lab query's ordering.
const BY_EVENT_TIME: &str = " ORDER BY o/data[at0001]/events[at0002]/time/value DESC";

/// The small query of the same report.
const SMALL: &str = "SELECT c/uid/value AS uid, c/context/start_time/value AS start_time \
    FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.report-result.v1] \
    WHERE e/ehr_id/value = $ehr ORDER BY c/context/start_time/value DESC";

/// An EHR-scoped `LIKE` on the template id (#5082).
const LIKE: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
    WHERE e/ehr_id/value = $ehr AND c/archetype_details/template_id/value LIKE 'Generic lab*'";

/// A population query, for contrast: the newest hundred lab reports.
const POPULATION: &str = "SELECT c/uid/value, c/context/start_time/value \
    FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.report-result.v1] \
    ORDER BY c/context/start_time/value DESC LIMIT 100";

/// The lab template the queries name.
const TEMPLATE: &str = "Generic lab test result example simple";

/// The warm runs per query.
const WARM: usize = 10;

/// One measured query: its name, its AQL, the request's EHR scope, the limit.
struct Case {
    name: &'static str,
    aql: String,
    scoped: bool,
    limit: i64,
}

/// Measures the store and writes `report.md` and one plan file per query into
/// `out`.
///
/// # Errors
/// No seeded EHR, a query that does not plan or lower, or a failed statement.
pub(crate) async fn run(
    svc: &FerroEhrService,
    pool: &PgPool,
    dsn: &str,
    host: &str,
    out: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(out)?;
    let (ehr, vo): (Uuid, Uuid) =
        sqlx::query_as("SELECT ehr_id, vo_id FROM scaleseed.sample ORDER BY vo_id LIMIT 1")
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow!("no sampled EHR: run `scaleseed seed` first"))?;
    let mut report = String::new();
    writeln!(report, "## Store\n")?;
    store(pool, ehr, &mut report).await?;
    writeln!(report, "\n## Host\n\n{host}\n")?;
    settings(pool, &mut report).await?;
    writeln!(
        report,
        "\n## Queries\n\nThe EHR `{ehr}`. Times in ms: the first run on a fresh connection, the \
         median of {WARM} warm runs on it, and planning and execution from `EXPLAIN (ANALYZE, \
         BUFFERS)` first and warm.\n"
    )?;
    writeln!(
        report,
        "| Query | Rows | First | Warm median | Plan first | Exec first | Plan warm | Exec warm | Buffers warm |"
    )?;
    writeln!(
        report,
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    )?;
    let cases = [
        Case {
            name: "lab, ORDER BY event time, pinned",
            aql: format!("{LAB}{BY_EVENT_TIME}"),
            scoped: false,
            limit: 501,
        },
        Case {
            name: "lab, unordered, pinned",
            aql: LAB.to_owned(),
            scoped: false,
            limit: 501,
        },
        Case {
            name: "lab, ORDER BY event time, ehr_id parameter",
            aql: format!("{LAB_UNPINNED}{BY_EVENT_TIME}"),
            scoped: true,
            limit: 501,
        },
        Case {
            name: "small, ORDER BY start time",
            aql: SMALL.to_owned(),
            scoped: false,
            limit: 10_000,
        },
        Case {
            name: "LIKE on the template id",
            aql: LIKE.to_owned(),
            scoped: false,
            limit: 10_000,
        },
        Case {
            name: "population, newest 100",
            aql: POPULATION.to_owned(),
            scoped: false,
            limit: 100,
        },
    ];
    for (index, case) in cases.iter().enumerate() {
        let line = query(dsn, case, ehr, &out.join(format!("plan-{index}.txt"))).await?;
        writeln!(report, "{line}")?;
    }
    let read = composition_read(svc, ehr, vo).await?;
    writeln!(report, "{read}")?;
    writeln!(report, "\n## ANALYZE\n")?;
    analyze(pool, &mut report).await?;
    std::fs::write(out.join("report.md"), report)?;
    Ok(())
}

/// A fresh connection reading the clinical domain, as the server's clinical
/// pool does.
async fn connect(dsn: &str) -> anyhow::Result<PgConnection> {
    let mut conn = PgConnection::connect(dsn).await?;
    sqlx::raw_sql("SET search_path = clinical, ext, public")
        .execute(&mut conn)
        .await?;
    Ok(conn)
}

/// One query's row of the table, its plans written to `plan`.
async fn query(dsn: &str, case: &Case, ehr: Uuid, plan: &Path) -> anyhow::Result<String> {
    let params = Params::default()
        .with("ehr", ParamValue::Str(ehr.to_string()))
        .with("template", ParamValue::Str(TEMPLATE.to_owned()));
    let ast = openehr_query::parser::parse_str(&case.aql).map_err(|e| anyhow!("{e:?}"))?;
    let ir = ferroehr::aql::plan(&ast, &params, SpecProfile::default())
        .map_err(|e| anyhow!("plan {}: {e}", case.name))?;
    let ctx = SqlCtx {
        system_id: "scaleseed".to_owned(),
        ehr_ids: if case.scoped {
            vec![EhrId(ehr)]
        } else {
            Vec::new()
        },
        ehr_scope_kind: EhrScopeKind::Named,
        subject_scope: None,
        ehr_access: EhrAccessScope::Unrestricted,
        limit: Some(case.limit),
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
        policy_parties: ferroehr::aql::sql::PolicyParties::Served,
        access_settings: ferroehr::aql::sql::EhrAccessReaders::Every,
    };
    let prepared = ferroehr::aql::sql::build(&ir, &params, &ctx)
        .map_err(|e| anyhow!("lower {}: {e}", case.name))?;
    let (first_plan, first_exec, first_text, _) = explain(dsn, &prepared, None).await?;
    let mut conn = connect(dsn).await?;
    let mut times = Vec::with_capacity(WARM + 1);
    let mut rows = 0;
    for _ in 0..=WARM {
        let started = Instant::now();
        rows = sqlx::query_with(
            sqlx::AssertSqlSafe(prepared.sql.clone()),
            prepared.values.clone(),
        )
        .fetch_all(&mut conn)
        .await
        .with_context(|| format!("run {}", case.name))?
        .len();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let (first, median) = first_and_median(times);
    let (warm_plan, warm_exec, warm_text, buffers) =
        explain(dsn, &prepared, Some(&mut conn)).await?;
    std::fs::write(
        plan,
        format!(
            "{}\n\n-- AQL\n{}\n\n-- first\n{first_text}\n\n-- warm\n{warm_text}\n",
            case.name, case.aql
        ),
    )?;
    Ok(format!(
        "| {} | {rows} | {first:.1} | {median:.1} | {first_plan:.1} | {first_exec:.1} | {warm_plan:.1} | {warm_exec:.1} | {buffers} |",
        case.name
    ))
}

/// Planning time, execution time, the text plan and the shared buffers hit
/// and read, on `conn` or on a fresh connection.
#[expect(
    clippy::disallowed_types,
    reason = "EXPLAIN (FORMAT JSON) is PostgreSQL's own document, read for four numbers (#1694)"
)]
async fn explain(
    dsn: &str,
    prepared: &ferroehr::aql::sql::PreparedQuery,
    conn: Option<&mut PgConnection>,
) -> anyhow::Result<(f64, f64, String, String)> {
    let mut fresh;
    let conn = if let Some(conn) = conn {
        conn
    } else {
        fresh = connect(dsn).await?;
        &mut fresh
    };
    let json: serde_json::Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {}",
            prepared.sql
        )),
        prepared.values.clone(),
    )
    .fetch_one(&mut *conn)
    .await?;
    let text: Vec<String> = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("EXPLAIN (ANALYZE, BUFFERS) {}", prepared.sql)),
        prepared.values.clone(),
    )
    .fetch_all(&mut *conn)
    .await?;
    let root = json.get(0).ok_or_else(|| anyhow!("an empty EXPLAIN"))?;
    let number = |key: &str| {
        root.get(key)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
    };
    let plan = root.get("Plan");
    let blocks = |key: &str| {
        plan.and_then(|p| p.get(key))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
    };
    Ok((
        number("Planning Time"),
        number("Execution Time"),
        text.join("\n"),
        format!(
            "{} hit, {} read",
            blocks("Shared Hit Blocks"),
            blocks("Shared Read Blocks")
        ),
    ))
}

/// The first of `times` and the median of the rest, in ms.
fn first_and_median(times: Vec<f64>) -> (f64, f64) {
    let first = times.first().copied().unwrap_or_default();
    let mut warm: Vec<f64> = times.into_iter().skip(1).collect();
    warm.sort_by(f64::total_cmp);
    let median = warm
        .get(warm.len().div_euclid(2))
        .copied()
        .unwrap_or_default();
    (first, median)
}

/// The composition read by id, through the service as the REST read calls it.
async fn composition_read(svc: &FerroEhrService, ehr: Uuid, vo: Uuid) -> anyhow::Result<String> {
    let mut times = Vec::with_capacity(WARM + 1);
    for _ in 0..=WARM {
        let started = Instant::now();
        svc.read_composition_raw(EhrId(ehr), VoId(vo), None).await?;
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let (first, median) = first_and_median(times);
    Ok(format!(
        "| composition read by id (service, pooled) | 1 | {first:.1} | {median:.1} | | | | | |"
    ))
}

/// The seeded counts and the size of every clinical relation.
async fn store(pool: &PgPool, ehr: Uuid, report: &mut String) -> anyhow::Result<()> {
    let (ehrs, compositions, versions): (i64, i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(ehrs), 0)::bigint, coalesce(sum(compositions), 0)::bigint, \
         coalesce(sum(versions), 0)::bigint FROM scaleseed.chunk",
    )
    .fetch_one(pool)
    .await?;
    let in_ehr: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM clinical.vo_head WHERE ehr_id = $1 AND kind = 'COMPOSITION'",
    )
    .bind(ehr)
    .fetch_one(pool)
    .await?;
    writeln!(
        report,
        "Seeded {ehrs} EHRs, {compositions} compositions and {versions} versions (EHR_STATUS \
         included); the measured EHR holds {in_ehr} compositions.\n"
    )?;
    writeln!(
        report,
        "| Relation | Rows (estimate) | Table | Indexes | Total |"
    )?;
    writeln!(report, "| --- | ---: | ---: | ---: | ---: |")?;
    let sizes: Vec<(String, f32, String, String, String)> = sqlx::query_as(
        "SELECT c.relname::text, c.reltuples, pg_size_pretty(pg_table_size(c.oid)), \
           pg_size_pretty(pg_indexes_size(c.oid)), pg_size_pretty(pg_total_relation_size(c.oid)) \
         FROM pg_class c WHERE c.relnamespace = 'clinical'::regnamespace AND c.relkind = 'r' \
         ORDER BY pg_total_relation_size(c.oid) DESC LIMIT 12",
    )
    .fetch_all(pool)
    .await?;
    for (name, rows, table, indexes, total) in sizes {
        writeln!(
            report,
            "| {name} | {rows:.0} | {table} | {indexes} | {total} |"
        )?;
    }
    let database: String =
        sqlx::query_scalar("SELECT pg_size_pretty(pg_database_size(current_database()))")
            .fetch_one(pool)
            .await?;
    writeln!(report, "\nThe whole database: {database}.")?;
    Ok(())
}

/// The PostgreSQL version and the settings that shape the plans.
async fn settings(pool: &PgPool, report: &mut String) -> anyhow::Result<()> {
    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(pool)
        .await?;
    writeln!(report, "{version}\n")?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, setting || coalesce(unit, '') FROM pg_settings WHERE name IN \
         ('shared_buffers', 'work_mem', 'effective_cache_size', 'random_page_cost', \
          'max_parallel_workers_per_gather', 'jit') ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    for (name, value) in rows {
        writeln!(report, "- `{name}` = {value}")?;
    }
    Ok(())
}

/// What `ANALYZE` costs on the hot partitions and the parents, and what
/// autovacuum has done on them.
async fn analyze(pool: &PgPool, report: &mut String) -> anyhow::Result<()> {
    writeln!(report, "| Relation | ANALYZE ms |")?;
    writeln!(report, "| --- | ---: |")?;
    for relation in [
        "clinical.node_hot",
        "clinical.version_hot",
        "clinical.vo_head",
        "clinical.node",
        "clinical.version",
    ] {
        let started = Instant::now();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ANALYZE {relation}")))
            .execute(pool)
            .await?;
        writeln!(
            report,
            "| {relation} | {:.0} |",
            started.elapsed().as_secs_f64() * 1000.0
        )?;
    }
    writeln!(
        report,
        "\n| Relation | Dead rows | Last autovacuum | Last autoanalyze |"
    )?;
    writeln!(report, "| --- | ---: | --- | --- |")?;
    let rows: Vec<(String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT relname::text, n_dead_tup, last_autovacuum::text, last_autoanalyze::text \
         FROM pg_stat_user_tables WHERE schemaname = 'clinical' \
           AND relname IN ('node_hot', 'version_hot', 'vo_head', 'commit_audit', 'contribution') \
         ORDER BY relname",
    )
    .fetch_all(pool)
    .await?;
    for (name, dead, vacuum, analyze) in rows {
        writeln!(
            report,
            "| {name} | {dead} | {} | {} |",
            vacuum.unwrap_or_else(|| "never".to_owned()),
            analyze.unwrap_or_else(|| "never".to_owned())
        )?;
    }
    Ok(())
}
