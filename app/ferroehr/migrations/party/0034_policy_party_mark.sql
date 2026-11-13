-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the mark of a party that ever had a policy version (#5485).
--
-- A version is a policy party when it holds, in any archetype or template, a
-- CAPABILITY whose credentials hold a DV_CODED_TEXT of FerroEHR's operation
-- terminology (ferroehr-operations), a PARTY_RELATIONSHIP whose name or
-- details hold that terminology's acts_for code, or a DV_IDENTIFIER of type
-- client_id (party_binding::policy_mark). Only an administrator reads or
-- writes such a party, through the Demographic API and demographic AQL
-- alike, every version of it: a party an administrator later turns into an
-- ordinary one stays hidden. Each party commit sets the mark in its own
-- transaction when the version it writes holds that content; nothing clears
-- it; an import, an archive load and `ferroehr party-binding refill` set it
-- from the stored versions. A read tests one column of the head row it
-- reaches by its primary key. No openEHR spec governs who reads a party: our
-- own design.
--
-- The statement below marks the parties already stored, testing every
-- stored body with party_binding::policy_mark::POLICY_PATH, which a unit
-- test pins to this file and runs against the code's own content test.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE vo_head ADD COLUMN policy_party_at timestamptz;

UPDATE vo_head h SET policy_party_at = now()
WHERE EXISTS (SELECT FROM version v
              WHERE v.vo_id = h.vo_id AND v.body IS NOT NULL
                AND jsonb_path_exists(v.body::jsonb, '$ ? (exists($.**?(@._type == "CAPABILITY").credentials.**?(@._type == "DV_CODED_TEXT" && @.defining_code.terminology_id.value == "ferroehr-operations")) || exists($.**?(@._type == "PARTY_RELATIONSHIP")?(exists(@.name?(@._type == "DV_CODED_TEXT" && @.defining_code.terminology_id.value == "ferroehr-operations" && @.defining_code.code_string == "acts_for")) || exists(@.details.**?(@._type == "DV_CODED_TEXT" && @.defining_code.terminology_id.value == "ferroehr-operations" && @.defining_code.code_string == "acts_for")))) || exists($.**?(@._type == "DV_IDENTIFIER" && @.type == "client_id")))'));

COMMENT ON COLUMN vo_head.policy_party_at IS 'When a version of this party first held a grant-coded capability, an acts-for relationship or a client identifier; NULL = never. Set in the commit transaction and never cleared: only an administrator reads or writes any version of a marked party. Our own design.';
