- A `POST /ehr/{ehr_id}/directory` and a `PUT /ehr/{ehr_id}/directory` that
  brings a logically deleted directory back, sent at the same moment, no
  longer both commit and leave the EHR with two live directories: the writes
  of the directory endpoint take turns per EHR, so the later create answers
  `409` and the later update `412`. A CONTRIBUTION that adds a folder
  hierarchy does not wait on them.
