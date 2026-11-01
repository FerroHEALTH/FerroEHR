- A cohort that names its subjects now finds, under `[ehr_index] provider =
  "local"`, the EHR created over `POST /ehr` with that subject in its
  `EHR_STATUS`; it read a cross-reference table no route wrote, so it matched
  nothing. A cohort predicate and the AQL demographic join resolve a party to
  the EHRs whose `EHR_STATUS.subject` names it. The `EHR_STATUS` subject is
  now the one source of which EHR belongs to which subject. **Upgrade note:**
  the `linkage` migration `0006` drops `linkage.subject_ehr` and the
  `linkage.erase_ehr` function. Rows written there through the service API are
  dropped with the table; the EHRs they named keep their subject in their own
  `EHR_STATUS`.
