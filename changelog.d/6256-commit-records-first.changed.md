- A COMPOSITION create, update and delete and a CONTRIBUTION of COMPOSITION
  members write their access record into the audit intake before their one
  commit statement, which reserves it beside the rows it writes, so the write
  needs no transaction of its own. A write that commits nothing discards its
  record at once. Each of those commits sends 5 statements.
