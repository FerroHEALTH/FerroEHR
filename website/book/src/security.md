# Security

A clinical data repository holds PHI, so its access controls and audit trail
are part of the product. This chapter covers the four
security surfaces you configure when you deploy FerroEHR: **authentication**
(who is calling), **authorization** (what they may do), the
**pseudonymisation boundary** (which role reads which domain), and the **ATNA
audit trail** (recording what happened). Each is independently configurable,
and each is described here in terms of the environment variables you actually
set.

This chapter tells you **how to configure each control**. Its companions tell you
the rest: the [Threat model](threat-model.md) states **what remains true after
each control has done its job**: the trust boundaries, the residual risk at
each, and what this software explicitly does not defend against; and
[Verifying releases](verifying-releases.md) covers the artifacts themselves:
how to establish that the binary, image, or chart you downloaded came from this
project's own build. Read the threat model before you decide a control is
sufficient for your deployment.

<!-- toc -->

Configuration follows the same pattern throughout: the server reads defaults,
then the single `ferroehr.toml` file, then environment variables, with `__`
separating nested keys. The security configuration groups live in
distinct sections of `ferroehr.toml`: `[auth]` (authentication), `[authz]`
(authorization), `[privacy]` (the pseudonymisation boundary), and `[audit]`
(the ATNA audit trail). Any key can be overridden with the matching
`FERROEHR_*` environment variable shown below.

## Authentication

Authentication is on by default (`FERROEHR__AUTH__ENABLED=true`). Setting
it to `false` lets all requests through unauthenticated, a development-only
mode.

There is no single "mode" switch. The server offers two mechanisms and enables
each by the presence of its configuration. Both are checked by
`ferrohealth-smart`, the resource-server crate every FerroHEALTH product
shares; FerroEHR checks tokens and never issues them.

- **HTTP Basic** is active when `[[auth.basic.users]]` lists a user. Each user
  has a user-id and an Argon2id password hash (a PHC string beginning
  `$argon2id$`) or a `password_hash_file` pointing at one, and nothing else:
  the SMART on openEHR scopes and the RBAC roles it holds are the
  `hold_scopes` and `hold_roles` grants of its `PERSON` in the CDR, and
  `[authz.rbac] first_admin` names the one Basic user that holds
  `admin_role` and the product scope from the configuration. Because it is
  a list of users, the Basic store is normally supplied through the TOML
  configuration file rather than environment variables.
- **OAuth 2.0 bearer tokens** are active when `[auth]` names an `issuer`. The
  server reads the issuer's metadata and key set at boot and validates every
  token's signature (asymmetric algorithms only), issuer, audience, lifetime
  and type under RFC 9068.

Every person a request comes from is bound to a demographic `PERSON`, and the
server writes each version's committer from that party:
[Who is the committer](security/committer.md).

Both mechanisms are validated at startup, and a configuration the server cannot
honour **refuses to boot** rather than degrading at the first request:

| Configuration | Boot outcome |
|---|---|
| `auth.enabled = true` with no mechanism | error: a `401` challenge must name a scheme the server implements (RFC 9110 §11.6.1) |
| an `issuer` with no `audience` | error: a server with no declared audience cannot reject another server's token (RFC 9068 §4, RFC 8725 §3.9) |
| an `issuer` not `https` (plain `http` is accepted on a loopback host only), carrying a query or fragment, or a user name or password | error: RFC 8414 §2, §6.2; OpenID Connect Core 1.0 §1.2 |
| an issuer whose metadata or key set cannot be read | error: the start is refused rather than the first request |
| `clock_skew` above `"5min"` | error: leeway may be "no more than a few minutes" (RFC 7519 §4.1.4) |
| `algorithms` naming an `HS*` algorithm, or empty | error: the server accepts asymmetric algorithms only (RFC 8725 §3.1, §3.2) |
| a `jwks_file` holding a symmetric key or a private-key member | error: a verifier holds public keys only |
| a `product_scope` that is neither a full URI nor starts with `__` | error: SMART App Launch 2.2.0, Scopes and Launch Context, §Extensibility |
| a `password_hash` below `m=19456,t=2,p=1` argon2id | error: the OWASP Argon2id floor |
| `[authz.assurance] minimum` with an empty `levels` map, or with no mapped value at or above it | error: every patient-data request would be refused |
| an `[authz.assurance] levels` value that is blank or carries whitespace, a quote or a backslash | error: the step-up challenge names the values in one quoted `acr_values` parameter (RFC 9470 §3) |
| two issuers sharing an audience, or no `[authz.party_binding] provider` | error: every person is bound to a party, under the provider the deployment acts for |
| a Basic user with no `[[authz.party_binding.basic_user]]` mapping | error: a Basic user is a person, bound by its key |
| an `[authz.rbac] role_issuers` entry that names a `federation` issuer | error: only the deployment's own people hold an administrator role |

