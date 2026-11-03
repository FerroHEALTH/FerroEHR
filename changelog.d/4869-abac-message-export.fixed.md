- With attribute-based access control on, both EHR Extract export routes pass
  the ABAC pre-check: `GET /message/export/{ehr_id}` the one a read of the EHR
  passes, and `POST /message/export` the patient scope on every EHR its
  manifest names, before any content is read. Before, neither route reached
  the ABAC policy-enforcement point, so a caller bound to one patient could
  export any EHR the coarse layers admitted.
