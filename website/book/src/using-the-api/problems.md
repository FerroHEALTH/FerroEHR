# Problem types

Every error on FerroEHR's openEHR and extension routes is an RFC 9457
problem details document ([Content negotiation & errors](content-negotiation.md#error-responses)).
Its `type` is either `about:blank`, an error that means no more than its HTTP
status, or one of the FerroEHR types below. Each type's URI is this page at
the type's anchor, such as
`https://ferroehr.eu/docs/latest/using-the-api/problems.html#aql-invalid`.
The title is the same for every occurrence; the members marked "with
`Prefer: return=representation`" appear only when the request carries that
header.

## `aql-invalid`

**Status:** `400 Bad Request`. **Title:** The AQL query is not valid.

The AQL text does not parse, or the query engine refuses it (a construct
outside the supported AQL, a path it cannot type, a parameter it cannot bind).
Correct the query; `detail` says what was refused.

With `Prefer: return=representation`: `position`, `{"start": …, "end": …}`,
the byte range of the query text a parse refusal points at.

## `query-timeout`

**Status:** `408 Request Timeout`. **Title:** The query ran longer than the
server allows.

The query ran past `query.timeout_ms` and the server aborted it. Narrow the
query (a tighter `WHERE`, a `LIMIT`, `fetch` and `offset`) or ask the operator
to raise the limit.

## `validation-failed`

**Status:** `422 Unprocessable Content`. **Title:** The content fails
validation.

The payload parsed but fails its template, the Reference Model, the
terminology, or the data-minimisation rules. Correct the content at each path.

With `Prefer: return=representation`: `errors`, one
`{"path": …, "detail": …}` per violation, keyed by the RM path of the
offending node.

A template upload (`POST /definition/template/adl1.4` and `…/adl2`) declares no
`422` in ITS-REST 1.1.0, so it answers the same refusal as an `about:blank`
**400**, with the same `detail` and `errors`.

## `content-invalid`

**Status:** `422 Unprocessable Content`. **Title:** The content cannot be
processed.

The payload is well-formed but cannot be processed, such as an archetype the
server cannot accept. `detail` names the rule. A template the server cannot
accept on upload is a **400**, as under `validation-failed`.

## `conflict`

**Status:** `409 Conflict`. **Title:** The request conflicts with the
resource's current state.

The resource already exists (an EHR for the subject, a composition with the
given id) or the write conflicts with a concurrent one. Read the resource and
decide whether to retry.

## `version-conflict`

**Status:** `412 Precondition Failed`. **Title:** The version named is not the
latest.

The `If-Match` header names a version other than the latest, so another write
came first. The `ETag` header carries the latest version; read it, merge, and
write again against it.

With `Prefer: return=representation`: `current_version`, the latest version,
the same value as the `ETag`.

## `unauthenticated`

**Status:** `401 Unauthorized`. **Title:** The request needs credentials.

The request carried no credentials, or credentials the server did not accept.
The `WWW-Authenticate` header says which scheme to use.

## `forbidden`

**Status:** `403 Forbidden`. **Title:** The caller may not perform this
operation.

The caller is authenticated, and the access policy (roles, SMART scopes, EHR
access, assurance level) refuses the operation. `detail` names the missing
grant.

## `processing-restricted`

**Status:** `403 Forbidden`. **Title:** Processing of this record is
restricted.

The record carries a restriction of processing (GDPR Art. 18), so the server
keeps it stored and refuses other processing until the restriction is lifted.

## `unavailable`

**Status:** `503 Service Unavailable`. **Title:** The server is temporarily
unavailable.

The server or its database is overloaded or draining. Retry after the
`Retry-After` header's delay.

## `rate-limited`

**Status:** `429 Too Many Requests`. **Title:** The caller sent more requests
than its rate allows.

The caller passed `[server.caller_rate]`. Retry after the `Retry-After`
header's delay.
