# Integrations

Query execution, the patient identity source and the optional
integrations: `[record.query]`, `[pixm]`, `[pix]`, `[record.change_feed]`,
`[terminology]`, `[record.multimedia]`. Everything except the query knobs and the identity source is
off by default, and a disabled integration contacts nothing and mounts no
routes. Precedence, the environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[record.query]`

AQL execution bounds.

```toml
[record.query]
plan_cache_capacity = 256
timeout = "30s"
max_result_rows = 10000
```

| Key | Type | Default | Description |
|---|---|---|---|
| `plan_cache_capacity` | int | `256` | Maximum distinct cached query plans; `0` disables the cache and every lookup runs the full parse-and-lower path. A cached plan also keeps the SQL text of up to 32 statement shapes, so a repeated query binds its values without rendering its SQL again. Cache activity is reported by the `aql_plan_cache_events` counter. |
| `timeout` | duration | `"30s"` | Per-query database execution budget; `"0s"` disables it. Overrun is refused `408`. |
| `max_result_rows` | int | `10000` | The largest page one query execution serves: the page of a query nothing else bounds, and the maximum an explicit `LIMIT` or `fetch` may ask for (a larger page is refused `400`); `0` means unbounded. |

**`timeout` is on by default, and deliberately tighter than
[`database.statement_timeout`](config-server.md#database).** Keeping this budget the
tighter of the two means an overrun surfaces as the engine's own typed `408`
rather than a driver error. When it fires, or when the client disconnects or
the HTTP request timeout gives up first, the query's connection is closed
instead of returned to the pool, so its pool slot frees at once and PostgreSQL
stops the statement within about a second (the session's
`client_connection_check_interval`).

**`max_result_rows` is the largest page one execution serves.** Without it,
`SELECT c FROM COMPOSITION c` with no `fetch` generates SQL with no `LIMIT` and
materialises every matching row: one request, unbounded allocation. A query that
nothing else bounds takes the ceiling as its page. An explicit AQL `LIMIT` or a
`fetch` parameter is honoured as written up to the ceiling; a page larger than the
ceiling is refused with `400` naming the ceiling. The refusal is deliberate: a
silently shortened page would let a client that pages with its own `fetch` as the
stride skip the rows between the shortened page and its next `offset`, and the
`RESULT_SET` has nothing to say so. ITS-REST leaves both the `fetch` default and
its maximum to the implementation. A bulk consumer pages with `offset` and a
`fetch` at or below the ceiling, or the operator raises the ceiling deliberately.

## `[pixm]`

Who translates a patient identifier into the clinical domain, so the EHR that
records the patient can be found (the EHR index). The EHR itself always says
whose record it is: its `EHR_STATUS.subject.external_ref` names the subject,
and FerroEHR finds a subject's EHR by matching that reference, which it keeps
indexed beside the EHR on every EHR create and `EHR_STATUS` update. No openEHR
specification governs where the patient's other identifiers live: the RM's
`PARTY_SELF` section says the association "may be done elsewhere for security
reasons", and FerroEHR's design puts it in an IHE patient identity source.

**`record.subject_index = "pixm"` asks an IHE patient identity source.** FerroEHR asks a PIX
Manager, as a PIXm Patient Identifier Cross-reference Consumer
([IHE ITI TF-2 §3.83](https://profiles.ihe.net/ITI/PIXm/ITI-83.html),
`$ihe-pix`), which identifier the clinical domain holds for a patient that
another domain's identifier names. That identifier is the pseudonym
`EHR_STATUS.subject` carries. FerroEHR sends only what ITI-83 specifies, so any
conformant PIX Manager serves; FerroPIX is the one the compose quickstart and
the Helm chart run.

**`subject_index = "local"`, with no identity source, is the other supported
posture**, for production as much as for development, tests and the
conformance suites: the subject-id and subject-namespace model. A client names
a patient in `EHR_STATUS.subject.external_ref` by any namespace and identifier
(`benchmark` / `1793131381`), as `OBJECT_REF.namespace` and a local
`GENERIC_ID.scheme` allow, and a subject resolves to itself: the openEHR REST
API's `GET /ehr?subject_id=…&subject_namespace=…` lookup and a cohort that
names its subjects both find the EHR whose `EHR_STATUS` names the subject,
matching namespace and identifier exactly. An EHR created over `POST /ehr` or
`PUT /ehr/{ehr_id}` is found from the moment it is created, and an
`EHR_STATUS` update that changes the subject moves it. The key has no
default: **unset**, the server refuses to start and the refusal names both
choices.

What the local posture means for pseudonymisation: with no
`[privacy] subject_namespaces` declared, the server stores a subject reference
as the client sends it, a real identifier included. Pseudonymisation then
belongs to the client, or to the demographic service the reference points at
(GDPR Art. 4(5)). There is no mixed mode: a deployment that declares pseudonym
namespaces keeps the rule in force under `local` too, so every subject
reference must name a declared namespace and carry a UUID, and no other
namespace is accepted beside them. A `production` deployment runs the
subject-id posture by accepting the `open_subject_namespace` gap by name
([`deployment.profile`](configuration.md#deploymentprofile)), which boot and
`GET /management/status` then state.

**Care relationships and representations reach a pseudonymous EHR through
the patient's `PERSON`.** Under `pixm` an EHR names its subject by the
pseudonym, which is no party of the CDR's party store, so the patient's
`PERSON` declares the subject reference it stands for: an element of its
`details` named by the `record_subject` code of FerroEHR's operation
terminology whose value is a
`DV_IDENTIFIER` with `issuer` set to the `[pixm] assigning_authority` and `id`
to the pseudonym. The registering system writes it in the same demographic
contribution as the `PERSON`, from the pseudonym ITI-83 answered, and FerroEHR
asks the identity source nothing for it. A subject reference under
`[authz.party_binding] namespace` names a `PERSON` by its container, as under
`local`, and any other reference names the `PERSON` that declares it. One
reference stands for one `PERSON`. After a PIX merge, one demographic
contribution moves the merged patient's pseudonym onto the survivor's
`PERSON`, and both EHRs inherit the survivor's care relationships and
representatives from the next request. Writing the element needs the
`write_care_relationship` grant or an administrator, as a care relationship
does ([The pseudonymisation boundary](../pseudonymisation.md#the-subject-link-in-the-party-domain)
names what this changes).

### The patient's EHRs

A lookup through the index reaches every EHR of the patient. ITI-83 turns
the identifier into the pseudonym of FerroEHR's domain. The EHRs are the one
whose `EHR_STATUS` names that pseudonym, and the one of each other
`record_subject` reference of the `PERSON` that declares it. After a PIX
merge FerroPIX answers the survivor's pseudonym for the merged patient's
identifier (PIXm 3.1.0 §2:3.83.4.2.2.5 leaves this to policy). Once the
merge contribution has moved the merged patient's pseudonym onto the
survivor's `PERSON`, the lookup finds both EHRs. Before that contribution, it
finds the survivor's EHR alone. Under [`[pix]`](#pix) FerroEHR commits that
contribution itself from FerroPIX's feed. A subject cohort reads the same set.

A contact point's TRC patient asks the identity source nothing. Its
identifier is matched against the identifiers the patient's `PERSON` carries
at the `[authz.party_binding.patient] identifier` path, by issuer and value,
and its EHRs are the ones that `PERSON`'s references name. FerroEHR projects
those identifiers into the party table `binding_patient_identifier` at the
commit of the `PERSON`, each value as the `PERSON` carries it. A national
identifier is not among them: it belongs in the master patient index, which
answers it with the patient's pseudonym, and the CDR holds the pseudonym
only. So the registering system writes
the patient's identifiers in the same contribution as its record subjects,
and a merge contribution moves both onto the survivor's `PERSON`. No request
of a contact point waits on the PIX Manager, and an unavailable PIX Manager
does not refuse one. **Our own design:** the projection, the match on issuer
and value, and the refusal of an identifier two `PERSON`s carry. eHDSI XCA
Profile 8.0.1 §4.2.1 names the TRC patient and leaves finding its record to
the data holder.

**`subject_index` alone switches the index**, in the file or as
`FERROEHR__RECORD__SUBJECT_INDEX`. A `[pixm]` table kept beside `"local"` is not
used, and boot names its keys, so a configuration written for the IHE
source runs locally with `FERROEHR__RECORD__SUBJECT_INDEX=local` and no
other change; its declared pseudonym namespaces stay in force. The compose
quickstart selects its local configuration with one variable
([The local EHR index](compose.md#the-local-ehr-index)).

| Key | Type | Default | Description |
|---|---|---|---|
| `record.subject_index` | enum{pixm,local} | required | Who translates a subject into the clinical domain. |
| `pixm.assigning_authority` | URI | required for `pixm` | The clinical domain, such as `urn:oid:2.999.7.1`: the `targetSystem` of every ITI-83 query and the namespace the answered pseudonym is stored under. It must be one of `privacy.subject_namespaces`. |
| `pixm.method` | enum{post,get} | `post` | `post` sends a `Parameters` body, so a patient identifier never appears in a URL or an access log; `get` is the form ITI-83 prescribes, for a PIX Manager that accepts no `POST`. |
| `pixm.endpoint` | [endpoint table](#the-endpoint-table) | required for `pixm` | The PIX Manager: `url`, its FHIR base (`[base]` of `[base]/Patient/$ihe-pix`), `timeout` for one ITI-83 exchange with its tries and the waits between them, the `retry` of an exchange that got no answer or a `5xx`, the TLS and the credential. |

```toml
[privacy]
subject_namespaces = ["urn:oid:2.999.7.1"]

