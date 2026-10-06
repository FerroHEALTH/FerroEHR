# SMART App Launch

FerroEHR can act as the **resource server** in a SMART App Launch setup: a
clinical app is launched with an OAuth2/OIDC token from your authorization
server (Keycloak or any standards-compliant IdP), and the CDR advertises that
server's endpoints, understands SMART resource scopes in the token, and binds
the launch context (the selected patient/EHR) to what the token may touch.
FerroEHR never issues tokens, registers clients, or serves the OAuth2
endpoints itself; those remain your authorization server's job.

Support is **off by default**. A stock server serves no discovery document and
runs no scope gate, so the wire is byte-identical to a non-SMART deployment
until you opt in.

<!-- toc -->

## Enabling it

Enabling SMART is not a single switch. The discovery document you start
publishing is read by third-party applications to decide where to send an
authorization request and where to exchange a code, so the server **refuses to
boot** on a configuration that would publish an unusable or unsafe one. Six
things must be true together:

```bash
export FERROEHR__SMART__ENABLED=true
# The external origin absolute service URLs are built from — required.
export FERROEHR__SMART__PUBLIC_BASE_URL=https://cdr.example.com
# Where apps obtain tokens. Both required, both absolute https.
export FERROEHR__SMART__ENDPOINTS__AUTHORIZATION_ENDPOINT=https://as.example/authorize
export FERROEHR__SMART__ENDPOINTS__TOKEN_ENDPOINT=https://as.example/token
# What the document claims the authorization server supports.
export FERROEHR__SMART__ENDPOINTS__RESPONSE_TYPES_SUPPORTED='["code"]'
export FERROEHR__SMART__ENDPOINTS__TOKEN_ENDPOINT_AUTH_METHODS_SUPPORTED='["client_secret_basic"]'
export FERROEHR__SMART__ENDPOINTS__CODE_CHALLENGE_METHODS_SUPPORTED='["S256"]'
# And the CDR must be able to validate the tokens apps come back with.
export FERROEHR__AUTH__OIDC__ISSUER=https://as.example/realms/ferroehr
export FERROEHR__AUTH__OIDC__AUDIENCES=ferroehr-api
```

The boot rules, and why each one exists:

| Rule | Why |
|---|---|
| `smart.public_base_url` is an absolute `http(s)` origin | every `services.*.baseUrl` in the document is an absolute URL, and it cannot be built without knowing this server's external origin |
| `authorization_endpoint` and `token_endpoint` are set | an enabled platform without them publishes a document an app cannot act on |
| `[auth.oidc]` is configured | the CDR directs apps to an authorization server, so it must be able to validate the tokens they return; without this every app would obtain a valid token and every request would be refused |
| `smart.endpoints.issuer`, when set, equals `auth.oidc.issuer` | one says where apps *get* tokens, the other which tokens this server *accepts*; a mismatch is silently broken in the most confusing way available |
| every advertised endpoint (and `public_base_url`) is absolute and `https` | a relative endpoint is unusable and a plaintext one exposes the authorization code and the access token (RFC 6749 §3.1.2.1, RFC 8414 §6.2). `allow_insecure_endpoints = true` opts a development authorization server out |
| `response_types_supported` is non-empty | RFC 8414 §2 marks it REQUIRED |
| `token_endpoint_auth_methods_supported` names at least one method | an empty list is not silence: it claims the authorization server supports none, and an app that reads it complies |
| `code_challenge_methods_supported` includes `S256` | SMART App Launch requires PKCE (RFC 7636), and `plain` alone is not sufficient |
| `grant_types_supported` names neither `implicit` nor a password grant | both are deprecated in SMART and must never be advertised |
| the advertised `issuer` carries no query or fragment | the RFC 8414 §2 issuer-identifier rules, the same ones `auth.oidc.issuer` is held to |
| no `smart` URL carries a user name or password, even with SMART disabled | the document publishes these URLs to every app, so a credential in one would be served to anyone who asks |

