-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: drop the horizon-keyed reapers (#4770).
--
-- The server reaps through audit.reap_audit_events_due (0013), which honours
-- each record's own retention end. The day-keyed audit.reap_audit_events
-- (0005, 0008), the year-keyed audit.reap_audit_events_years (0008) and the
-- cutoff-keyed body both delegated to (0008) have no caller left, and dropping
-- them drops the EXECUTE grants 0006 and 0008 gave. The deletion trigger's
-- refusal names the reaper that remains.
--
-- No openEHR spec governs audit retention: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

DROP FUNCTION audit.reap_audit_events(integer);
DROP FUNCTION audit.reap_audit_events_years(integer);
DROP FUNCTION audit.reap_audit_events_before(timestamptz);

CREATE OR REPLACE FUNCTION audit.audit_event_reject_deletion() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF coalesce(current_setting('ferroehr.audit_reaping', true), 'off') <> 'on' THEN
        RAISE EXCEPTION
            'audit.audit_event records are deleted only by audit.reap_audit_events_due() (record %)',
            OLD.id
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;
