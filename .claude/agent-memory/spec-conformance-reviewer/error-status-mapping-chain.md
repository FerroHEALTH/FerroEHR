---
name: error-status-mapping-chain
description: Where the ServiceError→StatusError→HTTP status table lives and how FK/integrity violations get classified
metadata:
  type: project
---

The service error → wire status chain in `app/ferroehr`:

- `app/ferroehr/src/service/error.rs` — `impl From<ServiceError> for StatusError` carries the authoritative table (NotFound→VersionedObjectDoesNotExist→404, VersionConflict→VersionMismatch→412, Conflict→CompositionAlreadyExists→409, Unprocessable/ValidationFailed→ContentInvalid→422, BadRequest→PreconditionViolation→400). `ServiceError::new(status, msg)` is the inbound (status→ServiceError) mirror.
- `app/ferroehr/src/storage/error.rs::classify_sqlx` — raw `sqlx::Error` classification: **SQLSTATE class 23 (integrity/FK/unique) → Conflict → 409**; 40001/40P01 (serialization/deadlock) → 409; PoolTimedOut → ServiceOverloaded → 503; everything else → Exception → 500.

Consequence worth remembering: a raw FK violation (e.g. deleting a `template_store` row still referenced by `vo_version` via `fk_vo_version_template` NO ACTION) surfaces as **409**, not 500. So a service path that omits an explicit reference pre-check still returns 409 on the FK — but leaks the raw sqlx constraint message. A sibling path with an explicit `count(*)` pre-check returns the same 409 with a friendly message. Both are wire-consistent; only message quality differs (`delete_opt` = raw, `admin_template_delete` = friendly).
