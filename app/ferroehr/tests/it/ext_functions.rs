// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ext` openEHR value helpers, against the generation they replace.
//!
//! The helpers used to be PL/pgSQL bodies ending in
//! `EXCEPTION WHEN others THEN RETURN NULL`, which opens a subtransaction on
//! every call; five of the seven now reach every cast through a guard instead,
//! and the two whose whole body is "parse a date or a time" keep the trap
//! because it measured cheaper than any guard that would replace it. Three
//! things are proven here against a real database, none of them from a recorded
//! fixture: the first generation's own bodies are installed beside the current
//! ones and both are read over the same corpus, so every agreement and every
//! divergence is live; the divergence set is pinned value by value; and exactly
//! the two measured parsers trap an error, while the ones written as a single
//! SQL expression fold into the plan of a representative AQL predicate.

#![expect(
    clippy::expect_used,
    reason = "integration tests fail loudly on harness errors"
)]

use sqlx::Row as _;
use sqlx::{Executor as _, PgPool, Postgres, Transaction};

/// The first generation's function bodies, verbatim.
///
/// Byte-identical to the seven `CREATE FUNCTION` statements of
/// `app/ferroehr/migrations/ext/0001_openehr_functions.sql` at the `v4.3.0`
/// release tag, which is the last release that shipped them. They are created
/// in a throwaway `gen1` schema that leads the transaction's `search_path`, so
/// the unqualified calls they make to each other reach the first generation and
/// not the current one, and nothing about them had to be rewritten to be
/// compared.
const GENERATION_1_BODIES: &str = r"
CREATE FUNCTION openehr_date_days(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    y integer; m integer := 1; d integer := 1;
    s text := replace(v, '-', '');
BEGIN
    IF length(s) < 4 THEN RETURN NULL; END IF;
    y := substring(s FROM 1 FOR 4)::integer;
    IF length(s) >= 6 THEN m := substring(s FROM 5 FOR 2)::integer; END IF;
    IF length(s) >= 8 THEN d := substring(s FROM 7 FOR 2)::integer; END IF;
    RETURN make_date(y, m, d) - DATE '0001-01-01';
EXCEPTION WHEN others THEN
    RETURN NULL;
END $$;

CREATE FUNCTION openehr_time_seconds(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    t text := regexp_replace(v, '([Zz]|[+-]\d{2}:?\d{0,2})$', '');
    parts text[];
    h numeric := 0; m numeric := 0; s numeric := 0;
BEGIN
    t := regexp_replace(t, '^[Tt]', '');
    IF t !~ ':' AND length(t) > 2 THEN
        -- compact HH[MM[SS]]
        h := substring(t FROM 1 FOR 2)::numeric;
        IF length(t) >= 4 THEN m := substring(t FROM 3 FOR 2)::numeric; END IF;
        IF length(t) >= 6 THEN s := substring(t FROM 5)::numeric; END IF;
    ELSE
        parts := string_to_array(t, ':');
        IF array_length(parts, 1) >= 1 AND parts[1] <> '' THEN h := parts[1]::numeric; END IF;
        IF array_length(parts, 1) >= 2 THEN m := parts[2]::numeric; END IF;
        IF array_length(parts, 1) >= 3 THEN s := parts[3]::numeric; END IF;
    END IF;
    RETURN h * 3600 + m * 60 + s;
EXCEPTION WHEN others THEN
    RETURN NULL;
END $$;

CREATE FUNCTION openehr_tz_offset_seconds(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    m text[];
BEGIN
    IF v ~ '[Zz]$' THEN RETURN 0; END IF;
    m := regexp_match(v, '([+-])(\d{2}):?(\d{2})?$');
    IF m IS NULL THEN RETURN 0; END IF;
    RETURN (CASE m[1] WHEN '-' THEN -1 ELSE 1 END)
         * (m[2]::numeric * 3600 + coalesce(m[3], '0')::numeric * 60);
EXCEPTION WHEN others THEN
    RETURN 0;
END $$;

CREATE FUNCTION openehr_date_time_seconds(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    date_part text := split_part(v, 'T', 1);
    time_part text := split_part(v, 'T', 2);
    days numeric;
    sod numeric := 0;
    off numeric := 0;
BEGIN
    days := openehr_date_days(date_part);
    IF days IS NULL THEN RETURN NULL; END IF;
    IF time_part <> '' THEN
        sod := coalesce(openehr_time_seconds(time_part), 0);
        off := openehr_tz_offset_seconds(time_part);
    END IF;
    RETURN days * 86400 + sod - off;
END $$;

CREATE FUNCTION openehr_duration_seconds(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    m text[];
    sign numeric := 1;
    date_part text;
    time_part text;
    total numeric := 0;
BEGIN
    m := regexp_match(v, '^(-)?P([^T]*)(?:T(.*))?$');
    IF m IS NULL THEN RETURN NULL; END IF;
    IF m[1] = '-' THEN sign := -1; END IF;
    date_part := coalesce(m[2], '');
    time_part := coalesce(m[3], '');

    total := total + coalesce((regexp_match(date_part, '(\d+(?:\.\d+)?)Y'))[1]::numeric, 0) * 365.24 * 86400;
    total := total + coalesce((regexp_match(date_part, '(\d+(?:\.\d+)?)M'))[1]::numeric, 0) * 30.42 * 86400;
    total := total + coalesce((regexp_match(date_part, '(\d+(?:\.\d+)?)W'))[1]::numeric, 0) * 7 * 86400;
    total := total + coalesce((regexp_match(date_part, '(\d+(?:\.\d+)?)D'))[1]::numeric, 0) * 86400;
    total := total + coalesce((regexp_match(time_part, '(\d+(?:\.\d+)?)H'))[1]::numeric, 0) * 3600;
    total := total + coalesce((regexp_match(time_part, '(\d+(?:\.\d+)?)M'))[1]::numeric, 0) * 60;
    total := total + coalesce((regexp_match(time_part, '(\d+(?:\.\d+)?)S'))[1]::numeric, 0);
    RETURN sign * total;
EXCEPTION WHEN others THEN
    RETURN NULL;
END $$;

CREATE FUNCTION openehr_magnitude(dv jsonb) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    t text := dv->>'_type';
    denom numeric;
BEGIN
    CASE t
        WHEN 'DV_QUANTITY', 'DV_COUNT' THEN
            RETURN (dv->>'magnitude')::numeric;
        WHEN 'DV_ORDINAL', 'DV_SCALE' THEN
            RETURN (dv->>'value')::numeric;
        WHEN 'DV_PROPORTION' THEN
            denom := (dv->>'denominator')::numeric;
            IF denom = 0 THEN RETURN NULL; END IF; -- NOTE: NULL, not MAX
            RETURN (dv->>'numerator')::numeric / denom;
        WHEN 'DV_DATE' THEN
            RETURN openehr_date_days(dv->>'value');
        WHEN 'DV_TIME' THEN
            RETURN openehr_time_seconds(dv->>'value');
        WHEN 'DV_DATE_TIME' THEN
            RETURN openehr_date_time_seconds(dv->>'value');
        WHEN 'DV_DURATION' THEN
            RETURN openehr_duration_seconds(dv->>'value');
        ELSE
            RETURN NULL;
    END CASE;
EXCEPTION WHEN others THEN
    RETURN NULL;
END $$;

CREATE FUNCTION openehr_timestamp(v text) RETURNS timestamptz
LANGUAGE plpgsql STABLE STRICT PARALLEL SAFE AS $$
DECLARE
    s text := replace(v, ',', '.');
    date_part text;
    time_part text;
    dcomp text;
    y int; mo int := 1; d int := 1;
    sod numeric := 0;
    ts timestamp;
BEGIN
    BEGIN
        RETURN s::timestamptz;
    EXCEPTION WHEN others THEN
        NULL; -- reduced precision or malformed: fall through to the completion
    END;
    IF s ~ '^[Tt]' OR s ~ '^\d{1,2}:' THEN
        -- time-only: floor onto 0001-01-01
        time_part := regexp_replace(s, '^[Tt]', '');
        y := 1;
    ELSE
        date_part := split_part(s, 'T', 1);
        time_part := split_part(s, 'T', 2);
        dcomp := replace(date_part, '-', '');
        IF dcomp !~ '^(\d{4}|\d{6}|\d{8})$' THEN
            RETURN NULL;
        END IF;
        y := substring(dcomp FROM 1 FOR 4)::int;
        IF length(dcomp) >= 6 THEN mo := substring(dcomp FROM 5 FOR 2)::int; END IF;
        IF length(dcomp) >= 8 THEN d := substring(dcomp FROM 7 FOR 2)::int; END IF;
    END IF;
    IF time_part <> '' THEN
        sod := openehr_time_seconds(time_part);
        IF sod IS NULL THEN RETURN NULL; END IF;
    END IF;
    ts := make_date(y, mo, d)::timestamp + make_interval(secs => sod::double precision);
    IF time_part ~ '([Zz]|[+-]\d{2}:?\d{0,2})$' THEN
        RETURN (ts - make_interval(secs => openehr_tz_offset_seconds(time_part)::double precision))
               AT TIME ZONE 'UTC';
    END IF;
    -- offset-less: session-TimeZone reading, matching the native-cast branch
    RETURN ts::timestamptz;
EXCEPTION WHEN others THEN
    RETURN NULL;
END;
$$;
";

/// The parser corpus: the ISO 8601 forms openEHR uses (BASE `foundation_types`
/// master06-time_types.adoc), their reduced-precision and basic-format
/// variants, the calendar edges, and text that is no temporal value at all —
/// the refusals matter as much as the readings, because a refusal is the NULL
/// that makes an AQL comparison miss instead of erroring.
const CORPUS: &[&str] = &[
    // Dates: extended, basic, reduced precision, the calendar edges.
    "2021-01-02",
    "20210102",
    "2021-01",
    "202101",
    "2021",
    "0001-01-01",
    "9999-12-31",
    "2020-02-29",
    "2021-02-29",
    "2021-02-30",
    "2021-13-01",
    "2021-00-05",
    "2021-01-00",
    "2021-04-31",
    "2020-12-31",
    "0000-01-01",
    "2021-1-2",
    "2021-002",
    "2021-0102",
    "2021-W01-1",
    // Dates that are not dates.
    "abcd",
    "202",
    "",
    "  12",
    "+2021-01-02",
    "2021abc",
    "202101ab",
    "20210102XYZ",
    "2021a",
    "202101a",
    "2021-01-02 BC",
    "2021-01-02 Europe/Amsterdam",
    // Date-times, every separator and every offset form.
    "2021-01-02T10:30:45",
    "2021-01-02T10:30:45Z",
    "2021-01-02T10:30:45.123Z",
    "2021-01-02T10:30:45,123Z",
    "2021-01-02T10:30:45.123456+01:30",
    "2021-01-02T10:30:45+02:00",
    "2021-01-02T10:30:45-0530",
    "2021-01-02T10:30:45+05",
    "20210102T103045Z",
    "20210102T103045",
    "20210102103045",
    "20210102T1030",
    "2021-01-02T10",
    "2021-01-02T10:30",
    "2021-01-02 10:30:45",
    "2021-01-02t10:30:45",
    "2021-01-02T",
    "2021t10:00",
    "2021 10:30",
    "2021-12-31T23:59:59.999Z",
    "0001-01-01T00:00:00Z",
    "2021-02-30T10:00:00Z",
    "2021-01-02T25:00:00",
    // The two suffixes PostgreSQL's own parser reads after a full-precision
    // date-time and BASE foundation_types master06-time_types.adoc
    // §Iso8601_date_time does not define: a zone NAME and a ` BC` era.
    "2021-01-02T10:30:45 Europe/Amsterdam",
    "2021-01-02T10:30:45 BC",
    // The daylight-saving edges of a zone with one: the ambiguous hour and the
    // hour that does not exist.
    "2021-10-31T02:30:00",
    "2021-03-28T02:30:00",
    "2021-10-31 02:30:00",
    // Times: extended, basic, reduced precision, offsets, and the forms that
    // are not times.
    "10:30",
    "10:30:45",
    "10:30:45.5",
    "103045",
    "1030",
    "10",
    "1",
    "T10:30",
    "t10:30:45",
    "10:",
    ":30",
    "10:ab",
    "103",
    "10304",
    "1030a",
    "10304a",
    "1030.45",
    "10304.5",
    "103045.5",
    "1030455",
    "10:30:45:99",
    "::",
    ".",
    "ab",
    "abc",
    "24:00:00",
    "10:30Z",
    "10:30+02:00",
    "10:30-05:30",
    "10:30+0530",
    "103045+05",
    // Durations.
    "P1Y",
    "P1Y2M3DT4H5M6S",
    "-P1Y",
    "P1W",
    "PT1S",
    "P0.5Y",
    "PT1.5H",
    "P",
    "PT",
    "1Y",
    "P1X",
    "P1Y2M",
    "P1M",
    "PT1M",
    "P1YT1M",
    "-PT30M",
    // The words PostgreSQL's own date/time input accepts and openEHR does not.
    "now",
    "yesterday",
    "today",
    "epoch",
    "infinity",
];

/// The canonical `DV_ORDERED` corpus: one value per ordered subtype the RM
/// defines (RM `data_types` master06-quantity_package.adoc), each with the
/// carriers that make it unreadable beside the one that reads.
const ORDERED_CORPUS: &[&str] = &[
    r#"{"_type":"DV_QUANTITY","magnitude":72.5,"units":"kg"}"#,
    r#"{"_type":"DV_QUANTITY","magnitude":-3}"#,
    r#"{"_type":"DV_QUANTITY","magnitude":"abc"}"#,
    r#"{"_type":"DV_QUANTITY"}"#,
    r#"{"_type":"DV_QUANTITY","magnitude":{"a":1}}"#,
    r#"{"_type":"DV_QUANTITY","magnitude":[1,2]}"#,
    r#"{"_type":"DV_COUNT","magnitude":4}"#,
    r#"{"_type":"DV_ORDINAL","value":2}"#,
    r#"{"_type":"DV_SCALE","value":1.5}"#,
    r#"{"_type":"DV_PROPORTION","numerator":1,"denominator":4}"#,
    r#"{"_type":"DV_PROPORTION","numerator":1,"denominator":0}"#,
    r#"{"_type":"DV_PROPORTION","numerator":1}"#,
    r#"{"_type":"DV_PROPORTION","denominator":4}"#,
    r#"{"_type":"DV_PROPORTION","numerator":"x","denominator":"y"}"#,
    r#"{"_type":"DV_DATE","value":"2021-01-02"}"#,
    r#"{"_type":"DV_DATE","value":"2021-02-30"}"#,
    r#"{"_type":"DV_DATE","value":"nope"}"#,
    r#"{"_type":"DV_TIME","value":"10:30:45"}"#,
    r#"{"_type":"DV_TIME","value":"bad"}"#,
    r#"{"_type":"DV_DATE_TIME","value":"2021-01-02T10:30:45Z"}"#,
    r#"{"_type":"DV_DATE_TIME","value":"garbage"}"#,
    r#"{"_type":"DV_DURATION","value":"P1Y2M3DT4H5M6S"}"#,
    r#"{"_type":"DV_DURATION","value":"nope"}"#,
    r#"{"_type":"DV_TEXT","value":"hi"}"#,
    r#"{"value":3}"#,
    "{}",
    "[1,2]",
    r#""scalar""#,
    "3",
];

/// The readings that deliberately differ from the first generation's, as
/// `(function, input, the reading now)` with `None` for NULL.
///
/// Every one of them is the first generation reading something that is not an
/// openEHR value, and the deviations fall into five groups.
///
/// The year came out of `text::integer`, which skips leading whitespace and
/// accepts a sign, so `  12` was the year 12 and `+2021-01-02` the year 202.
/// Neither is an ISO 8601 date (BASE `foundation_types` master06-time_types.adoc
/// and the `Iso8601_date` class table), and the length contract now says the
/// characters it reads are digits.
///
/// The compact-form seconds field came out of `text::numeric`, which accepts a
/// sign, so a DATE handed to the time parser read as a number: `2021-01-02`
/// stripped its `-02` as a zone offset and then read `-01` as its seconds. The
/// field is now unsigned, and a date is not a time.
///
/// `openehr_timestamp` tried PostgreSQL's own date/time input first and used
/// whatever came back, which accepted four things openEHR does not define: a
/// date that is not zero-padded or is ordinal (`2021-1-2`, `2021-002`,
/// `2021-0102`), the words `now`, `today`, `yesterday`, `epoch` and
/// `infinity` — of which the first two made a stored value read as the current
/// time — and, after a full-precision date-time, a zone NAME or a ` BC` era,
/// where BASE `foundation_types` master06-time_types.adoc `§Iso8601_date_time`
/// ends the value at the offset.
///
/// In the other direction the completion arms now accept what that fast path
/// used to reach by accident: the lowercase `t` and space separators on a
/// reduced-precision date, which the cast read only when the whole value was
/// full precision.
///
/// `openehr_date_days` read the leading four, six or eight characters and
/// ignored whatever followed, so a complete date with text after it was that
/// date (`20210102XYZ`, `2021-01-02 BC`, `2021-01-02 Europe/Amsterdam`) and a
/// year with a letter after it was that year (`2021a`, `202101a`, `1030a`).
/// The same section's `Iso8601_date` admits none of them, and the reading now
/// holds the tail to the date-time alphabet, so `openehr_date_time_seconds`
/// (which hands this helper the part before the `T`) refuses them too.
const DIVERGENCES: &[(&str, &str, Option<&str>)] = &[
    ("openehr_date_days", "  12", None),
    ("openehr_date_days", "+2021-01-02", None),
    ("openehr_date_days", "1030a", None),
    ("openehr_date_days", "2021-01-02 BC", None),
    ("openehr_date_days", "2021-01-02 Europe/Amsterdam", None),
    ("openehr_date_days", "2021-01-02T10:30:45 BC", None),
    (
        "openehr_date_days",
        "2021-01-02T10:30:45 Europe/Amsterdam",
        None,
    ),
    ("openehr_date_days", "20210102XYZ", None),
    ("openehr_date_days", "202101a", None),
    ("openehr_date_days", "2021a", None),
    ("openehr_date_time_seconds", "  12", None),
    ("openehr_date_time_seconds", "+2021-01-02", None),
    ("openehr_date_time_seconds", "1030a", None),
    ("openehr_date_time_seconds", "2021-01-02 BC", None),
    (
        "openehr_date_time_seconds",
        "2021-01-02 Europe/Amsterdam",
        None,
    ),
    ("openehr_date_time_seconds", "20210102XYZ", None),
    ("openehr_date_time_seconds", "202101a", None),
    ("openehr_date_time_seconds", "2021a", None),
    ("openehr_time_seconds", "0000-01-01", None),
    ("openehr_time_seconds", "0001-01-01", None),
    ("openehr_time_seconds", "2020-02-29", None),
    ("openehr_time_seconds", "2020-12-31", None),
    ("openehr_time_seconds", "2021-00-05", None),
    ("openehr_time_seconds", "2021-01-00", None),
    ("openehr_time_seconds", "2021-01-02", None),
    ("openehr_time_seconds", "2021-02-29", None),
    ("openehr_time_seconds", "2021-02-30", None),
    ("openehr_time_seconds", "2021-04-31", None),
    ("openehr_time_seconds", "2021-13-01", None),
    ("openehr_time_seconds", "9999-12-31", None),
    (
        "openehr_timestamp",
        "2021 10:30",
        Some("2021-01-01 10:30:00+00"),
    ),
    ("openehr_timestamp", "2021-002", None),
    ("openehr_timestamp", "2021-01-02 BC", None),
    ("openehr_timestamp", "2021-01-02 Europe/Amsterdam", None),
    ("openehr_timestamp", "2021-01-02T10:30:45 BC", None),
    (
        "openehr_timestamp",
        "2021-01-02T10:30:45 Europe/Amsterdam",
        None,
    ),
    ("openehr_timestamp", "2021-0102", None),
    ("openehr_timestamp", "2021-1-2", None),
    (
        "openehr_timestamp",
        "2021t10:00",
        Some("2021-01-01 10:00:00+00"),
    ),
    ("openehr_timestamp", "epoch", None),
    ("openehr_timestamp", "infinity", None),
    ("openehr_timestamp", "now", None),
    ("openehr_timestamp", "today", None),
    ("openehr_timestamp", "yesterday", None),
];

/// The text helpers both generations define, in the order the comparison reads
/// them.
const TEXT_HELPERS: &[&str] = &[
    "openehr_date_days",
    "openehr_time_seconds",
    "openehr_tz_offset_seconds",
    "openehr_date_time_seconds",
    "openehr_duration_seconds",
];

/// Open a transaction carrying the first generation's helpers in a `gen1`
/// schema that leads the `search_path`, so their unqualified calls to each
/// other reach the first generation. The transaction is never committed.
async fn with_generation_1(pool: &PgPool, time_zone: &str) -> Transaction<'static, Postgres> {
    let mut tx = pool.begin().await.expect("open transaction");
    tx.execute("CREATE SCHEMA gen1")
        .await
        .expect("create the gen1 schema");
    tx.execute("SET LOCAL search_path TO gen1, clinical, ext, public")
        .await
        .expect("lead the search_path with gen1");
    tx.execute(sqlx::AssertSqlSafe(format!(
        "SET LOCAL TIME ZONE '{time_zone}'"
    )))
    .await
    .expect("set the session time zone");
    tx.execute(sqlx::AssertSqlSafe(GENERATION_1_BODIES.to_owned()))
        .await
        .expect("install the first generation's bodies");
    tx
}

/// Every `(function, input, reading now)` where the two generations disagree,
/// over the text helpers and `openehr_timestamp`.
async fn text_divergences(
    tx: &mut Transaction<'static, Postgres>,
) -> Vec<(String, String, Option<String>)> {
    let mut found = Vec::new();
    for helper in TEXT_HELPERS {
        let sql = format!(
            "SELECT v, gen1.{helper}(v)::text AS old, ext.{helper}(v)::text AS new \
             FROM unnest($1::text[]) AS t(v) \
             WHERE gen1.{helper}(v) IS DISTINCT FROM ext.{helper}(v)"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(CORPUS)
            .fetch_all(&mut **tx)
            .await
            .expect("compare the generations over the corpus");
        for row in rows {
            found.push((
                (*helper).to_owned(),
                row.get::<String, _>("v"),
                row.get::<Option<String>, _>("new"),
            ));
        }
    }
    let rows = sqlx::query(
        "SELECT v, gen1.openehr_timestamp(v)::text AS old, ext.openehr_timestamp(v)::text AS new \
         FROM unnest($1::text[]) AS t(v) \
         WHERE gen1.openehr_timestamp(v) IS DISTINCT FROM ext.openehr_timestamp(v)",
    )
    .bind(CORPUS)
    .fetch_all(&mut **tx)
    .await
    .expect("compare openehr_timestamp over the corpus");
    for row in rows {
        found.push((
            "openehr_timestamp".to_owned(),
            row.get::<String, _>("v"),
            row.get::<Option<String>, _>("new"),
        ));
    }
    found.sort();
    found
}

/// The `CREATE FUNCTION` statements of `sql`, each from its `CREATE FUNCTION`
/// line through the line that closes the body (`$$;`), with the prose between
/// them dropped.
fn create_function_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in sql.lines() {
        if line.starts_with("CREATE FUNCTION") {
            current = Some(Vec::new());
        }
        if let Some(body) = current.as_mut() {
            body.push(line);
            if line.trim_end().ends_with("$$;") {
                out.push(body.join("\n"));
                current = None;
            }
        }
    }
    out
}

/// [`GENERATION_1_BODIES`] is byte-identical to the file it claims to copy.
///
/// Without this the comparison every other test here runs could be measuring a
/// re-vendored or hand-edited constant against the current helpers and still
/// pass, while proving nothing about the generation it names. The file is read
/// from the `v4.3.0` tag, the last release that shipped it; where the tag is
/// unreachable — a shallow clone fetches none — the test says so and stops
/// rather than passing quietly on an unread file.
#[test]
fn generation_1_bodies_are_the_v4_3_0_file() {
    const TAG: &str = "v4.3.0";
    const PATH: &str = "app/ferroehr/migrations/ext/0001_openehr_functions.sql";

    let shown = std::process::Command::new("git")
        .args(["show", &format!("{TAG}:{PATH}")])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run git show");
    if !shown.status.success() {
        eprintln!(
            "skipping: `git show {TAG}:{PATH}` failed, so the first generation's file cannot be \
             read here (a shallow clone carries no tags): {}",
            String::from_utf8_lossy(&shown.stderr).trim()
        );
        return;
    }
    let file = String::from_utf8(shown.stdout).expect("the migration file is UTF-8");

    let at_tag = create_function_statements(&file);
    let embedded = create_function_statements(GENERATION_1_BODIES);
    assert_eq!(
        embedded.len(),
        TEXT_HELPERS.len() + 2,
        "the constant must hold the seven first-generation helpers"
    );
    assert_eq!(
        embedded, at_tag,
        "GENERATION_1_BODIES is no longer byte-identical to {PATH} at {TAG}, so every comparison \
         in this file is running against something other than the first generation"
    );
}

#[tokio::test]
async fn the_corpus_reads_as_the_first_generation_read_it() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut tx = with_generation_1(&pool, "UTC").await;

    let found = text_divergences(&mut tx).await;
    let expected: Vec<(String, String, Option<String>)> = DIVERGENCES
        .iter()
        .map(|(helper, input, new)| {
            (
                (*helper).to_owned(),
                (*input).to_owned(),
                new.map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        found, expected,
        "the two generations read the corpus identically except for the pinned deviations"
    );

    let rows = sqlx::query(
        "SELECT d::text AS dv, gen1.openehr_magnitude(d)::text AS old, \
                ext.openehr_magnitude(d)::text AS new \
         FROM unnest($1::text[]) AS t(v), LATERAL (SELECT v::jsonb) AS j(d) \
         WHERE gen1.openehr_magnitude(d) IS DISTINCT FROM ext.openehr_magnitude(d)",
    )
    .bind(ORDERED_CORPUS)
    .fetch_all(&mut *tx)
    .await
    .expect("compare openehr_magnitude over the ordered corpus");
    let mismatches: Vec<String> = rows.iter().map(|row| row.get::<String, _>("dv")).collect();
    assert!(
        mismatches.is_empty(),
        "openehr_magnitude reads every ordered value as the first generation did; differs on {mismatches:?}"
    );
}

#[tokio::test]
async fn the_deviations_do_not_depend_on_the_session_time_zone() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut tx = with_generation_1(&pool, "Europe/Amsterdam").await;

    let found: Vec<(String, String)> = text_divergences(&mut tx)
        .await
        .into_iter()
        .map(|(helper, input, _)| (helper, input))
        .collect();
    let expected: Vec<(String, String)> = DIVERGENCES
        .iter()
        .map(|(helper, input, _)| ((*helper).to_owned(), (*input).to_owned()))
        .collect();
    assert_eq!(
        found, expected,
        "a zone with daylight saving changes which values read, not which values deviate"
    );
}

#[tokio::test]
async fn null_reads_as_null() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    // The helpers written as a single SQL expression are not STRICT, because a
    // strict function whose body holds a CASE is never folded into the calling
    // query. Each of those answers NULL itself; the PL/pgSQL ones are STRICT
    // and PostgreSQL answers for them.
    for helper in TEXT_HELPERS {
        let sql = format!("SELECT ext.{helper}(NULL::text) IS NULL AS null_in_null_out");
        let is_null: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&pool)
            .await
            .expect("read the helper with NULL input");
        assert!(is_null, "ext.{helper}(NULL) reads as NULL");
    }
    for sql in [
        "SELECT ext.openehr_timestamp(NULL::text) IS NULL",
        "SELECT ext.openehr_magnitude(NULL::jsonb) IS NULL",
        "SELECT ext.openehr_numeric(NULL::text) IS NULL",
    ] {
        let is_null: bool = sqlx::query_scalar(sql)
            .fetch_one(&pool)
            .await
            .expect("read the helper with NULL input");
        assert!(is_null, "{sql} holds");
    }
}

/// The helpers whose measurement earned them an error trap, in `pg_proc` order.
///
/// A trap opens a subtransaction on every call, so it survives in `ext` only
/// where the trapped cast measured cheaper than any guard that would let the
/// same cast run untrapped — which is the case for exactly these two, whose
/// whole body is "parse a date or a time".
const TRAPPING_HELPERS: [&str; 2] = ["openehr_date_days", "openehr_timestamp"];

#[tokio::test]
async fn exactly_the_two_measured_parsers_trap_an_error() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    // Both ends are asserted from the database itself: which functions carry an
    // EXCEPTION clause, read from their source, and whether each of those
    // carries the measurement that justifies it, read from its comment. A trap
    // added to a third helper fails the first assertion; a justification
    // dropped from one of these two fails the second.
    let trapping: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.proname::text, coalesce(obj_description(p.oid, 'pg_proc'), '') \
         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = 'ext' AND p.prosrc ILIKE '%exception%' \
           AND NOT EXISTS (SELECT FROM pg_depend d \
                           WHERE d.objid = p.oid AND d.deptype = 'e') \
         ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("read the ext function inventory");

    let names: Vec<&str> = trapping.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names, TRAPPING_HELPERS,
        "only the helpers whose measurement earned a trap carry one"
    );
    for (name, comment) in &trapping {
        assert!(
            comment.contains("error trap"),
            "ext.{name} says in its comment that it traps: {comment}"
        );
        assert!(
            comment.matches(" ms").count() >= 3,
            "ext.{name} carries the measurement that earned the trap — this reading \
             and the readings it was measured against: {comment}"
        );
    }
}

#[tokio::test]
async fn the_ordered_magnitude_folds_into_an_aql_predicate() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    // The shape the AQL emitter builds for Coercion::Magnitude: the ordered
    // magnitude of a leaf over a jsonb path extraction from `node.data`
    // (app/ferroehr/src/aql/sql/value.rs). The helper reads its argument twice,
    // so folding it is a win and the planner does it.
    let sql = "EXPLAIN (VERBOSE, COSTS OFF) SELECT count(*) FROM node n \
               WHERE ext.openehr_magnitude(jsonb_path_query_first(n.data, '$.value'::jsonpath)) > 5";
    let rows = sqlx::query(sql)
        .fetch_all(&pool)
        .await
        .expect("explain the predicate");
    let plan: String = rows
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !plan.contains("openehr_magnitude("),
        "the predicate leaves no call to the helper in the plan:\n{plan}"
    );
    assert!(
        plan.contains("'DV_QUANTITY'"),
        "the predicate carries the helper's own dispatch in the plan:\n{plan}"
    );
}
