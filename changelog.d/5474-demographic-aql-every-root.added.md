- AQL over the demographic store serves every demographic root: `GROUP` and
  `AGENT` as `PERSON` is, the abstract `PARTY` and `ACTOR` over the concrete
  classes below them, and a standalone `PARTY_RELATIONSHIP`. The policy
  filter applies under every root, through `CONTAINS` and the join to an
  `EHR`, so a caller who is no administrator never receives a system
  `AGENT`, a policy `ROLE`, an acts-for relationship or a party holding one.
  Each `reverse_relationships` reference now names exactly one relationship,
  by its `uid`, by a unique node id and name, or by its position.
