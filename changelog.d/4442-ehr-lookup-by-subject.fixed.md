- An AQL lookup of an EHR by the subject of its EHR_STATUS
  (`WHERE e/ehr_status/subject/external_ref/id/value = $u AND
  e/ehr_status/subject/external_ref/namespace = $n`) joined every EHR_STATUS
  version and read the subject out of its stored fragment: about 150 ms on a
  store of a few thousand EHRs. The two leaves now read the EHR row's
  `subject_id` and `subject_namespace`, the copy the commit keeps of the
  current EHR_STATUS subject, so the lookup enters through `uq_ehr_subject`
  and reads one row. The storage bench measures it as `ehr_by_subject`.
