# Using the API

FerroEHR exposes the openEHR **REST API** (ITS-REST Release-1.1.0, the version
the server reports and is conformance-tested against): a resource-based HTTP
interface for creating EHRs, committing and retrieving versioned clinical
documents, managing folders and contributions, and running queries. This part is
the practical reference for client developers: the resources and their
operations, the headers that drive versioning and content negotiation, and the
error contract. For the complete endpoint reference (every path, parameter, and
schema), open the Swagger UI of the live sandbox at
<https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui> and sign in with the
public demo credentials `ferroehr` / `ferroehr`. That server generates the
document from its own handlers, so it describes the running release and its
"Try it out" buttons issue real requests. The sandbox's landing surface,
<https://sandbox.ferroehr.eu>, is the viewer over the same server and
takes the same credentials. This book explains how to *use* the API.

## Base path

All clinical API routes hang off a configurable base path, which defaults to:

```text
/ferroehr/rest/openehr/v1
```

Every path in these chapters is relative to that base. So "`POST /ehr`" means
`POST http://your-host:8080/ferroehr/rest/openehr/v1/ehr`. The base path is set
with `FERROEHR__SERVER__BASE_PATH`, and it may be shortened as far as
`/ferroehr/v1` (see
[Shortening the REST base path](../installation/config-server.md#base_path-shortening-the-rest-base-path)).

The status and documentation routes hang off the **REST root**, which the server
derives from the base path by dropping the segments that name the openEHR API:
`/ferroehr/rest` by default. The public, unauthenticated status probe is at
`/ferroehr/rest/status` and interactive docs at `/ferroehr/rest/swagger-ui` at
the access level `swagger_ui` names (by default any authenticated principal, so
a browser prompts for the API credential; the documents list the whole enabled
operation surface). Every deployment serves that UI from its own routes, so your
own server always documents its own surface. The health probes stay at the
process root whatever the base path is: `/health`, `/health/liveness`,
`/health/readiness`.

## Capability discovery

An `OPTIONS` request to the **API base path** returns the server's
**conformance manifest**: the product name and version, the vendor, the openEHR
REST API version it implements, the conformance profile it claims, and the API
groups this deployment actually mounts.

```shell
curl -X OPTIONS -i \
  http://localhost:8080/ferroehr/rest/openehr/v1
```

The response carries an `Allow` header and a JSON body:

```json
{
  "solution": "FerroEHR",
  "solution_version": "…",
  "vendor": "Cadasto B.V.",
  "restapi_specs_version": "1.1.0",
  "conformance_profile": "…",
  "endpoints": ["/ehr", "/definition", "/query", "/demographic"]
}
```

Two things to know before you build discovery on it:

- **`endpoints` is the live set**, not a fixed list: `/admin` appears only when
  the admin API is enabled. It covers the **standardised** openEHR groups only;
  FerroEHR's own extension families (health, management, messaging, item tags,
  the archetype-source routes) declare themselves through the served OpenAPI
  document instead.
- **The identity fields are configurable** (`[server.identity]`, see
  [Server, database & telemetry](../installation/config-server.md#serveridentity));
  their defaults are the build's own provenance and the last machine-computed
  conformance verdict, so an unmodified deployment never over-claims.

> [!NOTE]
> The manifest lives at the API base path and nowhere else. A bare `/` alias
> existed in earlier versions and was removed. Point discovery at the base
> path.

## Authentication

Requests are authenticated unless auth is explicitly disabled. Two mechanisms
ship:

- **HTTP Basic:** a configured user store; send `Authorization: Basic …`.
  The examples in this book use `-u user:password` with curl.
- **OAuth2 / OIDC bearer tokens:** send `Authorization: Bearer <token>`,
  validated against a configured issuer (Keycloak, Entra ID, any
  standards-compliant provider).

Authorization is coarse role-based access control by default (a `USER` role for
clinical operations, an `ADMIN` role for admin operations), with optional
attribute-based policies on top. The full picture (mechanisms, roles, the
pseudonymisation boundary) is in [Security](../security.md).

### Which status a credential problem gets

The distinction matters when you are writing a client, because only one of these
means "fix your credential":

| Situation | Status | What it means |
|---|---|---|
| No `Authorization` header | `401` | with a `WWW-Authenticate` challenge listing the schemes this server implements, and **no** `error=` code — nothing has gone wrong yet ([RFC 6750 §3.1](https://www.rfc-editor.org/rfc/rfc6750#section-3.1)) |
| Credential presented and rejected | `401` | the challenge carries `error="invalid_token"`: expired, revoked, malformed, or simply wrong |
| `Authorization` header malformed | `400` | an unparsable header, an unknown scheme, or a bearer token outside the [RFC 6750 §2.1](https://www.rfc-editor.org/rfc/rfc6750#section-2.1) `b64token` grammar. The server never got as far as a credential, so this is a request defect (`error="invalid_request"`) |
| Authenticated, not permitted | `403` | for a bearer caller the challenge carries `error="insufficient_scope"`, naming what is missing |
| The token issuer is unreachable | `503` | with `Retry-After`. No token can be validated, so the server cannot decide; it is **not** a statement about your credential ([RFC 9110 §15.6.4](https://www.rfc-editor.org/rfc/rfc9110#section-15.6.4)). Retry; do not discard the token |

Two Basic-auth details worth knowing: the credential must be **padded** base64
(RFC 7617 §2 defers to RFC 4648, whose
[§3.2](https://www.rfc-editor.org/rfc/rfc4648#section-3.2) requires the pad
characters; an unpadded credential is refused), and an unknown username costs
the same time as a known one, so response timing reveals nothing about which
accounts exist.

> [!WARNING]
> The quickstart ships a throwaway Basic user (`ferroehr` / `ferroehr`), and so
> do the examples in this book. Replace it before any real use.

## The chapters here

- **[Resource walkthroughs](resources.md):** EHR, EHR_STATUS, COMPOSITION,
  DIRECTORY, and CONTRIBUTION, each with real curl examples, the headers they
  need, and the status codes they return.
- **[Content negotiation & errors](content-negotiation.md):** choosing JSON or
  XML, the simplified formats, the `Prefer` header, `ETag`/`If-Match` optimistic
  concurrency, the commit-metadata headers, and the error response shape.

For querying, see [Querying with AQL](../querying-aql.md); for loading
templates, [Templates & validation](../templates-validation.md); for the admin,
messaging and management surfaces,
[Admin & messaging APIs](../operations-admin-apis.md).
