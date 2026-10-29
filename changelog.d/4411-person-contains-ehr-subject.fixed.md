- `FROM PERSON p CONTAINS EHR e` (and the `ORGANISATION` and `ROLE` roots)
  joins a party to the EHR whose `EHR_STATUS.subject.external_ref` names it,
  so a party created with `POST /demographic/person` and an EHR created with
  `POST /ehr` answer their row with no other call. Before, the join read only
  the party-to-EHR map, which no REST route writes, and answered no rows on a
  deployed server. The reference matches when its `id.value` is the party's
  uid without `::<system>::<version>` and its `type` is the root class or its
  RM ancestor (`ACTOR`, `PARTY`), in any namespace; an EHR linked both ways
  answers once.
