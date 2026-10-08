# Authentication & access

Who may call the server, what they may do once identified, and the surfaces
that are gated rather than always-on: `[auth]`, `[authz]`, `[admin]`,
`[smart]`, `[management]`, and `[signing]`. Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[auth]`

Authentication: Basic credentials, OAuth2/OIDC bearer tokens, or both.

```toml
[auth]
enabled = true
verified_cache_ttl_seconds = 60

[[auth.basic.users]]
username = "clinician"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$…$…"   # never a plaintext password
roles = ["USER"]

[auth.oidc]
issuer = "https://keycloak.example.com/realms/ferroehr"
audiences = ["ferroehr"]
algorithms = ["RS256"]
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch. `false` = all requests pass unauthenticated (development only). With `true` and **no mechanism configured the server refuses to start**; see below. |
| `verified_cache_ttl_seconds` | int | `60` | Verified Basic-credential cache TTL (`0` disables). Argon2 verification costs real CPU per call by design, so a credential that has verified is remembered (as a digest of the presented header, never plaintext) and re-verified after the TTL. It bounds both the KDF cost of a busy client and how long a revoked credential keeps working. |

> [!WARNING]
> **`auth.enabled = true` with no mechanism is a boot error.** Such a server
> could only refuse every request while advertising an authentication scheme it
> does not implement, which RFC 9110 §11.6.1 forbids: a `401` challenge must
> name a scheme applicable to the target resource. Configure
> `[[auth.basic.users]]`, configure `[auth.oidc]`, or set
> `auth.enabled = false` for a development server.

### The Basic-auth user store

`[[auth.basic.users]]` is an array of tables and therefore **file-only**: the
environment grammar cannot spell an array index.

| Key | Type | Default | Description |
|---|---|---|---|
| `username` | string | required | Principal name. A blank or missing one is a boot error. |
| `password_hash` | secret | required | Argon2**id** PHC hash (`$argon2id$v=19$…`), never a plaintext password. Boot-validated against the OWASP floor; see below. |
| `password_hash_file` | path | unset | Read the hash from a file instead, for a mounted secret. A hash is an offline cracking target, so prefer this wherever the configuration file itself is not treated as sensitive. The Argon2id floor is validated identically either way, because validation runs after the file is resolved. Exactly one of the pair is required. |
| `roles` | list of string | `["USER"]` | Roles granted, upper-cased on authentication. Use `["ADMIN"]` for an administrative account. |

> [!WARNING]
> **Every `password_hash` must meet the OWASP Argon2id floor: `m>=19456`
> (19 MiB), `t>=2`, `p>=1`, algorithm `argon2id`.** Anything weaker (or a
> non-`argon2id` PHC string, or an unparsable one) is a boot error naming the
> user. This is checked at startup because the verifier takes its cost
> parameters *from the stored hash*, so a deliberately cheap hash would
> otherwise verify happily and silently weaken every password in the store.
> The floor is the
> [OWASP Password Storage Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)
> §Argon2id minimum.

### `[auth.oidc]`: bearer validation

The table's absence disables bearer authentication entirely. When present, the
server validates tokens as a resource server; it never issues them.

