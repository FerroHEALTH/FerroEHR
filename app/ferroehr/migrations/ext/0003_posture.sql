-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- ext: the deployment posture the server stamps at boot and the database
-- guards read.
--
-- One key-value row per posture fact. `subject_pseudonyms` (`required` |
-- `open`) is the one the clinical schema's subject guard reads: a deployment
-- that declares subject namespaces holds an opaque pseudonym on the clinical
-- side, never a national identifier, and the trigger refuses anything else.
--
-- No openEHR spec governs deployment posture: our own design/extension. What
-- the pseudonym posture realizes is GDPR Art. 4(5)
-- (https://eur-lex.europa.eu/eli/reg/2016/679/oj).
--
-- The table is owned by the migrator and granted to nobody; the writer reaches
-- it through the SECURITY DEFINER function alone (PostgreSQL 18, "Writing
-- SECURITY DEFINER Functions Safely",
-- https://www.postgresql.org/docs/18/sql-createfunction.html), because the
-- runtime roles must not be able to relax a guard by writing its posture row
-- directly.
--
-- Runs with search_path = ext.
CREATE TABLE posture (
    key        text        NOT NULL,
    value      text        NOT NULL,
    stamped_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_ext_posture PRIMARY KEY (key)
);

COMMENT ON TABLE ext.posture IS
    'Deployment posture the server stamps at boot and the database guards read; subject_pseudonyms = required | open. Written only through ext.stamp_posture.';

CREATE FUNCTION stamp_posture(a_key text, a_value text) RETURNS void
LANGUAGE sql SECURITY DEFINER
SET search_path = ext, pg_catalog
AS $$
    INSERT INTO ext.posture (key, value) VALUES (a_key, a_value)
    ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, stamped_at = now()
$$;

COMMENT ON FUNCTION ext.stamp_posture(text, text) IS
    'Write one posture key; the server calls it at boot with the state the configuration declares.';
