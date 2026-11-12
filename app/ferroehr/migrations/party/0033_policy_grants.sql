-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the system AGENTs and the policy grants the request path reads
-- (#5435).
--
-- A system client is an AGENT of the system AGENT archetype, identified by the
-- DV_IDENTIFIER of type client_id in its details (RM demographic
-- master02-demographic_package.adoc §Party Identification: identifiers are
-- "recorded as part of the data in the PARTY._details_ structure"), whose
-- issuer is the token issuer. What a party may do is the CAPABILITY of a
-- policy ROLE it performs (RM demographic org.openehr.rm.demographic.role.adoc:
-- `performer`, `capabilities`, `time_validity`;
-- org.openehr.rm.demographic.capability.adoc: `credentials`, `time_validity`).
-- The rows are derived from the party versions in each commit's own
-- transaction, after the pattern of binding_identifier
-- (0027_binding_side_table.sql), and a refill rebuilds them from the versions
-- alone. Each row holds the dates and instants a grant is valid for, so which
-- grants hold is judged at the request, on its own date. Reading a capability
-- as a grant is governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_agent ───────────────────────────────────────────────────────────
-- One row per client identifier of a system AGENT head that is not deleted.
CREATE TABLE binding_agent (
    -- The AGENT's version container.
    party_id        uuid NOT NULL,
    -- DV_IDENTIFIER.issuer: the token issuer.
    issuer          text NOT NULL,
    -- DV_IDENTIFIER.id: the OAuth 2.0 client_id.
    client_id       text NOT NULL,
    -- The AGENT head's version_lifecycle_state (RM common master06 §Version
    -- Lifecycle); an inactive or abandoned AGENT holds no grant.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT ck_binding_agent_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

-- One AGENT per client: a second AGENT naming a client another one names is
-- refused at its commit.
CREATE UNIQUE INDEX uq_binding_agent_client ON binding_agent (issuer, client_id);
CREATE INDEX ix_binding_agent_party ON binding_agent (party_id);

COMMENT ON TABLE binding_agent IS 'The client identifiers of each system AGENT head that is not deleted, by token issuer and client_id. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';

-- ── binding_capability ──────────────────────────────────────────────────────
-- One row per CAPABILITY of a policy ROLE head that is not deleted.
CREATE TABLE binding_capability (
    -- The ROLE's version container.
    role_id         uuid NOT NULL,
    -- The CAPABILITY's position in ROLE.capabilities, from 0.
    ordinal         integer NOT NULL,
    -- The container ROLE.performer names.
    performer_id    uuid NOT NULL,
    -- The kind of grant: the policy ROLE's kind and the CAPABILITY's coded
    -- name, such as system_client.relay; a code the contract does not define
    -- is kept as written and is never honoured.
    kind            text NOT NULL,
    -- The ROLE head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    -- The dates ROLE.time_validity holds: every date when absent, none when
    -- a bound cannot be read.
    role_validity   daterange NOT NULL,
    -- The dates CAPABILITY.time_validity holds, under the same rule, and none
    -- when a Not before or Not after bound in the credentials cannot be read.
    validity        daterange NOT NULL,
    -- The Not before and Not after instants of the credentials, for a bound
    -- finer than a day; NULL when the credentials carry none.
    not_before      timestamptz,
    not_after       timestamptz,
    -- CAPABILITY.credentials as written.
    credentials     jsonb,
    written_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (role_id, ordinal),
    CONSTRAINT ck_binding_capability_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

CREATE INDEX ix_binding_capability_performer ON binding_capability (performer_id);

COMMENT ON TABLE binding_capability IS 'The capabilities of each policy ROLE head that is not deleted, with the dates and instants each holds. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading a capability as a grant, our own design.';

-- ── binding_grant_revision ──────────────────────────────────────────────────
-- Moves with every change of binding_agent or binding_capability, in the same
-- transaction, so a replica whose copy is current answers from it with no
-- further read.
CREATE TABLE binding_grant_revision (
    singleton  boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    revision   bigint NOT NULL,
    revised_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO binding_grant_revision (singleton, revision) VALUES (true, 0);

COMMENT ON TABLE binding_grant_revision IS 'The revision of binding_agent and binding_capability, which moves with each change of either. Our own design.';
