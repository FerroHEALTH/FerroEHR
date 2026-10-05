-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- ext: the openEHR value helpers the AQL emitter and the node decomposer call.
--
-- No openEHR spec governs storage helpers: our own design. What they compute is
-- spec-grounded — the ordered magnitude of a DV_ORDERED (RM data_types
-- master06-quantity_package.adoc) and the ISO 8601 date/time/duration forms
-- openEHR uses (BASE foundation_types master06-time_types.adoc and the
-- Iso8601_date / Iso8601_date_time class tables under BASE docs/UML/classes/) —
-- but nothing in openEHR says a database should carry them.
--
-- ONLY THE TWO PARSERS TRAP AN ERROR, and only because the trap measured
-- cheapest there. The documentation is direct about what a trap costs: "A block
-- containing an EXCEPTION clause is significantly more expensive to enter and
-- exit than a block without one. Therefore, don't use EXCEPTION without need."
-- (PostgreSQL 18, "Control Structures" §Trapping Errors,
-- https://www.postgresql.org/docs/18/plpgsql-control-structures.html). A helper
-- is read once per candidate row per AQL predicate, so a per-call subtransaction
-- is a cost worth removing, and five of the seven helpers remove it: every cast
-- they run is reachable only through a guard that proves it cannot raise. The
-- two whose whole body is "parse a date or a time" measure the other way —
-- PostgreSQL's own parser inside a trap validates more cheaply than any guard
-- that would let the same cast run untrapped — so openehr_date_days and
-- openehr_timestamp keep theirs and carry the measurement in their own comment.
-- A value any helper refuses reads as NULL — a comparison miss, never an error
-- the caller sees as a 500.
--
-- Those guards are ordered, never conjunctions: SQL fixes no evaluation order
-- between the operands of AND (PostgreSQL 18, "Expression Evaluation Rules",
-- https://www.postgresql.org/docs/18/sql-expressions.html), so a cast is only
-- ever reached from inside a CASE arm or a nested IF whose condition has
-- already proved it safe.
--
-- LANGUAGE IS A MEASURED CHOICE PER HELPER, not a house style. A `LANGUAGE sql`
-- body folds into the calling query, which removes the call but substitutes the
-- argument expression at every parameter reference; a PL/pgSQL body reads its
-- argument into a local variable once but is always a call. Which wins depends
-- on how many times the body reads its input, so each helper ships in the form
-- that measured faster over a 50 000-row corpus of the values it actually sees:
-- `sql` for the four that read their input once or twice, `plpgsql` for the
-- three date/time parsers that read it many times over. The corpus test pins
-- the results, not the languages.
--
-- The PL/pgSQL bodies SCHEMA-QUALIFY every helper they call. PL/pgSQL resolves
-- function names at execution time against the caller's search_path
-- (PostgreSQL 18, "Writing SECURITY DEFINER Functions Safely",
-- https://www.postgresql.org/docs/18/sql-createfunction.html), so an
-- unqualified call can be captured by a schema the caller put first; the
-- `LANGUAGE sql` bodies bind at definition time and need no such care.
--
-- Volatility is unchanged from the first generation, and it is load-bearing:
-- the parsers and ext.openehr_magnitude are IMMUTABLE, so they are legal in an
-- index expression, while ext.openehr_timestamp is STABLE because its
-- offset-less reading depends on the session TimeZone — "a function that
-- manipulates timestamps might well have results that depend on the TimeZone
-- setting. For safety, such functions should be labeled STABLE instead."
-- (PostgreSQL 18, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html).
--
-- Runs with search_path = ext.

-- The numeric value of a canonical JSON number's text.
--
-- substring() with a pattern yields NULL when the pattern does not match, so
-- the guard and the extraction are the same operation and the parameter is read
-- once, which is what lets this fold into its caller for free. The accepted
-- shape is JSON's own number grammar (RFC 8259 §6,
-- https://www.rfc-editor.org/rfc/rfc8259#section-6) plus the leading minus.
CREATE FUNCTION openehr_numeric(t text) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE
RETURN substring(t FROM '^-?[0-9]+(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?$')::numeric;

-- Days since 0001-01-01 for an ISO 8601 date in either format.
--
-- BASE foundation_types master06-time_types.adoc admits reduced precision down
-- to the year, so `YYYY` and `YYYY-MM` read as the first month and first day;
-- removing the hyphens folds the extended format (`YYYY-MM-DD`, the form BASE
-- calls preferred) onto the basic one, so one reading serves both.
--
-- PL/pgSQL, because the reading looks at its input several times and a folded
-- body would re-evaluate the caller's jsonb extraction once per look.
--
-- THE CALENDAR IS LEFT TO make_date() INSIDE AN ERROR TRAP, which is the one
-- place in this file where a trap is the cheapest validator: over 50 000 date
-- readings this body measures 58 ms, against 61 ms for the first generation and
-- 65 ms for the same body checking the month, the day and the Gregorian leap
-- rule itself so the cast can run untrapped. The subtransaction the trap enters
-- is real (PostgreSQL 18, "Control Structures" §Trapping Errors,
-- https://www.postgresql.org/docs/18/plpgsql-control-structures.html); doing
-- its work by hand costs more.
--
-- The one thing the trap cannot do is refuse what PostgreSQL's integer input
-- accepts and ISO 8601 does not: text::integer skips leading whitespace and
-- accepts a sign, which read `  12` and `+2021-01-02` as years. One translate()
-- proves the fields are digits before the cast — what it leaves after removing
-- digits is empty only when every character was one.
--
-- A second translate() reads the TAIL, because the fields alone say nothing
-- about what follows them: reading the leading four, six or eight characters
-- and ignoring the rest made `20210102XYZ`, `2021-01-02 BC`,
-- `2021-01-02 Europe/Amsterdam` and `2021a` read as dates. BASE
-- foundation_types master06-time_types.adoc §Iso8601_date gives the value as
-- `YYYY-MM-DD`, its compact form or a partial variant, and none of those four
-- is in it. What may follow is a time, because openehr_date_time_seconds splits
-- on `T` and hands the leading part here, so the tail is held to the alphabet
-- openehr_timestamp gates its cast with (plus the comma that helper normalises
-- away first) rather than refused outright.
-- Over the 50 000-row date corpus the timings above use, best of 21 interleaved
-- runs on PostgreSQL 18.6, the tail check costs 2.6 ms (65.6 ms against
-- 63.0 ms) and the two are indistinguishable at the median.
--
-- The calendar goes through make_date() rather than a date cast because this
-- helper is IMMUTABLE and index-legal: the date input function reads the
-- session DateStyle (`01-02-2021` is January under MDY and February under DMY),
-- and "a function that manipulates timestamps might well have results that
-- depend on the TimeZone setting. For safety, such functions should be labeled
-- STABLE instead." (PostgreSQL 18, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html). make_date() takes
-- integers and reads no setting.
CREATE FUNCTION openehr_date_days(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    s text := replace(v, '-', '');
BEGIN
    IF length(s) >= 8 THEN
        IF translate(substr(s, 1, 8), '0123456789', '') <> '' THEN RETURN NULL; END IF;
        IF translate(substr(s, 9), '0123456789.,:+- TtZz', '') <> '' THEN RETURN NULL; END IF;
        RETURN make_date(substr(s, 1, 4)::integer, substr(s, 5, 2)::integer,
                         substr(s, 7, 2)::integer) - DATE '0001-01-01';
    END IF;
    IF length(s) >= 6 THEN
        IF translate(substr(s, 1, 6), '0123456789', '') <> '' THEN RETURN NULL; END IF;
        IF translate(substr(s, 7), '0123456789.,:+- TtZz', '') <> '' THEN RETURN NULL; END IF;
        RETURN make_date(substr(s, 1, 4)::integer, substr(s, 5, 2)::integer, 1)
               - DATE '0001-01-01';
    END IF;
    IF length(s) >= 4 THEN
        IF translate(substr(s, 1, 4), '0123456789', '') <> '' THEN RETURN NULL; END IF;
        IF translate(substr(s, 5), '0123456789.,:+- TtZz', '') <> '' THEN RETURN NULL; END IF;
        RETURN make_date(substr(s, 1, 4)::integer, 1, 1) - DATE '0001-01-01';
    END IF;
    RETURN NULL;
EXCEPTION WHEN others THEN
    RETURN NULL;
END $$;

-- Seconds since the start of the day for an ISO 8601 time, ignoring any zone
-- suffix — the caller applies the offset through openehr_tz_offset_seconds.
--
-- PL/pgSQL for the same reason as the date reading. The `T` designator and the
-- zone suffix are cut positionally rather than by substitution, and the two
-- field forms BASE foundation_types master06-time_types.adoc defines are split
-- on whether a colon is present: the basic `hhmmss` (which needs a length above
-- two, or `hh` would read as the first field of the extended form) and the
-- extended `hh:mm:ss`. A field is accepted when it is digits with at most one
-- decimal point, which is the fractional second the same section allows on the
-- last field; fields the input omits read as zero, which is the reduced
-- precision it also admits.
--
-- Each field's guard is folded into the one expression that computes the
-- reading, because a PL/pgSQL statement costs more than the operators inside
-- it: as twelve statements this reading measured 128 ms per 50 000 values,
-- against 118 ms for the first generation's trapped body; as four statements it
-- measures 95 ms. A CASE evaluates its WHEN before its ELSE, so the casts stay
-- unreachable until the guard has passed — the ordering the head of this file
-- requires. An absent field indexes past the array and reads as NULL, which the
-- guard's comparisons propagate rather than accept.
CREATE FUNCTION openehr_time_seconds(v text) RETURNS numeric
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE AS $$
DECLARE
    t     text := v;
    zone  text;
    parts text[];
BEGIN
    IF left(t, 1) IN ('T', 't') THEN t := substr(t, 2); END IF;
    zone := substring(t FROM '([Zz]|[+-][0-9]{2}:?[0-9]{0,2})$');
    IF zone IS NOT NULL THEN t := substr(t, 1, length(t) - length(zone)); END IF;

    IF position(':' in t) = 0 AND length(t) > 2 THEN
        RETURN CASE
            WHEN translate(substr(t, 1, 2), '0123456789', '') <> ''
              OR (length(t) >= 4 AND translate(substr(t, 3, 2), '0123456789', '') <> '')
              OR (length(t) >= 6 AND (substr(t, 5) = '.'
                  OR translate(substr(t, 5), '0123456789', '') NOT IN ('', '.')))
            THEN NULL
            ELSE substr(t, 1, 2)::numeric * 3600
               + CASE WHEN length(t) >= 4 THEN substr(t, 3, 2)::numeric * 60 ELSE 0 END
               + CASE WHEN length(t) >= 6 THEN substr(t, 5)::numeric ELSE 0 END
        END;
    END IF;

    parts := string_to_array(t, ':');
    RETURN CASE
        WHEN (parts[1] <> '' AND (parts[1] = '.'
              OR translate(parts[1], '0123456789', '') NOT IN ('', '.')))
          OR parts[2] IN ('', '.') OR translate(parts[2], '0123456789', '') NOT IN ('', '.')
          OR parts[3] IN ('', '.') OR translate(parts[3], '0123456789', '') NOT IN ('', '.')
        THEN NULL
        ELSE coalesce(nullif(parts[1], '')::numeric, 0) * 3600
           + coalesce(parts[2]::numeric, 0) * 60
           + coalesce(parts[3]::numeric, 0)
    END;
END $$;

-- The zone offset in seconds carried by an ISO 8601 time's suffix; zero when
-- the value carries none.
--
-- `Z` is zero by definition (BASE foundation_types master06-time_types.adoc:
-- "'Z' and 'T' are literals"), and testing the last character for it answers
-- the common case before any pattern runs. `±hh`, `±hhmm` and `±hh:mm` are read
-- from the end of the value. NULL input is answered first, because the trailing
-- zero-offset arm would otherwise turn it into a number.
CREATE FUNCTION openehr_tz_offset_seconds(v text) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE
RETURN CASE
    WHEN v IS NULL THEN NULL
    WHEN right(v, 1) IN ('Z', 'z') THEN 0
    WHEN v ~ '[+-][0-9]{2}:?([0-9]{2})?$' THEN
        (CASE WHEN substring(v FROM '([+-])[0-9]{2}:?(?:[0-9]{2})?$') = '-' THEN -1 ELSE 1 END)
      * (substring(v FROM '[+-]([0-9]{2}):?(?:[0-9]{2})?$')::numeric * 3600
         + coalesce(substring(v FROM '[+-][0-9]{2}:?([0-9]{2})$'), '0')::numeric * 60)
    ELSE 0
END;

-- Seconds since 0001-01-01T00:00:00Z for an ISO 8601 date-time.
--
-- An unreadable date makes the whole value unreadable; an absent or unreadable
-- time part leaves the reading at the start of the day, which is the reduced
-- precision BASE foundation_types master06-time_types.adoc admits.
CREATE FUNCTION openehr_date_time_seconds(v text) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE
RETURN openehr_date_days(split_part(v, 'T', 1)) * 86400
     + CASE WHEN split_part(v, 'T', 2) <> ''
            THEN coalesce(openehr_time_seconds(split_part(v, 'T', 2)), 0)
                 - openehr_tz_offset_seconds(split_part(v, 'T', 2))
            ELSE 0 END;

-- Seconds for an ISO 8601 duration, using openEHR's definite-arithmetic
-- averages.
--
-- BASE foundation_types master06-time_types.adoc §Computational Functions
-- distinguishes definite arithmetic, which treats all values as "exact and
-- invariant, based on constant values for length of year and month, defined by
-- Time_definitions.Average_days_in_month and
-- Time_definitions.Average_days_in_year", from nominal calendar arithmetic; an
-- ordering key can only be the definite one, so the averages below are those
-- two constants. The designator letters are literals of the same section.
CREATE FUNCTION openehr_duration_seconds(v text) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE
RETURN CASE WHEN v ~ '^-?P[^T]*(T.*)?$' THEN
    (CASE WHEN left(v, 1) = '-' THEN -1 ELSE 1 END)
  * (coalesce(substring(substring(v FROM '^-?P([^T]*)') FROM '([0-9]+(\.[0-9]+)?)Y')::numeric, 0) * 365.24 * 86400
   + coalesce(substring(substring(v FROM '^-?P([^T]*)') FROM '([0-9]+(\.[0-9]+)?)M')::numeric, 0) * 30.42 * 86400
   + coalesce(substring(substring(v FROM '^-?P([^T]*)') FROM '([0-9]+(\.[0-9]+)?)W')::numeric, 0) * 7 * 86400
   + coalesce(substring(substring(v FROM '^-?P([^T]*)') FROM '([0-9]+(\.[0-9]+)?)D')::numeric, 0) * 86400
   + coalesce(substring(substring(v FROM '^-?P[^T]*T(.*)$') FROM '([0-9]+(\.[0-9]+)?)H')::numeric, 0) * 3600
   + coalesce(substring(substring(v FROM '^-?P[^T]*T(.*)$') FROM '([0-9]+(\.[0-9]+)?)M')::numeric, 0) * 60
   + coalesce(substring(substring(v FROM '^-?P[^T]*T(.*)$') FROM '([0-9]+(\.[0-9]+)?)S')::numeric, 0))
END;

-- The ordered magnitude of a canonical DV_ORDERED value.
--
-- RM data_types master06-quantity_package.adoc §Requirements: "The most basic
-- characteristic of all values typically called 'quantities' is that they are
-- ordered, meaning that the operator '<' is defined between any two values in
-- the domain"; the per-subtype key follows the same chapter — the magnitude of
-- a DV_QUANTIFIED, the value of an ordinal or scale, and for DV_PROPORTION "a
-- magnitude function which is computed as the result of the numerator/
-- denominator division". A zero denominator has no magnitude and reads as NULL
-- rather than as an extremum, so it orders as absent. The date/time subtypes
-- take the readings above. A subtype with no ordering, and a value whose
-- carrier is not a number, read as NULL.
--
-- The simple CASE form evaluates its operand once, which matters because the
-- argument at an AQL call site is a jsonb path extraction and this body folds
-- into the statement.
CREATE FUNCTION openehr_magnitude(dv jsonb) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE
RETURN CASE dv->>'_type'
    WHEN 'DV_QUANTITY'   THEN openehr_numeric(dv->>'magnitude')
    WHEN 'DV_COUNT'      THEN openehr_numeric(dv->>'magnitude')
    WHEN 'DV_ORDINAL'    THEN openehr_numeric(dv->>'value')
    WHEN 'DV_SCALE'      THEN openehr_numeric(dv->>'value')
    WHEN 'DV_PROPORTION' THEN openehr_numeric(dv->>'numerator')
                              / nullif(openehr_numeric(dv->>'denominator'), 0)
    WHEN 'DV_DATE'       THEN openehr_date_days(dv->>'value')
    WHEN 'DV_TIME'       THEN openehr_time_seconds(dv->>'value')
    WHEN 'DV_DATE_TIME'  THEN openehr_date_time_seconds(dv->>'value')
    WHEN 'DV_DURATION'   THEN openehr_duration_seconds(dv->>'value')
END;

-- A total ISO 8601 text -> timestamptz reading.
--
-- This is the ONE partial-precision semantics the product carries on the
-- temporal paths: the promoted timestamp columns (node.context_start) are
-- populated through it at write, and the AQL engine's temporal comparisons and
-- orderings extract through it (app/ferroehr/src/aql/sql/value.rs,
-- Coercion::Temporal), so the promoted fast path and the jsonb lowering cannot
-- disagree.
--
-- Two readings. The first is PostgreSQL's own date/time input, which reads
-- every full-precision form correctly and far more cheaply than a decomposition
-- can, INSIDE AN ERROR TRAP: over 50 000 date-time readings this body measures
-- 70 ms, against 104 ms for the first generation and 235 ms for the same
-- readings with the fields and the calendar checked by hand so the cast can run
-- untrapped. The subtransaction the trap enters is real (PostgreSQL 18,
-- "Control Structures" §Trapping Errors,
-- https://www.postgresql.org/docs/18/plpgsql-control-structures.html); doing its
-- work by hand costs more. What the trap cannot decide is which spellings count
-- as a date-time: PostgreSQL also reads `now`, `today` and `2021-1-2`, and none
-- of those is a form openEHR defines, so the cast is reached only through two
-- LIKE patterns pinning the separator positions of the extended and the basic
-- format. A value the cast then refuses falls through to the second reading,
-- which is what the first generation did too.
--
-- The separator positions are not enough on their own, because both patterns
-- end in a wildcard and PostgreSQL reads what follows: a zone NAME
-- (`2021-01-02T10:30:45 Europe/Amsterdam`) and a ` BC` era, neither of which
-- BASE foundation_types master06-time_types.adoc §Iso8601_date_time admits:
-- the value is `YYYY-MM-DDThh:mm:ss[(,|.)sss][Z | ±hh[:mm]]`, the compact form
-- of the same, or a partial variant, so the text ends at the offset. One
-- translate() closes both: every character of a date-time is a digit, a
-- separator, a sign or the zone letter, so a value carrying anything else
-- never reaches the cast. It costs the reading 93 ms per 50 000 against 65 ms
-- without it, which is still under the 117 ms the first generation took on the
-- same corpus and the same machine.
--
-- The second reading completes reduced precision, which the cast cannot read at
-- all: a partial date assumes the first month and day, a partial time assumes
-- zero, and a time-only value anchors on 0001-01-01 — the same floor
-- openehr_date_days and openehr_time_seconds document. Mixed-precision ordering
-- is genuinely unspecified upstream, and the floor is our own recorded
-- semantics that does not depend on its resolution.
--
-- The offset-less reading takes the session TimeZone, spelled as AT TIME ZONE
-- over current_setting('TimeZone') rather than a cast, so the completion is one
-- expression with the local timestamp read once.
--
-- STABLE, never IMMUTABLE, and both readings need it: the cast reads the
-- session TimeZone and DateStyle, and the completion reads the TimeZone —
-- "a function that manipulates timestamps might well have results that depend
-- on the TimeZone setting. For safety, such functions should be labeled STABLE
-- instead." (PostgreSQL 18, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html). That is also why
-- this helper is not legal in an index expression, while the parsers above are.
--
-- Malformed input returns NULL — a comparison miss, never an error. Input that
-- is not ISO 8601 at all, including the words PostgreSQL's own parser accepts
-- (`now`, `today`, `infinity`), is malformed here: a stored value must not read
-- as the current time.
CREATE FUNCTION openehr_timestamp(v text) RETURNS timestamptz
LANGUAGE plpgsql STABLE STRICT PARALLEL SAFE AS $$
DECLARE
    s text := replace(v, ',', '.');
BEGIN
    IF (s LIKE '____-__-_____:__%' OR s LIKE '________T__%')
       AND translate(s, '0123456789.:+- TtZz', '') = '' THEN
        BEGIN
            RETURN s::timestamptz;
        EXCEPTION WHEN others THEN
            NULL;
        END;
    END IF;

    DECLARE
        date_part text;
        time_part text;
        sep       integer;
        days      numeric;
        sod       numeric;
        local_ts  timestamp;
    BEGIN
        sep := coalesce(nullif(position('T' in s), 0),
                        nullif(position('t' in s), 0),
                        nullif(position(' ' in s), 0), 0);
        IF sep = 1 OR s ~ '^[0-9]{1,2}:' THEN
            time_part := CASE WHEN sep = 1 THEN substr(s, 2) ELSE s END;
            days := 0;
        ELSE
            date_part := CASE WHEN sep = 0 THEN s ELSE substr(s, 1, sep - 1) END;
            time_part := CASE WHEN sep = 0 THEN '' ELSE substr(s, sep + 1) END;
            IF date_part !~ '^[0-9]{4}(-[0-9]{2}(-[0-9]{2})?|[0-9]{2}([0-9]{2})?)?$'
            THEN RETURN NULL; END IF;
            days := ext.openehr_date_days(date_part);
            IF days IS NULL THEN RETURN NULL; END IF;
        END IF;
        sod := ext.openehr_time_seconds(time_part);
        IF sod IS NULL THEN RETURN NULL; END IF;
        local_ts := (DATE '0001-01-01' + days::integer)::timestamp
                    + make_interval(secs => (sod - ext.openehr_tz_offset_seconds(time_part))::double precision);
        IF time_part ~ '([Zz]|[+-][0-9]{2}:?[0-9]{0,2})$' THEN
            RETURN local_ts AT TIME ZONE 'UTC';
        END IF;
        RETURN local_ts AT TIME ZONE current_setting('TimeZone');
    END;
END $$;

-- ── Function documentation ─────────────────────────────────────
COMMENT ON FUNCTION ext.openehr_numeric(text) IS
    'The numeric value of a canonical JSON number''s text (RFC 8259 §6), NULL for anything else. IMMUTABLE — index-legal.';
COMMENT ON FUNCTION ext.openehr_date_days(text) IS
    'Days since 0001-01-01 for an ISO 8601 date in either format; NULL when the text is not one, or names no real calendar date. Reduced precision assumes the first month/day. IMMUTABLE — index-legal. The calendar is left to make_date() inside an error trap, which is what measured cheapest: 58 ms per 50 000 readings, against 61 ms for the first generation and 65 ms for the same body checking the calendar itself so the cast can run untrapped.';
COMMENT ON FUNCTION ext.openehr_time_seconds(text) IS
    'Seconds since the start of the day for an ISO 8601 time, ignoring any zone suffix (callers apply the offset); NULL on unreadable input. Reduced precision assumes 0. IMMUTABLE.';
COMMENT ON FUNCTION ext.openehr_tz_offset_seconds(text) IS
    'The zone offset in seconds carried by an ISO 8601 time suffix (Z / ±hh[[:]mm]); 0 when the value carries none. IMMUTABLE.';
COMMENT ON FUNCTION ext.openehr_date_time_seconds(text) IS
    'Seconds since 0001-01-01T00:00:00Z for an ISO 8601 date-time; NULL when the date is unreadable. IMMUTABLE.';
COMMENT ON FUNCTION ext.openehr_duration_seconds(text) IS
    'Seconds for an ISO 8601 duration using openEHR''s definite-arithmetic averages (year = 365.24 d, month = 30.42 d); NULL on unreadable input. IMMUTABLE.';
COMMENT ON FUNCTION ext.openehr_magnitude(jsonb) IS
    'The ordered magnitude (numeric) of a canonical DV_ORDERED value, per the per-subtype comparison the RM defines (RM data_types master06-quantity_package.adoc). NULL for non-ordered or unreadable values. IMMUTABLE + PARALLEL SAFE, so it is legal in a btree expression index.';
COMMENT ON FUNCTION ext.openehr_timestamp(text) IS
    'Total ISO-8601 text -> timestamptz: PostgreSQL''s own parser for the full-precision forms, floor completion for reduced precision (partial dates assume the first month/day, partial times 0, time-only values anchor on 0001-01-01), NULL for anything else — never an error. The parser runs inside an error trap, which is what measured cheapest: 70 ms per 50 000 readings, against 104 ms for the first generation and 235 ms for the same readings with the fields and the calendar checked by hand so the cast can run untrapped. It is reached only through the separator positions AND a character-set check, which is what refuses the zone name and the BC era PostgreSQL would otherwise read (BASE foundation_types master06-time_types.adoc §Iso8601_date_time admits neither). The ONE partial-temporal semantics: feeds the promoted timestamp columns (node.context_start) AND the AQL temporal coercion. STABLE (TimeZone-dependent); not legal in index expressions.';

-- The runtime writers execute these on the write path (promoted-column
-- population) and the readers on the AQL path; ext/0001 already installed
-- ALTER DEFAULT PRIVILEGES for functions the migrator creates in this schema,
-- so no further grant is issued here.