[record]
subject_index = "pixm"

[pixm]
assigning_authority = "urn:oid:2.999.7.1"

[pixm.endpoint]
url = "https://pix.example.org/fhir"

[pixm.endpoint.credentials.oauth]
token_endpoint = "https://idp.example.org/realms/pix/protocol/openid-connect/token"
client_id = "ferroehr"
client_secret_file = "/run/secrets/pix-client"
scope = "system/Patient.rs"
```

`[pixm.endpoint.credentials.oauth]` is the client credentials grant every
ITI-83 request carries, with the [grant's keys](#the-endpoint-table); a `401`
is answered once more under a new token. The client id is the one the
identity source mints pseudonyms in the clinical domain for, and the scope is
the one its PIX Manager takes from a Consumer. `[pixm.endpoint.tls]` holds
the client's TLS material for a PIX Manager behind a private PKI or one that
asks for mutual TLS.

How a subject a cohort names is resolved:

| Step | Under `ihe` | Under `local` |
|---|---|---|
| the subject in the clinical domain | its clinical pseudonym, asked of the source; a subject already in the clinical domain names itself | the subject as given |
| the EHR of that subject | the one EHR whose `EHR_STATUS` names it | the one EHR whose `EHR_STATUS` names it |

The source is never asked to resolve a pseudonym back to another domain's
identifier.

**No fallback.** At start the server reads the source's
`CapabilityStatement` and refuses to start when it does not answer. A request
the source then fails, refuses or times out on answers with an error that
carries what the source said, and nothing is answered from the `EHR_STATUS`
subject instead.
An answered pseudonym the `[privacy]` subject rule refuses, or more than one
identifier in the clinical domain, is refused too.

**FerroEHR does no identity work in either mode.** It matches no patients,
merges none and links no identifier domains; the identity source does. It asks
only for the cross-reference ITI-83 answers. PDQm
([ITI-78](https://profiles.ihe.net/ITI/PDQm/ITI-78.html)) is not used: its
search answers with the demographic `Patient`, which FerroEHR would only
discard, so asking for it breaks data minimisation
([GDPR Art. 5(1)(c)](https://eur-lex.europa.eu/eli/reg/2016/679/oj)), and its
`$match` is patient matching.

## `[pix]`

**`[pix]` applies FerroPIX's merges, unmerges and links.** FerroEHR becomes
an IHE PMIR Patient Identity Consumer: it subscribes to the Patient Identity
Registry with [ITI-94](https://profiles.ihe.net/ITI/PMIR/ITI-94.html) for
every `Patient`, and the Registry posts each change as an
[ITI-93](https://profiles.ihe.net/ITI/PMIR/ITI-93.html) message to
`POST {public_base_url}/pmir/feed`. Any instance answers. The route admits
only the client `[pix]` names, by the access token it presents (IUA,
§2:3.93.5); any other caller gets `403`. The table holds the Registry's
endpoint and that trust alone, and it needs `[server] public_base_url`, under
which the subscription names the feed route.

What a message does to the patient `PERSON`s:

| The Registry announces | FerroEHR commits |
|---|---|
| a merge of A into B | A's `record_subject` references move onto B's `PERSON`, and B's identifiers follow B's announcement |
| the revert of that merge (A `active` again, no `replaced-by` link) | the moved references go back to A's `PERSON`, and each `PERSON` carries its own identifiers again |
| a link (an identifier added) | the `PERSON` carries it, moved from the `PERSON` that gave it up, or written in its domain's `DV_IDENTIFIER` type: the one `[[pix.identifier_domain]]` names, else the one the patient `PERSON`s carry that domain with |
| an unlink (an identifier dropped) | the `PERSON` no longer carries it |
| a delete | the `PERSON` carries none of its patient identifiers |

**A split closes at receipt.** FerroEHR stores each message before it
answers, and in the same transaction the identifiers and record subjects an
unmerge or an unlink splits off a `PERSON` stop resolving on every instance:
a contact point's TRC patient, the index and the care relationships no longer
reach the other patient. One instance then applies the messages in arrival
order, each as one demographic contribution committed as the Registry's
client, and records it in the access log as `pmir-message:<id>`. A
redelivered message changes nothing. The Registry's client needs the
`write_care_relationship` grant, which a record subject takes.

**A message is refused, never guessed.** A message about a merged identity
that states neither the merge nor its revert, an identity two `PERSON`s
carry, an identifier another identity still holds, a linked identifier of a
domain with no `DV_IDENTIFIER` type (the log names the domain), or a
contribution the commit checks refuse, is kept unapplied with its split in force and logged as
an error. An identity is bound to its `PERSON` when the feed first announces
it while one `PERSON` carries one of its identifiers; a merge of two
identities the feed never bound is refused.

**An operator settles a refused message.** A refused message keeps what it
split unresolved until an administrator acts on it, through the admin API
(the Admin class and `[auth] product_scope`, with `[api] admin` on) or the
CLI:

| Step | Admin API | CLI |
|---|---|---|
| List the refused messages: each with its refusal code, receipt and refusal time, apply attempts, the Registry's `Patient.id`s it names and the `PERSON`s it keeps split, as counts; no identifier value | `GET /admin/pmir/messages` | `ferroehr pmir list --operator <name>` |
| Retry a message once its cause is fixed, such as a `[[pix.identifier_domain]]` entry or a grant added: it goes back to its arrival position and a running instance applies it next | `POST /admin/pmir/messages/{message_id}/retry` | `ferroehr pmir retry <id> --operator <name>` |
| Settle it without applying it: `discard` keeps its splits, `lift-splits` removes them so its identifiers and record subjects resolve as before the message; the reason is recorded | `POST /admin/pmir/messages/{message_id}/settle` with `{"decision": …, "reason": …}` | `ferroehr pmir settle <id> --decision <decision> --reason <text> --operator <name>` |

A retry is refused (`409`) once a message received after it that names one of
the same identities has applied: applying the older announcement would roll
the identity back, so settle it instead. A discarded message can still have
its splits lifted. No step applies a message by the operator's choice of
`PERSON`, identifier or survivor; an identity conflict is corrected at the
Registry, whose next message applies. Every list (one record per message),
retry and settlement is an access record naming the operator and
`pmir-message:<id>`. The CLI records the `--operator` name as the
principal and the operating-system account that ran it, its effective user as
`os:<name>/<uid>`, as the record's client.

While refused messages wait, the `pmir_messages_refused` gauge counts them,
and the `pmir_refused_messages` health indicator reads `DEGRADED` from
`pix.refused_degraded_at` of them on. It never withdraws readiness.

| Key | Type | Default | Description |
|---|---|---|---|
| `pix.endpoint` | [endpoint table](#the-endpoint-table) | required | The Registry: `url`, its FHIR base (`[base]` of `[base]/Subscription`), `timeout` for one ITI-94 exchange, `retry`, the TLS and the credential the Registry authorizes a subscriber by. |
| `pix.feed_issuer` | URL | required | The issuer of the access token the Registry presents with each ITI-93 message, one of the `[auth]` issuers. |
| `pix.feed_client_id` | string | required | The `client_id` of that token. |
| `pix.identifier_domain` | array of tables | none | Per identifier domain, its `system` and the `identifier_type` a linked identifier of it is written with, such as `MR`; a protected scheme's code keeps the identifier sealed. |
| `pix.refused_degraded_at` | integer | `1` | How many refused messages still to retry or settle turn the `pmir_refused_messages` indicator `DEGRADED`; at least 1. |

```toml
[server]
public_base_url = "https://cdr.example.org"

