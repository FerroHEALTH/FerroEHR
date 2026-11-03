- With attribute-based access control on, both EHR Extract import routes
  (`POST /message/import` and `POST /message/import/{ehr_id}`) pass the ABAC
  pre-check a direct write passes, the patient scope on the target EHR
  included, and are audited as EHR writes. Before, neither route reached the
  ABAC policy-enforcement point, so a caller bound to one patient could import
  into any EHR the coarse layers admitted, and the request was audited under
  the generic application-activity class.
