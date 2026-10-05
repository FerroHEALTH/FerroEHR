-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- linkage: erasure reaches the cross-reference.
--
-- Nothing else in this schema deletes. A merge or a split CLOSES a period and
-- opens the next, so "which party was the subject of this EHR when that
-- composition was written" stays answerable, and the runtime role holds no
-- DELETE privilege to do otherwise with.
--
-- Erasure is the one exception, and it is why this function exists. When an EHR
-- is physically deleted (GDPR Art. 17(1),
-- https://eur-lex.europa.eu/eli/reg/2016/679/oj), a surviving row here still
-- asserts that a person is the subject of a record that no longer exists — the
-- additional information of Art. 4(5) outliving the data it was additional to.
-- Closing the period would not help: the row, open or closed, still names the
-- person and the erased record.
--
-- SECURITY DEFINER is what lets that happen without widening the role: the
-- function runs as its owner, so ferroehr_linkage may EXECUTE it while holding
-- no DELETE on the table (PostgreSQL 18, CREATE FUNCTION, "Writing SECURITY
-- DEFINER Functions Safely",
-- https://www.postgresql.org/docs/18/sql-createfunction.html). The same page
-- prescribes the fixed search_path the function carries, so the objects it
-- names cannot be captured by a caller's path. It takes one EHR id, deletes
-- only rows naming it, and returns how many — a narrower privilege than DELETE
-- on the relation by construction.
--
-- No openEHR spec governs erasure mechanics or database roles: our own
-- design/extension.
--
-- Runs with search_path = linkage, ext, public.
CREATE FUNCTION erase_ehr(an_ehr_id uuid) RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = linkage, pg_catalog
AS $$
DECLARE
    erased bigint;
BEGIN
    DELETE FROM subject_ehr WHERE ehr_id = an_ehr_id;
    GET DIAGNOSTICS erased = ROW_COUNT;
    RETURN erased;
END $$;

COMMENT ON FUNCTION erase_ehr(uuid) IS 'Remove every cross-reference row naming an erased EHR, in force or historical, and return how many (GDPR Art. 17(1)). SECURITY DEFINER so the linkage runtime role can reach erasure while holding no DELETE on the table. Our own design/extension.';