Successfully verified Basic credentials are cached for
`FERROEHR__AUTH__BASIC__CACHE_TTL` (default `5min`; `0s` turns the cache off
and checks every request) so a busy client pays the deliberately expensive Argon2 verification
once per TTL instead of on every request. The cache names each entry by a keyed
hash of the credential, never the password, and an entry exists only after a
successful verification. The user list and its password hashes are read at
boot, so removing a user or changing a password takes effect at the restart
that loads the new list, and that restart also empties the cache: the TTL never
keeps a removed user working.

The bearer settings:

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__AUTH__ISSUER__<NAME>__ISSUER` | unset (required for bearer tokens) | the expected `iss` of the `[auth.issuer.<name>]` table `<name>` names, and where the metadata is read, in the background from the start: RFC 8414 §3 first, OpenID Connect Discovery 1.0 §4 after a `404` |
| `FERROEHR__AUTH__ISSUER__<NAME>__KIND` | unset | `federation` for an issuer of another party's professionals or organisation; unset, a SMART App Launch 2.2 authorization server |
| `FERROEHR__AUTH__ISSUER__<NAME>__AUDIENCE` | the openEHR base | accepted `aud` values; unset, the openEHR REST API's `baseUrl` under `server.public_base_url` |
| `FERROEHR__AUTH__ISSUER__<NAME>__ALGORITHMS` | `RS256,RS384,PS256,PS384,ES256,ES384` | accepted signing algorithms, asymmetric only |
| `FERROEHR__AUTH__ISSUER__<NAME>__CLOCK_SKEW` | `60s` | leeway on `exp`, `nbf` and `iat`; capped at `5min` |
| `FERROEHR__AUTH__ISSUER__<NAME>__JWKS_FILE` | unset | a static JWK Set of public keys, read instead of the issuer's metadata |
| `FERROEHR__AUTH__ISSUER__<NAME>__JWKS_MAX_AGE` | `10min` | a key set older than this is read again |
| `FERROEHR__AUTH__ISSUER__<NAME>__JWKS_REFRESH_INTERVAL` | `1min` | an unknown `kid` reads the key set again at most once per interval |
| `FERROEHR__AUTH__ISSUER__<NAME>__TIMEOUT` | `10s` | one call to the authorization server |
| `FERROEHR__AUTH__REALM` | `ferroehr` | the realm every challenge names |
| `FERROEHR__AUTH__PRODUCT_SCOPE` | `__ferroehr.platform` | the scope that admits the ADMIN and DEMOGRAPHIC APIs |

There is no separate key-set URL to set: the server reads the `jwks_uri` from
the issuer's metadata unless you supply a `JWKS_FILE`. A rotated key is picked
up on the first token that names it, bounded by the refresh interval, and a
withdrawn key stops verifying within the max age.

Every route also enforces its SMART on openEHR requirement: a valid token or
Basic user whose scopes do not grant the operation is refused with `403`. See
[SMART App Launch](smart-app-launch.md).

> [!TIP]
> **Keycloak example.** Point the issuer at your realm and let discovery do the
> rest:
>
> ```bash
> export FERROEHR__AUTH__ISSUER__KEYCLOAK__ISSUER=https://keycloak.example/realms/ferroehr
> export FERROEHR__AUTH__ISSUER__KEYCLOAK__AUDIENCE=ferroehr-api
> export FERROEHR__AUTH__ISSUER__KEYCLOAK__PROFILE__TOKEN_TYPES=JWT
> ```
>
> Keycloak's access tokens carry `typ: JWT`, outside the RFC 9068 default,
> and need a mapper per client that puts `client_id` in every token. The
> client's default client scopes put the SMART on openEHR scopes the routes
> need into each token's `scope` claim.
>
> Entra ID, AD FS, Okta and plain-LDAP directories run upstream of FerroSMART
> ([Enterprise identity providers](identity-providers.md)). User accounts,
> roles, and lifecycle are administered in the IdP; the CDR has no user API.

An unauthenticated request to a protected route is refused with `401`; an
authenticated request that lacks the required role or scope is refused with
`403`. Two outcomes are neither: a **malformed** `Authorization` header, or two
of them, is a `400` (the server never read a credential), and an **unreadable
key set** is a `503` (no token can be validated, so the server cannot decide; it
is not a statement about the caller's credential). The per-status table for
client authors is in [Using the API](using-the-api/index.md#which-status-a-credential-problem-gets).

The `401` body deliberately says nothing about *why*, and neither does the
challenge: every token that does not hold gets the one `invalid_token` answer
(RFC 9068 §4), with no `error_description`. Rendering the rejection would tell
an unauthenticated caller whether a token was expired or forged, which is
exactly the distinction an attacker probes for; the reason stays in the debug
log, where the operator can read it and the caller cannot. A `403` names no
scope either, in its challenge or its body. A request that carried no
credential at all, or one for a scheme the server does not admit, gets no error
code: it has not made a mistake yet (RFC 6750 §3.1).

### Assurance level and the natural person on patient data

Two optional blocks under `[authz]` judge *how* a professional
authenticated and *who* stands behind a token. Both are off by default; with
neither configured nothing below changes the wire.

**`[authz.assurance]`** reads the authentication assurance level from a
token claim (`claim`, default `acr`) and maps each value your identity provider
emits to an eIDAS level (`levels`: `low`, `substantial` or `high`). With
`minimum` set, a patient-data request whose token maps below it, carries a value
the map does not name, or carries no such claim is refused with `401` and the
OAuth 2.0 step-up challenge of RFC 9470 §3:

```text
WWW-Authenticate: Bearer realm="ferroehr", error="insufficient_user_authentication",
  error_description="patient data requires authentication at assurance level substantial or higher",
  acr_values="http://eidas.europa.eu/LoA/high http://eidas.europa.eu/LoA/substantial"
