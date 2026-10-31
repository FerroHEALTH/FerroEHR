- **Breaking for clients that relied on the remaining commit-audit
  leniencies:** the audit rules of the CONTRIBUTION commit now hold on every
  write path.
  - A `description` object without `_type`, such as `{"value": "…"}`, is
    refused with **400** on a contribution `audit`, a `commit_audit` and an
    attestation, and so is an attestation `reason` without `_type`. Send
    `{"_type": "DV_TEXT", "value": "…"}` or a `DV_CODED_TEXT` with its
    `_type`.
  - A stated empty `system_id` is refused with **422** on the direct writes
    too (`openehr-audit-details: system_id=""` on a create, update or delete),
    where the server used to replace it with its own. Leave `system_id` out to
    get the server's, or send a non-empty one. A `system_id` that is not a
    string stays a **400** on both CONTRIBUTION routes.
  - A commit audit of type `UPDATE_ATTESTATION` handed to the library's direct
    write calls (`create_composition`, `update_composition`, the EHR_STATUS,
    DIRECTORY and party writes) is refused with **400**, as on the
    CONTRIBUTION routes. Send the audit as `UPDATE_AUDIT` and put each
    attestation in the version's `attestations` list.
- Members that `NewContribution` or `UPDATE_AUDIT` do not declare, such as a
  `reason` or `is_pending` on a plain audit or an extra top-level member of a
  contribution, are accepted and ignored: the released schemas leave both
  objects open.
