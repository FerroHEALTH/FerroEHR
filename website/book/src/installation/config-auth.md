# Authentication & access

Who may call the server, what they may do once identified, and the surfaces
that are gated rather than always-on: `[auth]`, `[authz]`, `[admin]`,
`[smart]`, `[management]`, and `[signing]`. Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[auth]`

Authentication: bearer tokens of one or more trusted issuers, Basic
credentials, or both. `[auth]` is the section every FerroHEALTH product shares,
checked by the `ferrohealth-smart` crate. FerroEHR is a resource server: it
checks tokens and never issues them.

```toml
[auth]
enabled = true
issuer = "https://keycloak.example.com/realms/ferroehr"
audience = ["ferroehr"]

[[auth.basic.users]]
user = "clinician"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$…$…"   # never a plaintext password
scopes = ["user/composition-*.cruds", "user/template-*.cruds", "user/aql-*.cruds"]
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch. `false` = all requests pass unauthenticated (development only). With `true` and **no mechanism configured the server refuses to start**; see below. |
| `issuer` | string, or `[[auth.issuer]]` tables | unset | The issuer whose tokens are accepted. Its metadata is read at boot, at the RFC 8414 §3 location first and the OpenID Connect Discovery 1.0 §4 location after a `404`, and with it the key set at its `jwks_uri`, so an authorization server that cannot be reached refuses the start. An `https` URL with no query and no fragment and no user name or password; plain `http` is accepted on a loopback host only. Several issuers are `[[auth.issuer]]` tables, each carrying the keys from `issuer` to `jwks_file`. |
| `audience` | list of string | **required with an issuer** | Accepted `aud` values; a token naming none of them is refused (RFC 9068 §4, RFC 8725 §3.9). |
| `algorithms` | list of string | `["RS256","RS384","PS256","PS384","ES256","ES384"]` | Accepted signature algorithms. Asymmetric only: an `HS*` algorithm is a boot error, and `none` is no algorithm at all (RFC 8725 §3.1, §3.2). |
| `token_profile` | `rfc9068` or `jwt` | `rfc9068` | `rfc9068` requires `typ` `at+jwt` and the `client_id`, `iat` and `jti` claims (RFC 9068 §2.2, §4). `jwt` admits a plain JWT bearer token (`typ` absent or `JWT`) for an issuer that mints no RFC 9068 tokens, and reads a `scp` array when `scope` is absent. A token typed as another kind of JWT is refused under either. |
| `clock_skew_seconds` | int | `60` | Leeway on `exp`, `nbf` and `iat`. At most `300`. |
| `jwks_max_age_seconds` | int | `600` | A key set older than this is read again. |
| `jwks_refresh_interval_seconds` | int | `60` | A token naming an unknown `kid` reads the key set again at most once per interval. |
| `jwks_file` | path | `""` | A JWK Set of public keys handed over out of band, read instead of the issuer's metadata. A symmetric key (`kty: "oct"`) or a private-key member (`d`, `p`, `q`, `dp`, `dq`, `qi`, `oth`, `k`) is a boot error. An issuer read from a file has no metadata, so it serves the SMART discovery document only with [`[smart] issuer_metadata_file`](#smart) set. |
| `timeout_ms` | int | `10000` | One call to the authorization server. |
| `realm` | string | `""` ⇒ `ferroehr` | The realm every `WWW-Authenticate` challenge names. |
| `scopeless_subjects` | list of string | `[]` | The exact `sub` values whose token, when it carries no resource scope of either the SMART on openEHR or the SMART on FHIR grammar, meets every SMART on openEHR requirement. A token that carries resource scopes gets only its scopes, and the allowlist reaches neither the ADMIN API nor the DEMOGRAPHIC API, which need the product scope. |
| `capabilities` | list of string | `[]` | SMART capabilities the discovery document adds: one SMART App Launch or the SMART on openEHR draft lists, or a full URI. |
| `scopes_supported` | list of string | `[]` | The scopes the discovery document lists. Empty, it lists the scopes the route table honours. |
| `product_scope` | string | `""` ⇒ `__ferroehr.platform` | FerroEHR's own scope. It admits the ADMIN API and, beside the bearer clients in `[smart] demographic_clients`, the DEMOGRAPHIC API, the two surfaces no SMART on openEHR resource family covers. A full URI or a string starting with `__` (SMART App Launch 2.2.0, Scopes and Launch Context, §Extensibility). |

