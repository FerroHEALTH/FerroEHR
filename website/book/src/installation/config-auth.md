# Authentication & access

Who may call the server, what they may do once identified, and the surfaces
that are gated rather than always-on: `[auth]`, `[api] admin`, the
access-delay register, `[auth.smart]`, `[management]`, `[deployment]`,
`[record.signing]` and `[record.versioning]`. `[authz]` is on [Authorization
keys](config-authz.md) and `[auth.scim]` on [SCIM provisioning](config-scim.md). Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[auth]`

Authentication: bearer tokens of one or more trusted issuers, Basic
credentials, or both. `[auth]` is the section every FerroHEALTH product shares,
checked by the `ferrohealth-smart` crate (0.32). FerroEHR is a resource server:
it checks tokens and never issues them. Every person a request comes from is
bound to a demographic `PERSON` (see [`[authz.party_binding]`](config-authz.md#authzparty_binding)).

Each issuer is a table named by you, `[auth.issuer.<name>]`, and every other
key that refers to an issuer names that table: `[auth] described_issuer`,
`[authz.rbac] role_issuers`, and the environment form
`FERROEHR__AUTH__ISSUER__<NAME>__<KEY>`. A name is a lower-case letter, then
lower-case letters, digits and `_`, at most 64 characters. A name no table
carries refuses the start, and the refusal lists the names there are.

```toml
[server]
public_base_url = "https://cdr.example.com"

[auth]
enabled = true

[auth.issuer.staff]
issuer = "https://smart.example.com"

[auth.issuer.staff.party]
identifier_issuer = "urn:example:staff-login"
identifier_type = "uuid"

[[auth.basic.users]]
user = "clinician"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$…$…"   # never a plaintext password

