- A representative on an EHR other than the one whose subject it represents
  is refused `403` with the new `access refusal` code
  `refused-not-represented`, recorded in the access log as step 0
  (representation), on every route that names an EHR, a query's or import's
  single-EHR scope and a manifest export included, and on an operation on EHR
  data that names no EHR. The SMART layer refused these with a bare `403`, no
  refusal code and no access-log step. The access evaluator now confines the
  representative alone, to the EHR of the subject its recorded representation
  names, and the SMART layer checks only a grant's resource family and
  permission.
