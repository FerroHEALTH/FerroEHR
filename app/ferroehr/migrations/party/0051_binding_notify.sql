-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: every change of what resolves a login to a PERSON notifies the
-- replicas that hold a copy of it (#5771).
--
-- Each replica keeps the lookups of the binding side table and the names of
-- the bound PERSONs in memory, read on a miss. A write of a key row, a
-- membership, a representation or a key space's state notifies
-- ferroehr_party_binding with an empty payload, and every replica drops every
-- lookup it holds; a write of a PERSON's version notifies its container's id,
-- and every replica drops that PERSON's name. PostgreSQL delivers a
-- notification when the writing transaction commits and never for one that
-- rolls back (https://www.postgresql.org/docs/18/sql-notify.html), and
-- collapses identical payloads within one transaction. No openEHR spec
-- governs resolving a login to a party: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE FUNCTION binding_changed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = party, pg_catalog
    AS $$
BEGIN
    PERFORM pg_notify('ferroehr_party_binding', '');
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION binding_changed() IS
    'Notifies ferroehr_party_binding that the binding side table changed, so every replica drops the lookups it holds.';

CREATE TRIGGER binding_identifier_changed
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON binding_identifier
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_membership_changed
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON binding_membership
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_representation_changed
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON binding_representation
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
-- A key space's fill progress decides no lookup; its state does.
CREATE TRIGGER binding_key_space_changed
    AFTER INSERT OR DELETE OR TRUNCATE OR UPDATE OF state ON binding_key_space
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();

CREATE FUNCTION person_version_changed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = party, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM pg_notify('ferroehr_party_binding', OLD.vo_id::text);
    ELSE
        PERFORM pg_notify('ferroehr_party_binding', NEW.vo_id::text);
    END IF;
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION person_version_changed() IS
    'Notifies ferroehr_party_binding with the container of a PERSON whose version was written, changed or removed, so every replica drops its name.';

CREATE TRIGGER version_person_written
    AFTER INSERT OR UPDATE ON version
    FOR EACH ROW WHEN (NEW.kind = 'PERSON') EXECUTE FUNCTION person_version_changed();
CREATE TRIGGER version_person_removed
    AFTER DELETE ON version
    FOR EACH ROW WHEN (OLD.kind = 'PERSON') EXECUTE FUNCTION person_version_changed();

-- A trigger function runs for the statement that fires it whatever the role's
-- rights on it; no role calls it, and the clinical role reaches no function of
-- the party schema.
REVOKE ALL ON FUNCTION binding_changed() FROM PUBLIC;
REVOKE ALL ON FUNCTION person_version_changed() FROM PUBLIC;