> [!WARNING]
> **`auth.enabled = true` with no mechanism is a boot error.** Such a server
> could only refuse every request while advertising an authentication scheme it
> does not implement, which RFC 9110 §11.6.1 forbids: a `401` challenge must
> name a scheme applicable to the target resource. Configure an `issuer`,
> configure `[[auth.basic.users]]`, or set `auth.enabled = false` for a
> development server.

The key-set fetches verify the identity provider's certificate against the
platform trust store and follow no redirects. An identity provider whose
certificate comes from a private CA is trusted once that CA is in the image's
system store or in a directory or file named by `SSL_CERT_DIR` or
`SSL_CERT_FILE`. A key set that cannot be read when a token needs it answers
`503`, never `401`: the server cannot decide, and a `401` would tell a caller
with a valid token that its credential was refused.

A validated token must also carry a non-blank `sub` claim: the authenticated
subject is stamped into the audit trail, so a token without one is refused with
`401` rather than recorded under a placeholder identity. A bearer token outside
the RFC 6750 §2.1 `b64token` alphabet is refused the same way, with `401` and
`error="invalid_token"`.

### The Basic-auth user store

`[[auth.basic.users]]` is an array of tables and therefore **file-only**: the
environment grammar cannot spell an array index.

| Key | Type | Default | Description |
|---|---|---|---|
| `user` | string | required | The user-id. Empty, or with a colon or a control character, is a boot error (RFC 7617 §2). |
| `password_hash` | secret | required | Argon2**id** PHC hash (`$argon2id$v=19$…`), never a plaintext password. Boot-validated against the OWASP floor; see below. |
| `password_hash_file` | path | unset | Read the hash from a file instead, for a mounted secret. A hash is an offline cracking target, so prefer this wherever the configuration file itself is not treated as sensitive. Exactly one of the pair is required. |
| `scopes` | list of string | `[]` | The SMART on openEHR scopes the user holds, checked on every route exactly like a token's `scope` claim. A user with none is refused everywhere a requirement applies. |

The user's RBAC roles are [`[authz.rbac.basic_roles]`](#authzrbac).

`[auth.basic]` itself takes one key beside the users:

| Key | Type | Default | Description |
|---|---|---|---|
| `cache_ttl_seconds` | int | `0` | How long a credential that verified is kept, so a repeat request skips the Argon2id derivation (`0` checks every request). The entry is named by a keyed hash of `user-id:password`, never the password, and only a credential that verified is kept, so a wrong password always pays the derivation. The TTL bounds how long a removed user keeps working. Argon2id costs tens of milliseconds of CPU per check by design, so a busy Basic client wants a value such as `60`. |

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

An unknown user-id is checked against a decoy hash of the same cost, so the
time a refusal takes does not say whether the user exists.

## `[authz]`

Role-based (RBAC) and attribute-based (ABAC) authorization, and the
patient-data assurance gate. The full evaluation order and design rationale are
in [Security](../security.md).

### `[authz.assurance]`: the assurance level patient data requires

