- **Breaking for clients that relied on the old leniencies:** `POST
  /ehr/{ehr_id}/contribution` and `POST /demographic/contribution` now read the
  body through the released `NewContribution` schema, and refuse every shape it
  does not define. Each refused shape answers **400**:
  - a `lifecycle_state` in the SM spelling `{"terminology_id": …,
    "code_string": …}`, as a plain string, or as a bare `{"value": …}`. Send
    a `DV_CODED_TEXT`: `{"value": "complete", "defining_code":
    {"terminology_id": {"value": "openehr"}, "code_string": "532"}}`;
  - a `description` sent as a plain string, on the contribution `audit`, on a
    `commit_audit` or on an attestation. Send a `DV_TEXT` or `DV_CODED_TEXT`
    object: `{"_type": "DV_TEXT", "value": "…"}`;
  - a `commit_audit` or contribution `audit` without `change_type`. The server
    no longer infers creation or modification from `preceding_version_uid`;
    send the code of the change, for example `249` (creation), `251`
    (modification), `523` (deleted) or `666` (attestation);
  - a `commit_audit` or contribution `audit` whose `_type` is `ATTESTATION` or
    `UPDATE_ATTESTATION`, or any value other than `UPDATE_AUDIT` or
    `AUDIT_DETAILS`. Send the audit as `UPDATE_AUDIT` (or with no `_type`) and
    put each attestation in the version's `attestations` list as an
    `UPDATE_ATTESTATION`. A `666` (attestation) version carries its
    attestations there too, and one with no attestation is refused;
  - a missing `audit`, which answered 422 before. A missing `versions` list,
    an attestation without `reason` or `is_pending`, a `system_id` that is not
    a string, and a `preceding_version_uid` sent as a plain string instead of
    `{"value": "…"}` are refused the same way.
- An attestation's own `system_id` is kept, and one that states none takes the
  `system_id` of the version audit it belongs to instead of the server's. An
  attestation committer is now checked against its `PARTY_PROXY` rules like
  every other committer, so an invalid one answers 422.
