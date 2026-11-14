- A replica no longer holds every patient's care relationships in its copy of
  the caller facts, which grew to about 1.6 GB and a 4 s reload at one million
  patients in care. A subject's care relationships, and the patients of a care
  team, are read on a miss and held in two caches of a fixed bound (4096
  subjects, and care teams holding 262 144 patients together), which the facts
  notification drops by the parties it stamps. Party migration 0041 indexes the
  care relationships by care team. A care relationship that cannot be read
  answers `503`.