| Key | Type | Default | Description |
|---|---|---|---|
| `issuer` | string | required when the table is present | Expected `iss`; also the OIDC discovery base. Must be an absolute `https` URL with no query and no fragment (RFC 8414 §2), boot-validated. |
| `audiences` | list of string | **required, non-empty** | Accepted `aud`. An empty or all-blank list is a boot error. |
| `algorithms` | list of string | `["RS256"]` | Accepted signature algorithms. Boot-bound to the key source: `HS*` requires `hmac_secret`, `RS*`/`ES*`/`PS*` require public keys (a static JWKS or the discovered one). `none` is refused outright. |
| `require_at_jwt` | bool | `false` | Refuse a token that does not carry `typ: at+jwt`. A token that *does* carry it is held to RFC 9068 §2.2 either way: `iat`, `jti` and `client_id` become mandatory for it. |
| `clock_skew_leeway_seconds` | int | `60` | Leeway on the time-based claims (`exp`/`nbf`). Capped at `300`; above that is a boot error. |
| `allow_insecure_issuer` | bool | `false` | Accept a non-`https` `issuer`. **Development and test only.** |
| `hmac_secret` / `hmac_secret_file` | secret / path | unset | Symmetric `HS*` secret (development/test), minimum 32 bytes. At most one of the pair. |
| `jwks_json` / `jwks_json_file` | string / path | unset | Static JWKS document of public keys. At most one of the pair. A symmetric key (`kty: "oct"`) or a private-key member (`d`, `p`, `q`, `dp`, `dq`, `qi`, `oth`, `k`) is a boot error, so the set holds no secret and `config check` and `GET {base}/admin/config` show it in clear. |
| `connect_timeout_ms` | int | `3000` | TCP connect timeout for the discovery + JWKS fetches. |
| `request_timeout_ms` | int | `5000` | Whole-request timeout for the discovery + JWKS fetches (connect, TLS, body read). |
| `negative_cache_ttl_seconds` | int | `10` | How long a *failed* discovery/JWKS fetch is remembered (`0` disables). |

The boot rules, and what each one prevents:

- **`audiences` must name at least one audience.** RFC 7519 §4.1.3 obliges a
  recipient that does not identify itself with a value in a present `aud` claim
  to reject the JWT, and RFC 9068 §4 step 4 makes the check unconditional for
  an access token. A resource server that declares no audience cannot reject a
  token minted for a *different* resource server, and cannot tell an OpenID
  Connect ID token (whose `aud` is a client id) from an access token
  (RFC 8725 §3.9, §3.12). Set it to whatever your identity provider puts in
  `aud` for this CDR.
- **`issuer` must be an `https` URL with no query or fragment.** That is the
  RFC 8414 §2 definition of an issuer identifier, and §6.2 requires TLS for
  issuer metadata: over plain HTTP an attacker on the network can serve their
  own signing keys. A development issuer is opted in explicitly with
  `allow_insecure_issuer = true`; the no-query/no-fragment rules still apply,
  since those are structural.
- **`clock_skew_leeway_seconds` is capped at 300.** RFC 7519 §4.1.4 allows
  "some small leeway, usually no more than a few minutes, to account for clock
  skew", and RFC 9068 §4 step 6 repeats the bound. A large leeway silently
  extends the life of *every* token past its `exp`.
- **`hmac_secret` must be at least 32 bytes.** RFC 8725 §3.5: a
  human-memorizable password must not be used directly as the key to a
  keyed-MAC algorithm such as `HS256`. A symmetric key is also shared with the
  authorization server (meaning this server could mint the very tokens it
  accepts) so the boot log warns that it is a development posture. Prefer
  discovery or `jwks_json`.
- **The algorithm set is bound to the key source.** A key belongs to one
  algorithm family, and accepting an algorithm the configured key material
  cannot verify is the algorithm-confusion setup RFC 8725 §3.1 warns about:
  most famously an `RS256` deployment that also accepts `HS256`, letting an
  attacker sign with the public key as if it were a shared secret.

The signing-key source is exactly one of: the symmetric secret, the static
JWKS, or (when neither is set) the issuer's OIDC discovery document.
Configuring both `hmac_secret` and `jwks_json` (in either direct or `*_file`
form) is a boot error, never resolved by silent precedence. A validated token
must also carry a non-blank `sub` claim: the authenticated subject is stamped
into the audit trail, so a token without one is refused with `401` rather than
recorded under a placeholder identity.