```

`acr_values` lists every mapped value at or above the minimum, so a client can
send the user back to the identity provider with that request. EHDS Annex II 3.1
asks an EHR system used by health professionals for reliable identification and
authentication of them; Implementing Regulation (EU) 2026/2099 Art. 6(3) sets
`substantial` for cross-border exchange, and `high` from 26 March 2032.

**The natural person** behind a token is the issuer's to say.
`ferrohealth-smart` reads every token's caller from its standard claims: a
user by the type of their `fhirUser`, a client credentials token, whose
`sub` is its `client_id`, as a system, and an RFC 8693 `act` as the actor
acting for the user (RFC 9068 §2.2, RFC 8693 §4.1). The authorization
server's registry is the one list of clients, so a token whose `aud` names
FerroEHR admits its client. The access record names the person by their
`sub`, and a delegated token's actor beside them; see [Audit
trail](ehds-logging.md#the-ehds-logging-elements-mapped).

What counts as **patient data**: every API route except the definition family
(`/definition/...`: templates, archetypes, stored-query texts), terminology,
the aggregate counts under `/admin/report`, and the admin routes that manage
configuration, the retention policy, templates and
stored queries. The list of exceptions is closed, so a route
added later is judged patient data until it is listed. The health family,
`/rest/status`, discovery and `/management` are outside the API and never
judged.

**Basic credentials** carry no assurance level and no issuer-asserted person.
With `minimum` set a Basic request for patient data gets the same `401` step-up
challenge; definition routes stay reachable with Basic. A Basic user is a
person, bound by its key (`[[authz.party_binding.basic_user]]`).

The assurance level is judged before RBAC, so a caller below the level learns
it must step up rather than that it lacks a role. A client token acting for a
professional is held to the same level as any other token: the `acr` it
carries is the one judged, so a client-credentials token needs your
authorization server to put the professional's assurance level on it.

### Two limits worth planning around

**Plain-HTTP issuers are accepted on a loopback host only.** The server reads
the issuer's metadata and keys over TLS, because over plain HTTP an attacker on
the network could serve their own signing keys (RFC 8414 §6.2); there is no
switch that admits a plain-HTTP issuer elsewhere.

**Revocation latency equals the access-token lifetime.** Tokens are validated
offline against the issuer's published keys; the CDR calls no introspection
endpoint (RFC 7662 defines that mechanism but does not require a resource server
to use it), so a token revoked at the identity provider stays acceptable here
until its `exp` passes. That is deliberate: introspecting per request would put
the identity provider's availability in the request path, and caching the results
only shortens the lag. **The control is therefore the token lifetime, which your
authorization server owns**: keep access-token lifetimes short (minutes, not
hours) if prompt revocation matters, and use refresh tokens for session length.
`clock_skew_leeway_seconds` adds at most its own value on top of `exp`.

## Authorization

Who may do what is decided by the per-EHR access evaluator, the coarse role
gate and, when enabled, the SMART scopes. How the parts fit is on [How FerroEHR
decides access](explanation/access-model.md), and each layer in detail is on
[Access control in detail](access-control.md).

## Response security headers

Three surfaces, three honest answers: the set differs because what they serve
differs.

**The REST API** carries, on every response including the transport-layer ones
(`413` from the body limit, `408` from the timeout, `503` from the overload
bound, `500` from the panic handler), the health routes and the management
listener on its own port. The family server stack sets them outermost
(`ferrohealth-server`), each only where a response does not set its own:

| Header | Value | Why |
|---|---|---|
| `Cache-Control` | `no-store` | responses carry patient data; the OWASP cheat sheet names `no-store` for exactly that. It does not affect openEHR's `ETag`/`If-Match` concurrency control, which is a precondition mechanism, not a caching one |
| `X-Content-Type-Options` | `nosniff` | stops a proxy or browser re-guessing `application/json` as something executable |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | request paths carry `ehr_id` and version identifiers; this keeps them out of cross-origin `Referer` headers |
| `Cross-Origin-Resource-Policy` | `same-site` | refuses cross-site embedding of API responses |
| `X-Frame-Options` | `DENY` | for the HTML this origin can serve (Swagger UI) |
| `Content-Security-Policy` | `default-src 'none'; frame-ancestors 'none'` | the defensive minimum, applied wherever a response does not set its own; Swagger UI needs a real policy and brings one. The cheat sheet is explicit that CSP "might be meaningless in the response of a REST API that returns content that is not going to be rendered", so this is not a policy pretending to govern scripts: it says nothing loads and nothing frames |

`X-XSS-Protection` and `X-Powered-By` are absent because the cheat sheet says to
remove them, and no `Server` header is sent at all.

**`Strict-Transport-Security: max-age=31536000` is sent only when FerroEHR
serves TLS itself** (`[server.tls]`). [RFC 6797 §7.2](https://www.rfc-editor.org/rfc/rfc6797#section-7.2)
requires a browser to *ignore* it over plain HTTP, which is how this server is
commonly reached behind a terminating proxy, and forbids a server to send it
there. Behind such a proxy, set it at the proxy or ingress that owns TLS.
`includeSubDomains` is left out, because FerroEHR does not decide for the hosts
beside it.

**The viewer** carries the FerroHEALTH console set instead, because it serves
HTML: a CSP that admits scripts and styles from its own origin alone
(`script-src 'self'; style-src 'self'`, no nonce, no inline script or
style), `Referrer-Policy: no-referrer` and `Cross-Origin-Opener-Policy:
same-origin` (the [viewer chapter](viewer/index.md#browser-security-headers)
lists them), with `Cross-Origin-Resource-Policy: same-site` from the API set
beside them. Its `Cache-Control` has one scoped exception to `no-store`:
FerroUI's stylesheet, theme and scripts under `/ferrohealth-ui/<version>/`
are served `no-cache`, so the browser keeps them and revalidates them, and
they hold nothing clinical (the viewer reaches the CDR from its own server,
never from the browser). Every document carries `no-store`, because
documents carry patient data. An asset response that is not a served body
(a `404`, a redirect) is never kept either.

**The published documentation site cannot carry response headers at all:** it is
static files on GitHub Pages, so its policy travels as a `<meta http-equiv>`
element, which is weaker by specification (`frame-ancestors`, `report-uri` and
`sandbox` are ignored in meta form). Its `img-src` admits one outside path,
`https://raw.githubusercontent.com/FerroHEALTH/FerroEHR/`, for the conformance
badges the Conformance workflow pushes. Anyone re-hosting these docs behind a
real web server should send the headers properly instead.

