-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the client identifiers and the policy grants the request path reads
-- (#5435).
--
-- A client is an ACTOR of any archetype carrying a DV_IDENTIFIER of type
-- client_id in its details or identities (RM demographic
-- master02-demographic_package.adoc §Party Identification: identifiers are
-- "recorded as part of the data in the PARTY._details_ structure"), whose
-- issuer is the token issuer. What a party may do is an operation coded in
-- FerroEHR's operation terminology (ferroehr-operations) anywhere in the
-- credentials of a CAPABILITY of a ROLE of any archetype it performs (RM
-- demographic org.openehr.rm.demographic.role.adoc: `performer`,
-- `capabilities`, `time_validity`; org.openehr.rm.demographic.capability.adoc:
-- `credentials`, `time_validity`).
-- The rows are derived from the party versions in each commit's own
-- transaction, after the pattern of binding_identifier
-- (0027_binding_side_table.sql), and a refill rebuilds them from the versions
-- alone. Each row holds the dates and instants a grant is valid for, so which
-- grants hold is judged at the request, on its own date. Reading a capability
-- as a grant is governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_client ──────────────────────────────────────────────────────────
-- One row per client identifier of an ACTOR head that is not deleted.
CREATE TABLE binding_client (
    -- The ACTOR's version container.
    party_id        uuid NOT NULL,
    -- DV_IDENTIFIER.issuer: the token issuer.
    issuer          text NOT NULL,
    -- DV_IDENTIFIER.id: the OAuth 2.0 client_id.
    client_id       text NOT NULL,
    -- The ACTOR head's version_lifecycle_state (RM common master06 §Version
    -- Lifecycle); an inactive or abandoned ACTOR holds no grant.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT ck_binding_client_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

-- One ACTOR per client: a second ACTOR naming a client another one names is
-- refused at its commit.
CREATE UNIQUE INDEX uq_binding_client ON binding_client (issuer, client_id);
CREATE INDEX ix_binding_client_party ON binding_client (party_id);

COMMENT ON TABLE binding_client IS 'The client identifiers of each ACTOR head that is not deleted, by token issuer and client_id. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';

-- ── binding_capability ──────────────────────────────────────────────────────
-- One row per operation a CAPABILITY of a ROLE head that is not deleted grants.
CREATE TABLE binding_capability (
    -- The ROLE's version container.
    role_id         uuid NOT NULL,
    -- The CAPABILITY's position in ROLE.capabilities, from 0.
    ordinal         integer NOT NULL,
    -- The ferroehr-operations code the credentials hold, such as import_ehr;
    -- a code the terminology does not define is kept as written and is never
    -- honoured.
    operation       text NOT NULL,
    -- The container ROLE.performer names.
    performer_id    uuid NOT NULL,
    -- The ROLE head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    -- The dates ROLE.time_validity holds: every date when absent, none when
    -- a bound cannot be read.
    role_validity   daterange NOT NULL,
    -- The dates CAPABILITY.time_validity holds, under the same rule, and none
    -- when a Not before or Not after bound in the credentials cannot be read.
    validity        daterange NOT NULL,
    -- The Not before and Not after instants of the credentials, the elements
    -- the terminology names not_before and not_after, for a bound finer than a
    -- day; NULL when the credentials carry none.
    not_before      timestamptz,
    not_after       timestamptz,
    -- CAPABILITY.credentials as written.
    credentials     jsonb,
    written_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (role_id, ordinal, operation),
    CONSTRAINT ck_binding_capability_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

CREATE INDEX ix_binding_capability_performer ON binding_capability (performer_id);

COMMENT ON TABLE binding_capability IS 'The operations the capabilities of each ROLE head that is not deleted grant, with the dates and instants each holds. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading a capability as a grant, our own design.';

-- ── binding_grant_revision ──────────────────────────────────────────────────
-- Moves with every change of binding_client or binding_capability, in the same
-- transaction, so a replica whose copy is current answers from it with no
-- further read.
CREATE TABLE binding_grant_revision (
    singleton  boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    revision   bigint NOT NULL,
    revised_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO binding_grant_revision (singleton, revision) VALUES (true, 0);

COMMENT ON TABLE binding_grant_revision IS 'The revision of binding_client and binding_capability, which moves with each change of either. Our own design.';
