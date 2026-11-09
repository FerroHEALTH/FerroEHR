- A deployment whose FerroBRIDGE facade or FerroFED gateway writes to
  FerroEHR adds its `persons` issuers only after the bridge and the gateway
  reach it under the caller: FerroBRIDGE v0.0.23 and FerroFED v0.0.17. Until
  then the committer rule refuses their writes. FerroFED v0.0.17 follows this
  release, so a deployment with a FerroFED gateway adds its `persons` issuers
  only once that release is out. The book's new "Party binding: bootstrap and
  migration" page carries the bootstrap path, the four migration steps and
  these prerequisites, and "Breaking changes of party binding" lists every
  incompatible change with its upgrade step.
