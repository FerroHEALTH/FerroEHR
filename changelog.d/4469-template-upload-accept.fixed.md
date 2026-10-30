- An ADL 1.4 template upload with `Prefer: return=representation` now answers
  the representation `Accept` names: the OPT XML, or the Web Template under
  `application/openehr.wt+json` or `application/json`. An `Accept` outside
  those answers `406` and stores nothing. The upload always answered XML.
