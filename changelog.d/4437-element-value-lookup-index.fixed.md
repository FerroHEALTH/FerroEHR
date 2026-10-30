- An AQL lookup by an ELEMENT value, such as a person by an identifier
  (`WHERE id/items[at0001]/value/id = $v`) or a composition by a text value,
  read every node of the archetype in both stores and tested the value last:
  about 350 ms at 50 000 persons. The engine now anchors such an equality on
  the ELEMENT rows of the same version by their search text
  (`ext.node_search_text`, ext migration `0006`), which the new partial
  expression index `idx_node_hot_value_search` and its statistics object
  serve (clinical migration `0025`, party migration `0016`), so the lookup
  reads the one object: 26 buffers and under a millisecond at 50 000
  persons. The party store also gets the archetype dependency statistics the
  clinical store has had since 5.6.0-rc2 (party migration `0017`). The
  storage bench measures the lookup as `person_identifier_lookup`.
