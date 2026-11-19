# Privacy & data minimisation

`[privacy]` is what the clinical side refuses to hold. Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## What this section is for

FerroEHR keeps clinical content, demographic parties and the map between them
in three separate database schemas reached by separate roles. That separation
is only worth something if the clinical side does not carry the subject's
identity anyway, in the subject reference or in a party proxy. This section
is the two rules that keep it out.

No openEHR specification governs any of them. The Reference Model leaves
`EHR_STATUS.subject.external_ref` open and only advises against identifying
content on a party proxy (`PARTY_IDENTIFIED`: "Should not be used to include
patient identifying information"). GDPR
[Art. 4(5) and Art. 25(2)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) are
what make it a duty; this section is where a deployment states its side of it.

Both rules read the Reference Model structure of a write. Neither guesses from
a value's shape whether it identifies a person, so a laboratory order number,
a specimen number or any other clinical digit run is stored as written,
whatever national checksum it happens to pass.

Both rules run on the write path, before anything is stored. A refusal is
`422 Unprocessable Entity` with one `errors[]` entry per finding (sent when
the request carries `Prefer: return=representation`),
each naming the RM path and the rule that matched. **A refusal never echoes the
offending value** — it would otherwise travel into the response body, the
access log and the traces, which is the leak the rule exists to prevent.

```toml
[privacy]
subject_namespaces = []
allow_identified_parties_in_ehr = false
```

## `[privacy]`

| Key | Type | Default | Description |
|---|---|---|---|
| `subject_namespaces` | list of strings | `[]` | The pseudonymisation domains this deployment issues subject pseudonyms in. |
| `allow_identified_parties_in_ehr` | bool | `false` | Accept a `PARTY_RELATED` whose relationship is `self` carrying `name` or `identifiers` in clinical content. Off already accepts every other party proxy; see the matrix below. |

### The subject reference

`EHR_STATUS.subject` is a `PARTY_SELF` whose optional `external_ref` points at
a demographic service. Once `subject_namespaces` names at least one namespace,
that reference must name one of them and carry a UUID as its `id.value`:

```toml
[privacy]
subject_namespaces = ["urn:ferroehr:pseudonym"]
```

```bash
FERROEHR__PRIVACY__SUBJECT_NAMESPACES=urn:ferroehr:pseudonym,mpi.example
```

What a deployment gets by default, and what declaring a namespace adds:

| | `subject_namespaces` empty (default) | declared |
|---|---|---|
| `EHR_STATUS.subject.external_ref` written by a client | accepted with any namespace and any identifier, as the openEHR REST API admits | must name a declared namespace and carry a UUID; anything else is a `422` |
| The database | no shape held | a trigger on `ehr` refuses a non-UUID subject reference whichever session writes it |
| Minting | FerroEHR mints none | FerroEHR mints none: the IHE patient identity source does |

FerroEHR mints no pseudonym. Under the IHE patient identity source
([`[pixm]`](config-integrations.md#pixm)), the source answers the
pseudonym the clinical domain holds for a patient, and the identity source
mints it on the first ask; that domain's assigning authority is a declared
namespace. Every value that arrives from elsewhere, a client writing
`EHR_STATUS` directly, an EHR-Extract, an archive load, meets the same rule,
which is why declaring the namespace is the deployment's act.

**Empty is the default, and it leaves the rule out of force.** A pseudonym
namespace is a deployment fact — which service mints the tokens — and there is
no name a server could invent for an operator. Declaring one is the same act as
turning the rule on. An EHR with no subject reference at all is unaffected
either way: that is the default `EHR_STATUS` the openEHR REST API describes,
and it carries nothing to minimise.

Both spellings of a UUID's case are accepted; the braced, URN and unhyphenated
forms are not, because the promoted subject column is compared as text and four
spellings of one pseudonym would be four subjects.

The rule has a second line of defence in the database. On every boot the
server stamps whether namespaces are declared, and a trigger on the `ehr`
table then refuses a subject reference that is not a UUID whichever code path
or session writes it (`ehr_subject_pseudonym_guard`). Declaring namespaces
over an existing store does not rewrite stored rows: the boot log names how
many EHRs carry a subject reference that is not a pseudonym, and those need
re-pseudonymising.

### Identified parties

A party proxy inside clinical content — the composer, participations, the
health care facility and the feeder-audit party slots — is governed by which
class it is:

| | `name` | `identifiers` |
|---|---|---|
| `PARTY_IDENTIFIED` | accepted | accepted |
| `PARTY_RELATED`, relationship a third party (mother, guardian, donor, …) | accepted | accepted |
| `PARTY_RELATED`, relationship `self` | refused | refused |

The two classes mean different things, so one rule for both was the wrong
shape. `PARTY_IDENTIFIED` is the Reference Model's own provider proxy: "Proxy
data for an identified party **other than the subject of the record**",
"Typically for health care providers, e.g. name and provider number of an
institution". A composer name or a performing clinician's name is that, not
patient identity, and refusing it invented a prohibition the specification
does not contain. `ctx/composer_name` in the simplified formats produces
exactly this shape, and so does the example composition this server generates
for a template.

`identifiers` follows the same line. The class is "Used to describe parties
where only identifiers may be known … e.g. name and provider number of an
institution", so a clinician's registration number on the composer is the
class's own paradigm case, and the simplified formats build exactly that from
`ctx/participation_identifiers`. What the separation between the two database
schemas exists to prevent is the *subject's* identity on the clinical side,
and a `self` party's `identifiers` slot is where the Reference Model puts it,
so that slot is refused by this rule.

`PARTY_RELATED` is the "Proxy type for identifying a party **and its
relationship to the subject** of the record", and that relationship "is coded
as self" where the party *is* the patient. A name on a `self` party is the
subject's own identity and is refused. A name on any other relationship, the
mother who consented, the guardian, the donor, is a third party the Reference
Model models on purpose, and the openEHR REST API obliges a server to accept a
composition that carries it, so it is accepted. A relationship the server
cannot read counts as `self`: the rule refuses what it cannot prove harmless,
and the Reference Model validator names the missing attribute on its own.

A refused proxy stays expressible: `PARTY_IDENTIFIED`'s own validity rule is
satisfied by `external_ref` alone, so it points into the demographic domain
instead of restating the identity.

The commit's own `AUDIT_DETAILS.committer` is not clinical content and is never
touched by this rule.

A deployment that needs a name or formal identifiers on a `self` party sets:

```toml
[privacy]
allow_identified_parties_in_ehr = true
```

which is announced at boot with a warning naming what it permits.

## `[privacy.cohort]`

Cross-domain **cohort queries**: select a population in the demographic domain,
resolve it to the EHRs whose `EHR_STATUS` subject names it, and run AQL over
exactly those EHRs. A FerroEHR extension — no openEHR spec governs it. The wire contract and a
worked example are in [Querying with AQL](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary).

**Off until a predicate is bound.** Which demographic leaf may be selected on is
a deployment fact about the archetypes in use, never something the server can
infer, so `[privacy.cohort.predicates]` is empty by default and `POST /query/cohort`
answers `404` until it is not. `subjects = true` turns the surface on without
a predicate, for cohorts that name their subjects by identifier. That is the one
form an IHE patient identity source serves (`[pixm] subject_index = "pixm"`):
it holds no party-to-EHR map, so a predicate is refused there.

```toml
[privacy.cohort]
small_cell_threshold = 5
max_cohort_size = 100000

[privacy.cohort.predicates]
city = { archetype = "openEHR-DEMOGRAPHIC-ADDRESS.address.v1", node = "at0012", kind = "text" }
```

| Key | Type | Default | Description |
|---|---|---|---|
| `small_cell_threshold` | int | `5` | The distinct-EHR floor a result set must reach to be served. Below it the rows are withheld and the response is marked `suppressed`; `0` disables suppression. |
| `max_cohort_size` | int | `100000` | The largest cohort a predicate may select. A wider one is refused `422` rather than truncated: a silently shortened cohort is a wrong denominator. |
| `predicates` | table | empty | The allow-list, keyed by the name a caller uses. Empty, with `subjects` off, leaves the surface off. |
| `subjects` | bool | `false` | Let a request name its subjects by identifier, in any identifier domain, each resolved through the `[pixm]` provider: under the IHE provider by a forward PIXm translation into the server's clinical domain, never back. |

### The allow-list

Exactly five keys are bindable — `city`, `postcode_area`, `sex`, `age_band` and
`organisation` — and an unknown key is a **boot error**. Each binding carries
three fields:

| Field | Description |
|---|---|
| `archetype` | The archetype HRID of the leaf ELEMENT's nearest archetyped ancestor, e.g. `openEHR-DEMOGRAPHIC-PERSON.person.v1`. Must be a demographic HRID; anything else is a boot error. |
| `node` | The ELEMENT's `archetype_node_id` at-code, e.g. `at0012`. |
| `kind` | How a caller's value is matched: `text` (exact, against `value/value`), `text_prefix` (prefix, `LIKE`-escaped), `coded` (exact, against `value/defining_code/code_string`), or `birth_date` (an inclusive age band in whole years, `40-49`). |

`age_band` must be bound `birth_date`, and no other key may be — both halves are
boot errors, because a mismatch would bind a predicate that matches nothing
while reporting an empty cohort.

**The archetype must be one the node model reaches.** A party is decomposed into
its own rows for the party root, for each `PARTY_IDENTITY`, `CONTACT`, `ADDRESS`
and `CAPABILITY` nested in it, and for every archetyped `ITEM_TREE` or `CLUSTER`
under any of their `details`. An `ADDRESS` under a `CONTACT` is therefore
bindable like anything else.

## Interaction with the audit trail

The ATNA Patient-Number participant is filled from the promoted subject column,
so constraining the subject reference to an opaque pseudonym is also what keeps
the audit trail and its forwarding sinks free of the identity. The contribution
outbox carries no subject at all. Both are covered by a CI test that fails when
any subject identifier other than the opaque UUID appears in an outbox payload,
an ATNA message or a trace record.

## Protecting national identifiers in the demographic domain

The rules above keep national identifiers off the **clinical** side. The
demographic side is where a party's identifiers legitimately live, and
`[privacy.identifier_protection]` decides how they are held there.

With the table present, an identifier of a configured scheme never sits in the versioned
body. The value moves to `party.national_identifier`, sealed with
AES-256-GCM under a key derived per domain, and the body keeps a reference in
its place. Beside the ciphertext sits an HMAC-SHA-256 digest of the value, which
is what makes "which party holds this identifier" answerable without decrypting
anything — and, because it is keyed, what stops the database, a backup or a read
replica from reversing a nine-digit space.

```toml
#? [privacy.identifier_protection]
#? schemes = ["nl-bsn"]
#? key_file = "/run/secrets/ferroehr-identifier-key"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Whether protection is in force. Off leaves identifiers stored as written. |
| `schemes` | list of string | `["nl-bsn"]` | The `DV_IDENTIFIER.type` values to protect. Each must exist in the `party.identifier_scheme` registry; with protection on, the server refuses to boot when one does not. |
| `key` | secret | unset | The root key, 64 hex characters. Prefer `key_file`. |
| `key_file` | path | unset | A file holding the root key, read at boot. |

Turning it on with no key, with an empty `schemes` list, or with a scheme the
registry does not hold, is a **boot error**.
A server that believes it seals national identifiers and does not is worse than
one that never claimed to.

Four properties worth knowing before you enable it:

- **The stored, signed and served body are the same form.** Sealing runs before
  the body is decomposed and signed, exactly like the multimedia offload, so a
  signature still verifies against the bytes the server holds. What a reader
  receives carries the reference; the value is available through the admin
  read below. A create or update response serves the stored form as well, so
  the value you submitted does not come back in it.
- **Each party version keeps its own sealed rows.** A later version retires
  the rows of the one before it without rewriting them, so an older version
  still expands to the value it committed. Only live rows take part in the
  one-party-per-value rule.
- **Nothing resolves an identifier to a party.** The server offers no lookup
  from an identifier value to the party that holds it. The keyed digest keeps a
  value live for at most one party at a time.
- **Reading a value back is audited and needs its own grant.**
  `GET /ferroehr/rest/extensions/v1/admin/party/identifiers` serves a party version with its values
  put back, for a caller holding both the admin role and a `read_identifiers`
  grant in the CDR, and records each scheme it expands
  ([reading a party's protected identifiers](../operations-admin-apis.md#reading-a-partys-protected-identifiers)).
- **The key is load-bearing and rotation is a re-encryption.** The runbook is in
  [Operations](../operations.md#rotating-the-national-identifier-key).

Only the demographic writer role reaches the sealed value. The read-only twin
sees that a party holds a protected identifier and which party it is, and is
refused the ciphertext and the digest by column-level grant; the clinical roles
are refused the table outright.

### The care projection

The same root key also drives the care projection the access evaluator reads.
A clinical query that names no EHR asks, for each EHR it reads, whether one of
your care teams cares for the EHR's subject. With protection on, each replica
keeps a copy of the care relationships in the clinical database
(`clinical.access_care`). It holds only the patient's and the team's container
UUIDs, the care dates, and an HMAC-SHA-256 digest of each subject reference
under the clinical lookup subkey. The `ehr` row holds the same digest of its
own subject. The key never reaches the database, so the digests name nobody to
a reader of the database, a backup or a replica.

A replica reads the projection only while it is as recent as the care facts
the replica holds. A discharge, a removed membership or an erasure is never
granted from a stale row: until the projector catches up, the query sends the
list of every patient in your teams' care, as it did before. With protection
off, the projection is not written and every query sends that list, which
grows with the patients in your teams' care. A production deployment
configures `[privacy.identifier_protection]` for this reason as well as for
the identifiers it seals; the census without it is a fallback for a sandbox or
a deployment that has not set the key yet. Rotating the key rebuilds the
projection and the digests on the next projector run.