[pix]
feed_issuer = "https://idp.example.org/realms/hospital"
feed_client_id = "ferropix"

[[pix.identifier_domain]]
system = "urn:oid:2.999.1.1"
identifier_type = "MR"

[pix.endpoint]
url = "https://pix.example.org/fhir"

[pix.endpoint.credentials.oauth]
token_endpoint = "https://idp.example.org/realms/hospital/protocol/openid-connect/token"
client_id = "ferroehr"
client_secret_file = "/run/secrets/pix-client"
```

**Our own design:** the subscription criteria, acknowledging once a message
is stored, the binding of an identity to its `PERSON`, the split at receipt,
the refusals and their retry and settlement. PMIR 1.6.0 §2:3.93.4.1.3 leaves a Consumer's handling of a
message to its capabilities.

## `[record.change_feed]`

The commit-event outbox and its cursor feed. Off until a reader is granted:
a reader is the `reader` element of a `read_change_feed` grant in the CDR, not
a key ([Operational policy](../operational-policy.md#what-the-grants-decide)).
The envelopes are PHI-free by design. The feed, its routes and its scope are on
[Change events (outbox feed)](../beyond-core/outbox-feed.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `retention` | duration | `"7d"` | How long an acknowledged row is kept after its commit; at least a second. |
| `prune_interval` | duration | `"1h"` | How often the retention prune runs; never `"0s"`. |

## `[terminology]`

The one FHIR terminology server the CDR asks; the table's presence turns it
on. The terminology extension API is `api.terminology` (bool, `false`).

| Key | Type | Default | Description |
|---|---|---|---|
| `endpoint` | [endpoint table](#the-endpoint-table) | required | The server: `url`, its FHIR R4B base URL (for FerroTERM `http://ferroterm:8080/r4b`; a `[terminology]` table without it is a boot error), `timeout` for one call, the `retry` of a call that got no answer or a `5xx`, the TLS and the credential. |
| `on_error` | enum{accept,refuse} | `accept` | What an unresolvable binding does to a commit: `accept` accepts it (fail-open), `refuse` rejects it. |

