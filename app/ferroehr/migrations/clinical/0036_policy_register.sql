-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the operational-policy admin registers (#5442).
--
-- The audit category map, the EHDS Art. 3(3) access delay and the Cedar
-- policy set are operational policy with no home in the openEHR demographic
-- model, so each is an admin register: every write appends one version with
-- its committer, and the request path reads the newest version of each
-- register, so a change applies on the next request on every replica. A
-- register with no version holds its default.
--
-- No openEHR spec governs operational policy: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE policy_register (
    register     text        NOT NULL,
    version      bigint      NOT NULL,
    document     jsonb       NOT NULL,
    committer    text        NOT NULL,
    committed_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_policy_register PRIMARY KEY (register, version),
    CONSTRAINT ck_policy_register_name CHECK (
        register IN ('audit-categories', 'access-delay', 'cedar-policies')
    ),
    CONSTRAINT ck_policy_register_version CHECK (version > 0)
);

COMMENT ON TABLE policy_register IS
    'The operational-policy admin registers: one row per version of the audit category map, the EHDS Art. 3(3) access delay and the Cedar policy set, with its committer. Append-only; our own design/extension.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT ON policy_register TO ferroehr_clinical;
        REVOKE ALL ON policy_register FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping policy register grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
