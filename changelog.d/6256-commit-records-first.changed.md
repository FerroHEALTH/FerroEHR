- A COMPOSITION create, update and delete and a CONTRIBUTION of COMPOSITION
  members write their access record into the audit log before their one
  commit statement, which reserves it beside the rows it writes, so the write
  needs no transaction of its own; a held batch of access-log entries goes in
  by one statement. A write that commits nothing discards its record at once.
  A create, an update and a CONTRIBUTION of one composition send 4 statements
  and a delete 5.