Every key is a scalar, so the whole section is reachable through the
environment: `FERROEHR__TERMINOLOGY__ENDPOINT__URL`,
`FERROEHR__TERMINOLOGY__ON_ERROR`,
`FERROEHR__TERMINOLOGY__ENDPOINT__TIMEOUT`.

```toml
[terminology]
on_error = "accept"

[terminology.endpoint]
url = "http://ferroterm:8080/r4b"
```

The server answers every terminology the bundled openEHR terminology does not
carry: the `/terminology/*` extension API, AQL `TERMINOLOGY(…)` resolution, the
ADL 2 binding check and the composition-commit binding checks below. A response
that is not a valid FHIR R4B `Parameters` or `ValueSet` resource (for example an
`$expand` result missing the required `ValueSet.status` or
`expansion.timestamp`) is treated as an upstream fault rather than partially
read, so it takes the same path as an unreachable server and `on_error`
decides what the commit does.

> [!NOTE]
> The terminology server needs the `fhir` build feature, which is on in the
> published binary and container images. A binary built with
> `--no-default-features` refuses at startup if `[terminology]` is configured;
> the in-process openEHR terminology bundle remains available.

### The endpoint table

Every outbound HTTP integration, `[terminology.endpoint]`, `[pixm.endpoint]` and
`[audit.fhir_feed.endpoint]`, is the
FerroHEALTH family's one endpoint
table, so an operator writes the same keys for each integration of each
product. Each integration's client is built from it by `ferrohealth-smart`'s
outbound client, the one every product of the family uses.