[authz.rbac]
first_admin = "clinician"
role_issuers = ["staff"]
```

The issuer is a SMART App Launch 2.2 authorization server whose access
tokens follow RFC 9068, and its audience is the openEHR REST API's `baseUrl`
this server publishes, `https://cdr.example.com/ferroehr/rest/openehr/v1`,
since the table names none (see [the audience](#the-audience)).

A Basic user is a credential alone: `user` and its Argon2id hash
(`password_hash` or `password_hash_file`). What it may do is policy in the
CDR: the roles and scopes the `hold_roles` and `hold_scopes` grants of its
`PERSON` name ([Operational policy](../operational-policy.md)).
`[authz.rbac] first_admin` names the one Basic user that holds `admin_role`
and the product scope from the configuration: the administrator who writes
the first policy parties. The authorization server's registry is the one
list of clients: a token whose `aud` names this server admits the client its
`client_id` names (RFC 9068 §2.2), and an RFC 8693 §4.1 `act` names someone
acting for the user. The caller is read from the token's standard claims:
`sub`, `fhirUser` and its resource type, the `patient` launch context, `act`
and `aud` ([Identity providers](../identity-providers.md)).

Beside a party table, a Basic user is a person's credential and must map to
the key of its `PERSON` with `[[authz.party_binding.basic_user]]`,
`first_admin` included; an unmapped one refuses the start. Until that
`PERSON` exists, the user's requests are refused as unbound. Such a deployment
starts with the operator grant, never with `first_admin`: see
[The first grant](../operational-policy.md#the-first-grant).

The section keys:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch. `false` = all requests pass unauthenticated (development only). With `true` and **no mechanism configured the server refuses to start**; see below. |
| `issuer` | `[auth.issuer.<name>]` tables | none | The trusted issuers, one table each by name; see below. |
| `tls` | table | platform roots | `[auth.tls]`, the family's outbound TLS table: `trust_roots_file`, a PEM bundle of certificate authorities trusted in addition to the platform roots for an issuer behind a private PKI (every issuer's metadata and key set are read trusting it), `client_identity_file` (one PEM holding a client certificate chain and its key) and `min_version` (`"1.3"` by default). An introspection grant's calls use the grant's own `tls` table. A file that cannot be read, holds no certificate, or does not parse refuses the start. |
| `realm` | string | `""` ⇒ `ferroehr` | The realm every `WWW-Authenticate` challenge names. |
| `capabilities` | list of string | `[]` | SMART capabilities the discovery document adds: one SMART App Launch or the SMART on openEHR draft lists, or a full URI. |
| `scopes_supported` | list of string | `[]` | The scopes the discovery document lists. Empty, it lists the scopes the route table honours. |
| `described_issuer` | string | unset | The name of the issuer the discovery document describes: one whose metadata FerroEHR reads, introspects, or takes from [`[auth.smart] issuer_metadata_file`](#authsmart). Read only with `[auth.smart] discovery = true`, where naming an unknown table or one without metadata is a boot error. Required with discovery when more than one issuer could be described. See [the described issuer](#the-described-issuer). |
| `product_scope` | string | `""` ⇒ `__ferroehr.platform` | FerroEHR's own scope. It admits the ADMIN API and, beside the clients the CDR grants `access_demographic`, the DEMOGRAPHIC API, the two surfaces no SMART on openEHR resource family covers. A full URI or a string starting with `__` (SMART App Launch 2.2.0, Scopes and Launch Context, §Extensibility). |

The `[[auth.issuer]]` array, the section-level `[[auth.party]]`,
`[[auth.federation]]` and `[[auth.attributes]]` tables, an issuer's `profile =
"…"` string and the profile keys on the issuer table itself are unknown keys
and refuse the start: each issuer is a named table, the three tables sit under
it, and the profile keys sit in its `profile` table.

### `[auth.issuer.<name>]`: the trusted issuers

Each issuer's metadata is read in the background from the start, at the RFC
8414 §3 location first and the OpenID Connect Discovery 1.0 §4 location after
a `404`, and with it the key set at its `jwks_uri`. A read that fails is
tried again after one second, then after twice as long each time, up to one
minute. Until an issuer is read, its tokens are refused with `503` and the
reason code `issuer-unread`, and `/health/readiness` reports the
`auth_issuers` indicator down, so the server starts whether or not its
authorization server is up and an orchestrator sends it no traffic until it
can check a token. An issuer whose key set is a `jwks_file` is read from that
file at start, and a file that cannot be read refuses the start.

Every setting of an issuer lives on its own table. No two issuers of one node
share an audience, so one audience is one issuer and one token form; the
node's audience must also differ from every other node's, an operator rule
boot cannot see.

| Key | Type | Default | Description |
|---|---|---|---|
| `issuer` | string | required | The issuer identifier: an `https` URL with no query, no fragment and no user name or password; plain `http` on a loopback host only. Two tables naming one issuer refuse the start. |
| `kind` | `federation` | unset | Unset, the issuer is a SMART App Launch 2.2 authorization server, such as [FerroSMART](../behind-ferrosmart.md), signing the tokens of the deployment's own users and clients. `federation` signs another party's professionals or organisation. |
| `audience` | list of string | the openEHR base | Accepted `aud` values; a token naming none of them is refused (RFC 9068 §4, RFC 8725 §3.9). Unset, the openEHR REST API's `baseUrl` (see [the audience](#the-audience)). |
| `algorithms` | list of string | `["RS256","RS384","PS256","PS384","ES256","ES384"]` | Accepted signature algorithms. Asymmetric only: an `HS*` algorithm is a boot error, and `none` is no algorithm at all (RFC 8725 §3.1, §3.2). |
| `clock_skew` | duration | `"60s"` | Leeway on `exp`, `nbf` and `iat`. At most `"5min"`. |
| `jwks_max_age` | duration | `"10min"` | A key set older than this is read again. |
| `jwks_refresh_interval` | duration | `"1min"` | A token naming an unknown `kid` reads the key set again at most once per interval. |
| `jwks_file` | path | `""` | A JWK Set of public keys handed over out of band, read at start instead of the issuer's metadata. A symmetric key (`kty: "oct"`) or a private-key member (`d`, `p`, `q`, `dp`, `dq`, `qi`, `oth`, `k`) is a boot error. An issuer read from a file has no metadata, so it serves the SMART discovery document only with [`[auth.smart] issuer_metadata_file`](#authsmart) set. |
| `timeout` | duration | `"10s"` | One call to the authorization server. |
| `mode` | `jwt` or `introspection` | `jwt` | How the issuer's tokens are checked: as JWTs against its key set, or posted to its RFC 7662 endpoint (see [Opaque tokens](#opaque-tokens-by-introspection)). |
| `role_claims` | list of string | `["roles","groups","entitlements"]` | Token claim paths the caller's roles are read from; dotted paths walk nested claims, and a role is matched without regard to case. **`scope` is not a role source**. A `federation` issuer reads no role. Whether a role grants `admin_role` is [`[authz.rbac] role_issuers`](config-authz.md#authzrbac). |
| `sender_constraint` | `none` or `required` | `none` | `required` admits only a token bound to a DPoP key (RFC 9449) or to the client certificate of a mutual-TLS connection (RFC 8705 §3). A token carrying `cnf` is held to it under either value. |
| `assurance` | table | unset | The assurance level the issuer's tokens carry, in the shape of [`[authz.assurance]`](config-authz.md#authzassurance-the-assurance-level-patient-data-requires). |

### `[auth.issuer.<name>.profile]`: the token shape

Every issuer reads its tokens the same way, and this table sets only what
an authorization server's tokens may differ in:

| Key | Type | Default | Description |
|---|---|---|---|
| `token_types` | list of string | `["at+jwt", "application/at+jwt"]` | The accepted `typ` header values (RFC 9068 §4). |
| `required_claims` | list of string | the RFC 9068 §2.2 set with `scope` | The claims every token must carry. |
| `max_actor_chain` | integer | `1` | The deepest RFC 8693 `act` chain admitted; prior actors are informational only (RFC 8693 §4.1). |
| `onward_*` | | | The onward grants of `ferrohealth-smart`, through which FerroEHR calls another resource server for the caller. |

A user is read by the resource type of their `fhirUser` (SMART App Launch
2.2.0, Scopes and Launch Context, `fhirUser`): a `Patient`, a
`Practitioner`, a `PractitionerRole`, a `RelatedPerson`, a `Person`, or a
user of no stated type. A token's `patient/` grants are confined to the EHR
its `ehrId` claim names (master07 §Context Selection), so a token that names
none reaches no EHR through them. They never make a caller the record's
person, which the party binding alone decides ([A person's own
record](../behind-ferrosmart.md#a-persons-own-record)). The discovery
document lists `permission-patient` and the `patient/` scopes.

### The audience

An issuer whose table names no `audience` admits the tokens whose `aud`
names the openEHR REST API's `baseUrl`: [`[server]
public_base_url`](config-server.md#public_base_url-the-one-public-url) with
`server.base_path` and `api.base_path`, the `baseUrl` the SMART discovery
document publishes (master04 §Services). An authorization server names the
resource the app asked for in `aud` (RFC 8707 §2), and a SMART app asks for
that `baseUrl`, so a deployment writes its public URL once. Without
`public_base_url`, every issuer names its `audience` or the start is refused.

### The tables under an issuer and `[auth.dpop]`

| Table | Description |
|---|---|
| `[auth.issuer.<name>.party]` | One per issuer whose users sign in to FerroEHR: where their logins are looked up. `identifier_issuer` (the issuer's own identifier when empty) and `identifier_type` (`pairwise`, FerroSMART's pairwise `sub`, when empty; `guid` and `uuid` compare in lower case) name the identifiers each token's `sub` is matched against (OpenID Connect Core 1.0 §5.7); `group_claim` with `group_identifier_issuer` reads the groups a token asserts. A user token of an issuer without one is refused `403` `refused-unbound`, so an issuer without a table admits its clients' own tokens alone. Boot refuses a table under a `federation` issuer, two tables sharing an `identifier_issuer`, and a table whose `identifier_type` is the `type` of no [`[[authz.party_binding.person.template]]`](config-authz.md#authzparty_binding) `key`: SCIM and just-in-time creation write the key under that type and the lookup compares the type exactly, so no login of the issuer would bind. |
| `[auth.issuer.<name>.federation]` | Required on a `federation` issuer and refused on any other: how its tokens' claims map to the fields of another party's professional or organisation (`mapping = "iua"`, `"ehdsi"` or per field), the countries, issuing authorities and provider prefixes that bound them, and `contact_point` for a MyHealth@EU contact point. `fields` maps a field to its claim (`{ claim = "..." }`), its `authorization_details` member, or a coded constant with its code system (`{ constant = "…", system = "…" }`), among them `consent.reference`, the consent decision reference of a consent-conditioned read, `correlation_id`, the requester's id a `chained` organisation request carries, and `trc.care_role`, the care role a treatment relationship attests (see [the attested care path](../operational-policy.md)). |
| `[auth.issuer.<name>.attributes]` | At most one per issuer with a party table: where its tokens state the professional's attributes (`mapping = "iua"`, the IHE IUA claims under `extensions.ihe_iua`, or per field in `fields`), and `authorization_details_purposes`, whether the `purpose_of_use` of each RFC 9396 `authorization_details` entry is a purpose of use too. Its purposes of use and roles go into each [access record](../ehds-logging.md#where-the-purpose-of-use-comes-from), and a `[audit] purpose_header` naming a purpose the token does not state refuses the request with `403`. |
| `[auth.dpop]` | The DPoP proof check of a sender-constrained issuer: `max_age` (`"60s"`), `clock_skew` (`"10s"`) and `algorithms`. Each accepted proof is recorded in the database every replica shares until its window ends, so a proof is accepted once across all of them (RFC 9449 §11.1), and a database that cannot record it refuses the request with `503`. A proof's `htu` must name [`server.public_base_url`](config-server.md#public_base_url-the-one-public-url), `server.base_path` and the request path, so the table needs that key. |

> [!WARNING]
> **`auth.enabled = true` with no mechanism is a boot error.** Such a server
> could only refuse every request while advertising an authentication scheme it
> does not implement, which RFC 9110 §11.6.1 forbids: a `401` challenge must
> name a scheme applicable to the target resource. Configure an issuer,
> configure `[[auth.basic.users]]`, or set `auth.enabled = false` for a
> development server.

The key-set fetches verify the identity provider's certificate against the
platform trust store and follow no redirects. An identity provider whose
certificate comes from a private CA is trusted once that CA is in the image's
system store or in a directory or file named by `SSL_CERT_DIR` or
`SSL_CERT_FILE`. A key set that cannot be read when a token needs it answers
`503`, never `401`: the server cannot decide, and a `401` would tell a caller
with a valid token that its credential was refused.

Every refusal names a stable reason code in the `error_description` of its
`WWW-Authenticate` challenge (RFC 6750 §3): `audience-mismatch`,
`issuer-unknown`, `issuer-unread`, `delegate-not-granted` and the rest the
`ferrohealth-smart` book lists. A missing credential and a Basic challenge
carry none. The server logs each refusal with the same code, the token's
issuer, client id and key id, never the token.

A validated token must also carry a non-blank `sub` claim: the authenticated
subject is stamped into the audit trail, so a token without one is refused with
`401` rather than recorded under a placeholder identity. A bearer token outside
the RFC 6750 §2.1 `b64token` alphabet is refused the same way, with `401` and
`error="invalid_token"`.

### Opaque tokens by introspection

An authorization server that issues opaque access tokens is served with
`mode = "introspection"` on its `[auth.issuer.<name>]` table, whose
`[auth.issuer.<name>.introspection]` table holds the keys below. The
introspection endpoint is found at start, so one that cannot be read refuses
the start. Each bearer token is posted to the issuer's
introspection endpoint (RFC 7662 §2.1), and the active response stands in for
the JWT claims: `sub`, `scope`, the roles of `role_claims`, `aud` against
`audience` and `iss` against the issuer. An inactive token is a `401`. An
endpoint that cannot answer is a `503`, because the server cannot decide. An
active response is reused until its `exp`, at most `cache_max_age`.

| Key | Type | Default | Description |
|---|---|---|---|
| `endpoint` | URL | unset | The introspection endpoint; unset reads the `introspection_endpoint` of the issuer's metadata. `https`, or `http` on a loopback host. |
| `cache_max_age` | duration | `"60s"` | The longest an active response is used; `"0s"` caches nothing. |
| `cache_capacity` | int | `10000` | The most responses held at once. |
| `client` | table | unset | The client authentication of the call (RFC 7662 §2.1), with the keys of an outbound grant: `client_id` and `client_secret`, `private_key_jwt` or mutual TLS (see [the grant keys](config-integrations.md#the-endpoint-table)). |
| `bearer` | table | unset | A client credentials grant whose token authorizes the call instead (SMART App Launch 2.2.0, Token Introspection). Exactly one of `client` and `bearer`. |

```toml
[auth.issuer.idp]
issuer = "https://idp.example.org/realms/ferroehr"
mode = "introspection"

[auth.issuer.idp.introspection.client]
client_id = "ferroehr-rs"
client_secret_file = "/run/secrets/introspection-client"
```

### The Basic-auth user store

`[[auth.basic.users]]` is an array of tables. A Basic user is always a
person: it maps to the `PERSON` carrying its binding key through
[`[[authz.party_binding.basic_user]]`](config-authz.md#authzparty_binding), and a Basic user
with no mapping refuses the start, naming the user. A machine authenticates
with the client credentials grant (RFC 6749 §4.4), and a `system/` scope on a
Basic user refuses the start.

| Key | Type | Default | Description |
|---|---|---|---|
| `user` | string | required | The user-id. Empty, or with a colon or a control character, is a boot error (RFC 7617 §2). |
| `password_hash` | secret | required | Argon2**id** PHC hash (`$argon2id$v=19$…`), never a plaintext password. Boot-validated against the OWASP floor; see below. |
| `password_hash_file` | path | unset | Read the hash from a file instead, for a mounted secret. A hash is an offline cracking target, so prefer this wherever the configuration file itself is not treated as sensitive. Exactly one of the pair is required. |
| `scopes` | list of string | `[]` | The SMART on openEHR scopes the user holds, checked on every route exactly like a token's `scope` claim. A user bound to a `PERSON` also holds the scopes of its `hold_scopes` grants ([What the grants decide](../operational-policy.md#what-the-grants-decide)). A user with none is refused everywhere a requirement applies. |
| `roles` | list of string | `[]` | The user's RBAC roles, upper-cased on authentication. A user bound to a `PERSON` also holds the roles of its `hold_roles` grants, never a privileged one. A user with none holds no role and is refused wherever [`[authz.rbac]`](config-authz.md#authzrbac) asks for one. |

`[auth.basic]` itself takes one key beside the users:

| Key | Type | Default | Description |
|---|---|---|---|
| `cache_ttl` | duration | `"5min"` | How long a credential that verified is kept, so a repeat request skips the Argon2id derivation; `"0s"` turns the cache off and checks every request. The entry is named by a keyed hash of `user-id:password` under a key drawn at boot, never the password, and only a credential that verified is kept, so a wrong password always pays the derivation. The cache holds at most 4096 credentials and empties at a restart. The user list and its hashes are read at boot, so removing a user or changing a password takes effect at the restart that loads the new list, which also empties the cache: the TTL never keeps a removed user working. Argon2id is slow by design, so with the cache off its cost bounds how many Basic requests a core can answer. |

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

## `[api] admin`

| Key | Type | Default | Description |
|---|---|---|---|
| `admin` | bool | `false` | Mount the ADMIN API (physical, irreversible delete). Off ⇒ every admin route answers `405 Method Not Allowed` with an empty `Allow` header, never `403`, and never touches the backend. |

Physical deletion is irreversible, so the group stays off by default. With it
on, `/admin` also joins the group list the `OPTIONS` System-Options manifest
advertises, so the manifest never names a group that answers `404`.

## The access-delay register

The EHDS Art. 3(3) delay of the person's own access to newly registered data
is a Member State option, and an
[admin register](../access-policy.md#the-admin-registers), not a
configuration key: you turn it on through
`{ext}/admin/registers/access-delay` only where your Member State's rule
provides for the delay, every version is kept with its committer, and a
change applies on the next request without a restart. What the delay does is
on [delayed access by the person](../access-control.md#delayed-access-by-the-person-ehds-art-33).

| Member | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Whether the delay applies. Off, the person's access is immediate (Art. 3(1)), every hold already placed stops withholding, and the admin route refuses a new hold with **400**. |
| `categories` | list of string | `[]` | The categories held at creation, named as the [audit category register](config-audit.md#the-audit-category-register) names one: a priority category of Art. 14(1) by its code (`Laboratory-Reports`, `Medical-Imaging`, …) or as `<system>\|<code>`, or a national category the map declares. A name the map does not hold, and `none`, refuse the write. Empty holds nothing at creation; a professional still holds a composition by hand. |
| `max_delay` | duration | required while enabled | The limit of a hold, a whole number of hours from `"1h"` to `"8784h"` (one year): new compositions of the listed categories are held from patient-context requests until a professional releases them or it passes. It is the length of a hold placed at creation and the latest end a professional may give one. Art. 3(3) allows the delay "for a limited period of time" only; the period is the Member State's to set. |

```json
{
  "enabled": true,
  "categories": ["Laboratory-Reports", "Medical-Imaging"],
  "max_delay": "7d"
}
```

## `[auth.smart]`

The SMART on openEHR scopes are enforced on **every route** whenever
`[auth]` is on: each operation carries a requirement from FerroEHR's route
table, which the `ferrohealth-smart` crate checks against the token's scopes
or a Basic user's `scopes`. There is no switch for it. This section decides
what beside the route table: whether and where the discovery document is
served. A `patient/` grant of any trusted issuer counts, confined to the EHR
its token's `ehrId` names. See [SMART App Launch](../smart-app-launch.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `discovery` | bool | `false` | Serve `/.well-known/smart-configuration`. It is built from the metadata of one `[auth]` issuer, the [described issuer](#the-described-issuer), so it needs an issuer whose metadata names a `token_endpoint` (SMART App Launch 2.2.0, Conformance, §Metadata), and [`server.public_base_url`](config-server.md#public_base_url-the-one-public-url) for its absolute `services.*.baseUrl` values. Until the described issuer's metadata is read, the route answers `503` with `Retry-After` and the reason `issuer-unread`; metadata that names no `token_endpoint` answers `503` with `issuer-metadata-incomplete`. |
| `issuer_metadata_file` | path | unset | A JSON file holding the RFC 8414 §2 metadata of the `[auth]` issuer it names, whose key set is a `jwks_file`, read at boot. It must name such an issuer, repeat no member name, and every endpoint it names must be `https` (a loopback host excepted); the discovery document takes its `token_endpoint` and the other endpoints from it. Naming any other issuer is a boot error. |
| `platform_base_url` | string | unset ⇒ the REST root | The path the discovery document hangs off, e.g. `/gateway/v1`. |
| `episode_launch` | bool | `false` | Advertise episode context, accept the `launch/episode` scope and narrow each request of a launch to the token's `episodeId` claim, read as `issuer\|id`. |

The `launch-base64-json` capability (master09) is advertised by listing it in
`[auth] capabilities`; the base64 JSON launch object is the application's to
consume. `discovery` alone turns the document on.

Which clients count as backend clients and which reach the DEMOGRAPHIC API is
policy the CDR holds, never a key here: the `query_system_wide` and
`access_demographic` grants of the client's `AGENT`
([Operational policy](../policy-examples.md#a-backend-client-and-a-demographic-client)).

No `smart` URL may carry a user name or password, whether or not the document
is served: the discovery document publishes these URLs to every application,
so a credential in one would be served to anyone who asks.

### The described issuer

The discovery document describes one authorization server: SMART App Launch
2.2.0, Conformance, §Metadata requires its `authorization_endpoint` whenever
the document advertises `launch-standalone` or `launch-ehr`, so it is where
every user launch starts. FerroEHR describes an issuer that serves a user
launch and whose metadata it has:

- **A SMART App Launch issuer:** eligible. A practitioner's launch and a
  patient's standalone launch both start at the issuer's login.
- **`federation`:** eligible. Another party's professionals sign in at it, and
  an agreement admits them.

An issuer has metadata when it is introspected, when its metadata is read from
it (in the background, so the document waits for that read), or when
`issuer_metadata_file` supplies it. An issuer read from a `jwks_file` alone
has none.

With `discovery = true`, the start is refused unless exactly one issuer is
described:

- `[auth] described_issuer` set: it must name the table of an eligible
  issuer, else the start is refused, naming the key and why.
- `[auth] described_issuer` unset: the one eligible issuer is described. With
  more than one, the start is refused, naming them and `described_issuer`.
  With none, the start is refused, saying no issuer can be described.

With `discovery = false`, no document is described and `described_issuer` is
not read.

The names of the `[auth.issuer.<name>]` tables never decide. SMART does not say
which of several trusted issuers a resource server describes. The rule is the
FerroHEALTH family's own design, shared by every product through
`ferrohealth-smart`, and `described_issuer` is the same `[auth]` key in each.

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
max_duration = "30s"
max_frequency = 999
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Mount the management router. |
| `base_path` | string | `/management` | Base path for the management endpoints. |
| `port` | int | unset ⇒ share the main listener | Serve management on its own listener and port. Must differ from the `server.listen` port, and it is plain HTTP, an internal surface. |
| `flame_file` | path | unset | Span-timing capture: write the folded stacks of every span to this file, and render offline with `inferno-flamegraph`. Unset, the layer is not installed. For diagnostic sessions: the file grows with span traffic. |

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
| `max_duration` | duration | `"30s"` | Longest sample window one request may ask for, in whole seconds. A request asking for more is refused with `400`, never clamped. |
| `max_frequency` | int | `999` | Highest sampling frequency (Hz) a request may ask for. Same refusal semantics. |

> [!WARNING]
> `probes_enabled` and `endpoints.health` do not exist. Configuration is
> strict, so a file or environment variable still setting either one fails at
> boot with an unknown-key error: delete the key; the probes are always on.

## `[deployment]`

The commercial licence token. Every build embeds the licensor's
`non-commercial` grant; a deployment that holds a `commercial` licence installs
its token here. The server behaves identically under either grant and reports
the one in force on `GET /management/status` (`licence.use`, `licence.licensee`,
`licence.not_after`, `licence.configured_token`). See
[Licensing & legal](../licensing.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `licence_file` | path | unset | The licence token the licensor issued. A token that cannot be read or does not verify is reported as `refused` on `/management/status`; the embedded grant stays in force. |

## `[record.signing]`

VERSION signing. On by default in `digest` mode, with read-time verification of
the server's own signatures **strict** by default.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Server-side signing of committed versions. |
| `mode` | enum{digest,pgp} | `digest` | A SHA-256 integrity digest, or an OpenPGP (RFC 9580) detached signature, ML-DSA-65+Ed25519 (RFC 9980). |
| `key_file` | path | unset | Armored ML-DSA-65+Ed25519 secret key; **required for `pgp`** (a boot error otherwise). |
| `key_passphrase` / `key_passphrase_file` | secret / path | unset | Key passphrase. At most one of the pair. |
| `retired_key_files` | list of path | `[]` | Armored **public** keys retired from signing and kept for verification, so versions signed before a key rotation keep verifying. |
| `verify_on_read` | enum{off,warn,strict} | unset ⇒ `strict` when signing is enabled | Read-time recompute-and-compare policy for the server's own signatures. |

`retired_key_files` exists because a stored `VERSION.signature` records no key
identifier and is an immutable committed fact that cannot be re-issued;
keeping the retired public key is the only way history stays verifiable across
a rotation, and a public key can verify but never sign. Its environment form
takes a comma-separated list
(`FERROEHR__RECORD__SIGNING__RETIRED_KEY_FILES=/keys/a.pub.asc,/keys/b.pub.asc`).

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

### Sign with an ML-DSA-65+Ed25519 key

`pgp` mode signs with an ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30), a
composite post-quantum signature that verifies only when both its Ed25519 and
its ML-DSA-65 half verify. Generate one with `ferrohealth-sop`, the Stateless
OpenPGP command line of the `ferrohealth-openpgp-pq` crate
(`cargo install ferrohealth-openpgp-pq --version =0.1.0 --locked`):

```shell
ferrohealth-sop generate-key --signing-only --with-key-password=pgp-pass \
  "FerroEHR records <records@example.org>" > signing.asc
```

A **classical** key (Ed25519, EdDSA, RSA, ECDSA or Ed448) is refused at boot,
naming its fingerprint. To move off one:

1. Generate an ML-DSA-65+Ed25519 key as above and point `key_file` at it.
2. Export the **public** half of the old certificate and add it to
   `retired_key_files`, so versions signed with it still verify.
3. Restart. New versions are signed with ML-DSA-65+Ed25519; old ones keep
   verifying, because verification uses only public values.

**On Kubernetes**, `digest` mode needs nothing; it is the default. `pgp` mode
needs the key as a file and its passphrase as a secret, both of which the chart
mounts for you:

```yaml
# values.yaml
config:
  signing:
    enabled: true
    mode: pgp
    key_file: /etc/ferroehr/signing-key.asc
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

## `[record.versioning]`

What a commit stores is not a setting: a version whose content equals the
current version's is committed like any other, with its own commit audit,
contribution, version id and `ETag`, and the content is stored once (the new
version reads its predecessor's node rows). The one setting here is the
answer to a repeated send of the same content.

| Key | Type | Default | Description |
|---|---|---|---|
| `record.versioning.idempotent_resend` | bool | `false` | When `true`, a `PUT` whose content equals the current version's commits nothing and answers like a successful update with the existing version: `200` (or `204` under `Prefer: return=minimal`), and that version's `ETag` and `Location`. |

Turn it on only for the integration that needs it: with the rule on, the
repeated send leaves no commit audit and no contribution, so the version
history does not record that the content was confirmed again, where the
default records every committal as the RM asks (RM common `master06`
§Committal and Audits). What the repeated send does leave is one record on
the access log (`[audit]`), operation `resend`, naming the version the
content matched and its content digest (`<version uid>#sha256:<digest>`),
the committer and the time: the entry Cadasto writes for a skipped send,
which says the last version was saved one more time. The content is matched
against the current version only, the one `If-Match` names; content equal to
an earlier version's is committed as a new version. The openEHR REST API
does not govern skipping an identical commit; the answer is FerroEHR's own,
chosen to match what a client sees from Cadasto.

```toml
[record.versioning]
idempotent_resend = true
```
