- Under a separated clinical credential, `POST /ehr`, `PUT /ehr/{ehr_id}` and
  an `EHR_STATUS` update that name a subject no longer fail with a database
  error: the subject pseudonym guard reads the deployment posture, and the
  clinical role now holds `SELECT` on `ext.posture` through the `ext`
  migration `0009`.