| Key | Type | Default | Description |
|---|---|---|---|
| `url` | URL | required | The service's base URL. Credentials embedded in it are masked in every rendering, and a credential here beside one in `credentials` is a boot error: the service would receive two authorizations. |
| `timeout` | duration | `"10s"` | How long one whole call may take; never zero. |
| `connect_timeout` | duration | unset | How long opening the connection may take; unset, only `timeout` bounds it. |
| `retry` | table | three attempts, a fifth of a second doubling to five seconds | How a call that got no answer, or a `5xx`, is tried again: `max_attempts` (`3`) counts the first try, so `1` never retries; the wait starts at `initial_backoff` and doubles up to `max_backoff` (`"5s"`). A refusal the service answered on purpose is never retried. |
| `tls` | table | platform roots | The family's outbound TLS table: `trust_roots_file`, a PEM bundle trusted beside the platform's roots for a service behind a private PKI; `client_identity_file` (or `client_identity` inline), one PEM holding the client certificate chain and its key for mutual TLS; and `min_version` (`"1.3"`, or `"1.2"`). |
| `credentials` | table | none | What every call carries: HTTP Basic (`user` with `password` or `password_file`) or the OAuth2 client-credentials grant `[...credentials.oauth]`, never both. |

The grant's access token is cached and re-requested shortly before it
expires, and a call the service answers with `401` is sent once more under a
new token. Its keys:

