- A `553|incomplete|` commit no longer refuses an empty mandatory archetype
  slot: RM common master06 §Incomplete Content sets every occurrences lower
  limit to zero, a slot's included, while too many fillers are still refused.
  The OPT 1.4 writer now closes a mandatory slot once the fillers placed in it
  carry its lower bound, as ADL 2 §Slot Filling and Redefinition allows, and
  still refuses to close one they leave unmet.
