- A read under a federation agreement with a `consent` condition is served
  when the token carries the consent decision reference for its patient
  (Wabvpz Art. 1(j) and Art. 15a(1)), in the claim or `authorization_details`
  member the issuer's `[[auth.federation]]` table maps to
  `consent.reference`, and refused with `403` without it. The reference is
  one string, the token's `patient/` grant holds it to the launch-context
  patient, and the access record carries it with the other external fields.
  `ferrohealth-smart` moves to 0.9.3, which reads the field.