| Key | Type | Default | Description |
|---|---|---|---|
| `token_endpoint` | URL | one of `token_endpoint` and `issuer` | The OAuth2 token endpoint. Credentials embedded in it are masked in every rendering. |
| `issuer` | string | one of `token_endpoint` and `issuer` | The issuer whose metadata names the token endpoint (RFC 8414). |
| `client_id` | string | required | The registered client identifier. Empty is a boot error. |
| `client_secret` / `client_secret_file` | secret / path | one is required | The client secret, inline or read from a mounted file. |
| `client_auth` | enum{client_secret_basic,client_secret_post,private_key_jwt,tls_client_auth,self_signed_tls_client_auth} | `client_secret_basic` | How the client authenticates at the token endpoint. |
| `scope` | string | unset | The `scope` request parameter. |
| `audience` | string | unset | The `audience` request parameter some authorization servers read. |
| `resource` | URL | unset | The RFC 8707 `resource` request parameter. |
| `private_key_file` | path | unset | The PEM private key that signs the client assertion under `private_key_jwt` (RFC 7523). |
| `kid` | string | the RFC 7638 thumbprint | The `kid` the client assertion names. |
| `assertion_alg` | string | by key type | The algorithm the client assertion is signed with. |
| `security_profile` | enum{oauth2,fapi2} | `oauth2` | The security profile the grant is held to. Under `oauth2` a `private_key_jwt` client assertion names the token endpoint as `aud` under `typ` `JWT` (SMART App Launch 2.2.0, Client Authentication: Asymmetric); under `fapi2` it names the issuer under `typ` `client-authentication+jwt` (FAPI 2.0 Security Profile §5.3.3.1), which needs `issuer` set. |
| `tls` | table | platform roots | The token endpoint's own TLS table, with the keys above. |
| `refresh_margin` | duration | `"30s"` | How long before the token expires a new one is fetched. |
| `timeout` | duration | `"10s"` | How long one call to the authorization server may take. |

```toml
[terminology.endpoint.credentials.oauth]
token_endpoint = "https://idp.example.org/realms/ts/protocol/openid-connect/token"
client_id = "ferroehr-cdr"
client_secret_file = "/run/secrets/ts-client"
scope = "system/*.read"
```

