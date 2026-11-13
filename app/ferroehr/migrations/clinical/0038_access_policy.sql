-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the controller's ACCESS_POLICY (#5512).
--
-- The access policy holds the facts that hold for every EHR the controller
-- keeps: the sensitivity of kinds of data, whether clinical access needs a
-- care relationship and a declared purpose, and the named directives a
-- subject can accept by reference. It is the one VERSIONED_ACCESS_POLICY of
-- the instance, kept outside every EHR and outside the demographic service.
-- Each row is one ORIGINAL_VERSION on the trunk, committed in its own
-- CONTRIBUTION with its AUDIT_DETAILS (RM common change_control, Common IM
-- master06 §Contributions and §Committal and Audits). Append-only.
--
-- ACCESS_POLICY is a class of FerroEHR's draft access control package, which
-- is no openEHR release, and ITS-REST defines no resource for it: our own
-- design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE access_policy_version (
    version          integer     NOT NULL,
    vo_id            uuid        NOT NULL,
    system_id        text        NOT NULL,
    contribution_id  uuid        NOT NULL,
    commit_audit     jsonb       NOT NULL,
    time_committed   timestamptz NOT NULL,
    lifecycle_state  text        NOT NULL,
    data             jsonb       NOT NULL,
    CONSTRAINT pk_access_policy_version PRIMARY KEY (version),
    CONSTRAINT uq_access_policy_version_contribution UNIQUE (contribution_id),
    CONSTRAINT ck_access_policy_version_trunk CHECK (version > 0)
);

COMMENT ON TABLE access_policy_version IS
    'The controller ACCESS_POLICY: one ORIGINAL_VERSION per row on the trunk of the one VERSIONED_ACCESS_POLICY, each in its own CONTRIBUTION with its AUDIT_DETAILS. Append-only; our own design/extension.';
COMMENT ON COLUMN access_policy_version.vo_id IS 'VERSIONED_ACCESS_POLICY.uid: the same in every row.';
COMMENT ON COLUMN access_policy_version.commit_audit IS 'ORIGINAL_VERSION.commit_audit, the canonical AUDIT_DETAILS.';
COMMENT ON COLUMN access_policy_version.data IS 'ORIGINAL_VERSION.data, the canonical ACCESS_POLICY.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT ON access_policy_version TO ferroehr_clinical;
        REVOKE ALL ON access_policy_version FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping access policy grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
