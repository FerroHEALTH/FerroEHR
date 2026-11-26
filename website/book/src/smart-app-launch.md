# SMART App Launch

FerroEHR is the **resource server** of a SMART on openEHR platform: a clinical
app is launched with an OAuth 2.0 access token from an authorization server,
and the CDR checks the token, enforces the SMART on openEHR scopes it carries,
and holds a `patient/` grant to the EHR of the launch context. FerroEHR never
issues tokens, registers clients, or serves the OAuth 2.0 endpoints itself.
Token checking and scope enforcement run on `ferrohealth-smart`, the crate
every FerroHEALTH product shares.

The authorization server is **FerroSMART**, the FerroHEALTH family's SMART App
Launch 2.x authorization server ([Behind FerroSMART](behind-ferrosmart.md)).
It narrows each request to the client's ceiling and the person's roles, asks
the person's consent, and puts the launch context in the token response and
the access token. An OpenID Connect provider, such as Keycloak or ZITADEL,
signs the person in upstream of it and issues no token FerroEHR reads
([Enterprise identity providers](identity-providers.md#behind-ferrosmart)).
The three ways an app starts:

- **EHR launch:** the viewer's Launch button registers the EHR on screen at
  FerroSMART and opens the app with `iss` and `launch`
  ([Launching an app from the viewer](#launching-an-app-from-the-viewer);
  SMART App Launch 2.2.0, Launch App: EHR Launch).
- **Standalone launch:** an app that asks for `launch/patient` gets
  FerroSMART's patient picker, which looks the EHR up at the openEHR base
  under its `subject_namespace` as the signed-in person
  (`GET /ehr?subject_id=…&subject_namespace=…`) and puts its `ehrId` in
  context (SMART on openEHR, Context Selection; SMART App Launch 2.2.0,
  Launch App: Standalone Launch).
- **Backend Services:** a system client takes the client credentials grant
  with a `private_key_jwt` assertion; FerroEHR reads its token as the system,
  since its `sub` is its `client_id` (SMART App Launch 2.2.0, Backend
  Services; RFC 9068 §2.2).

`docker-compose.ferrosmart.yml` runs the whole platform from the quickstart
([Docker Compose](installation/compose.md#behind-ferrosmart)). FerroSMART's
own setup is its [architecture](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.8.0/docs/architecture.md), sections 13.17
(launching an app from an EHR) and 13.21 (a product behind FerroSMART).

<!-- toc -->

## Scopes are enforced on every route

Whenever `[auth]` is on, every route carries a SMART on openEHR requirement,
and a credential whose scopes do not meet it is refused with `403` and the
`insufficient_scope` challenge (RFC 6750 §3.1). There is no switch: a route the
table does not know is refused too, so a route can never answer to a token
that carries no grant for it. A bearer token's scopes come from its `scope`
claim; a Basic user's from the `hold_scopes` grants of its `PERSON`
([Operational policy](operational-policy.md)), and the first administrator's
`[auth] product_scope` from `[authz.rbac] first_admin`. With
authentication off nothing is checked.

The scopes, the role gate and the access evaluator are all checked: SMART only narrows,
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
| the DEMOGRAPHIC API | a client the CDR grants `access_demographic`, or the `[auth] product_scope`: no SMART on openEHR scope covers demographic data |
| the ADMIN API | the `[auth] product_scope` |
| the terminology API, the ITI-81 audit retrieval | an authenticated caller |

> [!NOTE]
> **An unresolved resource id matches only a bare wildcard.** A template upload
> or list, an ad-hoc query and an EHR-level operation name no template or query,
> so only a scope on every resource (`*` or `**`) covers them. Grant those
> scopes deliberately.

A `system/aql-*` grant "would grant access to all registered and ad-hoc AQL
queries system-wide" (master08 §Resource Scopes), so it counts only for a
backend client: a client the CDR grants `query_system_wide`.

Both client grants are policy the CDR holds, read on every request with no
restart: a capability of a `ROLE` the client's `AGENT` performs, coded
`openehr_access::query_system_wide` or
`openehr_access::access_demographic`, under an `[authz.grants] allowed`
that admits the code. See
[Operational policy](policy-examples.md#a-backend-client-and-a-demographic-client)
for a worked example. A grant counts for a client's own app-only token; a
person signed in through that client holds the person's grants.

## Launch context: binding to one EHR

When an app is launched for a patient, your authorization server puts the
openEHR EHR id of the launch context in the token's `ehrId` claim
(master07 §Context Selection). The SMART layer checks a `patient/` grant for
its resource family and permission, and the
[access evaluator](security.md#a-patients-own-ehr) holds a caller whose
grants are `patient/` grants alone to that one EHR:

- the grant reaches only the EHR its token's `ehrId` names, so a token that
  names none reaches no EHR through it; the person's own records by identity
  alone need `user/` grants (master08 §Resource Scopes);
- any operation on another EHR is refused with `403` `refused-not-subject`,
  recorded as step 0 (subject), whether its path, its single-EHR query or
  import scope, its manifest or its result names that EHR; so is an operation
  that names no EHR, such as creating an EHR;
- a query runs only within the launch context's EHR, named by its single-EHR
  `ehr_id` parameter or `openehr-ehr-id` header; a query naming no EHR, or
  another one, is refused;
- it reaches EHR data alone, never a template definition;
- a write under a grant of `patient/` scopes alone is the person's own: it is
  marked as inserted by the person and may not alter a version a professional
  committed ([EHDS Art. 5](security.md#information-a-person-inserts-ehds-art-5)).

`user/` and `system/` grants carry no such binding, and when a `user/` grant
covers the same operation the wider grant wins.

A `system/` grant belongs to a backend client acting for no person. A token
its issuer's profile marks app-only holds `system/` grants alone: one that
carries a `user/` or `patient/` scope is refused with `403`, because no person
stands behind it. Such a client commits only under a grant of the CDR
([Who the caller is](identity-providers.md#who-the-caller-is)).

## The discovery document

With `[auth.smart] discovery = true`, the server serves the SMART configuration
document, **unauthenticated**, at:

```text
GET /ferroehr/rest/.well-known/smart-configuration
```

(relative to the REST root, or to `[auth.smart] platform_base_url` when set). The
document is built from the metadata of one `[auth]` issuer, read at boot:
`[auth] described_issuer`, else the one issuer whose metadata FerroEHR has. Several such issuers without
`described_issuer`, or none, refuse the start
([the described issuer](installation/config-auth.md#the-described-issuer)). It carries the
authorization, token, registration, introspection and revocation endpoints,
the grant types and client authentication methods it supports, and its
`jwks_uri`. It therefore needs an issuer whose metadata names a
`token_endpoint`, which the document requires (SMART App Launch 2.2.0,
Conformance, §Metadata). An issuer read from a `jwks_file` has no metadata of
its own: `[auth.smart] issuer_metadata_file` names a JSON file with its RFC 8414 §2
metadata, which must name an `[auth]` issuer read from a `jwks_file` and hold
every endpoint to `https` (a loopback host excepted). The issuer it names may
stand anywhere in the list, beside a Basic users' issuer listed first. Without
it, `enabled = true` refuses the start.

```bash
export FERROEHR__AUTH__ISSUER__IDP__ISSUER=https://as.example/realms/ferroehr
export FERROEHR__AUTH__ISSUER__IDP__AUDIENCE=ferroehr-api
export FERROEHR__AUTH__SMART__DISCOVERY=true
# The URL clients reach the server at, which the absolute service URLs hang
# off; required.
export FERROEHR__SERVER__PUBLIC_BASE_URL=https://cdr.example.com
```

The authorization server's registry is the one list of clients, so the app
signs persons in once it is registered there.

A document looks like:

```json
{
  "jwks_uri": "https://as.example/realms/ferroehr/certs",
  "authorization_endpoint": "https://as.example/authorize",
  "token_endpoint": "https://as.example/token",
  "token_endpoint_auth_methods_supported": ["client_secret_basic", "private_key_jwt"],
  "token_endpoint_auth_signing_alg_values_supported": ["RS384", "ES384"],
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

- **`token_endpoint_auth_signing_alg_values_supported`** is copied from the
  authorization server's metadata whenever `private_key_jwt` is listed, as
  SMART App Launch 2.2.0 (Client Authentication: Asymmetric, Advertising
  server support) requires of a server that supports it.
- **`services`** names the openEHR REST service; `org.fhir.rest` is never
  advertised, because FerroEHR serves no FHIR API for clinical data. Each
  `baseUrl` is **absolute**, built from
  [`server.public_base_url`](installation/config-server.md#public_base_url-the-one-public-url),
  `server.base_path` and `api.base_path`, which is why that key is required.
- **`capabilities`** carries what the server enforces: `context-openehr-ehr`
  and `openehr-permission-v1` always, `permission-user`, and
  `permission-patient` whenever an issuer is trusted; the client
  types and launch modes the issuer supports; `sso-openid-connect`, with the
  `issuer` member it requires, when the issuer's metadata lists the `openid`
  scope; `context-openehr-episode` when `episode_launch` is on;
  and your `[auth] capabilities`, where `launch-base64-json` goes if your apps
  read it.
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

## Launching an app from the viewer

SMART App Launch 2.2.0 has the EHR start an EHR launch by opening the app's
launch URL "in a new browser instance (or iframe)" with `iss`, the base, and
`launch`, "an opaque handle to the EHR context" (Launch App: EHR Launch,
Request). The specification gives the EHR no way to hand the authorization
server that context, so FerroSMART takes it through its own EHR launch API,
the `launch_registration_endpoint` its discovery document
`{issuer}/.well-known/smart-configuration` names
(FerroSMART's architecture, [§13.17, Launching an app from an EHR](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.8.0/docs/architecture.md)).
The viewer is that EHR: with `[launch]` set, an EHR's **More** menu carries a
**Launch** item per configured app, and a click does this:

1. The viewer reads the EHR's status with your own session, so you launch
   only for an EHR you may read.
2. Its server posts the context to FerroSMART, authenticated by its own
   `private_key_jwt` client assertion (`aud` FerroSMART's issuer, `typ`
   `client-authentication+jwt`): the app's `client_id`, `resource` (the
   openEHR base), `ehrId` of the EHR on screen, `patient` when the EHR's
   subject namespace is one of `launch.patient_namespaces`, and
   `need_patient_banner=false`, since the viewer already shows whose record
   it is. No `encounter` is sent: an openEHR CDR holds none to send, and the
   context is optional in SMART.
3. FerroSMART answers `201` with a fresh `launch` value, single use, valid
   for five minutes, for that app and that base alone. The viewer answers
   the click's new window with `303 See Other` to
   `{launch_url}?iss={resource}&launch={value}`, `Cache-Control: no-store`.
   The app then reads the base's discovery document, which names FerroSMART,
   and the person logs in and consents there; the token response carries
   the context the viewer registered.

The key that authenticates the viewer never reaches the browser, and the
viewer logs no launch value. A launch FerroSMART refuses draws the refusal on
the EHR's page in that window.

Register the viewer and each app at FerroSMART with `ferrosmart client apply
<file>...`, one RFC 7591 client document per file (FerroSMART's architecture,
§13.27), and list the apps in the viewer. The viewer is the EHR client, its
public key by value:

```json
{
  "client_id": "ferroehr-viewer",
  "client_name": "FerroEHR viewer",
  "token_endpoint_auth_method": "private_key_jwt",
  "jwks": {"keys": [{"kty": "RSA", "kid": "viewer-1", "alg": "RS384", "n": "…", "e": "AQAB"}]},
  "scope": "system/launch.c",
  "grant_types": ["client_credentials"],
  "resources": ["https://ehr.example.org/ferroehr/rest/openehr/v1"]
}
```

Each app it launches is a client of the authorization code grant:

```json
{
  "client_id": "growth-chart",
  "client_name": "Growth chart",
  "token_endpoint_auth_method": "none",
  "redirect_uris": ["https://apps.example.org/growth/callback"],
  "scope": "openid fhirUser launch patient/composition-*.r",
  "grant_types": ["authorization_code"],
  "resources": ["https://ehr.example.org/ferroehr/rest/openehr/v1"]
}
```

```toml
# ferroehr-viewer.toml
[launch]
issuer = "https://smart.example.org"
client_id = "ferroehr-viewer"
private_key_file = "/run/secrets/ferroehr-viewer-launch.pem"
kid = "viewer-1"
resource = "https://ehr.example.org/ferroehr/rest/openehr/v1"

[[launch.app]]
name = "Growth chart"
client_id = "growth-chart"
launch_url = "https://apps.example.org/growth/launch"
```

The document's `jwks` holds the key's public half under the same `kid`. For
an RSA key:

```sh
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 -out ferroehr-viewer-launch.pem
n=$(openssl rsa -in ferroehr-viewer-launch.pem -noout -modulus | cut -d= -f2 | xxd -r -p \
  | openssl base64 -A | tr '+/' '-_' | tr -d '=')
jq -n --arg n "$n" '{keys: [{kty: "RSA", kid: "viewer-1", alg: "RS384", n: $n, e: "AQAB"}]}'
```

The viewer's configuration keys are in [the viewer's
configuration](viewer/index.md#configuration). The launch API, the menu and
the context the viewer sends are our own design; SMART fixes only the launch
URL.

### Launch Tasks

SMART App Launch 2.2.0 lets a system ask staff to launch an app with a FHIR
`Task` of the `task-ehr-launch` or `task-standalone-launch` profile (Task
profile for requesting a SMART launch): the app's URL in the
`smartonfhir-application` input, an optional `smartonfhir-appcontext`, the
patient in `for` and an optional `encounter`. With `launch.fhir_base` set to
the FHIR facade's base, the page of an EHR whose subject namespace is one of
`launch.patient_namespaces` carries a **Launch tasks** section:

1. The viewer asks FerroSMART for an access token for the facade on your
   session's grant: a refresh grant naming `resource={fhir_base}` (RFC 8707
   §2.2), the token kept with the session until it is due.
2. It searches `Task?patient={subject}&code=` for the two launch codes of
   the SMART codes system, follows the bundle's `next` links under the base,
   and checks each Task against its profile. A Task off its profile is
   logged and not listed.
3. A Task whose app URL is a configured app's `launch_url` (compared after
   RFC 3986 §6.2.2 normalization, as FerroSMART compares it) gets a
   **Launch** button into a new window; any other is listed as **App not
   configured**.
4. The button posts only the Task's id. The viewer reads the Task again at
   the facade, refuses it unless its `for` is the EHR's subject and its URL a
   configured app's, and then launches. An EHR launch Task registers
   `app_launch_uri`, `resource={fhir_base}`, `patient`, `encounter` and the
   Task's `appContext` unchanged at FerroSMART and opens the configured
   `launch_url` with `iss={fhir_base}` and the fresh `launch`; a standalone
   launch Task opens the configured `launch_url` as it is. The browser never
   goes to a URL the Task carries.

Launching a Task changes no Task status. The section needs these, beside the
EHR launch setup above:

- **An OpenID Connect sign-in at FerroSMART** over a `memory` session store
  with `oidc.offline_access = true`, so the session holds the refresh token
  the facade token comes from. A Basic sign-in or a `sealed` store holds
  none, and the section asks you to sign in with FerroSMART.
- **The FHIR scopes in the sign-in**: `oidc.scopes` lists the scopes the
  facade token needs, such as `user/Task.rs`, and the viewer's sign-in client
  at FerroSMART carries them in its `scopes` ceiling. FerroSMART narrows the
  facade token to the grant's FHIR scopes.
- **The pairing at FerroSMART**: the openEHR `[[ferrosmart.resource]]` the
  viewer signs in for names the facade as its `fhir_base`, so FerroSMART
  accepts that `resource` on refresh.
- **Each app's launch URL at FerroSMART**: the app's client registers the
  URL its Tasks carry as `launch_uri`, the same as the viewer's `launch_url`.

```toml
# ferroehr-viewer.toml
[auth]
oidc_enabled = true

[oidc]
issuer = "https://smart.example.org"
client_id = "ferroehr-viewer-sign-in"
scopes = ["openid", "profile", "user/Task.rs"]
offline_access = true

[session]
store = "memory"

[launch]
patient_namespaces = ["demo.hospital.mrn"]
fhir_base = "https://ehr.example.org/fhir"
```

Which Tasks the viewer lists and launches is our own design.

## Configuration keys

The keys live in `[auth]` and `[auth.smart]`; see the
[configuration reference](installation/config-auth.md#authsmart).

| Key | Default | Meaning |
|---|---|---|
| `FERROEHR__AUTH__SMART__DISCOVERY` | `false` | Serve the discovery document. |
| `FERROEHR__SERVER__PUBLIC_BASE_URL` | unset (**required when enabled**) | The URL clients reach the server at, e.g. `https://cdr.example.com`: the absolute `services.*.baseUrl` values are it, `server.base_path` and `api.base_path`. |
| `FERROEHR__AUTH__SMART__PLATFORM_BASE_URL` | unset | Base the discovery document hangs off. Unset = the REST root (`/ferroehr/rest`). A leading path is honoured (`/gateway/v1` → `/gateway/v1/.well-known/smart-configuration`). |
| `FERROEHR__AUTH__SMART__EPISODE_LAUNCH` | `false` | Advertise episode launch context and narrow each request of a launch to the token's `episodeId` (experimental). |
| `FERROEHR__AUTH__PRODUCT_SCOPE` | `__ferroehr.platform` | The scope that admits the ADMIN API and, beside a client the CDR grants `access_demographic`, the DEMOGRAPHIC API. |
| `FERROEHR__AUTH__CAPABILITIES` | `[]` | Capabilities the discovery document adds. |
| `FERROEHR__AUTH__SCOPES_SUPPORTED` | `[]` | The scopes the discovery document lists. |

> [!NOTE]
> Episode context is experimental, and SMART on openEHR leaves the meaning
> of `episodeId` to the platform. FerroEHR reads it as `issuer|id`, the
> `DV_IDENTIFIER` the care delivery environment gives the episode of care,
> and narrows every request of the launch to it: the app reads and queries
> that episode's compositions, the compositions placed in no episode and
> the data your access policy's `episode_shared` names. A composition of
> another episode answers `403` with `refused-outside-episode` on a named
> read and is left out of every query, count and page. A launch in an
> episode that no live care relationship of the caller names answers `400`
> on every request, as a query's `caseload_episodes` does.
