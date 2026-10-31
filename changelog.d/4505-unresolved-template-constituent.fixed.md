- **An ADL 2 template whose constituent archetype the CDR does not hold is
  refused at upload**, with a `422` whose `errors` carry one `VARXR` entry per
  missing archetype (AOM2 `C_ARCHETYPE_ROOT` validity rule VARXR); the 2013
  CKM `t_encounter_opt_test`, built on the refused `apgar`, no longer loads,
  so the sandbox library pin is 230 accepted / 92 refused. A stored template
  whose constituent was deleted later answers its JSON form and example with
  the same `VARXR` list, and the viewer says so on every tab of the template,
  names each missing archetype and links to uploading it, instead of a raw
  `422`.
