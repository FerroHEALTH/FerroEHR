-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the instances of one deployment and what each must share
-- (#5589).
--
-- Every instance writes its row when it starts and touches it every 30
-- seconds: the fingerprint of the OpenPGP key it signs versions and
-- access-log links with ('digest' without one) and the SHA-256 of the
-- configuration every instance must share. Each compares its own with the
-- rows touched within the last 90 seconds, so a second key or a second
-- configuration is reported in health from the start rather than found when
-- a chain fails to verify. An instance that stops deletes its row; a row no
-- instance touched for a day is deleted by the next one.
--
-- No openEHR spec governs the process model: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE instance (
    instance_id   uuid        NOT NULL,
    started_at    timestamptz NOT NULL DEFAULT now(),
    seen_at       timestamptz NOT NULL DEFAULT now(),
    -- The FerroEHR release the instance runs.
    version       text        NOT NULL,
    -- The fingerprint of the signing key's certificate, or 'digest'.
    signing_key   text        NOT NULL,
    -- The SHA-256 of the configuration every instance shares, as hex.
    configuration text        NOT NULL,
    CONSTRAINT pk_instance PRIMARY KEY (instance_id)
);

CREATE INDEX ix_instance_seen ON instance (seen_at);

COMMENT ON TABLE instance IS 'The running instances of the deployment: each one''s signing key fingerprint and configuration digest, touched every 30 seconds. Our own design.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON instance TO ferroehr_clinical;
        REVOKE ALL ON instance FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping the instance grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
