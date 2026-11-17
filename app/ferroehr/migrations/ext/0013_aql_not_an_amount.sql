-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The refusal of a value SUM or AVG cannot add (#5917).
--
-- QUERY master03 §SUM, §AVG: "Input values type should be either Integer or
-- Real". A whole data value reached through an ELEMENT's value is typed only
-- at run time, so the statement adds a JSON number or the magnitude of a
-- DV_AMOUNT (RM data_types §DV_AMOUNT: `+` is defined for relative amounts and
-- not for the date/time types) and calls this function for any other value,
-- failing with SQLSTATE ZA003, which the engine answers as a 400. Our own
-- design: the specification says what SUM and AVG take, not how a server
-- refuses the rest.
--
-- Runs with search_path = ext.

CREATE FUNCTION aql_not_an_amount(found text) RETURNS numeric
    LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE
    AS $$
BEGIN
    -- A RAISE with no level raises an error (PostgreSQL docs §41.9); the
    -- function traps nothing.
    RAISE 'SUM and AVG add numbers and amounts, not a %', found
        USING ERRCODE = 'ZA003';
END
$$;

COMMENT ON FUNCTION aql_not_an_amount(text) IS 'Raises SQLSTATE ZA003: SUM or AVG met a value that is no number and no DV_AMOUNT. Our own design.';
