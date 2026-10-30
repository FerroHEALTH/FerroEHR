- A directory create or update whose root FOLDER names an archetype in
  `archetype_node_id` and carries no `archetype_details` now answers `422` and
  commits nothing: an archetype root carries `archetype_details` (RM common
  §The LOCATABLE Class). FerroEHR created such a directory. A
  `553|incomplete|` commit and a nested FOLDER keep it optional. The viewer's
  empty directory root now carries `archetype_details`.
