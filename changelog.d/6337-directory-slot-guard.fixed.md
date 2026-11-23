- A `PUT /ehr/{ehr_id}/directory` on a logically deleted directory and a
  concurrent `POST /ehr/{ehr_id}/directory` no longer both commit, which left
  the EHR with two live directories: each write requires the directory slot
  its check found to be unchanged when it commits, so the later update answers
  `412` and the later create answers `409`.
