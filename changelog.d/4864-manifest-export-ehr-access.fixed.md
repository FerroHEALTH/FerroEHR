- `POST /message/export` checks the `EHR_ACCESS` settings of every EHR its
  manifest names, by `ehr_id` or `subject_id`, before it reads any content,
  and refuses the whole export with the `403` or `500` a read of that EHR
  gets. Before, the manifest form passed no `EHR_ACCESS` gate: any caller the
  route admitted could export an EHR whose access list refused them, or one
  held closed because its settings do not parse.