Off by default. What it refuses and the `401` challenge it sends are described
in [Security](../security.md#assurance-level-and-the-natural-person-on-patient-data).

| Key | Type | Default | Description |
|---|---|---|---|
| `claim` | string | `acr` | The token claim carrying the authentication context class; a dotted path reaches into a JSON object. |
| `levels` | table of string → `low` \| `substantial` \| `high` | empty | Maps each value your identity provider puts in `claim` to an eIDAS level of assurance. A value the table does not name carries no level. File-only: the keys are usually URIs. |
| `minimum` | `low` \| `substantial` \| `high` | unset | The level a patient-data request must carry. Unset refuses nothing. |

```toml
[authz.assurance]
minimum = "substantial"

[authz.assurance.levels]
"http://eidas.europa.eu/LoA/substantial" = "substantial"
"http://eidas.europa.eu/LoA/high" = "high"
```

Boot refuses a `minimum` with an empty `levels` table or with no value mapped at
or above it (every patient-data request would be refused), a blank `claim`
beside a declared table, and a `levels` value that is blank or carries
whitespace, a quote or a backslash, because the refusal names the accepted
values in one quoted, space-separated `acr_values` parameter (RFC 9470 §3).

### `[authz.professional]`: the natural person behind a token

Absent by default, and tokens are then not judged on it. Present, a
patient-data request whose token names no natural person is refused with `403`,
unless it is a client token acting for a named professional.

| Key | Type | Default | Description |
|---|---|---|---|
| `claim` | string | `sub` | The claim identifying the natural person; a dotted path reaches into a JSON object. A token lacking it is a client token. |
| `client_tokens` | `sub_is_client` \| `claim_absent` | `sub_is_client` | How else a client-credentials token is recognised. `sub_is_client` also treats a token whose `sub` equals its `client_id` or `azp` as a client token (RFC 9068 §2.2); `claim_absent` relies on the missing claim alone. |
| `acting_for_claim` | string | unset | The claim a client token carries the professional's identifier in, such as `act.sub`. Unset refuses every client token on patient data. |

The access record then carries the acting mode and the professional; see
[Audit trail](../audit.md#the-ehds-logging-elements-mapped). Both tables are
reachable from the environment too (`FERROEHR__AUTHZ__ASSURANCE__MINIMUM`,
`FERROEHR__AUTHZ__PROFESSIONAL__ACTING_FOR_CLAIM`), except the `levels` map.

### `[authz.rbac]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | The coarse role gate (active when authentication is enabled). Off, every authenticated caller may use every enabled surface, reported as the `rbac_off` deployment gap. `ehr_access_default` and `admin_role` apply either way. |
| `admin_role` | string | `ADMIN` | Role required for admin-class operations. A blank value is a boot error. |
| `user_role` | string | `USER` | Baseline clinical role. |
| `readonly_role` | string | `READONLY` | Role marking a principal read-only: refused on every write operation (create/update/delete/upload), even alongside granting roles. Reads and AQL queries are still allowed. |
| `identifier_reader_role` | string | unset | Role that may read a party's protected national identifiers through `GET {base}/admin/party/identifiers`, beside the admin role. Every read is an access record naming the reader, the party and the scheme. Unset, nobody may. See [reading a party's protected identifiers](../operations-admin-apis.md#reading-a-partys-protected-identifiers). |
| `subject_audit_role` | string | unset | Role that may read the access log for one subject at a time: `GET /fhir/r4/AuditEvent` with the `patient` parameter required, refused without it. The grant a patient portal holds to serve a person's right to know who accessed their record (GDPR Art. 15, EHDS Art. 9), instead of an admin credential over every patient's log. Unset, the log stays admin-only. See [Audit trail](../audit.md#retrieving-audit-records-iti-81). |
| `role_claims` | list of string | `["roles","groups","entitlements","realm_access.roles"]` | JWT claim paths mined for roles, in order. Dotted paths walk nested claims. Must be non-empty and contain no blank path. **`scope` is not a role source**; see [Security](../security.md#rbac-role-based-coarse). |
| `basic_roles` | table of user-id → list of string | empty | The roles of each `[[auth.basic.users]]` user, upper-cased on authentication. A user it does not list holds `user_role`. File-only. |
| `ehr_access_default` | enum{open,restricted} | `open` | What an EHR carrying no `ACCESS_CONTROL_SETTINGS` admits. `restricted` is object-level default-deny: only `admin_role` reaches a setting-less EHR, so an operator can still author the settings that open it. It is enforced whether `enabled` is on or off. With `[auth] enabled = false` nothing could enforce `restricted`, so that combination is a boot error. See [Security](../security.md#per-ehr-access-control-ehr_access). |

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
| `patient_claim` | string | `patient_id` | JWT claim carrying the patient id. Set, it scopes every request to the patient it names (see below). Empty turns the patient scope off. |
| `patient_namespace` | string | unset | The subject namespace (`EHR_STATUS.subject.external_ref.namespace`) the patient ids in `patient_claim` are issued in. Set, a claim names the EHR whose subject is that id under this namespace. Unset, a claim names the EHR holding that id only when no other namespace holds the same id. |
| `check_directory` | bool | `false` | Submit DIRECTORY (`FOLDER`) operations to the decision point. Engine-independent, so it works under Cedar as well as a remote PDP. |

**The patient scope.** With ABAC on and `patient_claim` set, a token
without that claim is refused with `403`. With the claim:

- A route that names an EHR (`/ehr/{ehr_id}/…`) answers `403` unless the
  claim names that EHR's subject. An EHR that records no subject is in no
  patient's scope and is refused.
- Without `patient_namespace`, an id held under two or more namespaces names
  none of those subjects, so it reaches none of their EHRs. Set
  `patient_namespace` to read the claim in one namespace.
- An AQL query returns rows only from EHRs the claim names, whatever the query
  selects, including `SELECT e/ehr_id/value FROM EHR e`.

After a query runs, the decision point receives the subject ids of every EHR
the query touched as its `patient` attribute, so a patient-keyed policy
applies to queries whether or not `patient_claim` is set. Without
`patient_namespace`, a query that touches a subject id held under several
namespaces is refused with `403`, because a bare id would name both subjects.

- **`[authz.abac.cedar]`:** `policy_dir` (path, required when
  `engine = "cedar"` and ABAC is on) and `reload_secs` (int, unset, an
  optional hot-reload interval).
- **`[authz.abac.remote]`:** `server` (secret URL, required when
  `engine = "remote"`, and it must end with `/`, because the policy name is
  appended; credentials embedded in it are redacted from every rendering), `connect_timeout_ms` (int, `2000`), `request_timeout_ms` (int,
  `5000`).
- **`[authz.abac.remote.oauth]`:** calls the policy service as an OAuth 2.0
  client instead of with the Basic credentials of `server`: FerroEHR obtains a
  token with the client credentials grant and sends it as a bearer token. The
  keys are those of [`[audit.fhir_feed.oauth]`](config-audit.md#auditfhir_feedoauth-authenticating-with-oauth-20-client-credentials)
  (`token_url` or `issuer`, `client_id`, `client_secret` or
  `client_secret_file`, `client_auth`, `scope`, `audience`, `resource`).
  When the policy service refuses the token with `401`, a new one is fetched
  and the decision asked once more; a token that cannot be obtained, or that
  is refused again, is no decision, and the request fails closed with a `500`,
  never a `403`. `server` with credentials beside this table stops startup.
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

The SMART on openEHR scopes are enforced on **every route** whenever
`[auth]` is on: each operation carries a requirement from FerroEHR's route
table, which the `ferrohealth-smart` crate checks against the token's scopes
or a Basic user's `scopes`. There is no switch for it. This section decides
what beside the route table: whether the discovery document is served, and
whose grants count for what. See [SMART App Launch](../smart-app-launch.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Serve `/.well-known/smart-configuration`. It is built from the first `[auth]` issuer's metadata, so it needs an issuer whose metadata names a `token_endpoint` (SMART App Launch 2.2.0, Conformance, §Metadata); an issuer read from a `jwks_file` refuses the start with `enabled = true` unless `issuer_metadata_file` supplies that metadata. |
| `issuer_metadata_file` | path | unset | A JSON file holding the RFC 8414 §2 metadata of the first `[auth]` issuer when its key set is a `jwks_file`, read at boot. It must name that issuer, and every endpoint it names must be `https` (a loopback host excepted); the discovery document takes its `token_endpoint` and the other endpoints from it. Set for an issuer whose metadata is read from the issuer, it is a boot error. |
| `platform_base_url` | string | unset ⇒ the REST root | The path the discovery document hangs off, e.g. `/gateway/v1`. |
| `public_base_url` | string | **required when enabled** | The server's externally reachable origin (e.g. `https://cdr.example.com`), from which the discovery document's absolute `services.*.baseUrl` values are built. |
| `launch_base64_json` | bool | `false` | Advertise the `launch-base64-json` capability. Experimental, and advisory: the base64 JSON launch object is consumed by the application, not the CDR. |
| `backend_clients` | list of string | `[]` | The `client_id`s a `system/aql-*` grant counts for: such a grant "would grant access to all registered and ad-hoc AQL queries system-wide" (master08 §Resource Scopes). |
| `patient_issuers` | list of string | `[]` | The issuers whose `patient/` grants count, each confined to the EHR the token's `ehrId` claim names (master07 §Context Selection). Empty, a `patient/` grant grants nothing. |
| `demographic_clients` | list of string | `[]` | The bearer `client_id`s that reach the DEMOGRAPHIC API, which no SMART on openEHR scope covers. A Basic user-id never matches an entry. A caller holding `[auth] product_scope` reaches it too. |

`[smart.episode]`: `enabled` (bool, `false`) advertises episode context and
accepts the `launch/episode` scope and `episodeId` claim, but applies no
episode-scoped filtering.

No `smart` URL may carry a user name or password, whether or not the document
is served: the discovery document publishes these URLs to every application,
so a credential in one would be served to anyone who asks.

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
the one in force on `GET /management/status` (`licence.use`, `licence.licensee`,
`licence.not_after`, `licence.configured_token`). See
[Licensing & legal](../licensing.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `file` | path | unset | The licence token the licensor issued. A token that cannot be read or does not verify is reported as `refused` on `/management/status`; the embedded grant stays in force. |

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
