- Every error on the openEHR routes (`/ferroehr/rest/openehr/v1/`), the
  extension routes (`/ferroehr/rest/extensions/v1/`) and the management
  routes answers RFC 9457 problem details, `application/problem+json`, in
  place of the `{ message, validationErrors, error }` body shaped after the
  ITS-REST example. The status codes are unchanged. A problem carries
  `type`, `title`, `status`, `detail` and the request id as a `urn:uuid:`
  `instance`; the refusals a client acts on have FerroEHR types documented
  on the book's Problem types page (`aql-invalid`, `query-timeout`,
  `validation-failed`, `content-invalid`, `conflict`, `version-conflict`,
  `unauthenticated`, `forbidden`, `processing-restricted`, `unavailable`,
  `rate-limited`), every other error is `about:blank`. A request with
  `Prefer: return=representation` receives the additional details ITS-REST
  allows: the violations of a validation refusal as `errors`
  (`{path, detail}`, where `validationErrors` held `"<path>: <message>"`
  strings), the byte `position` of an AQL parse refusal, and the
  `current_version` of a `412`. The stack's own `408`, `413`, `429`, `503`
  and panic `500` are problems too, and an `OperationOutcome` under the
  FHIR base, where an authentication refusal is now an `OperationOutcome`
  as well. The served OpenAPI document declares the `Problem` schema for
  every `4xx` and `5xx`, and the viewer reads problem details. A client
  that read `message` reads `detail`; one that read `validationErrors`
  sends `Prefer: return=representation` and reads `errors`.
