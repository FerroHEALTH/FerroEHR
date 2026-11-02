- the `db_transactions` counter (`outcome="commit"`) now counts every committed single-object
  write. It used to leave out `EHR_STATUS` and DIRECTORY writes.
