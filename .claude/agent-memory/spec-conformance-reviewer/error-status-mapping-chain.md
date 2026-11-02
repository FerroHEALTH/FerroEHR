---
name: error-status-mapping-chain
description: Where the ServiceError→ApiError→HTTP status table lives and how FK/integrity violations get classified
metadata:
  type: project
---

The service error → wire status chain, with no intermediate status enum:

- `app/ferroehr-rest/src/overview/service_error.rs::api_error` is the one mapping of a `ServiceError` onto the wire (`openehr_its::rest::runtime::ApiError`): NotFound→404, BadRequest→400, Conflict→409, VersionConflict→412, Unprocessable→422, ValidationFailed→422 with `validationErrors[]`, Overloaded/Unrecorded→503, Restricted/Forbidden→403, Timeout→500 marked so `RestError` answers 408, and Storage/Database/Acquire re-classified through `ServiceError::classified()`. `impl From<ServiceError> for RestError` (`overview/error.rs`) calls it.
- `app/ferroehr/src/storage/error.rs::classify_sqlx` classifies a raw `sqlx::Error`: SQLSTATE `23505`/`23503`/`23001`/`23P01` (unique, FK, restrict, exclusion) and serialization/deadlock → Conflict → 409; pool timeout, a cut connection and `57014` → Overloaded → 503 plus `Retry-After`; everything else → Fault → 500. `classify_acquire` and `classify_storage` sit beside it.

Consequence worth remembering: a raw FK violation surfaces as **409**, not 500, so a service path without an explicit reference pre-check still answers 409; only the message differs from a path with a friendly pre-check.
