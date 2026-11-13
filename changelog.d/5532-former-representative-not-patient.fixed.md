- A former patient-portal representative is no longer admitted as the patient
  once their representation ends or is deleted (#5532). On the own-login
  path, a login whose `PERSON` holds or has ever held a representation of
  the EHR's subject is judged only as a representative: admitted while a
  representation is live, refused with `refused-representation-unrecorded`
  once none is. FerroEHR keeps who once represented whom in the binding side
  table, and a refill rebuilds it from every version.