### Archetype value-set bindings at commit

With `[terminology]` enabled, committing a COMPOSITION also resolves
the archetype **constraint bindings** its template declares: where a template
binds an `ac` code to an external terminology query, the coded value in the
composition must be a member of the value set that query returns.

- The code is in the value set → the commit proceeds.
- The code is **not** in the value set → `422` naming the path, the code, and
  the bound query. That is a real constraint violation, not a service failure,
  so `on_error` does not change it.
- The value set could **not be resolved** (server down, error response, unknown
  value set) → `on_error` decides: `false` (the default) accepts the
  commit and logs a warning; `true` rejects it with `422`.

With `[terminology]` disabled (the default) no binding is resolved
and no request is made, so commit behaviour is exactly as if this section did
not exist.

> [!NOTE]
> The composition's `terminology_id` is sent verbatim as the FHIR `system`
> parameter, and no openEHR specification defines a mapping between
> `terminology_id` values (`SNOMED-CT`) and FHIR system URIs
> (`http://snomed.info/sct`). If your archetypes and your terminology server
> disagree, align them in the terminology-server configuration; the CDR does
> not rewrite the value.

## `[record.multimedia]`

`DV_MULTIMEDIA` externalization to a blob store. Off by default: blobs stay
inline, byte-identical, and no blob store is built or contacted. The table
keeps `enabled` where other optional features use the table's presence,
because the backend applies while it is off: the blobs already offloaded are
read back from it.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Externalize large multimedia data. |
| `threshold` | size | `"256KiB"` | Decoded size strictly above which data is offloaded; at or below it stays inline. |
| `backend` | `"s3"` | `"s3"` | The storage service. S3 is the one this release offers; its settings are the `[record.multimedia.s3]` table below. |
| `max_concurrent_requests` | integer | `64` | The most requests to the blob store in flight at once, across every request the server serves. |
| `request_timeout` | duration | `"30s"` | How long one request to the blob store, or one chunk of its body, may take. A request that fails with a temporary error is retried twice. |
| `part_size` | size | `"8MiB"` | A blob larger than this is uploaded as a multipart upload in parts of this size, and read in ranged chunks of it. |
| `part_concurrency` | integer | `4` | How many parts of one blob move at once. |
| `orphan_grace` | duration | `"1h"` | How old a blob no version references must be before `ferroehr db sweep-blobs` deletes it. Keep it well above your longest commit. With offload enabled, `"0s"` is refused at start. |

### `[record.multimedia.s3]`

| Key | Type | Default | Description |
|---|---|---|---|
| `url` | secret URL | unset ⇒ the AWS endpoint of `region` | S3-compatible endpoint (MinIO, SeaweedFS, AWS). Must be an absolute `http`/`https` URL when set; an `http://` URL is plain HTTP, development only, which `production` names as the `plaintext_object_store` gap. Credentials embedded in it are redacted from every rendering and never sent. |
| `bucket` | string | `openehr-multimedia` | Target bucket for content-addressed blobs. |
| `region` | string | `us-east-1` | AWS region; S3 requires one even for non-AWS endpoints. |
| `access_key_id` | string | unset | S3 access key id. Unset, with no secret key either, sends the requests unsigned. |
| `secret_access_key` / `secret_access_key_file` | secret / path | unset | S3 secret access key. At most one of the pair. |

The server takes S3 credentials from this table only. It reads no `AWS_*`
environment variable, no profile file and no instance metadata.

An enabled integration whose `url` is set but blank, not an absolute URL,
or carries a scheme other than `http`/`https` is a **boot error**. That case is
easy to reach by accident (an unset Compose variable expanding to nothing, or
an empty Helm value), so it is refused at boot, where an operator can still
act on it.

> [!WARNING]
> Offloaded blobs are PHI. The bucket must be private and encrypted, and reached
> over HTTPS. Multimedia externalization also needs the `multimedia` build
> feature (on in the published binary and container images); a slim build
> refuses at startup rather than silently keeping blobs inline.
