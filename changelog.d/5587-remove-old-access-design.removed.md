- **Breaking: the restriction and opt-out marks are gone.** Clinical
  migration 0043 drops `ehr.access_restricted_at`, `vo_head.access_restricted_at`
  and `ehr.opted_out_at` with the trigger that copied a whole-EHR mark onto a
  new object, and the boot no longer projects them. The change feed now
  serves only the events the access evaluator lets its reader read, judged as
  any read of that EHR's data: a directive that binds the reader and denies
  the data, an access policy it does not meet (`purpose_required` with no
  permitted purpose declared) or `[authz.rbac] ehr_access_default` for an EHR
  with no settings withholds the event. A page is no longer cut
  short by the rows it leaves out. A data scope that names an object by a
  reference naming no versioned object is refused `400` for every directive
  kind, where only a restriction was before. The draft access-control and
  access-log specifications move from `docs/plans/` to
  `docs/specifications/access-control/` and `docs/specifications/access-log/`.
