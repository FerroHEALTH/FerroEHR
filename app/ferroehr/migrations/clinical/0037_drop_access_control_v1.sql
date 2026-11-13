-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: drop every EHR_ACCESS of the removed FERROEHR_ACCESS_CONTROL_V1
-- scheme (#5508).
--
-- EHR_ACCESS.settings now hold only the access_directives scheme
-- (ACCESS_DIRECTIVE_SETTINGS), and a stored body of the old scheme would hold
-- its EHR closed. The scheme is gone with no conversion, so every EHR_ACCESS
-- versioned object any version of which carries it is removed whole: its
-- versions (their node and attestation rows cascade), the contributions and
-- commit audits nothing else references, and its head row. Each EHR left
-- without an EHR_ACCESS gets the server's default one at the next boot
-- (`FerroEhrService::restore_default_ehr_access`), so EHR.ehr_access stays
-- 1..1 (RM ehr `ehr.adoc` invariant Ehr_access_valid).
--
-- No openEHR spec governs the scheme or its removal: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE TEMPORARY TABLE dropped_access ON COMMIT DROP AS
    SELECT DISTINCT vo_id FROM version
    WHERE kind = 'EHR_ACCESS' AND strpos(body, '"FERROEHR_ACCESS_CONTROL_V1"') > 0;

CREATE TEMPORARY TABLE dropped_contribution ON COMMIT DROP AS
    SELECT DISTINCT contribution_id AS id FROM version
    WHERE vo_id IN (SELECT vo_id FROM dropped_access);

CREATE TEMPORARY TABLE dropped_audit ON COMMIT DROP AS
    SELECT commit_audit_id AS id FROM version WHERE vo_id IN (SELECT vo_id FROM dropped_access)
    UNION
    SELECT c.commit_audit_id FROM contribution c
    WHERE c.id IN (SELECT id FROM dropped_contribution);

DELETE FROM version WHERE vo_id IN (SELECT vo_id FROM dropped_access);

DELETE FROM contribution c
WHERE c.id IN (SELECT id FROM dropped_contribution)
  AND NOT EXISTS (SELECT 1 FROM version v WHERE v.contribution_id = c.id)
  AND NOT EXISTS (SELECT 1 FROM vo_attestation a WHERE a.contribution_id = c.id);

DELETE FROM commit_audit a
WHERE a.id IN (SELECT id FROM dropped_audit)
  AND NOT EXISTS (SELECT 1 FROM version v WHERE v.commit_audit_id = a.id)
  AND NOT EXISTS (SELECT 1 FROM contribution c WHERE c.commit_audit_id = a.id);

DELETE FROM vo_head WHERE vo_id IN (SELECT vo_id FROM dropped_access);
