- A COMPOSITION create, update and delete commit in one guarded statement
  that writes nothing unless what the write's gate read judged still holds:
  the EHR modifiable and unrestricted, the object's head and restriction mark
  unchanged, and the policy and definition revisions the decision rested on.
  The event outbox row rides in the same statement. A write that finds
  anything moved is judged again from a new gate read, so it answers as it
  did when it waited for the per-object lock, at most eight times before
  `503` with `Retry-After`; the per-object advisory lock is gone, and the
  version's keys refuse a second version at a place already taken. The new
  `commit_retries_total{reason}` counter counts the writes judged again.