SMART scopes ride only **Bearer** tokens, so the OIDC bearer requirement above
is also what makes the scope gate able to see a scope at all (see
[Security](security.md#authentication)).

## Configuration keys

The full key set lives in the `[smart]` section of `ferroehr.toml`; each key
can be overridden with the shown `FERROEHR__SMART__*` environment variable (`__`
separates nested fields).

| Key | Default | Meaning |
|---|---|---|
| `FERROEHR__SMART__ENABLED` | `false` | Master switch. Off = no discovery document (404) and an inert scope gate. |
| `FERROEHR__SMART__PUBLIC_BASE_URL` | unset (**required when enabled**) | The external origin absolute `services.*.baseUrl` values are built from, e.g. `https://cdr.example.com`. |
| `FERROEHR__SMART__PLATFORM_BASE_URL` | unset | Base the discovery document hangs off. Unset = the REST root (the configured base path without its `/openehr/v1` tail, i.e. `/ferroehr/rest`). A leading path is honoured (`/gateway/v1` → `/gateway/v1/.well-known/smart-configuration`). |
| `FERROEHR__SMART__EHR_ID_CLAIM` | `ehrId` | Token claim carrying the launch context's openEHR EHR id. |
| `FERROEHR__SMART__PATIENT_CLAIM` | `patient` | Fallback launch-context claim when the EHR-id claim is absent. |
| `FERROEHR__SMART__REQUIRE_SMART_SCOPES` | `false` | Fail-closed switch; see [Advisory vs required](#advisory-vs-required) below. |
| `FERROEHR__SMART__LAUNCH_BASE64_JSON` | `false` | Advertise the base64-JSON launch-parameter capability (experimental; consumed by the app, not the CDR). |
| `FERROEHR__SMART__EPISODE__ENABLED` | `false` | Advertise + accept episode launch context (experimental; advisory only, no episode filtering). |
| `FERROEHR__SMART__ENDPOINTS__ISSUER` | unset | Advertised token issuer. Unset = falls back to the configured OIDC bearer issuer; set, it must equal it. |
| `FERROEHR__SMART__ENDPOINTS__JWKS_URI` | unset | Advertised `jwks_uri`. |
| `FERROEHR__SMART__ENDPOINTS__AUTHORIZATION_ENDPOINT` | unset (**required when enabled**) | Advertised OAuth2 authorization endpoint. |
| `FERROEHR__SMART__ENDPOINTS__TOKEN_ENDPOINT` | unset (**required when enabled**) | Advertised OAuth2 token endpoint. |
| `FERROEHR__SMART__ENDPOINTS__REGISTRATION_ENDPOINT` | unset | Advertised dynamic-client registration endpoint. |
| `FERROEHR__SMART__ENDPOINTS__INTROSPECTION_ENDPOINT` | unset | Advertised token introspection endpoint. |
| `FERROEHR__SMART__ENDPOINTS__REVOCATION_ENDPOINT` | unset | Advertised token revocation endpoint. |
| `FERROEHR__SMART__ENDPOINTS__MANAGEMENT_ENDPOINT` | unset | Advertised user management endpoint. |
| `FERROEHR__SMART__ENDPOINTS__TOKEN_ENDPOINT_AUTH_METHODS_SUPPORTED` | `[]` (**must be non-empty when enabled**) | Advertised client auth methods (e.g. `client_secret_basic`, `private_key_jwt`). |
| `FERROEHR__SMART__ENDPOINTS__GRANT_TYPES_SUPPORTED` | `[]` | Advertised grant types. `implicit` and the password grant are **rejected at boot**. |
| `FERROEHR__SMART__ENDPOINTS__RESPONSE_TYPES_SUPPORTED` | `[]` (**must be non-empty when enabled**) | Advertised response types (e.g. `code`). |
| `FERROEHR__SMART__ENDPOINTS__CODE_CHALLENGE_METHODS_SUPPORTED` | `[]` (**must include `S256` when enabled**) | Advertised PKCE methods. |
| `FERROEHR__SMART__ENDPOINTS__SCOPES_SUPPORTED` | `[]` | Advertised scopes. Empty = a default list reflecting what the CDR enforces; set = emitted verbatim. |
| `FERROEHR__SMART__ENDPOINTS__CAPABILITIES` | `[]` | Extra HL7-defined base capabilities to advertise (e.g. `launch-ehr`, `sso-openid-connect`), the ones your external framework owns. Appended to the CDR's own, deduplicated. |
| `FERROEHR__SMART__ENDPOINTS__ALLOW_INSECURE_ENDPOINTS` | `false` | Accept a non-`https` advertised endpoint. Development/testing only. |

> [!WARNING]
> Everything under `[smart.endpoints]` is **published to third-party apps**, so
> an empty list is a claim, not a silence: it says the authorization server
> supports none of that thing, and a compliant app will believe it. Advertise
> only what your authorization server really offers.

## The discovery document

When enabled, the server serves the standard SMART configuration document,
**unauthenticated**, at:

```text
GET /ferroehr/rest/.well-known/smart-configuration
```

(relative to the REST root, or to `PLATFORM_BASE_URL` when set). A launching app
reads it to find your authorization server. It looks like:

```json
{
  "issuer": "https://as.example/realms/ferroehr",
  "authorization_endpoint": "https://as.example/authorize",
  "token_endpoint": "https://as.example/token",
  "response_types_supported": ["code"],
  "code_challenge_methods_supported": ["S256"],
  "capabilities": ["context-openehr-ehr"],
  "scopes_supported": [
    "openid", "profile", "offline_access",
    "launch", "launch/patient",
    "patient/composition-*.cruds", "patient/aql-*.rs",
    "user/composition-*.cruds", "user/template-*.cruds", "user/aql-*.cruds",
    "system/composition-*.cruds", "system/aql-*.cruds"
  ],
  "services": {
    "org.openehr.rest": {
      "baseUrl": "https://cdr.example.com/ferroehr/rest/openehr/v1",
      "description": "The openEHR REST API baseUrl"
    }
  }
}
```

- **`services`** is an object keyed by service type. It always names the openEHR
  REST service, and adds the FHIR façade
  (`org.fhir.rest`) when the FHIR routes are enabled. Each `baseUrl` is
  **absolute**, built by prefixing the CDR's own base path with
  `PUBLIC_BASE_URL`, which is why that key is required.
- **`capabilities`** always contains `context-openehr-ehr` (the CDR binds the
  `ehrId` launch context). `openehr-permission-v1` is advertised **only in
  fail-closed mode** (`REQUIRE_SMART_SCOPES=true`): the capability announces
  fine-grained scope enforcement over openEHR resources, and advisory mode does
  not enforce against a scope-less caller, so advertising it there would
  over-claim. `context-openehr-episode` and `launch-base64-json` appear when
  their switches are on, and your own `CAPABILITIES` entries are appended.
- **`scopes_supported`** is the default list above unless you configure your
  own, which is then emitted verbatim. Enabling episode context adds
  `launch/episode` to the default list.
- Every unset optional endpoint is simply **omitted** from the document rather
  than emitted as null.

With SMART disabled the path is not mounted at all (404).

## The scope grammar

SMART resource scopes have the form
**`<compartment>/<resource>.<permissions>`**:

- **Compartment:** `patient` (the launch context's EHR only), `user` (what
  the user may see), or `system` (a backend service, no user).
- **Resource:** one of three families, `composition-<template-id>`,
  `template-<template-id>`, or `aql-<stored-query-name>`.
- **Permissions:** any combination of `c` create, `r` read, `u` update,
  `d` delete, `s` search/execute (order-free, e.g. `.crud`, `.rs`).

Resource ids accept wildcards: `*` matches within one `::`-delimited namespace
segment, `**` matches across namespaces, and a bare `*` or `**` matches every
id. Every other character, `::` and `.` included, is literal. The permission
tail is split at the *last* dot, so template ids and query names keep their
internal dots and versions.

| Scope | Grants |
|---|---|
| `patient/composition-*.crud` | Create, read, update, and delete any composition, but only in the launched patient's EHR. |
| `patient/aql-*.rs` | Read and execute any stored query. |
| `patient/composition-MyHospital::Template.v0.r` | Read compositions of exactly that template, in the launched patient's EHR. |
| `user/composition-MyHospital::*.r` | Read compositions of any template in the `MyHospital` namespace (not sub-namespaces). |
| `user/template-*.cruds` | Full access to template definitions. |
| `system/aql-org.openehr::bloodpressure.v1.rs` | A backend service may read and execute that one stored query. |

Scopes the server does not recognise are retained but inert: never granted,
never fatal; the identity scopes (`openid`, `profile`, `offline_access`, …) and
the `launch`/`launch/patient` context scopes pass through untouched.

## What the gate enforces

All three resource families are enforced. An operation is mapped to its family
and to the CRUDS permission it exercises, and at least one of the caller's
scopes for that family must permit it:

| Family | Operations | Resource id taken from |
|---|---|---|
| `composition-…` | composition and versioned-composition reads and writes | the resolved template id |
| `template-…` | operational-template operations | the `{template_id}` path parameter |
| `aql-…` | stored-query execution and AQL definition management | the `{qualified_query_name}` path parameter |

EHR, `EHR_STATUS`, CONTRIBUTION and DIRECTORY operations have **no SMART
resource type** (the specification defines exactly three) so the SMART gate
does not deny them; they stay governed by the RBAC/ABAC layers and the
per-EHR `EHR_ACCESS` gate.

> [!NOTE]
> **An unresolved resource id matches only a broad wildcard.** A template upload
> or list, and an ad-hoc (non-stored) query, carry no id, so a scope naming a
> specific id cannot match one, which is refused rather than assumed. Grant
> `*`/`**` scopes for those operations deliberately.

## Launch context: binding to one EHR

When an app is launched for a patient, your authorization server puts the
resolved openEHR EHR id in the token: by default in an `ehrId` claim, with
the standard SMART `patient` claim as fallback (both claim names are
configurable). A **composition** operation permitted *only* by a `patient/…`
scope is then bound to that one EHR: a request against any other EHR is
refused, and a token holding only patient-compartment scopes but **no**
launch-context claim is refused outright. `user/` and `system/` scopes carry no
such binding.

Template and AQL operations are not per-EHR resources (templates are unscoped
and queries are cross-EHR) so a `patient/` scope permits them without a
per-request EHR binding. Per-row patient scoping for queries is the ABAC
subject-scope layer's job, not the scope gate's.

## How scopes compose with RBAC and ABAC

The SMART gate is one more layer in the authorization chain, evaluated
**after** authentication, the per-EHR `EHR_ACCESS` gate, RBAC, and ABAC (see
[Security](security.md#authorization)). Every active layer
must allow the request; SMART never overrides a denial from another layer, and
it can only narrow. A scope denial is a **403 Forbidden**.

Granted scopes are also visible to the ABAC layer as a subject attribute, so a
Cedar or external policy can reason about them; the built-in gate is the floor,
not the ceiling.

### Advisory vs required

- **Advisory (default, `REQUIRE_SMART_SCOPES=false`):** the gate enforces
  only when the token actually carries SMART resource scopes for the resource
  family in question. A non-SMART token, a Basic-auth caller (which has no
  scopes), and a server with authentication disabled are all unaffected. Once a
  token *does* carry, say, composition scopes, at least one of them must match
  the operation or the request is refused.
- **Required (`REQUIRE_SMART_SCOPES=true`):** fail-closed. A caller with no
  matching SMART resource scope for a scope-governed operation is denied,
  including a caller with no token at all. Use this where every app is a SMART
  app. This is also the mode in which `openehr-permission-v1` is advertised.

> [!NOTE]
> Episode context is experimental: enabling it advertises the capability and
> accepts `launch/episode`, but the server applies no episode-scoped
> filtering: openEHR has no first-class episode resource yet.
