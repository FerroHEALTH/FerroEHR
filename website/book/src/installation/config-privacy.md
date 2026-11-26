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
subject_namespaces = ["urn:example:pseudonym"]
```

```bash
FERROEHR__PRIVACY__SUBJECT_NAMESPACES=urn:example:pseudonym,mpi.example
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

## National identifiers belong in the master patient index

The rules above keep national identifiers off the clinical side, and the
demographic side holds none either. A national identifier such as the Dutch
BSN lives in the master patient index of your deployment (FerroPIX, over IHE
PIX/PDQ and PIXm), never in the CDR. The CDR knows the subject only as a
pseudonym: `EHR_STATUS.subject.external_ref` names a configured pseudonym
namespace and carries a UUID, which the subject rule above enforces.

The server stores a party exactly as it was committed. An identifier you put in
a `PARTY_IDENTITY` or a `PARTY.details` structure is part of the version body,
its content digest and its signature, and every read serves it. It does not
seal an identifier, and it ships no registry of national identifier schemes.
Keep national identifiers out of the parties you write, and resolve a person to
their pseudonym through the master patient index.

The databases hold every party and every clinical record, so encrypt the
database volumes and every backup at rest. The
[go-live checklist](../security/go-live-checklist.md) makes this a requirement.

### The care projection

A clinical query that names no EHR asks, for each EHR it reads, whether one of
your care teams cares for the EHR's subject. Each replica keeps a copy of the
care relationships in the clinical database (`clinical.access_care`). Each row
holds the patient's and the carer's container UUIDs, what the carer is (a
`PERSON`, a `ROLE`, a `GROUP` or an `ORGANISATION`), the care role code, the
provider's identifier, the care dates, the container UUID of the party that
wrote the relationship, and the SHA-256 digest of one subject reference. The
`ehr` row holds the same digest of its own subject. A subject reference is a
pseudonym, so its digest needs no key.

Two indexes serve the projection. One finds a patient's relationships by the
subject digest, which is how each EHR a query reads is checked. The other
finds a carer's patients by the carer and its role. When your policy admits
no EHR outside your care, a query also starts from your own rows of that
second index, so its cost grows with your caseload, whatever the size of the
store; each EHR it reaches is still checked one by one.

The planner orders such a query by how many rows it expects in your caseload,
so the projector runs `ANALYZE` on `clinical.access_care` after any write of
1,000 rows or more, and autovacuum re-analyzes the table once 2% of its rows
have changed (PostgreSQL docs, Populating a Database). The runtime role holds
the `MAINTAIN` privilege on the table for this.

A replica reads the projection only while it is as recent as the care facts
the replica holds. A discharge, a removed membership or an erasure is never
granted from a stale row: until the projector catches up, the query sends the
list of every patient in your teams' care.