## Request limits and rate limiting

Four different protections, four different statuses, and an operator should be
able to tell them apart from the status alone.

**Connection bounds:** `[server] header_read_timeout`. The limit that applies
*before* a request exists, because a client that opens a socket and trickles
headers, or sends nothing at all, reaches none of the others: a client that has
not sent its request head in time, counted from accept, is disconnected, on the
plain and the TLS listener alike. On a kept-alive connection the bound counts
from the first byte of each later request head, and `[server] idle_timeout`
(two minutes by default) closes a connection that waits that long for its next request.
Keep it longer than the idle timeout of your clients (90 s for a Go client) and
of any load balancer in front, so they close an idle connection first.

**Body size:** `[server] body_limit` and `[api] body_limit`. Two
tiers: the routes that accept bulk by design (template upload and
`/message/import`) take the listener's limit, and the clinical surface the
narrower `[api]` one when it is set. Over-limit is `413`. The listener's
default is the family's 1 MiB; a deployment that uploads operational
templates raises it (the chart and the Compose files set 64 MiB), and one whose
compositions embed large `DV_MULTIMEDIA` data raises the clinical tier
deliberately.

**Request rate:** `[server.caller_rate]`, unset by default and set by the chart
and the Compose quickstart. It sits inside authentication, keyed on the
authenticated subject, because a hospital behind one NAT is a single address and
address-keying a clinical API would throttle a whole site for one busy client.
Refusal is `429` with `Retry-After`.

