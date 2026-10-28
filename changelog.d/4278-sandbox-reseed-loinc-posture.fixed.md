- The hosted sandbox seeds its full demo dataset again. The v5.5.0 reseed
  stopped at the ADL 2 archetype library (233 stored / 89 refused against the
  pinned 231 / 91) because the box's `.env` still forced FerroTERM 0.1.6, which
  reads none of the FerroTERM 0.3 settings the sandbox posture passes, so it
  served no LOINC and VETDF could only warn on `braden_scale`'s mistyped
  LA9605-4. The corpus, the pin and the CDR were right. The seed now asks the
  CDR for a LOINC code before the library step and names a posture without
  LOINC (`SANDBOX_TERMINOLOGY=without-loinc` opts a local run into 233 / 89),
  checks that the two files it names move together, and a corpus test holds
  the manifest's pin to the adjudicated split with and without LOINC. The
  FerroPIX rollout script retires a stale `FERROTERM_IMAGE` override, checks
  that FerroTERM runs the pinned version and serves LOINC, and wipes before it
  seeds over an unfinished seed.
