- A revision history or a current-version read of a COMPOSITION committed
  after the partitioned `version` table last had statistics could walk the
  whole commit audit index: the planner held the highest commit audit id
  of the seed for the highest there is and rated a merge join over
  `pk_commit_audit` as cheap. On a store of 1 300 compositions the history
  of a four-version object read 1 318 buffers instead of 21, and the
  storage bench read `revision_history` at about 440 buffers per call in 3
  of 8 rounds. The server now keeps the partitioned parents' statistics
  (#4453), and each read probes the commit audit by key.
