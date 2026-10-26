- **The local EHR index holds an EHR created for a subject.** Under
  `[ehr_index] provider = "local"`, `create_ehr_for_subject` now enters the
  subject in the local map with the EHR's creation, as a `Primary`
  association with no location, so `subject_ehrs` and a subject cohort find
  the EHR. Before, only `EHR_STATUS` named the subject and the map stayed
  empty until `add_ehr_subject` ran.
