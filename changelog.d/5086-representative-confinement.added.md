- A bound representative reaches only the EHR whose `EHR_STATUS` names the
  subject it represents. Every composition, `EHR_STATUS`, directory,
  contribution and query request is held to that EHR whatever the token's
  scopes cover, and a query must name it with `ehr_id`. A representative
  whose token names a subject its party has no live representation
  relationship to, or whose issuer no `[[auth.party]]` table binds, is
  refused with `403` and `refused-representation-unrecorded`.
