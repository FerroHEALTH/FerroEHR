- Archetype validation accepts a term binding keyed by a path that runs through
  a `use_node` internal reference, as AOM 2 VTTBK allows ("a path that is valid
  in the flat archetype") and ADL 2 §Paths and Proxy Objects forms it, with the
  target's node id. The 2013 CKM `openEHR-EHR-OBSERVATION.apgar.v1` no longer
  fails VTTBK; a CDR whose terminology server serves LOINC still refuses it
  under VETDF, since it binds LOINC answer codes without their check digit.
