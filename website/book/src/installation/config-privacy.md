# Privacy & data minimisation

`[privacy]` is what the clinical side refuses to hold. Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## What this section is for

FerroEHR keeps clinical content, demographic parties and the map between them
in three separate database schemas reached by separate roles. That separation
is only worth something if the clinical side does not carry the subject's
identity anyway, in the subject reference, in a party proxy, or in free text.
This section is the three rules that keep it out.

No openEHR specification governs any of them. The Reference Model leaves
`EHR_STATUS.subject.external_ref` open and only advises against identifying
content on a party proxy (`PARTY_IDENTIFIED`: "Should not be used to include
patient identifying information"). GDPR
[Art. 4(5) and Art. 25(2)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) are
what make it a duty; this section is where a deployment states its side of it.

All three rules run on the write path, before anything is stored. A refusal is
`422 Unprocessable Entity` with one `errors[]` entry per finding (sent when
the request carries `Prefer: return=representation`),
each naming the RM path and the rule that matched. **A refusal never echoes the
offending value** — it would otherwise travel into the response body, the
access log and the traces, which is the leak the rule exists to prevent.

```toml
[privacy]
subject_namespaces = []
allow_identified_parties_in_ehr = false

[privacy.identifier_scan]
mode = "strict"
rules = ["ch-ahvn13", "ch-epd-pid", "de-kvnr", "fi-hetu", "gb-nhs-number", "nl-bsn", "no-fodselsnummer", "se-personnummer"]
patterns = []
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
([`[ehr_index]`](config-integrations.md#ehr_index)), the source answers the
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
schemas exists to prevent is the *subject's* national identifier on the
clinical side, and that is caught in two places: a `self` party's
`identifiers` slot is refused by this rule, and a national-identifier *value*
is refused by the identifier scanner below wherever it sits, `DV_IDENTIFIER.id`
included.

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

## `[privacy.identifier_scan]`

Every string leaf of every clinical write is checked against the active
identifier rules: the EHR, `EHR_STATUS`, COMPOSITION and directory writes,
the CONTRIBUTION path, EHR-Extract import and the admin archive load. The two
replay paths store a record verbatim, so for them a finding refuses the whole
import or load rather than rewriting the content; demographic parties are the
domain that holds identity and are not scanned.

| Key | Type | Default | Description |
|---|---|---|---|
| `mode` | `strict` or `warn` | `strict` | `strict` refuses the write; `warn` accepts it and records a warning naming the RM path and the rule. |
| `rules` | list of rule keys | every rule the build ships | The national personal-identifier rules to scan for. An unknown key is a boot error listing the shipped keys. |
| `patterns` | list of regular expressions | `[]` | Extra patterns for the kinds no build can ship a rule for. |

### The shipped rules

Each rule transcribes the checksum its own issuing register publishes. A kind
whose current algorithm could not be established from its own register is not
shipped, because a rule that guesses is worse than an absent one: it tells an
operator their data was scanned.

| Key | Jurisdiction | Identifier | Published by |
|---|---|---|---|
| `ch-ahvn13` | CH | AHV-Nummer (AHVN13) | [AHVV Art. 133](https://www.fedlex.admin.ch/eli/cc/63/1185_1183_1185/de) for the structure; the check digit from the [BSV Wegleitung VA/IK 318.106.02 d, Anhang 7](https://sozialversicherungen.admin.ch/de/d/6938/download); matches the plain run and the `756.3047.5009.62` display form |
| `ch-epd-pid` | CH | Patientenidentifikationsnummer (EPD-PID) | [EPDV-EDI Anhang 1](https://www.fedlex.admin.ch/eli/cc/2017/205/de), vendored at `docs/law/ch/epdv-edi/`: the fixed prefix `76133761` and a mod-10 check digit |
| `de-kvnr` | DE | Krankenversichertennummer (the unchangeable part) | [GKV-Spitzenverband, Richtlinie nach § 290 SGB V, 3.4.1, with Anlage 1](https://www.gkv-datenaustausch.de/kvnr/kvnr.jsp); the letter-to-digit convention (A=01 … Z=26) is stated in prose only in the same Richtlinie's Anlage 2 and fixed for A and C by the KVNR's own worked examples |
| `fi-hetu` | FI | henkilötunnus | [Valtioneuvoston asetus väestötietojärjestelmästä 128/2010, 2 §](https://www.finlex.fi/fi/laki/ajantasa/2010/20100128): the date of birth, the century sign, the individual number and the mod-31 check character |
| `gb-nhs-number` | GB | NHS Number | [NHS Data Model and Dictionary](https://www.datadictionary.nhs.uk/attributes/nhs_number.html) |
| `nl-bsn` | NL | burgerservicenummer | [Rijksdienst voor Identiteitsgegevens, Logisch Ontwerp BSN](https://www.rvig.nl/logisch-ontwerp-bsn) |
| `no-fodselsnummer` | NO | fødselsnummer | [Skatteetaten](https://skatteetaten.github.io/folkeregisteret-api-dokumentasjon/nytt-fodselsnummer-fra-2032/) |
| `se-personnummer` | SE | personnummer | [Skatteverket](https://www4.skatteverket.se/rattsligvagledning/edition/2020.2/330245.html) |

Two are deliberately absent. The Danish CPR-nummer has been
[issued without its modulus-11 control since 2007](https://cpr.dk/cpr-systemet/personnumre-uden-kontrolciffer-modulus-11-kontrol),
and those numbers are fully valid, so a checksum rule would pass most recent
ones through while reporting Denmark as covered. The Belgian
rijksregisternummer's modulo-97 check is widely reproduced but no definition
published by the Rijksregister itself could be retrieved.

### Why every rule is active by default

Clinical data crosses borders: a Dutch hospital receives referrals carrying a
Norwegian fødselsnummer. An operator who has not configured the scanner is
exactly the operator who has not yet worked out which identifiers their content
carries, so the default covers all of them.

The two costs are not symmetric. A false positive is a loud `422` naming the RM
path and the rule, which an operator answers by narrowing `rules` or moving to
`warn`. A false negative is a national identifier stored on the clinical side
indefinitely.

A deployment that knows its jurisdictions narrows the list:

```toml
[privacy.identifier_scan]
rules = ["no-fodselsnummer"]
```

```bash
FERROEHR__PRIVACY__IDENTIFIER_SCAN__RULES=no-fodselsnummer
```

The boot log states which rules are active, so a jurisdiction that is not
covered is visible rather than assumed.

### False positives, and what narrows them

Every checksum accepts some fraction of random digit runs, and clinical content
is full of numbers. Three things narrow it:

- **A digit run must be delimited** — bounded by something other than a letter,
  a digit or an underscore. Without that, every hex digest and long numeric
  identifier produces hits at the checksum's own rate. It also means a UUID can
  never match any rule: its groups are 8, 4, 4, 4 and 12 characters, so no
  delimited run of 9, 10 or 11 digits occurs in one, and the opaque subject
  pseudonym the boundary mandates passes unconditionally.
- **`CODE_PHRASE.code_string` is skipped.** A terminology code system's
  identifiers are digit runs by construction, so this one slot is where the
  collision is systematic rather than incidental. Measured over this
  repository's vendored corpora: of the 30 distinct nine-digit values that
  satisfy the Dutch elfproef, 27 are SNOMED CT concept identifiers, all of them
  in a code slot. The carve-out names an RM slot, not a country.
- **Each rule narrows itself with whatever structure it has.** Two independent
  control digits (`no-fodselsnummer`, 1 in 120), an embedded date
  (`se-personnummer`, 1 in 139 with the Luhn check), a non-numeric token shape
  (`fi-hetu`, 1 in 31 among tokens of that shape). `nl-bsn` at 1 in 11 is the
  loosest shipped rule, which is why the code-slot carve-out matters most to
  it. Every figure here is measured, over 100 000 random tokens of each rule's
  own shape, by a test that fails if a rule drifts from the rate it publishes.

A deployment measuring its own content runs `warn` first, reads the recorded
findings, then moves to `strict`.

### Local patterns

Medical-record numbers, payer references and postcode forms have no single
issuing register to transcribe, so a deployment declares them itself as Rust
regular expressions:

```toml
[privacy.identifier_scan]
patterns = [
  "\\bMRN-[0-9]{6}\\b",
  "(^|[^0-9A-Za-z])[1-9][0-9]{3} ?[A-Z]{2}[^0-9A-Za-z]{1,3}[0-9]{1,4}([^0-9A-Za-z]|$)",
]
```

The second refuses a Dutch postcode paired with a house number — the pair is
what identifies a household; a postcode alone does not. A pattern that does not
compile is a boot error naming it.

## `[cohort]`

Cross-domain **cohort queries**: select a population in the demographic domain,
resolve it to the EHRs whose `EHR_STATUS` subject names it, and run AQL over
exactly those EHRs. A FerroEHR extension — no openEHR spec governs it. The wire contract and a
worked example are in [Querying with AQL](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary).

**Off until a predicate is bound.** Which demographic leaf may be selected on is
a deployment fact about the archetypes in use, never something the server can
infer, so `[cohort.predicates]` is empty by default and `POST /query/cohort`
answers `404` until it is not. `subjects = true` turns the surface on without
a predicate, for cohorts that name their subjects by identifier. That is the one
form an IHE patient identity source serves (`[ehr_index] provider = "ihe"`):
it holds no party-to-EHR map, so a predicate is refused there.

```toml
[cohort]
small_cell_threshold = 5
max_cohort_size = 100000

[cohort.predicates]
city = { archetype = "openEHR-DEMOGRAPHIC-ADDRESS.address.v1", node = "at0012", kind = "text" }
```

| Key | Type | Default | Description |
|---|---|---|---|
| `small_cell_threshold` | int | `5` | The distinct-EHR floor a result set must reach to be served. Below it the rows are withheld and the response is marked `suppressed`; `0` disables suppression. |
| `max_cohort_size` | int | `100000` | The largest cohort a predicate may select. A wider one is refused `422` rather than truncated: a silently shortened cohort is a wrong denominator. |
| `predicates` | table | empty | The allow-list, keyed by the name a caller uses. Empty, with `subjects` off, leaves the surface off. |
| `subjects` | bool | `false` | Let a request name its subjects by identifier, in any identifier domain, each resolved through the `[ehr_index]` provider: under the IHE provider by a forward PIXm translation into the server's clinical domain, never back. |

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
`[demographic.identifier_protection]` decides how they are held there.

With it on, an identifier of a configured scheme never sits in the versioned
body. The value moves to `party.national_identifier`, sealed with
AES-256-GCM under a key derived per domain, and the body keeps a reference in
its place. Beside the ciphertext sits an HMAC-SHA-256 digest of the value, which
is what makes "which party holds this identifier" answerable without decrypting
anything — and, because it is keyed, what stops the database, a backup or a read
replica from reversing a nine-digit space.

```toml
[demographic.identifier_protection]
enabled = false
schemes = ["nl-bsn"]
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
  receives carries the reference; the value is available through the resolution
  path below. A create or update response serves the stored form as well, so
  the value you submitted does not come back in it.
- **Each party version keeps its own sealed rows.** A later version retires
  the rows of the one before it without rewriting them, so an older version
  still expands to the value it committed. Only live rows take part in
  resolution.
- **Resolution is audited.** Going from an identifier to a party is recorded as
  a `linkage`-domain access naming the scheme and whether it matched — never the
  value. A miss is recorded too: it says someone asked whether this deployment
  holds that identifier.
- **Reading a value back is audited and needs its own role.**
  `GET /ferroehr/rest/extensions/v1/admin/party/identifiers` serves a party version with its values
  put back, for a caller holding both the admin role and
  `authz.rbac.identifier_reader_role`, and records each scheme it expands
  ([reading a party's protected identifiers](../operations-admin-apis.md#reading-a-partys-protected-identifiers)).
- **The key is load-bearing and rotation is a re-encryption.** The runbook is in
  [Operations](../operations.md#rotating-the-national-identifier-key).

Only the demographic writer role reaches the sealed value. The read-only twin
sees that a party holds a protected identifier and which party it is, and is
refused the ciphertext and the digest by column-level grant; the clinical roles
are refused the table outright.
