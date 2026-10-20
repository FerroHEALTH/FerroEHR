# SMART App Launch

FerroEHR is the **resource server** of a SMART on openEHR platform: a clinical
app is launched with an OAuth 2.0 access token from your authorization server
(Keycloak or any standards-compliant identity provider), and the CDR checks
the token, enforces the SMART on openEHR scopes it carries, and binds a
`patient/` grant to the EHR of the launch context. FerroEHR never issues
tokens, registers clients, or serves the OAuth 2.0 endpoints itself; those
remain your authorization server's job. Token checking and scope enforcement
run on `ferrohealth-smart`, the crate every FerroHEALTH product shares.

<!-- toc -->

## Scopes are enforced on every route

Whenever `[auth]` is on, every route carries a SMART on openEHR requirement,
and a credential whose scopes do not meet it is refused with `403` and the
`insufficient_scope` challenge (RFC 6750 §3.1). There is no switch: a route the
table does not know is refused too, so a route can never answer to a token
that carries no grant for it. A bearer token's scopes come from its `scope`
claim; a Basic user's from its `scopes` in `[[auth.basic.users]]`. With
authentication off nothing is checked.

The scopes and the RBAC and ABAC layers are all checked: SMART only narrows,
and never lets through what a role or a policy refuses (see
[Security](security.md#authorization)).

## The scope grammar

SMART on openEHR resource scopes have the form
**`<compartment>/<resource>.<permissions>`**:

- **Compartment:** `patient` (the launch context's EHR only), `user` (what
  the user may see), or `system` (a backend service, no user).
- **Resource:** one of three families, `composition-<template-id>`,
  `template-<template-id>`, or `aql-<stored-query-name>`.
- **Permissions:** any combination of `c` create, `r` read, `u` update,
  `d` delete, `s` search/execute, in any order (`.crud`, `.rs`, `.sr`).

Resource ids accept wildcards: `*` matches within one `::`-delimited namespace
segment, `**` matches across namespaces, and a bare `*` or `**` matches every
id. Every other character, `::` and `.` included, is literal. The permission
tail is split at the *last* dot, so template ids and query names keep their
internal dots and versions.

| Scope | Grants |
|---|---|
| `user/composition-*.crud` | Create, read, update, and delete any composition, and the EHR's other data. |
| `user/composition-MyHospital::*.r` | Read compositions of any template in the `MyHospital` namespace (not sub-namespaces). |
| `user/template-*.cruds` | Full access to template definitions. |
| `user/aql-*.s` | Run any query. |
| `patient/composition-*.r` | Read the compositions of the launch context's EHR only. |
| `system/aql-org.openehr::bloodpressure.v1.s` | A backend service may run that one stored query. |

The FHIR v1 suffixes (`.read`, `.write`) are not part of this grammar. A token
may carry SMART on FHIR scopes beside these; they grant nothing on an openEHR
route.

## What each route requires

| Routes | Requirement |
|---|---|
| COMPOSITION and versioned-composition operations | `composition-<template>` with the operation's permission, the template read from the body or the stored version |
| EHR, `EHR_STATUS`, `DIRECTORY`, `CONTRIBUTION`, the item tags, the EHR extract (`/message`) | `composition-` with the operation's permission over every template: they are the EHR's data, and the draft names no family of their own |
| operational templates, archetypes and artefacts | `template-<template>` with the operation's permission, the template from the `{template_id}` path parameter |
| query execution | `aql-<query>` with `s`, the query from the `{qualified_query_name}` path parameter |
| stored-query definitions | `aql-<query>` with the operation's permission |
| the DEMOGRAPHIC API | a client listed in `[smart] demographic_clients`, or the `[auth] product_scope`: no SMART on openEHR scope covers demographic data |
| the ADMIN API | the `[auth] product_scope` |
| the terminology API, the ITI-81 audit retrieval | an authenticated caller |

> [!NOTE]
> **An unresolved resource id matches only a bare wildcard.** A template upload
> or list, an ad-hoc query and an EHR-level operation name no template or query,
> so only a scope on every resource (`*` or `**`) covers them. Grant those
> scopes deliberately.

A `system/aql-*` grant "would grant access to all registered and ad-hoc AQL
queries system-wide" (master08 §Resource Scopes), so it counts only for a
backend client listed in `[smart] backend_clients`.

## Launch context: binding to one EHR

When an app is launched for a patient, your authorization server puts the
openEHR EHR id of the launch context in the token's `ehrId` claim
(master07 §Context Selection). A `patient/` grant then reaches that one EHR:

- it counts only for an issuer listed in `[smart] patient_issuers`, because an
  `ehrId` names an EHR of the platform that issued it;
- an EHR route whose `{ehr_id}` names another EHR is refused, and so is an
  operation that names no EHR, such as creating an EHR;
- a query runs only within the launch context's EHR, named by its single-EHR
  `ehr_id` parameter or `openehr-ehr-id` header; a query naming no EHR, or
  another one, is refused;
- it reaches EHR data alone, never a template definition.

`user/` and `system/` grants carry no such binding, and when a `user/` grant
covers the same operation the wider grant wins.

## The discovery document

With `[smart] enabled = true`, the server serves the SMART configuration
document, **unauthenticated**, at:

```text
GET /ferroehr/rest/.well-known/smart-configuration
```

(relative to the REST root, or to `[smart] platform_base_url` when set). The
document is built from the first `[auth]` issuer's metadata, read at boot: the
authorization, token, registration, introspection and revocation endpoints,
the grant types and client authentication methods it supports, and its
`jwks_uri`. It therefore needs an issuer whose metadata names a
`token_endpoint`, which the document requires (SMART App Launch 2.2.0,
Conformance, §Metadata). An issuer read from a `jwks_file` has no metadata of
its own: `[smart] issuer_metadata_file` names a JSON file with its RFC 8414 §2
metadata, which must name that issuer and hold every endpoint to `https` (a
loopback host excepted). Without it, `enabled = true` refuses the start.

```bash
export FERROEHR__AUTH__ISSUER=https://as.example/realms/ferroehr
export FERROEHR__AUTH__AUDIENCE=ferroehr-api
export FERROEHR__SMART__ENABLED=true
# The external origin absolute service URLs are built from; required.
export FERROEHR__SMART__PUBLIC_BASE_URL=https://cdr.example.com
```

A document looks like:

```json
{
  "jwks_uri": "https://as.example/realms/ferroehr/certs",
  "authorization_endpoint": "https://as.example/authorize",
  "token_endpoint": "https://as.example/token",
  "token_endpoint_auth_methods_supported": ["client_secret_basic", "private_key_jwt"],
  "grant_types_supported": ["authorization_code", "client_credentials"],
  "scopes_supported": [
    "openid", "profile", "launch",
    "user/composition-*.cruds", "user/template-*.cruds", "user/aql-*.cruds",
    "system/composition-*.cruds", "system/template-*.cruds", "system/aql-*.cruds"
  ],
  "services": {
    "org.openehr.rest": {
      "baseUrl": "https://cdr.example.com/ferroehr/rest/openehr/v1",
      "description": "The openEHR REST API baseUrl"
    }
  },
  "response_types_supported": ["code"],
  "code_challenge_methods_supported": ["S256"],
  "capabilities": [
    "permission-user", "client-confidential-symmetric",
    "client-confidential-asymmetric", "launch-standalone",
    "context-openehr-ehr", "openehr-permission-v1"
  ]
}
```

- **`services`** names the openEHR REST service; `org.fhir.rest` is never
  advertised, because FerroEHR serves no FHIR API for clinical data. Each
  `baseUrl` is **absolute**, built by prefixing the CDR's own base path with
  `public_base_url`, which is why that key is required.
- **`capabilities`** carries what the server enforces: `context-openehr-ehr`
  and `openehr-permission-v1` always, `permission-user`, and
  `permission-patient` only when `patient_issuers` binds an issuer; the client
  types and launch modes the issuer supports; `sso-openid-connect`, with the
  `issuer` member it requires, when the issuer's metadata lists the `openid`
  scope; `context-openehr-episode` and `launch-base64-json` when their switches
  are on; and your `[auth] capabilities`.
- **`grant_types_supported`** names only `authorization_code` and
  `client_credentials`, whatever else the issuer advertises: the implicit and
  password grants are deprecated (master06 §Deprecated Flows).
- **`code_challenge_methods_supported`** always carries `S256` and never
  `plain`.
- **`scopes_supported`** lists `[auth] scopes_supported` when set, otherwise
  the scopes the route table honours above, with the `patient/` ones only when
  an issuer is bound. It never copies the issuer's own list: "the server SHALL
  support all scopes listed here".

With `enabled = false` the path is not mounted at all (404).

## Configuration keys

The keys live in `[auth]` and `[smart]`; see the
[configuration reference](installation/config-auth.md#smart).

| Key | Default | Meaning |
|---|---|---|
| `FERROEHR__SMART__ENABLED` | `false` | Serve the discovery document. |
| `FERROEHR__SMART__PUBLIC_BASE_URL` | unset (**required when enabled**) | The external origin absolute `services.*.baseUrl` values are built from, e.g. `https://cdr.example.com`. |
| `FERROEHR__SMART__PLATFORM_BASE_URL` | unset | Base the discovery document hangs off. Unset = the REST root (`/ferroehr/rest`). A leading path is honoured (`/gateway/v1` → `/gateway/v1/.well-known/smart-configuration`). |
| `FERROEHR__SMART__BACKEND_CLIENTS` | `[]` | The `client_id`s a `system/aql-*` grant counts for. |
| `FERROEHR__SMART__PATIENT_ISSUERS` | `[]` | The issuers whose `patient/` grants count. |
| `FERROEHR__SMART__DEMOGRAPHIC_CLIENTS` | `[]` | The `client_id`s that reach the DEMOGRAPHIC API. |
| `FERROEHR__SMART__LAUNCH_BASE64_JSON` | `false` | Advertise the base64 JSON launch-parameter capability (experimental; consumed by the app, not the CDR). |
| `FERROEHR__SMART__EPISODE__ENABLED` | `false` | Advertise and accept episode launch context (experimental; advisory only, no episode filtering). |
| `FERROEHR__AUTH__PRODUCT_SCOPE` | `__ferroehr.platform` | The scope that admits the ADMIN API and the DEMOGRAPHIC API. |
| `FERROEHR__AUTH__CAPABILITIES` | `[]` | Capabilities the discovery document adds. |
| `FERROEHR__AUTH__SCOPES_SUPPORTED` | `[]` | The scopes the discovery document lists. |

> [!NOTE]
> Episode context is experimental: enabling it advertises the capability and
> accepts `launch/episode`, but the server applies no episode-scoped
> filtering: openEHR has no first-class episode resource yet.
