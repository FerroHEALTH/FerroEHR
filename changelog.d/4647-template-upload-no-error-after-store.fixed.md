- `POST /definition/template/adl1.4` builds the template's Web Template
  before it stores the template, so a template it cannot build is refused
  with `400` and nothing is stored. Once the template is stored, the upload
  answers `201` with its `Location`; with `Prefer: return=representation` and
  a Web Template `Accept` it serves the Web Template built before the store,
  and the OPT if that cannot be served. It no longer answers `422` for a
  template it has already stored.
