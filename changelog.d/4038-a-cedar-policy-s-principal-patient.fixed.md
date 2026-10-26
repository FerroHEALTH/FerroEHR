- A Cedar policy's `principal.patient` is the caller's patient claim
  (`[authz.abac] patient_claim`) (#4038). It was the patient of the object in
  play, so a rule such as `principal.patient == resource.patient` in the shipped
  `consent.cedar` held for every object a query touched.
