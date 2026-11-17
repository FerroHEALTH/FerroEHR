-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: every change to a stored object that can change how one of its
-- versions is served notifies every replica's copy of read versions (#5778).
--
-- A COMPOSITION version never changes once committed (RM common master06
-- §Version Identification), so a replica keeps the versions it read in
-- memory. What it judges a version by can change: the object's head facts
-- move with a new version, the restriction mark (GDPR Art. 18(2)), the
-- access delay that withholds it from the person (EHDS Art. 3(3)), its tier,
-- an erasure that rewrites a body or removes the object (GDPR Art. 17). Each
-- of these updates or deletes a vo_head or version row, and a creation
-- inserts one, which no copy holds yet. The statement that changes them
-- names each object it changed on 'ferroehr_object_version' at commit
-- (PostgreSQL 18, NOTIFY); a statement that changes more than 64 objects,
-- such as a whole-EHR restriction, sends '*', which drops the whole copy.
-- No openEHR spec governs the copy: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE FUNCTION object_version_notify() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog
    AS $$
BEGIN
    IF (SELECT count(DISTINCT c.vo_id) FROM changed c) > 64 THEN
        PERFORM pg_notify('ferroehr_object_version', '*');
    ELSE
        PERFORM pg_notify('ferroehr_object_version', d.vo_id::text)
           FROM (SELECT DISTINCT c.vo_id FROM changed c) d;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER tg_vo_head_updated_notify
    AFTER UPDATE ON vo_head REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION object_version_notify();

CREATE TRIGGER tg_vo_head_deleted_notify
    AFTER DELETE ON vo_head REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION object_version_notify();

CREATE TRIGGER tg_version_updated_notify
    AFTER UPDATE ON version REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION object_version_notify();

CREATE TRIGGER tg_version_deleted_notify
    AFTER DELETE ON version REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION object_version_notify();

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the triggers call it, so no role of any domain reaches it.
REVOKE ALL ON FUNCTION object_version_notify() FROM PUBLIC;
