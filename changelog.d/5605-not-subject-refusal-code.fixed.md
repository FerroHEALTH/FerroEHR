- A patient, the `subject` caller kind of a SMART App Launch patient-context
  credential, asking for an EHR other than the one its launch context names
  is refused `403` with the new `access refusal` code `refused-not-subject`
  in place of `refused-consent-required`, since no consent is involved. The
  problem body, the access-log entry's `outcome.reason` and the
  `access_decisions_total` metric carry the new code, and the access record
  names the refusal as step 0 (subject).
