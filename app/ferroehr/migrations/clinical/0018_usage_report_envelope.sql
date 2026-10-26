-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the usage report's shared window in the shape of the family's
-- report crate, ferrohealth-usage-report.
--
-- No openEHR spec governs this — our own design/extension. The report no
-- longer carries the AQL series or the slow-query count, so the `aql` rows and
-- the `slow` column go, and the window gains the counters the crate keeps
-- beside its groups (`responses_4xx` among them). A counter name follows the
-- report envelope v2's key pattern; a group name is one of FerroEHR's route
-- groups.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM usage_report_window WHERE series = 'aql';

ALTER TABLE usage_report_window
    DROP CONSTRAINT ck_usage_report_window_series,
    DROP CONSTRAINT ck_usage_report_window_non_negative,
    DROP COLUMN slow,
    ADD CONSTRAINT ck_usage_report_window_series CHECK (series IN (
        'ehr', 'composition', 'contribution', 'query', 'definition',
        'directory', 'admin', 'other'
    )),
    ADD CONSTRAINT ck_usage_report_window_non_negative CHECK (
        errors_5xx >= 0 AND max_ms >= 0
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

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON usage_report_counter TO ferroehr_clinical;
        REVOKE ALL ON usage_report_counter
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping usage report grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
