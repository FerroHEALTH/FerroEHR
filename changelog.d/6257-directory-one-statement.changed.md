- A directory create, update or delete writes its access record first and
  then commits in one statement, a new directory's membership in the EHR
  included, judged on one read of the EHR, its directory and the versions the
  FOLDER's items reference. A create sends 4 statements where it sent 14, an
  update 4 where it sent 13 and a delete 4 where it sent 12. Their answers
  stay as they were.
