- A write to a restricted COMPOSITION (`POST`, `PUT` or `DELETE` on
  `/ehr/{ehr_id}/composition`) refused under a GDPR Art. 18 restriction now
  answers with the `processing-restricted` problem type, as the reads of a
  restricted record already did, in place of the generic `forbidden` type.
  The status stays `403`.
