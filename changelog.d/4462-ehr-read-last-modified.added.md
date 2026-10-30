- `GET /ehr/{ehr_id}` and `GET /ehr?subject_id=…` now send `Last-Modified`:
  the commit that created the EHR, or the later commit that added or deleted
  one of its folder hierarchies.
