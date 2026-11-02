- The served OpenAPI description of `POST /query/cohort` no longer names the
  linkage domain, which the server no longer has. It says the predicate selects
  parties on the demographic database credential, and that those parties are
  resolved to the EHRs whose `EHR_STATUS.subject` names one of them on the
  clinical credential. The route, its fields and its status codes are
  unchanged.
