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
checked by the `ferrohealth-smart` crate (0.9). FerroEHR is a resource server:
it checks tokens and never issues them. Every person a request comes from is
bound to a demographic `PERSON` (see [`[authz.party_binding]`](#authzparty_binding)).

```toml
[auth]
enabled = true

[[auth.issuer]]
issuer = "https://keycloak.example.com/realms/staff"
kind = "persons"
profile = "keycloak"
audience = ["ferroehr"]
first_party_clients = ["ferroehr-viewer"]

[[auth.party]]
issuer = "https://keycloak.example.com/realms/staff"
identifier_issuer = "urn:example:staff-login"
identifier_type = "uuid"

[[auth.basic.users]]
user = "clinician"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$…$…"   # never a plaintext password
scopes = ["user/composition-*.cruds", "user/template-*.cruds", "user/aql-*.cruds"]
```

The section keys:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch. `false` = all requests pass unauthenticated (development only). With `true` and **no mechanism configured the server refuses to start**; see below. |
| `issuer` | `[[auth.issuer]]` tables | none | The trusted issuers, one table each; see below. The flat `issuer = "…"` form is refused at boot, naming the table form. |
| `ca_file` | path | unset | A PEM bundle of certificate authorities trusted in addition to the platform roots, on every platform, for an issuer behind a private PKI: every issuer's metadata and key set are read trusting it, and under `mode = "introspection"` the introspection and token calls too. A file that cannot be read, holds no certificate, or does not parse refuses the start. |
| `realm` | string | `""` ⇒ `ferroehr` | The realm every `WWW-Authenticate` challenge names. |
| `scopeless_subjects` | list of `{ issuer, sub }` | `[]` | The subjects whose token, when it carries no resource scope of either the SMART on openEHR or the SMART on FHIR grammar, meets every SMART on openEHR requirement. Only a system (app-only) caller of that issuer is admitted by an entry. A token that carries resource scopes gets only its scopes, and the allowlist reaches neither the ADMIN API nor the DEMOGRAPHIC API, which need the product scope. A string entry refuses the start. |
| `capabilities` | list of string | `[]` | SMART capabilities the discovery document adds: one SMART App Launch or the SMART on openEHR draft lists, or a full URI. |
| `scopes_supported` | list of string | `[]` | The scopes the discovery document lists. Empty, it lists the scopes the route table honours. |
| `product_scope` | string | `""` ⇒ `__ferroehr.platform` | FerroEHR's own scope. It admits the ADMIN API and, beside the bearer clients in `[smart] demographic_clients`, the DEMOGRAPHIC API, the two surfaces no SMART on openEHR resource family covers. A full URI or a string starting with `__` (SMART App Launch 2.2.0, Scopes and Launch Context, §Extensibility). |

The keys `audience`, `algorithms`, `token_profile`, `clock_skew_seconds`,
`jwks_max_age_seconds`, `jwks_refresh_interval_seconds`, `jwks_file`,
`timeout_ms`, `mode`, `introspection`, `role_claims`, `acting_for_claim` and
`client_tokens` are refused at the section level, each naming what replaces
it: the first ones live on each `[[auth.issuer]]` table, and the caller
classification replaces the last two.

### `[[auth.issuer]]`: the trusted issuers

Each issuer's metadata is read at boot, at the RFC 8414 §3 location first and
the OpenID Connect Discovery 1.0 §4 location after a `404`, and with it the key
set at its `jwks_uri`, so an authorization server that cannot be reached
refuses the start. Every setting of an issuer lives on its own table. No two
issuers of one node share an audience, so one audience is one issuer and one
token form; the node's audience must also differ from every other node's, an
operator rule boot cannot see.

| Key | Type | Default | Description |
|---|---|---|---|
| `issuer` | string | required | The issuer identifier: an `https` URL with no query, no fragment and no user name or password; plain `http` on a loopback host only. |
| `kind` | `persons`, `patients`, `machines` or `federation` | **required** | Whose tokens it signs. `persons` signs the deployment's own people (and the app-only tokens of its machines), `patients` tokens naming only the record subject, `machines` client credentials tokens only, and `federation` another party's professionals or organisation. One issuer is one kind, so patients and professionals sign in at separate issuers, a realm or tenant each. |
| `profile` | `rfc9068`, `entra`, `keycloak`, `okta` or `nl` | `rfc9068` | The preset of what the issuer's tokens look like, each value labelled with its source in the crate. Every profile key below may be set on the table over it. |
| `audience` | list of string | **required** | Accepted `aud` values; a token naming none of them is refused (RFC 9068 §4, RFC 8725 §3.9). |
| `algorithms` | list of string | `["RS256","RS384","PS256","PS384","ES256","ES384"]` | Accepted signature algorithms. Asymmetric only: an `HS*` algorithm is a boot error, and `none` is no algorithm at all (RFC 8725 §3.1, §3.2). |
| `clock_skew_seconds` | int | `60` | Leeway on `exp`, `nbf` and `iat`. At most `300`. |
| `jwks_max_age_seconds` | int | `600` | A key set older than this is read again. |
| `jwks_refresh_interval_seconds` | int | `60` | A token naming an unknown `kid` reads the key set again at most once per interval. |
| `jwks_file` | path | `""` | A JWK Set of public keys handed over out of band, read instead of the issuer's metadata. A symmetric key (`kty: "oct"`) or a private-key member (`d`, `p`, `q`, `dp`, `dq`, `qi`, `oth`, `k`) is a boot error. An issuer read from a file has no metadata, so it serves the SMART discovery document only with [`[smart] issuer_metadata_file`](#smart) set. |
| `timeout_ms` | int | `10000` | One call to the authorization server. |
| `mode` | `jwt` or `introspection` | `jwt` | How the issuer's tokens are checked: as JWTs against its key set, or posted to its RFC 7662 endpoint (see [Opaque tokens](#opaque-tokens-by-introspection)). |
| `role_claims` | list of string | `["roles","groups","entitlements"]` | Token claim paths the caller's roles are read from; dotted paths walk nested claims, and a role is matched without regard to case. **`scope` is not a role source**. A `federation` issuer reads no role. Whether a role grants `admin_role` and the other privileged roles is [`[authz.rbac] role_issuers`](#authzrbac). |
| `sender_constraint` | `none` or `required` | `none` | `required` admits only a token bound to a DPoP key (RFC 9449) or to the client certificate of a mutual-TLS connection (RFC 8705 §3). A token carrying `cnf` is held to it under either value. |
| `assurance` | table | unset | The assurance level the issuer's tokens carry, in the shape of [`[authz.assurance]`](#authzassurance-the-assurance-level-patient-data-requires). |
| `person_key` | string | `sub` (preset) | The claim a person is bound by. `email`, `phone_number`, `preferred_username` and `name` are never accepted as a key (OpenID Connect Core 1.0 §5.7). |
| `first_party_clients` | list of string | `[]` | The deployment's own clients. **A `persons` token is admitted only through a client listed here or in `delegates`.** |
| `delegates` | list of `{ client }` or `{ sub, issuer }` | `[]` | Third-party clients or RFC 8693 actors that act for a person; their tokens are recorded as the client acting for the person the token names. |
| `token_types`, `required_claims`, `scope_claim`, `app_only_scope_claim` | | preset | The token shape that replaces `token_profile`: the accepted `typ` values, the claims every token carries, and where its scopes are read. For an issuer that minted under `token_profile = "jwt"`, set `token_types = ["JWT"]` and the `required_claims` its tokens carry. |
| `client_claims`, `app_only`, `guest`, `impersonation_marker`, `expect`, `user_claims`, `person_equality`, `unstable_claims`, `max_actor_chain`, `representation`, `tenant_claim`, `adapter`, `onward_*` | | preset | The other profile keys of `ferrohealth-smart`: which claim names the client, what marks an app-only token or a guest, what refuses an impersonation, the claims a token must carry exactly, how a representative names whom it acts for, the claim naming the tenant, the vendor adapter (`idp-entra`) and the onward grants. |

### `[[auth.party]]`, `[[auth.federation]]`, `[auth.dpop]` and `[auth.idp.entra]`

| Table | Description |
|---|---|
| `[[auth.party]]` | One per `persons` issuer, keyed by `issuer`: where its logins are looked up. `identifier_issuer` and `identifier_type` (`guid` and `uuid` compare in lower case) name the identifiers the issuer profile's `person_key` is matched against; `group_claim` with `group_identifier_issuer` reads the groups a token asserts; `guests` looks a guest up. **Boot refuses a `persons` issuer without one**, a table that sets `claim` (the profile's `person_key` is the key), and two tables sharing an issuer or an `identifier_issuer`. |
| `[[auth.federation]]` | One per `federation` issuer: how its tokens' claims map to the fields of another party's professional or organisation (`mapping = "iua"`, `"ehdsi"` or per field), the countries, issuing authorities and provider prefixes that bound them, and `contact_point` for a MyHealth@EU contact point. `fields` maps a field to its claim (`{ claim = "..." }`) or `authorization_details` member, among them `consent.reference`, the consent decision reference of a consent-conditioned read, and `correlation_id`, the requester's id a `chained` organisation request carries. |
| `[auth.dpop]` | The DPoP proof check of a sender-constrained issuer: `external_url` (required, the base URL clients address; a proof's `htu` is it plus the request path), `max_age_seconds` (`60`), `clock_skew_seconds` (`10`) and `algorithms`. Each accepted proof is recorded in the database every replica shares until its window ends, so a proof is accepted once across all of them (RFC 9449 §11.1), and a database that cannot record it refuses the request with `503`. |
| `[auth.idp.entra]` | The Microsoft Graph access of the `idp-entra` adapter, which an issuer turns on with `adapter = "idp-entra"` to read the groups of an Entra ID groups overage: `base_url` (`https://graph.microsoft.com/v1.0`), `timeout_ms` (`2000`), `max_response_bytes`, `security_enabled_only` and the client credentials of `[auth.idp.entra.credentials]`. An issuer naming the adapter without this table refuses the start. |

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

### Opaque tokens by introspection

An authorization server that issues opaque access tokens is served with
`mode = "introspection"` on its `[[auth.issuer]]` table, whose
`[auth.issuer.introspection]` table holds the keys below. Each bearer token is posted to the issuer's
introspection endpoint (RFC 7662 §2.1), and the active response stands in for
the JWT claims: `sub`, `scope`, the roles of `role_claims`, `aud` against
`audience` and `iss` against the issuer. An inactive token is a `401`. An
endpoint that cannot answer is a `503`, because the server cannot decide. An
active response is reused until its `exp`, at most `cache_max_age_seconds`.

| Key | Type | Default | Description |
|---|---|---|---|
| `endpoint` | URL | unset | The introspection endpoint; unset reads the `introspection_endpoint` of the issuer's metadata. `https`, or `http` on a loopback host. |
| `cache_max_age_seconds` | int | `60` | The longest an active response is used; `0` caches nothing. |
| `cache_capacity` | int | `10000` | The most responses held at once. |
| `client` | table | unset | The client authentication of the call (RFC 7662 §2.1), with the keys of an outbound grant: `client_id` and `client_secret`, `private_key_jwt` or mutual TLS (see [the grant keys](config-integrations.md#authenticating-to-the-terminology-server)). |
| `bearer` | table | unset | A client credentials grant whose token authorizes the call instead (SMART App Launch 2.2.0, Token Introspection). Exactly one of `client` and `bearer`. |

```toml
[[auth.issuer]]
issuer = "https://idp.example.org/realms/ferroehr"
kind = "persons"
audience = ["ferroehr"]
mode = "introspection"

[auth.issuer.introspection.client]
client_id = "ferroehr-rs"
client_secret_file = "/run/secrets/introspection-client"
```

### The Basic-auth user store

`[[auth.basic.users]]` is an array of tables. A Basic user is always a
person: it maps to the `PERSON` carrying its binding key through
[`[[authz.party_binding.basic_user]]`](#authzparty_binding), and a Basic user
with no mapping refuses the start, naming the user. A machine authenticates
with the client credentials grant (RFC 6749 §4.4), and a `system/` scope on a
Basic user refuses the start.

| Key | Type | Default | Description |
|---|---|---|---|
| `user` | string | required | The user-id. Empty, or with a colon or a control character, is a boot error (RFC 7617 §2). |
| `password_hash` | secret | required | Argon2**id** PHC hash (`$argon2id$v=19$…`), never a plaintext password. Boot-validated against the OWASP floor; see below. |
| `password_hash_file` | path | unset | Read the hash from a file instead, for a mounted secret. A hash is an offline cracking target, so prefer this wherever the configuration file itself is not treated as sensitive. Exactly one of the pair is required. |
| `scopes` | list of string | `[]` | The SMART on openEHR scopes the user holds, checked on every route exactly like a token's `scope` claim. A user with none is refused everywhere a requirement applies. |
| `roles` | list of string | `[]` | The user's RBAC roles, upper-cased on authentication. A user with none holds no role and is refused wherever [`[authz.rbac]`](#authzrbac) asks for one. The former `[authz.rbac.basic_roles]` table is refused at boot. |

`[auth.basic]` itself takes one key beside the users:

| Key | Type | Default | Description |
|---|---|---|---|
| `cache_ttl_seconds` | int | `300` | How long a credential that verified is kept, so a repeat request skips the Argon2id derivation; `0` turns the cache off and checks every request. The entry is named by a keyed hash of `user-id:password` under a key drawn at boot, never the password, and only a credential that verified is kept, so a wrong password always pays the derivation. The cache holds at most 4096 credentials and empties at a restart. The TTL bounds how long a removed user keeps working, which matters only until the restart that applies the new user list. Argon2id is slow by design, so with the cache off its cost bounds how many Basic requests a core can answer. |

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

The access record carries the acting mode and the professional the caller
classification names; see [Audit trail](../audit.md#the-ehds-logging-elements-mapped).
`[authz.professional]` is refused at boot: the issuer's `kind` and profile
classify the caller, and a client acting for a person is one the issuer lists
in `delegates`.

### `[authz.party_binding]`

How a person is attributed. There is no switch: binding is how FerroEHR
attributes a person, and `enabled` is refused at boot.

| Key | Type | Default | Description |
|---|---|---|---|
| `provider` | `{ issuer, id }`, or `[[authz.party_binding.provider]]` tables | **required** once `[auth]` admits a credential | The healthcare provider this deployment acts for, as an identifier issuer and id (in the Netherlands the URA). Every access record of a bound or provisional person and every version a person commits name it (EHDS Art. 13(4), Annex II 3.2(a)). A deployment that serves several providers declares one table per value of its issuers' `tenant_claim` (`tenant`, `issuer`, `id`) instead; the two forms cannot be combined, and a bound issuer that names no `tenant_claim` is refused beside the table form. |
| `namespace` | string | `demographic` | The `PARTY_REF.namespace` a bound party is written under, the value of the ITS-REST overview example (`committer.external_ref.namespace="demographic"`), held to the `OBJECT_REF.namespace` pattern `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*`. |
| `basic_user` | `[[authz.party_binding.basic_user]]` tables | `[]` | One per Basic user: `user`, `key_issuer` (the `identifier_issuer` of an `[[auth.party]]` table) and `key`, the binding key the user's `PERSON` carries. A Basic user with no table, a table naming no Basic user and a `key_issuer` no table names refuse the start. |

```toml
[authz.party_binding]
provider = { issuer = "urn:oid:2.16.528.1.1007.3.3", id = "12345678" }

[[authz.party_binding.basic_user]]
user = "clinician"
key_issuer = "urn:example:staff-login"
key = "3f2b0c1e-0000-4000-8000-000000000001"
```

#### The party mapping

SCIM provisioning, just-in-time creation and the login binding read and
write the same demographic parties under one mapping. It names the
templates each party is valid against, the identifier selectors of the
binding key, `userName` and `externalId`, and where each SCIM attribute and
OpenID Connect claim goes. Names go in `PARTY.identities` and identifiers
in `PARTY.details` (RM demographic §Names and Addresses, §Party
Identification), and boot refuses a name mapped into `details` or an
identifier mapped into `identities`. No openEHR specification governs this
mapping: it is FerroEHR's own design, and its targets are RM demographic
paths.

| Section | Keys | Description |
|---|---|---|
| `[[authz.party_binding.person.template]]` | `template_id`, `key`, `user_name`, `external_id`, `default` | One per template a professional's `PERSON` may use, so a migrated template can stay beside a new one. `key`, `user_name` and `external_id` are selectors, each `{ path, type }`: the path of the `ELEMENT` holding the `DV_IDENTIFIER` and its `type`. Every identifier can sit at one path (the repeated element of an identifier cluster), so two selectors at one path are told apart by `type`, and the key by its issuer, the `identifier_issuer` of an `[[auth.party]]` table. Exactly one table sets `default = true`: SCIM and just-in-time creation create new persons with it. |
| `[[authz.party_binding.person.attribute]]` | `scim`, `oidc`, `jit`, `name`, `path` | One per mapped value: a SCIM attribute path (the RFC 7644 `valuePath` form, `emails[type eq "work"].value`, and the FerroEHR extension `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier` that feeds the binding key), an OpenID Connect claim, whether just-in-time creation writes it (`jit`, default `false`), whether it is the name the committer reads (`name`, at most one), and `path`, one path per template id. `id`, `schemas`, `meta`, `active`, `userName` and `externalId` are SCIM's own and are never mapped. |
| `[authz.party_binding.professional_role]` | `template_id` | The template of the professional `ROLE`, rooted at `ROLE`. A `PERSON` is a professional while it lists a `ROLE` of this template that names it as `performer`, is `complete` or `incomplete`, and whose `time_validity` holds today. |
| `[authz.party_binding.group]` | `template_id`, `external_id`, `display_name` | The template of a SCIM Group's `GROUP`, its `externalId` selector and the path of its `displayName`. |
| `[authz.party_binding.membership]` | `archetype_id` | The `PARTY_RELATIONSHIP` archetype of a group membership. |
| `[authz.party_binding.representation]` | `archetype_id`, `relationship_codes` | The `PARTY_RELATIONSHIP` archetype a representative acts through, from the representative's `PERSON` to the subject's, and the code of the openEHR `subject_relationship` group each of its relationship types maps to. |

A worked example over the `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2`
cluster, with the Microsoft Entra object id as the binding key:

```toml
[[authz.party_binding.person.template]]
template_id = "FerroEHR professional"
default = true
key = { path = "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2]/items[at0001]", type = "guid" }
user_name = { path = "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2]/items[at0001]", type = "scim_user_name" }
external_id = { path = "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2]/items[at0001]", type = "scim_external_id" }

[[authz.party_binding.person.attribute]]
scim = "name.formatted"
oidc = "name"
jit = true
name = true
path = { "FerroEHR professional" = "/identities[openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v1]/details[at0001]/items[at0002]" }

[authz.party_binding.professional_role]
template_id = "FerroEHR professional role"

[authz.party_binding.group]
template_id = "FerroEHR care team"
external_id = { path = "/details[at0001]/items[at0001]", type = "scim_external_id" }
display_name = "/identities[at0002]/details[at0003]/items[at0004]"

[authz.party_binding.membership]
archetype_id = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1"

[authz.party_binding.representation]
archetype_id = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1"
relationship_codes = { at0010 = "254", at0011 = "264" }
```

With Microsoft Entra ID, map the object id to `externalId` (Entra's
matching) and to the binding-key attribute (the binding): two attributes,
two selectors. The key's `[[auth.party]] identifier_type` is `guid`, so a
key differing only in case binds one login.

Boot refuses, naming the key: no or two `default` tables; a selector path
that does not compile; two equal selectors; a key whose scheme
`[demographic.identifier_protection]` protects; any selector under the Dutch
BSN namespace (OID `2.16.840.1.113883.2.4.6.3`); a key fed by an email
address, a phone number, `preferred_username`, `name`, `upn` or
`unique_name`; a template that is absent, rooted at another class or lacks
a mapped path; a path whose type cannot carry its source; and a
representation type with no `subject_relationship` code. Deleting a template
the mapping names is refused with `409`.

The parties the binding relies on are protected: a `PERSON` of a mapped
template carrying a selector identifier, its professional `ROLE`, a
representation relationship, and a `GROUP` of the group template. Only an
administrator (`authz.rbac.admin_role`) writes one through the demographic
API, including the creation of a party that carries a selector identifier.
An administrator's version that leaves a protected identifier out is refused
with `422`. A present binding key is immutable (RFC 7643 §2.2): a version
that changes it is refused with `403`, and
[`POST /admin/party/binding-key`](../operations-admin-apis.md) is the one
change. Each selector value binds one live party: a commit that gives a
binding key, a `userName` (compared after RFC 8265 width mapping, lower case
and NFC), an `externalId` or a group `displayName` a second party is refused
with `409`. A deleted `PERSON` leaves its key as a tombstone that refuses
the login until an administrator binds the key again.

#### Changing the mapping

The side table is filled from the configured mapping and the party
versions. The first boot fills it and stores the mapping's digest; a replica
whose mapping digest differs from the stored one refuses to start, naming
both digests. Each running replica holds a lease it renews every 20 seconds.
A mapping change is a stop-the-world step:

1. Stop every replica. With Helm, scale the deployment to zero first: a
   rolling upgrade would start new replicas beside old ones.
2. Run `ferroehr party-binding refill` with the new configuration. It
   refuses while a replica's lease was renewed within the last minute, and
   refuses a store in which two parties carry one selector value, naming
   both.
3. Start the replicas with the new mapping. A replica still holding the old
   mapping refuses to start.

#### How a login is looked up

Every request of a person is looked up in the binding side table of the
party store, on the party database role, with no cache, so a change to a
party takes effect on the next request on every instance. The table holds
the binding key of each `PERSON` of a mapped template that performs a
professional `ROLE` (a `ROLE` of the mapped role template that its `roles`
list, that names it as `performer`, whose state is `complete` or
`incomplete` and whose `time_validity` holds today's date in UTC) or that
is the source of a representation relationship. A key of type `guid` or
`uuid` is compared in lower case. The table is written in each party
commit's transaction and can be rebuilt from the party versions alone.

The request is refused with `403` and the binding code in the problem body
when the login matches two parties (`refused-ambiguous`), when its party is
`inactive` (`refused-inactive`) or `abandoned` (`refused-abandoned`), when
its key belongs to a deleted party (`refused-deprovisioned`), when the token
carries no key claim (`refused-key-claim-missing`), when the party lacks the
marker the caller's kind needs (`refused-wrong-marker`), when the groups a
token asserts cannot be read (`refused-groups-unavailable`), when a guest
comes from an issuer whose table does not set `guests` (`refused-guest`), and
when a representative's token names a subject its party has no live
representation relationship to (`refused-representation-unrecorded`). A party
store or an identity provider that cannot answer is answered `503`, with
`Retry-After` when the identity provider gave one.

A representative reaches only the EHR whose `EHR_STATUS.subject.external_ref`
names the subject it represents: the issuer's `representation` claim carries
the subject's party id, matched without regard to case, and the subject must
be the target of one of the representative's live relationships, so a
representative of an issuer no `[[auth.party]]` table binds is refused. Every
composition, `EHR_STATUS`, directory, contribution and query request is held
to that EHR, whatever the token's scopes cover; a query must name it with
`ehr_id`. Templates stay readable. A `persons` token whose only clinical scopes
are `patient/` is a professional confined to its launch EHR, and its commits
record the professional, never `PARTY_SELF`.

### `[authz.import]` and `[[authz.system_client]]`

`[authz.import] trusted_systems` lists the `creating_system_id` values whose
wrapped committers may name a local party on import; this node's own
`[rest] system_id` is refused.

Each OAuth client-credentials client that commits is declared in one
`[[authz.system_client]]` table, keyed by `issuer` (a `machines` or `persons`
issuer) and `client_id`. A Basic user is never a system client. A system
caller with no table reads under its `system/` scopes and commits nothing into
an EHR (`403`); a declared one commits a COMPOSITION only of a template its
`writes` lists, at a facility its `facilities` lists, as the
[caller kinds](../identity-providers.md#who-the-caller-is) describe.

| Key | Type | Default | Description |
|---|---|---|---|
| `issuer`, `client_id` | string | required | The client, as its tokens name it. A duplicate pair refuses the start. |
| `provider` | `{ issuer, id }` | required | The healthcare provider it acts for, recorded on every system access (EHDS Annex II 3.2(a)). |
| `facilities` | list of `{ issuer, id }` or `{ issuer, id_prefix }` | `[]` | The facility identifiers it may commit for. |
| `writes` | list of template ids | `[]` | The templates it may commit; a template the store does not hold refuses the start. |
| `origins` | list of `relay`, `device` | required | `relay`: it relays another system's record of a human act, such as HL7 v2. `device`: a device registers with no human acting, which needs `device_identifier_issuers`. |
| `device_identifier_issuers` | list of string | `[]` | The issuers of the device identifiers a `device` client names. |

### `[[authz.federation.agreement]]`

What a `federation` issuer's verified external persons, or its organisation,
may do here. Each table is keyed by `issuer` and `accessor` and admits only its
own accessor kind; every agreement is listed on the boot line and in
`GET /management/info`, and one that runs without a named person is logged at
`warn` on every boot.

| Key | Type | Default | Description |
|---|---|---|---|
| `issuer` | string | required | The `federation` issuer. |
| `accessor` | `person` or `organisation` | required | Named persons, or the organisation without a named person. A duplicate (issuer, accessor) pair refuses the start. |
| `name`, `version` | string | `""` | The agreement; required for an organisation. |
| `jurisdiction` | ISO 3166-1 alpha-2 | `""` | An issuer with `profile = "nl"` implies `NL`. |
| `exchange` | string | `""` | The kind of exchange the agreement covers. |
| `legal_ground` | `{ gdpr_6_1, gdpr_9_2, national }` | unset | The GDPR Art. 6(1) point (`a` to `f`), the Art. 9(2) point (`a` to `j`) and the national provision; in the Netherlands Art. 9(2)(h) with UAVG Art. 30(3)(a). Required for an organisation. |
| `consent` | `mitz` or `local` | unset | The source of the consent the exchange is conditioned on, never its legal ground. A read under it needs the consent decision reference the token carries in the field its `[[auth.federation]]` table maps to `consent.reference`, and is refused with `403` without it. Required for every NL agreement with `reads` (Wabvpz Art. 15a(1)); an organisation may not take it from Mitz, whose closed question needs a responsible professional. |
| `reads` | list of `patient/` scopes | `[]` | The most an external caller may read; a `user/` or `system/` scope refuses the start. |
| `writes` | list of template ids | `[]` | The templates an external person may commit; an organisation agreement takes none (EHDS Art. 13(4)), and a template the store does not hold refuses the start. |
| `discard`, `discard_window_hours` | bool, int | `false`, `24` | Whether an external person may discard a dispensation of the same point of care within the window (MyHealth@EU Requirements Catalogue 10.0.0, requirement 07.04). |
| `caller_rate` | `{ requests_per_second, burst }` | **required** | The admission rate per caller, keyed on the token's subject and kept per replica: each caller gets `burst` requests at once and `requests_per_second` after that, and a request above it is answered `429` with `Retry-After` before any patient lookup, a contact point's included, and counted into the caller's [summary record](../audit.md#refusal-records-and-their-budgets). |
| `opt_out_override` | bool | `false` | Whether a professional may read an opted-out EHR under national vital-interest access (EHDS Art. 10(2)); never on an organisation agreement. |
| `access_class` | `automated` or `chained` | unset | For an organisation, required: no natural person views the data, or the requester logs the person and every request carries its correlation id as an attested claim. A `chained` agreement needs its issuer's `[[auth.federation]]` table to map `correlation_id` from a claim or an attested `authorization_details` member (a constant refuses the start); a request without the id is refused with `403` and recorded as `refused-correlation-missing`, a request header never stands in for it, and the access record carries the id with its source. |
| `chained_log_retention_years` | int | unset | For `chained`, required: how long the requester keeps its log, at least `[audit.store]`'s retention. |

An organisation agreement on a MyHealth@EU `contact_point` issuer refuses the
start.

### `[authz.rbac]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | The coarse role gate (active when authentication is enabled). Off, every authenticated caller may use every enabled surface, reported as the `rbac_off` deployment gap. `ehr_access_default` and `admin_role` apply either way. |
| `admin_role` | string | `ADMIN` | Role required for admin-class operations. A blank value is a boot error. |
| `readonly_role` | string | `READONLY` | Role marking a principal read-only: refused on every write operation (create/update/delete/upload), even alongside granting roles. Reads and AQL queries are still allowed. |
| `emergency_access_role` | string | unset | Role that reads data under an EHDS Art. 8 access restriction when the caller also declares one of `[audit] emergency_purpose_codes` (EHDS Art. 11(5)); the access record then carries the emergency mark. Unset, nobody may override a restriction. See [restriction of access](../security.md#restriction-of-access-by-health-professionals-ehds-art-8). |
| `access_service_roles` | list of strings | `[]` | Roles of the EHDS Art. 10 access-service callers: the Art. 4 access service of persons and the Art. 12 health professional access service. A caller holding one reads an EHR whose person opted out as absent, unless it declares one of `[audit] emergency_purpose_codes` and does not act for the person (Art. 10(2)). Empty, the opt-out holds for nobody and recording one is refused. Environment: `FERROEHR__AUTHZ__RBAC__ACCESS_SERVICE_ROLES`, comma-separated. See [opt-out from the access services](../security.md#opt-out-from-the-access-services-ehds-art-10). |
| `identifier_reader_role` | string | unset | Role that may read a party's protected national identifiers through `GET /ferroehr/rest/extensions/v1/admin/party/identifiers`, beside the admin role. Every read is an access record naming the reader, the party and the scheme. Unset, nobody may. See [reading a party's protected identifiers](../operations-admin-apis.md#reading-a-partys-protected-identifiers). |
| `subject_audit_role` | string | unset | Role that may read the access log for one subject at a time: `GET /fhir/r4/AuditEvent` with the `patient` parameter required, refused without it. The grant a patient portal holds to serve a person's right to know who accessed their record (GDPR Art. 15, EHDS Art. 9), instead of an admin credential over every patient's log. Unset, the log stays admin-only. See [Audit trail](../audit.md#retrieving-audit-records-iti-81). |
| `import_role` | string | unset | Role that may import an EHR or an EHR extract (`message_import_ehr`, `message_import_ehr_extract`). |
| `role_issuers` | list of string | `[]` | The `persons` issuers whose role claims may grant `admin_role`, `emergency_access_role`, `import_role` and `identifier_reader_role`. A token of any other issuer never holds them, and a Basic user keeps the roles its own table grants. An issuer of another kind refuses the start. Empty, no token grants them. |
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
  (`token_endpoint` or `issuer`, `client_id`, `client_secret` or
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

## `[scim]`

FerroEHR is a SCIM 2.0 service provider (RFC 7644) at `/scim/v2/`, the
endpoint an identity provider provisions each professional through. It is off
by default.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Serve `/scim/v2/`. Off, every path under it answers `404`. |
| `public_base_url` | string | none | The externally reachable origin, such as `https://cdr.example.com`, that every `meta.location` and discovery document names as `<public_base_url>/scim/v2` (RFC 7643 §3.1). Required when `enabled`; it carries no userinfo, query or fragment. |
| `max_results` | integer | `100` | The most resources one list response returns, which `ServiceProviderConfig` declares as `filter.maxResults`. At least 1. |

Each `[[scim.client]]` table declares one provisioning client:

| Key | Description |
|---|---|
| `issuer` | The issuer of the client's access tokens: your own authorization server, an `[[auth.issuer]]` of kind `machines`. FerroEHR issues no tokens. |
| `client_id` | The client the token names (`client_id`, failing that `azp`). |
| `scope` | The one scope a token of the client must hold. SMART on openEHR defines none for SCIM, so the value is yours to choose. |
| `binding_issuer` | The identifier issuer of the binding keys the client provisions: the `identifier_issuer` of an `[[auth.party]]` table. Each User belongs to the one client whose `binding_issuer` issues its key, so no two clients share one. |

The start is refused when `enabled` is set without `public_base_url` or
without a client, and for a client that misses a key, is declared twice,
shares a `binding_issuer`, names an issuer that is not of kind `machines`, or
names a `binding_issuer` no `[[auth.party]]` table carries.

```toml
[scim]
enabled = true
public_base_url = "https://cdr.example.com"

[[scim.client]]
issuer = "https://as.example.com"
client_id = "entra-provisioning"
scope = "ferroehr/scim"
binding_issuer = "urn:example:staff-login"
```

`/scim/v2/ServiceProviderConfig` answers without a token, so a client can read
the authentication scheme before it holds one. Every other path needs a
bearer token of a declared client that acts for nobody and holds its `scope`:

- no token, or one that does not verify, is **401** with
  `WWW-Authenticate: Bearer`;
- a person's token, a client no table declares, and a token without the
  scope are **403**, the last with `WWW-Authenticate: Bearer
  error="insufficient_scope"` (RFC 6750 §3.1).

Every refusal is an RFC 7644 §3.12 `Error` message in `application/scim+json`.
The client's requests count against
[`[server.caller_rate]`](config-server.md) like any caller's, so each
replica gives the client an allowance of its own, and past it a request is
**429** with `Retry-After`. `/scim/v2/Schemas` serves the core User schema
reduced to `userName`, `active` and the SCIM attributes
`[[authz.party_binding.person.attribute]]` maps, and FerroEHR's extension
`urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User` with
`loginIdentifier`, the attribute that feeds the binding key. `/ResourceTypes`
lists the `User` type at `/Users`; that endpoint is not served yet, and
`ServiceProviderConfig` declares PATCH and entity tags unsupported until it
is.

## `[access_delay]`

The EHDS Art. 3(3) delay of the person's own access to newly registered data,
a Member State option: turn it on only where your Member State's rule provides
for the delay. What it does is on [delayed access by the
person](../security.md#delayed-access-by-the-person-ehds-art-33).

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Hold new compositions of the listed categories from patient-context requests until a professional releases them or `hours` pass. Off, the person's access is immediate (Art. 3(1)), every hold already placed stops withholding, and the admin route refuses a new hold with **400**. |
| `categories` | list of string | `[]` | The categories held at creation, named as [`[audit.categories]`](config-audit.md) names one: a priority category of Art. 14(1) by its code (`Laboratory-Reports`, `Medical-Imaging`, …) or as `<system>|<code>`, or a national category the map declares. A name the map does not hold, and `none`, refuse the start. Empty holds nothing at creation; a professional still holds a composition by hand. |
| `hours` | integer | `0` | The limit of a hold, 1 to 8784 (one year), required while `enabled`: the length of a hold placed at creation and the latest end a professional may give one. Art. 3(3) allows the delay "for a limited period of time" only; the period is the Member State's to set. |

```toml
[access_delay]
enabled = true
categories = ["Laboratory-Reports", "Medical-Imaging"]
hours = 168
```

The environment forms are `FERROEHR__ACCESS_DELAY__ENABLED`,
`FERROEHR__ACCESS_DELAY__CATEGORIES` (comma-separated) and
`FERROEHR__ACCESS_DELAY__HOURS`.

## `[smart]`

The SMART on openEHR scopes are enforced on **every route** whenever
`[auth]` is on: each operation carries a requirement from FerroEHR's route
table, which the `ferrohealth-smart` crate checks against the token's scopes
or a Basic user's `scopes`. There is no switch for it. This section decides
what beside the route table: whether the discovery document is served, and
whose grants count for what. See [SMART App Launch](../smart-app-launch.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Serve `/.well-known/smart-configuration`. It is built from the metadata of one `[auth]` issuer (the first introspected `[auth]` issuer, else the issuer `[smart] issuer_metadata_file` names, else the first issuer whose metadata is read from it, else the first issuer), so it needs an issuer whose metadata names a `token_endpoint` (SMART App Launch 2.2.0, Conformance, §Metadata); an issuer read from a `jwks_file` refuses the start with `enabled = true` unless `issuer_metadata_file` supplies that metadata. |
| `issuer_metadata_file` | path | unset | A JSON file holding the RFC 8414 §2 metadata of the `[auth]` issuer it names, whose key set is a `jwks_file`, read at boot. It must name an `[auth]` issuer, wherever it stands in the list, repeat no member name, and every endpoint it names must be `https` (a loopback host excepted); the discovery document takes its `token_endpoint` and the other endpoints from it. Set for an issuer whose metadata is read from the issuer, it is a boot error. |
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

### Choose an Ed25519 signing key

The server refuses an **ECDSA** (P-256, P-384, P-521, secp256k1, Brainpool)
or **Ed448** signing key at boot: their private-key arithmetic runs on
`crypto-bigint` 0.5, whose constant-time operations can branch on secret
values (RUSTSEC-2026-0387), with no fixed release on that line. The boot error
names the advisory. Move to Ed25519 with the steps below; the old certificate
goes in `retired_key_paths`, where it keeps verifying, because verification
uses only public values.

An **RSA** signing key is accepted, but it makes every commit perform an RSA
private-key operation, the operation the Marvin timing sidechannel concerns
(RUSTSEC-2023-0071 / CVE-2023-49092), for which the underlying `rsa` crate has
no fixed release. An Ed25519 key keeps that code off the signing path entirely.

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
subkey** to the existing certificate, export the armored secret key again to
`key_path`, and restart. The server signs with the newest subkey whose binding
grants signing and that is neither expired nor revoked
([the rule](../signing/pgp.md#configuring-the-key)), with no
`retired_key_paths` entry needed: the certificate keeps the previous subkey, so
past signatures continue to verify. For a refused ECDSA or Ed448 key this works
when the new Ed25519 subkey is the one chosen. The steps above, with a new
certificate, work in every case.

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

## `[versioning]`

What a commit stores is not a setting: a version whose content equals the
current version's is committed like any other, with its own commit audit,
contribution, version id and `ETag`, and the content is stored once (the new
version reads its predecessor's node rows). The one setting here is the
answer to a repeated send of the same content.

| Key | Type | Default | Description |
|---|---|---|---|
| `skip_identical_resend` | bool | `false` | When `true`, a `PUT` whose content equals the current version's commits nothing and answers like a successful update with the existing version: `200` (or `204` under `Prefer: return=minimal`), and that version's `ETag` and `Location`. |

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
[versioning]
skip_identical_resend = true
```