**Concurrency:** `[server] max_concurrent_requests`, the admission bound.
Refusal is `503` with `Retry-After`; the health routes are never refused.

So: `503` means the server is full right now, `429` means you are asking too
fast, `413` means your payload is too big. Full key tables are on the
[configuration page](installation/configuration.md).

**If you benchmark this server, turn the rate limiter off first**, or you will
measure the limiter. Our own measurement lanes compose an overlay that disables
it, and both instruments refuse to write a record if the server answered any
`429`: a performance number that is really a configuration key is worse than no
number.

## Operational surfaces: what is reachable, and by whom

Everything reachable without a credential is listed here, with the reason it
is public. Every other route answers `401` to a request without one, and a
test drives every mounted route to keep this list closed. A public answer says
only what a client or probe needs; the detail behind it is on the
authenticated `/management` surface.

| Surface | Default | Why it is public, or how it is gated |
|---|---|---|
| `/health`, `/health/liveness` | **always on, unauthenticated** | Orchestrator probes carry no credential. Liveness answers `{"state":"up"}` with no I/O behind it; `/health` answers the readiness status with the state alone. |
| `/health/readiness` | **always on, unauthenticated** | An orchestrator probe: `200` while ready, `503` while the process boots or drains or an indicator is `down`. The body is the state, the phase and each indicator's name and state, nothing more. The detail is `/management/health`. |
| `{rest root}/status` | **always on, unauthenticated** | A documented client contract: status, server version, ITS-REST version, timestamp and the deployment profile. The licence, the deployment gaps and the support period are `/management/status`. |
| `{rest root}/.well-known/smart-configuration` | mounted only with SMART on | SMART App Launch service discovery, read before the client holds any token (ITS-REST SMART App Launch, *Service Discovery*). |
| `OPTIONS {base path}` | **always on, unauthenticated** | The ITS-REST System API's options manifest, which its released OpenAPI document declares without security. |
| `{rest root}/swagger-ui`, `{rest root}/api-docs/*` | **`private`** (`api.openapi_ui`) | Public only if you set `public`; `admin_only` and `off` are the other levels. |
| `/management/*` | **not mounted at all** (`management.enabled = false`) | With the master switch off, every route is `404`. |
| `/management/{info,metrics,prometheus,env,loggers,flamegraph,health,status}` | each **`off`** individually | Even with the master switch on, each endpoint stays unmounted until you name a level for it. There is no global fallback: silence means `off`, so a surface this privileged opens one endpoint at a time, by name. |
| `management.listen` | unset (shares the API listener) | Set it to serve ops introspection from **its own listener** on its own port. It binds all interfaces and always stays plain HTTP even with `[server.tls]` on, so treat it as an internal surface and keep it off any publicly routed port; the interface half of the separation is your network's, not this key's. Under `deployment.profile = "production"` setting it opens the `plaintext_listener` gap, so the server refuses to start unless the gap is accepted by name. |

