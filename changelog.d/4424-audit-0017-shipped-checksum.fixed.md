- An audit store that 5.6.0-rc2 migrated starts again. Audit migration 0017
  changed after 5.6.0-rc2 shipped it, so the server refused such a store on
  the migration's checksum at boot. 0017 is back to its 5.6.0-rc2 text, and
  the new audit migration `0019_search_tokens_pending_list.sql` gives the
  search token index its pending list of 512 kB, so a store that 5.6.0-rc2
  migrated upgrades in place.
