-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the render policy states the access log retention of the
-- controller's ACCESS_POLICY, and no longer the [audit.store] retention
-- horizon (#5660).
--
-- Every record renders under the audit_render_policy row it names. That row
-- held the store horizon, a key this release removes; it now holds the
-- access policy's log retention (whether EHDS binds the controller, the
-- period, and the floors and ceilings with their provisions), which a record
-- written before cannot state. The access log is never rewritten, and a
-- stored policy is never changed under its digest, so this migration refuses
-- a store that holds a render policy of the earlier shape, naming each one.
-- Nothing is in production: a 5.7.1-rc1 store holds test data.
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

DO $earlier$
DECLARE
    earlier text;
BEGIN
    SELECT string_agg(encode(digest, 'hex'), ', ' ORDER BY stored_at, digest)
      INTO earlier
      FROM audit_render_policy
     WHERE policy ? 'horizon';
    IF earlier IS NOT NULL THEN
        RAISE EXCEPTION 'audit.audit_render_policy holds render policies written before migration 0035, which state the removed [audit.store] retention horizon: %', earlier
            USING ERRCODE = 'object_not_in_prerequisite_state',
                  HINT = 'FerroEHR 5.7.2 starts on an empty audit store: export what you need over ITI-81 with the earlier release, then drop the audit schema and start again. The access log retention is the controller''s ACCESS_POLICY log_retention from now on.';
    END IF;
END
$earlier$;