`env` and `flamegraph` deserve particular caution: `env` renders the
effective configuration (redacted, but still configuration), and `flamegraph`
starts a CPU profiler on request, which is both a disclosure and a
denial-of-service lever. Both are `off` until you name a level, and the profiler
additionally caps the window and sampling frequency, refusing an out-of-range
request rather than clamping it silently.

## Secrets: mount files, never bake values

Every secret this server reads has a `*_file` sibling, and the loader reads and
trims the file at startup. Exactly one of a pair may be set; both is a boot
error naming the pair:

| Secret | File sibling |
|---|---|
| the database DSN, per storage domain, and the credential that prepares the schema | `database.url_file`, `database.<domain>.url_file`, `db.migrate_url_file` |
| a Basic user's Argon2 hash | `auth.basic.users[].password_hash_file` |
| the PGP signing-key passphrase | `record.signing.key_passphrase_file` |
| a terminology OAuth2 client secret | `[terminology].oauth.client_secret_file` |
| the object-store secret key | `multimedia.secret_access_key_file` |

(The TLS `certificate_file` / `client_ca_file` settings and a static key
set's `auth.jwks_file` are paths by nature and have no inline form at all,
which is the same property arrived at from the other direction.)

That is deliberately the shape Docker Secrets and Kubernetes Secrets deliver: a
file mounted into the container. So the recommended posture needs no extra
machinery.

```yaml
services:
  ferroehr:
    environment:
      # Point the key at the mount path; the VALUE never appears anywhere.
      FERROEHR__RECORD__SIGNING__KEY_PASSPHRASE_FILE: /run/secrets/signing_passphrase
    secrets:
      - signing_passphrase

secrets:
  signing_passphrase:
    file: ./secrets/signing_passphrase.txt   # or `external: true` in swarm
```

Why files rather than environment variables: an environment variable is readable
from `/proc/<pid>/environ` by anything in the container's namespace, appears in
`docker inspect` output, is inherited by every child process, and is routinely
captured whole by crash reporters and process listings. A mounted file is none of
those, and it can be rotated without recreating the container.

Two properties worth knowing because they are not obvious:

- **Redaction is a property of the type, not a list.** Secret-bearing fields are a
  `Secret`/`SecretUrl` newtype whose `Debug` and serialization render `***`, so a
  new secret key cannot be forgotten by a per-endpoint redactor: `/management/env`
  and `ferroehr config check` show `***` because the type does, not because
  something remembered to hide it. A Basic password hash in `[auth]` is a
  `secrecy` secret of the shared crate's section, rendered `***` the same way.
- **A Kubernetes `Secret` is base64, not encryption.** The cheat sheet is blunt
  about this: Secrets are stored unencrypted in etcd by default. Enable
  encryption at rest or use an external manager; the chart mounts whatever you
  give it and cannot make an unencrypted store safe.

> [!WARNING]
> The downloadable quickstart carries an inline Argon2 hash for its throwaway
> `ferroehr` user, because a self-contained demo file cannot reference a secret you
> do not have. That is the one place a credential appears in our own artifacts, and
> it is a development credential by construction; replace it before any real use.

## Verifying what you pulled