The last three keys apply only when keys come from OIDC discovery. The timeouts
stop an unresponsive identity provider from parking bearer requests until the
operating system's TCP timeout; the negative cache means a provider outage
costs one discovery attempt per `negative_cache_ttl_seconds` rather than one
per incoming request, so callers get fast `401`s instead of slow ones. Keep the
negative TTL short: it is also how long recovery takes to be noticed after the
provider comes back.

## `[authz]`

Role-based (RBAC) and attribute-based (ABAC) authorization. The full evaluation
order and design rationale are in [Security](../security.md).

### `[authz.rbac]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | The coarse role gate (active when authentication is enabled). |
| `admin_role` | string | `ADMIN` | Role required for admin-class operations. A blank value is a boot error. |
| `user_role` | string | `USER` | Baseline clinical role. |
| `readonly_role` | string | `READONLY` | Role marking a principal read-only: refused on every write operation (create/update/delete/upload), even alongside granting roles. Reads and AQL queries are still allowed. |
| `subject_audit_role` | string | unset | Role that may read the access log for one subject at a time: `GET /fhir/r4/AuditEvent` with the `patient` parameter required, refused without it. The grant a patient portal holds to serve a person's right to know who accessed their record (GDPR Art. 15, EHDS Art. 9), instead of an admin credential over every patient's log. Unset, the log stays admin-only. See [Audit trail](../audit.md#retrieving-audit-records-iti-81). |
| `role_claims` | list of string | `["roles","groups","entitlements","realm_access.roles"]` | JWT claim paths mined for roles, in order. Dotted paths walk nested claims. Must be non-empty and contain no blank path. **`scope` is not a role source**; see [Security](../security.md#rbac-role-based-coarse). |
| `ehr_access_default` | enum{open,restricted} | `open` | What an EHR carrying no `ACCESS_CONTROL_SETTINGS` admits. `restricted` is object-level default-deny: only `admin_role` reaches a setting-less EHR, so an operator can still author the settings that open it. See [Security](../security.md#per-ehr-access-control-ehr_access). |

> [!NOTE]
> The management surface is **not** configured under `[authz.rbac]`.
> [`[management.endpoints]`](#management) owns it, one level per endpoint, with
> no global default beside it: an endpoint you do not name is `off` and is not
> mounted. Only the `admin_only` level consults `authz.rbac.admin_role`.

### `[authz.abac]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Master ABAC switch. |
| `engine` | enum{cedar,remote} | `cedar` | Embedded Cedar, or a remote decision point. |
| `organization_claim` | string | `organization_id` | JWT claim carrying the caller's organization. |
| `patient_claim` | string | `patient_id` | JWT claim carrying the patient id. |
| `check_directory` | bool | `false` | Submit DIRECTORY (`FOLDER`) operations to the decision point. Engine-independent, so it works under Cedar as well as a remote PDP. |

- **`[authz.abac.cedar]`:** `policy_dir` (path, required when
  `engine = "cedar"` and ABAC is on) and `reload_secs` (int, unset, an
  optional hot-reload interval).
- **`[authz.abac.remote]`:** `server` (string, required when
  `engine = "remote"`, and it must end with `/`, because the policy name is
  appended), `connect_timeout_ms` (int, `2000`), `request_timeout_ms` (int,
  `5000`).
- **`[authz.abac.policy.<kind>]`:** one entry per resource kind, with `kind` ∈
  `ehr`, `ehr_status`, `composition`, `contribution`, `query`, `directory`.
  Keys: `name` (string, the policy to evaluate) and `parameters` (list of
  enum{organization,patient,template}). A key that is not one of those six
  kinds is a boot error, and `template` is rejected on `ehr` and `ehr_status`,
  since neither carries a template.

> [!WARNING]
> **With `engine = "remote"`, every resource kind the enforcement point
> consults needs a policy entry: `ehr`, `ehr_status`, `composition`,
> `contribution`, `query`**, plus `directory` when `check_directory = true`. A
> missing one is a boot error. At runtime a kind with no policy can only be
> **denied**, since there is no policy to ask and permitting would be a silent
> hole, so the misconfiguration is caught at startup rather than turning into
> blanket `403`s on live traffic. The Cedar engine reads its policies from
> `authz.abac.cedar.policy_dir` and needs no entries here.

## `[admin]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Mount the ADMIN API (physical, irreversible delete). Off ⇒ every admin route answers `405 Method Not Allowed` with an empty `Allow` header, never `403`, and never touches the backend. |

Physical deletion is irreversible, so the group stays off by default. With it
on, `/admin` also joins the group list the `OPTIONS` System-Options manifest
advertises, so the manifest never names a group that answers `404`.

## `[smart]`

SMART App Launch. Off by default; when off the discovery document is not served
and the scope gate is inert. See [SMART App Launch](../smart-app-launch.md).

`[smart]`:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Serve the discovery document and activate the scope gate. |
| `platform_base_url` | string | unset ⇒ the REST root | The path the discovery document hangs off, e.g. `/gateway/v1`. |
| `public_base_url` | string | **required when enabled** | The server's externally reachable origin (e.g. `https://cdr.example.com`), from which the discovery document's absolute `services.*.baseUrl` values are built. |
| `ehr_id_claim` | string | `ehrId` | Token claim carrying the launch context's openEHR EHR id. |
| `patient_claim` | string | `patient` | Fallback launch-context claim when `ehr_id_claim` is absent. |
| `require_smart_scopes` | bool | `false` | When `true`, the resource-scope gate is fail-closed across the composition, template and AQL families, and the `openehr-permission-v1` capability is advertised: every caller on those families needs a Bearer token carrying a matching SMART resource scope, so a Basic-authenticated caller, which carries no scopes at all, is refused with `403` there (the EHR family is not scope-governed). When `false` the gate is advisory (it enforces only when a token actually carries SMART resource scopes) and the capability is not claimed. |
| `launch_base64_json` | bool | `false` | Advertise the `launch-base64-json` capability. Experimental, and advisory: the base64-JSON launch object is consumed by the application, not the CDR. |

`[smart.episode]`: `enabled` (bool, `false`) advertises episode context and
accepts the `launch/episode` scope and `episodeId` claim, but applies no
episode-scoped filtering.

`[smart.endpoints]` carries the external authorization-server endpoints the
discovery document publishes verbatim: `issuer`, `jwks_uri`,
`authorization_endpoint`, `token_endpoint`, `registration_endpoint`,
`introspection_endpoint`, `revocation_endpoint`, `management_endpoint` (all
string, unset ⇒ omitted from the document); the advertised lists
`token_endpoint_auth_methods_supported`, `grant_types_supported`,
`response_types_supported`, `code_challenge_methods_supported`,
`scopes_supported` and `capabilities` (all list of string, `[]`;
`capabilities` appends operator-advertised HL7 base capabilities such as
`launch-ehr` or `sso-openid-connect` to the derived openEHR set); and
`allow_insecure_endpoints` (bool, `false`).

Everything in `[smart.endpoints]` is **published** at
`/.well-known/smart-configuration` for third-party applications to act on, so
`smart.enabled = true` boot-validates it rather than relaying whatever is
configured:

| Rule | Why |
|---|---|
| Deprecated grant types (`implicit`, password) rejected, whether or not SMART is enabled | the SMART App Launch specification's deprecated-flows section |
| `public_base_url`, `authorization_endpoint`, `token_endpoint` required | an enabled Platform without them publishes an unusable document |
| Every advertised endpoint an absolute `https` URL | the document tells apps where to send an authorization request and exchange a code, so a plaintext endpoint exposes the code and the access token ([RFC 6749 §3.1.2.1](https://www.rfc-editor.org/rfc/rfc6749#section-3.1.2.1), [RFC 8414 §6.2](https://www.rfc-editor.org/rfc/rfc8414#section-6.2)). `allow_insecure_endpoints = true` opts out for development |
| `issuer` has no query and no fragment | [RFC 8414 §2](https://www.rfc-editor.org/rfc/rfc8414#section-2), the same rule `auth.oidc.issuer` follows, because it is the same identity |
| `response_types_supported` non-empty | RFC 8414 §2 marks the field **REQUIRED** |
| `token_endpoint_auth_methods_supported` non-empty | an empty list advertises a server that authenticates no client |
| `code_challenge_methods_supported` includes `S256` | SMART App Launch requires PKCE ([RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)); publishing a list without it tells every app the server cannot do PKCE, and `plain` alone is not sufficient |
| `smart.endpoints.issuer` equals `auth.oidc.issuer` | one says where apps **obtain** tokens, the other which tokens this server **accepts**. A mismatch means every app gets a valid token and every request is refused |
| `smart.enabled` requires `[auth.oidc]` | the CDR cannot validate the tokens it directs applications to obtain |

> [!NOTE]
> An empty advertised list is not silence: it claims the authorization server
> supports none of that thing, and a conforming application will believe it.

## `[management]`

The ops-introspection surface: build info, Prometheus, metric views, the
effective configuration, runtime log control, and the on-demand profiler. Off
by default, and every endpoint off individually.

The **health probes are not configured here**: `/health`, `/health/liveness`
and `/health/readiness` are always served on the main API port without
authentication, whatever this section says (see
[Operations → Health probes](../operations.md#health-probes)).

```toml
[management]
enabled = false
base_path = "/management"

[management.endpoints]
info = "off"
metrics = "off"
prometheus = "off"
env = "off"
loggers = "off"
flamegraph = "off"

[management.profiling]
max_seconds = 30
max_frequency = 999
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Mount the management router. |
| `base_path` | string | `/management` | Base path for the management endpoints. |
| `port` | int | unset ⇒ share the main listener | Serve management on its own listener and port. Must differ from the `server.bind` port, and it is plain HTTP, an internal surface. |

`[management.endpoints]`: `info`, `metrics`, `prometheus`, `env`, `loggers`
and `flamegraph`, each enum{off,admin_only,private,public}, default `off`.

| Level | Meaning |
|---|---|
| `off` | Not mounted at all; the route answers `404`. |
| `admin_only` | Requires an authenticated principal carrying `authz.rbac.admin_role` (`401` unauthenticated, `403` authenticated but not admin). |
| `private` | Requires any authenticated principal. |
| `public` | Served **outside** authentication. |

`env` renders the effective configuration and `flamegraph` starts a profiler on
request, so the boot log prints exactly which endpoints a configuration turned
on and at which level.

`[management.profiling]`: limits for the on-demand CPU flamegraph behind
`endpoints.flamegraph` (see
[Operations → Profiling](../operations.md#profiling-the-on-demand-cpu-flamegraph)):

| Key | Type | Default | Description |
|---|---|---|---|
| `max_seconds` | int | `30` | Longest sample window one request may ask for. A request asking for more is refused with `400`, never clamped. |
| `max_frequency` | int | `999` | Highest sampling frequency (Hz) a request may ask for. Same refusal semantics. |

> [!WARNING]
> `probes_enabled` and `endpoints.health` do not exist. Configuration is
> strict, so a file or environment variable still setting either one fails at
> boot with an unknown-key error: delete the key; the probes are always on.

## `[licence]`

The commercial licence token. Every build embeds the licensor's
`non-commercial` grant; a deployment that holds a `commercial` licence installs
its token here. The server behaves identically under either grant and reports
the one in force on `GET /rest/status` (`licence.use`, `licence.licensee`,
`licence.not_after`, `licence.configured_token`). See
[Licensing & legal](../licensing.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `file` | path | unset | The licence token the licensor issued. A token that cannot be read or does not verify is reported as `refused` on `/rest/status`; the embedded grant stays in force. |

## `[signing]`

VERSION signing. On by default in `digest` mode, with read-time verification of
the server's own signatures **strict** by default.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Server-side signing of committed versions. |
| `mode` | enum{digest,pgp} | `digest` | A SHA-256 integrity digest, or an OpenPGP (RFC 4880) detached signature. |
| `key_path` | path | unset | Armored secret key; **required for `pgp`** (a boot error otherwise). |
| `key_passphrase` / `key_passphrase_file` | secret / path | unset | Key passphrase. At most one of the pair. |
| `retired_key_paths` | list of path | `[]` | Armored **public** keys retired from signing and kept for verification, so versions signed before a key rotation keep verifying. |
| `verify_on_read` | enum{off,warn,strict} | unset ⇒ `strict` when signing is enabled | Read-time recompute-and-compare policy for the server's own signatures. |

`retired_key_paths` exists because a stored `VERSION.signature` records no key
identifier and is an immutable committed fact that cannot be re-issued;
keeping the retired public key is the only way history stays verifiable across
a rotation, and a public key can verify but never sign. Its environment form
takes a comma-separated list
(`FERROEHR__SIGNING__RETIRED_KEY_PATHS=/keys/a.pub.asc,/keys/b.pub.asc`).

> [!NOTE]
> **`verify_on_read` resolves to `strict` when signing is enabled.** On every
> read the server recomputes the signature of a version it signed and, on a
> mismatch, returns a `500` integrity fault rather than silently serving a
> provably corrupt record. Set it explicitly to `warn` (log + meter
> `version_signature_invalid_total`, still serve) or `off` (never
> check). **Client-supplied signatures** (an author's own, or one carried by an
> imported version) are always stored verbatim and **never** re-verified,
> whatever this setting says, because the author may have signed a different
> agreed serialization.

> [!WARNING]
> `pgp` mode **fails closed at boot** if the key is missing or unusable: the
> server will not start. Verify the key and passphrase before switching modes.

### Choose an Ed25519 (or other ECC) signing key

Any OpenPGP key algorithm is accepted, but an **RSA** signing key makes every
commit perform an RSA private-key operation, the operation the Marvin timing
sidechannel concerns (RUSTSEC-2023-0071 / CVE-2023-49092), for which the
underlying `rsa` crate has no fixed release. An Ed25519 or ECDSA key keeps that
code off the signing path entirely.

The server does **not** refuse an RSA key: a repository whose history is
already RSA-signed needs that key to keep verifying, and signatures are
immutable committed facts that cannot be re-issued. Instead it logs a warning
at boot naming the advisory. To clear it:

1. Generate a new signing key: `gpg --quick-generate-key "…" ed25519 sign`, and
   export the armored secret key to `key_path`.
2. Export the **public** half of the old certificate and add it to
   `retired_key_paths`, so versions signed with it still verify (see above).
3. Restart. New versions are signed with Ed25519; old ones keep verifying.

Rotation inside one certificate is cheaper still: add an Ed25519 **signing
subkey** to the existing certificate and the server signs with it
automatically, with no `retired_key_paths` entry needed: the certificate keeps
the previous subkey, so past signatures continue to verify.

**On Kubernetes**, `digest` mode needs nothing; it is the default. `pgp` mode
needs the key as a file and its passphrase as a secret, both of which the chart
mounts for you:

```yaml
# values.yaml
config:
  signing:
    enabled: true
    mode: pgp
    key_path: /etc/ferroehr/signing-key.asc
  files:
    signing-key.asc: |
      -----BEGIN PGP PRIVATE KEY BLOCK-----
      …
secrets:
  signingKeyPassphrase: "…"
```

Every `config.files` key becomes `/etc/ferroehr/<key>`, mounted read-only from
a chart Secret at mode `0440`, and that volume holds the private key, so it is
never world-readable inside the container. **To go back to `digest`**, set
`mode: digest` and drop the key material; versions already signed keep their
signatures and still verify.
