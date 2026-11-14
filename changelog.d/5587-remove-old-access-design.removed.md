- **Breaking: the restriction and opt-out marks are gone.** Clinical
  migration 0043 drops `ehr.access_restricted_at`, `vo_head.access_restricted_at`
  and `ehr.opted_out_at` with the trigger that copied a whole-EHR mark onto a
  new object, and the boot no longer projects them. The change feed now serves
  only the events the access evaluator lets an access service of the `system`
  kind read, judged against the EHR's own `EHR_ACCESS` settings when the feed
  is read: a restriction or a consent deny that binds such a caller withholds
  the event, with its data scope, validity and exceptions, and an EHR with no
  settings follows `[authz.rbac] ehr_access_default`. A page is no longer cut
  short by the rows it leaves out. A data scope that names an object by a
  reference naming no versioned object is refused `400` for every directive
  kind, where only a restriction was before. The draft access-control and
  access-log specifications move from `docs/plans/` to
  `docs/specifications/access-control/` and `docs/specifications/access-log/`.