Release binaries, container images and the Helm chart carry signed build
provenance, SBOMs and checksums, so you can establish that the bytes you are
about to run came from this repository's own pipeline and see what went into
them. The commands, the SBOM formats, the SLSA levels claimed per artifact, and
the published VEX justifications for scanner findings are all in
**[Verifying releases](verifying-releases.md)**. Enforcing image provenance at
admission time inside a Kubernetes cluster is covered separately, in
[Images: build, provenance, scanning](installation/hardening-supply-chain.md).

## The pseudonymisation boundary

The clinical record and the parties it is about live in two domains, each
behind its own PostgreSQL role, and no credential the server uses reads both. The roles, the subject link and the
posture you declare are on [The pseudonymisation
boundary](pseudonymisation.md).

## One instance, one organisation

FerroEHR is single-tenant, and that is where openEHR puts the boundary. BASE
`architecture_overview/master06-design_of_the_ehr.adoc` §The EHR System defines
a system as "a distinct logical repository corresponding to an organisational
entity that is _legally responsible_ for the management and governance of the
healthcare data contained within", and says it is "distinct from any underlying
virtualisation infrastructure or cloud computing facility, which may house
multiple logical EHR systems in a multi-tenant fashion". Multi-tenancy belongs
to the layer that hosts several systems, not inside one of them.

So several organisations are served by several instances: one instance, one
database, one set of domain roles each. That is a stronger boundary than a row
predicate: a defect in a query cannot cross it, because there is nothing to
cross, and it is what the §System Identity rule needs, since `system_id`
"becomes embedded in the version identifiers of committed -- and possibly
signed -- content" and "cannot easily be changed afterwards". One `system_id`
shared across organisations mints version identifiers that cannot tell the
responsible parties apart.

There is therefore no tenant column on any relation, no row policy, no session
variable to set and no `[tenancy]` configuration. What separates the clinical
record from the identity of its subject is the pseudonymisation boundary above,
which is a different question and stays.

## Version signing

Every version the server commits can carry a `VERSION.signature`, computed
inside the write transaction over the canonical form of the version itself.
Signing is **on by default** in `digest` mode, and read-time verification of
the server's own signatures defaults to `strict`, so a served version that no
longer matches its stored signature is a `500` rather than a silently served
record.

The chapter **[Version signing](signing/index.md)** covers the mechanism in two
pages: [Digest signing](signing/digest.md) (what is signed, when, what the
stored value proves, and how to reproduce it yourself) and
[PGP signing](signing/pgp.md) (key configuration and rotation, client-supplied
signatures, and the signature an import wrapper carries). The `[record.signing]` keys
are in the [configuration reference](installation/config-auth.md#recordsigning).

## ATNA audit trail

Separately from openEHR's own provenance, FerroEHR keeps an IHE ATNA
security audit trail of API access: **on by default**, persisted in the
local Audit Record Repository (the dedicated `audit` PostgreSQL schema),
rendered in both official formats (FHIR R4 `AuditEvent` per IHE BALP, and
the DICOM PS3.15 audit message for the classic syslog feed), retrievable via
the RESTful-ATNA **ITI-81** FHIR search, and optionally forwarded to an
external ARR over syslog and/or the ITI-20 FHIR feed. Node authentication
(ITI-19) is available as native mutual TLS on the listener.

Stored records are **tamper-evident**: each is covered by its content digest
in a signed seal window, the table refuses every rewrite path, and `ferroehr
audit-chain verify` and `ferroehr access-log verify` name any record that was
modified, removed or inserted inside a sealed window. That is detection, not prevention: the
controls that make it hard to forge wholesale are the least-privilege database
role and the off-box sinks.

The full chapter (record content, sinks, tamper evidence, the ITI-81 search,
fail-mode semantics, and mTLS) is **[Audit trail (IHE ATNA)](audit.md)**;
every `[audit]` key is in the
[configuration reference](installation/config-audit.md#audit).

> [!NOTE]
> The ATNA trail is orthogonal to openEHR's own `CONTRIBUTION` and
> `AUDIT_DETAILS`, which the server always writes in the same transaction as
> every change. openEHR audit records what a version says about its own
> authorship; ATNA records security surveillance of API access. Both coexist.
> Identified data never enters telemetry (metrics, traces, logs), so the audit
> trail is the single place where access to identified data is recorded; see
> [Operations](operations.md).
