-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access-event fields, beside the ATNA record's own.
--
-- An ATNA record says who touched what. A care record's access log has to say
-- more: NEN 7513 lists the role or authority the person acted under; EHDS
-- Annex II 3.2(e) asks for "the origin or origins of data" (Regulation (EU)
-- 2025/327, https://eur-lex.europa.eu/eli/reg/2025/327/oj); and a purpose and
-- a legal basis are what make an access explicable at all. These columns carry
-- them.
--
-- The domain column names the pseudonymisation domain the access read or
-- wrote, so a reviewer can see which side of the boundary an operation
-- touched. `system` means no domain.
--
-- Two columns for the origins, because a set may be capped: `origins` holds up
-- to the cap the server records and `origin_count` the true number, so a
-- truncated record says it is truncated rather than claiming to be the whole
-- answer. A JSON array rather than text[] in each case: the batched write path
-- binds one array per column and unnests it, which an array of jsonb scalars
-- can express row by row and a two-dimensional text array cannot.
--
-- Append-only needs no new machinery: the BEFORE UPDATE trigger the tamper
-- chain installs compares the whole row as jsonb minus the two delivery
-- stamps, so these columns are covered by it the moment they exist.
--
-- No openEHR spec governs the read-side model: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    -- The pseudonymisation domain the operation read or wrote:
    -- `ehr` (clinical), `demographic` (parties), `linkage` (the resolve map),
    -- or `system` for an operation that touches no domain (authentication,
    -- templates, stored queries, node management).
    ADD COLUMN domain       text,
    -- The purpose of use the caller declared, as a code from the deployment's
    -- own allow-list. NULL when the caller declared none.
    ADD COLUMN purpose      text,
    -- The legal basis the deployment attributes to this access, as a code.
    ADD COLUMN legal_basis  text,
    -- How many records the operation served: the row count of an AQL result
    -- set, the entry count of a search. NULL for an operation that serves no
    -- countable set.
    ADD COLUMN result_count bigint,
    -- The request correlation id (the `x-request-id` the response carries), so
    -- an access record joins to the request it belonged to.
    ADD COLUMN request_id   text;

-- NULL is a real answer on the three code columns and means "not recorded":
-- records written before this migration carry no domain, and inventing one for
-- them would be a guess about what a past operation touched.
ALTER TABLE audit_event
    ADD CONSTRAINT ck_audit_event_domain
        CHECK (domain IS NULL OR domain IN ('clinical', 'party', 'linkage', 'system')),
    ADD CONSTRAINT ck_audit_event_result_count
        CHECK (result_count IS NULL OR result_count >= 0);

COMMENT ON COLUMN audit_event.domain IS
    'The pseudonymisation domain read or written: clinical | party | linkage | system (no domain). NULL when the operation names none.';
COMMENT ON COLUMN audit_event.purpose IS
    'The purpose-of-use code the caller declared, from the deployment allow-list; NULL when none was declared.';
COMMENT ON COLUMN audit_event.legal_basis IS
    'The legal basis code the deployment attributes to this access; NULL when the deployment configures none.';
COMMENT ON COLUMN audit_event.result_count IS
    'The number of records served (AQL result rows, search entries); NULL when the operation serves no countable set.';
COMMENT ON COLUMN audit_event.request_id IS
    'The request correlation id, matching the x-request-id header on the response.';

-- The subject access-log export (IHE ITI-81, GET /fhir/r4/AuditEvent?patient=)
-- reads ix_audit_event_patient, which the baseline already declares; a
-- per-domain slice of one subject's log filters that index's result and needs
-- no index of its own.

ALTER TABLE audit_event
    ADD COLUMN organisation text;

-- NULL is a real answer and means "not recorded": a Basic-authenticated
-- caller carries no claims, a deployment may configure no organisation claim,
-- and records written before this migration have none — inventing an
-- organisation for any of them would be a guess about who a caller acted for.
COMMENT ON COLUMN audit_event.organisation IS
    'The organisation the caller acted for, from the configured identity-token claim; NULL when the deployment resolves none and on records written before the column existed.';

ALTER TABLE audit_event
    ADD COLUMN roles jsonb;

-- NULL means "not recorded": an unauthenticated refusal has no principal and
-- no roles, and records written before this migration carry none. An
-- authenticated caller with no roles is recorded as the empty array, which is
-- a different fact from an unknown one.
COMMENT ON COLUMN audit_event.roles IS
    'The roles the caller held at access time, as a JSON array of role names; NULL when no principal was authenticated and on records written before the column existed.';

ALTER TABLE audit_event
    ADD COLUMN origins      jsonb,
    ADD COLUMN origin_count bigint;

COMMENT ON COLUMN audit_event.origins IS
    'The distinct originating systems of the data this access served, as a JSON array of system ids, capped; NULL when the operation served no version body or the record predates the column.';
COMMENT ON COLUMN audit_event.origin_count IS
    'The number of distinct origins the access served; greater than the array length when the array was capped.';
