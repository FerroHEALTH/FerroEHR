-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the instance identity and the usage report counters.
--
-- Runs with search_path = clinical, ext, public.

-- The instances of one deployment and what each must share
-- (#5589).
--
-- Every instance writes its row when it starts and touches it every 30
-- seconds: the fingerprint of the OpenPGP key it signs versions and
-- access-log links with ('digest' without one) and the SHA-256 of the
-- configuration every instance must share. Each compares its own with the
-- rows touched within the last 90 seconds, so a second key or a second
-- configuration is reported in health from the start rather than found when
-- a chain fails to verify. An instance that stops deletes its row; a row no
-- instance touched for a day is deleted by the next one.
--
-- No openEHR spec governs the process model: our own design.
CREATE TABLE instance (
    instance_id   uuid        NOT NULL,
    started_at    timestamptz NOT NULL DEFAULT now(),
    seen_at       timestamptz NOT NULL DEFAULT now(),
    -- The FerroEHR release the instance runs.
    version       text        NOT NULL,
    -- The fingerprint of the signing key's certificate, or 'digest'.
    signing_key   text        NOT NULL,
    -- The SHA-256 of the configuration every instance shares, as hex.
    configuration text        NOT NULL,
    CONSTRAINT pk_instance PRIMARY KEY (instance_id)
);
CREATE INDEX ix_instance_seen ON instance (seen_at);
COMMENT ON TABLE instance IS 'The running instances of the deployment: each one''s signing key fingerprint and configuration digest, touched every 30 seconds. Our own design.';

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
    max_ms     double precision NOT NULL DEFAULT 0,
    CONSTRAINT pk_usage_report_window PRIMARY KEY (series),
    CONSTRAINT ck_usage_report_window_series CHECK (series IN (
        'ehr', 'composition', 'contribution', 'query', 'definition',
        'directory', 'admin', 'other'
    )),
    CONSTRAINT ck_usage_report_window_counts CHECK (
        cardinality(counts) = 11 AND array_ndims(counts) = 1
    ),
    CONSTRAINT ck_usage_report_window_non_negative CHECK (
        errors_5xx >= 0 AND max_ms >= 0
    )
);
COMMENT ON TABLE usage_report_window IS
    'Aggregate latency histograms of the current usage-report window, per route-template group; no request content.';

CREATE TABLE usage_report_counter (
    counter text   NOT NULL,
    count   bigint NOT NULL DEFAULT 0,
    CONSTRAINT pk_usage_report_counter PRIMARY KEY (counter),
    CONSTRAINT ck_usage_report_counter_key CHECK (counter ~ '^[a-z][a-z0-9_]{0,31}$'),
    CONSTRAINT ck_usage_report_counter_non_negative CHECK (count >= 0)
);
COMMENT ON TABLE usage_report_counter IS
    'The counters of the current usage-report window, such as responses_4xx; no request content.';
