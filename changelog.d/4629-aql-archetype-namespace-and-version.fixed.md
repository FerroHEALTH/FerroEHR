- An AQL archetype predicate matches the namespace exactly and reads the
  version it names as the oldest release it accepts within the major:
  `org.a::…` no longer finds data recorded in `org.b`, an id without a
  namespace finds only data recorded without one, and `.v1.4.2` no longer
  finds data recorded under `v1.0.0` (BASE Artefact Identification
  §Referencing and §Supporting Archetype-based Querying; #4629, #4630).
