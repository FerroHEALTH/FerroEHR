-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the usage report's installation identity, send bookkeeping and the
-- shared 24-hour metrics window.
--
-- No openEHR spec governs this — our own design/extension. The report holds
-- no patient data; the relations sit in the clinical schema because the
-- clinical pool is the one every replica of an instance shares, so the
-- instance id, the send claims and the window are one per installation however
-- many replicas run.
--
-- The grants are in this file rather than 0010_grants.sql, because that file
-- is in a release and a released migration is never edited.
--
-- Runs with search_path = clinical, ext, public.

-- One row per installation. `instance_id` is random (gen_random_uuid() is
-- version 4, PostgreSQL 18, "UUID Functions",
-- https://www.postgresql.org/docs/18/functions-uuid.html) and never derived
-- from a host, a licence or anything else about the deployment.
CREATE TABLE usage_report_instance (
    singleton          boolean     NOT NULL DEFAULT true,
    instance_id        uuid        NOT NULL DEFAULT gen_random_uuid(),
    created_at         timestamptz NOT NULL DEFAULT now(),
    -- The last start report claimed; a replica sends one only when this is
    -- older than ten minutes.
    last_start_sent_at timestamptz,
    -- The last daily report claimed; the conditional update on it is what lets
    -- exactly one replica send per window.
    last_daily_sent_at timestamptz,
    CONSTRAINT pk_usage_report_instance PRIMARY KEY (singleton),
    CONSTRAINT ck_usage_report_instance_singleton CHECK (singleton)
);

COMMENT ON TABLE usage_report_instance IS
    'The usage report''s installation identity (a random UUIDv4) and the times the last start and daily reports were claimed. One row.';

-- The metrics window every replica adds its counts to and the replica that
-- claims the daily report takes and clears. One row per route-template group
-- plus `aql`; `counts` is the request count per latency bucket, upper bounds in
-- ms 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, +Inf.
CREATE TABLE usage_report_window (
    series     text             NOT NULL,
    counts     bigint[]         NOT NULL,
    errors_5xx bigint           NOT NULL DEFAULT 0,
    slow       bigint           NOT NULL DEFAULT 0,
    max_ms     double precision NOT NULL DEFAULT 0,
    CONSTRAINT pk_usage_report_window PRIMARY KEY (series),
    CONSTRAINT ck_usage_report_window_series CHECK (series IN (
        'ehr', 'composition', 'contribution', 'query', 'definition',
        'directory', 'admin', 'other', 'aql'
    )),
    CONSTRAINT ck_usage_report_window_counts CHECK (
        cardinality(counts) = 11 AND array_ndims(counts) = 1
    ),
    CONSTRAINT ck_usage_report_window_non_negative CHECK (
        errors_5xx >= 0 AND slow >= 0 AND max_ms >= 0
    )
);

COMMENT ON TABLE usage_report_window IS
    'Aggregate latency histograms of the current usage-report window, per route-template group and for AQL; no request content.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE
            ON usage_report_instance, usage_report_window
            TO ferroehr_clinical;
        REVOKE ALL ON usage_report_instance, usage_report_window
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping usage report grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
