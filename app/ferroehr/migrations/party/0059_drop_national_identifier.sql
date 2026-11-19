-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: drop the national-identifier protection's tables (#6092).
--
-- A national identifier belongs in the master patient index, never in the
-- CDR, which holds the subject only as a pseudonym, so the server no longer
-- seals an identifier out of a party body and reads no row here. Dropping the
-- tables also drops the grants 0008 and 0010 gave on them. No openEHR spec
-- governs this: our own design.
--
-- Runs with search_path = party, ext, public.

DROP TABLE IF EXISTS national_identifier;
DROP TABLE IF EXISTS identifier_scheme;

COMMENT ON TABLE binding_patient_identifier IS 'The identifiers each patient PERSON head that is not deleted carries at the [authz.party_binding.patient] identifier path, by issuer, each id as the body carries it. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as the way to a patient''s EHRs, our own design.';
