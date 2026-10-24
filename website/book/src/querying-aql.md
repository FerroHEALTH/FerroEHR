# Querying with AQL

The **Archetype Query Language (AQL 1.1)** is how you read data out of FerroEHR.
Instead of querying hidden database tables, you query the clinical model
directly: you name the RM types and archetypes you want, express structural
nesting with `CONTAINS`, and select values by their path within an archetype. The
same query runs unchanged on any conformant openEHR system. This chapter is a
practical walkthrough: the language, how to run queries over HTTP, parameters,
stored queries, version scope, terminology, pagination and limits, and the
supported feature envelope.

<!-- toc -->

## The shape of a query

An AQL statement has the familiar `SELECT … FROM … WHERE … ORDER BY` skeleton,
but the "tables" are RM types and the "columns" are archetype paths:

```text
SELECT
    c/name/value AS composition_name,
    o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude AS systolic
FROM EHR e
    CONTAINS COMPOSITION c
        CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]
WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude > 140
ORDER BY systolic DESC
```

- **`FROM`** binds variables to RM types (`EHR e`, `COMPOSITION c`,
  `OBSERVATION o`). A type can be constrained by archetype id in square brackets
  (`OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]`). Naming a
  **parent** archetype also returns data recorded under its specialisations, as
  openEHR requires: for ADL 1.4 identifiers the specialisation is the
  hyphen-extended concept (`…blood_pressure` matches `…blood_pressure-cuff`),
  and for ADL 2 identifiers (where the hyphen carries no such meaning) the
  lineage is read from the ADL 2 archetypes and templates you have uploaded, so
  the parent matches every stored specialisation of it whatever its concept is
  named. In both cases the major version is a hard boundary: `.v1` never matches
  `.v2` data.
- **`CONTAINS`** expresses structural containment: "an EHR that contains a
  composition that contains a blood-pressure observation". Chains can nest
  several deep, and combine with `AND`, `OR`, and `NOT`. Folder containment
  follows the RM's reference model: `FOLDER f CONTAINS COMPOSITION c` matches
  the compositions a folder's `items` reference, transitively over the
  folder's sub-tree, and `FOLDER f1 CONTAINS FOLDER f2` matches strict
  sub-folders plus the folders of a versioned folder the `items` reference —
  one reference hop, so a chain of references is expressed by chaining
  `CONTAINS`. A pair the RM defines no containment relationship for (say
  `COMPOSITION CONTAINS COMPOSITION`) is refused with a typed error.
  Containment is strict: a node does not contain itself, so
  `CLUSTER cl1 CONTAINS CLUSTER cl2` binds only clusters nested inside
  another cluster. A variable declared inside an `OR` branch can be read in
  `SELECT`, `WHERE`, and `ORDER BY`: a row where its branch did not match
  reads `null` for it, and when both branches match, each match of one pairs
  with each match of the other. This holds for a branch that is a versioned
  object of its own too: `EHR e CONTAINS (COMPOSITION c1[...] OR COMPOSITION
  c2[...])` returns the current version of each matching composition, its
  `c1/uid/value` included, and a `FOLDER` branch reads the compositions the
  folder references.
- **`SELECT`** projects values by path. Paths use archetype node ids (`at0004`)
  and RM attribute names (`value/magnitude`); `AS` names a column. A path
  through a repeated node returns one row per node it reaches: selecting the
  systolic and the diastolic of a blood-pressure `EVENT` returns one row per
  event, each pairing that event's own two values, with `null` where an
  event lacks one. Paths that share a prefix share that node in a row, and a
  path that reaches nothing reads `null` without removing the row. The AQL
  specification does not define this row shape; it is this engine's
  documented convention.
- **`WHERE`** filters on typed leaf values, with comparisons, `EXISTS`, `LIKE`,
  `MATCHES`, and boolean combinators. The AQL specification does not say how a
  test behaves on a path that reaches several nodes; this engine's rule is:
  - Every repeated step a `WHERE` path descends, and the `SELECT` has not
    already bound, is bound **once for the whole `WHERE` clause**, the way a
    `SELECT` path binds it: one node per binding, and one `null` binding when
    nothing matches. Steps written identically (predicate included) are one
    binding, so `events[at0006]` and `events` are two.
  - A row is kept when **some** assignment of those bindings makes the whole
    clause true, with SQL's three-valued logic: a test of a `null` value is
    unknown, and an unknown clause does not keep the row.
  - A path that shares a prefix with a selected path reads the node that row
    is bound to.

  The boolean laws therefore hold. `<systolic> >= 140 AND <diastolic> >= 90`
  holds only for an event carrying both values, also under `OR` and `NOT`,
  whether or not the `SELECT` reads the event; `NOT (<systolic> > 140)` holds
  when some event's systolic is 140 or lower, and is unknown when the path
  reaches nothing; `NOT (A OR B)` equals `NOT A AND NOT B`; adding a test with
  `AND` never adds rows. Paths compared with each other, and paths inside a
  function call, read through the same bindings. Elements of a list attribute
  inside one node's data (`links`, `participations`, `identifiers`, ...) are
  not bindings: a test of such a path against a value holds when **any**
  element of the bound node satisfies it, and is unknown when the list is
  empty or absent, so `NOT (c/links/target/value = 'x')` drops a composition
  without links. Beside another path or inside a function, such a path reads
  its first element.

  `EXISTS <path>` asks whether the data at that path exists (the AQL
  specification's own definition). It reads through the `SELECT` bindings,
  and otherwise looks at the whole path on its own, outside the clause's
  bindings: `EXISTS` holds when any node on the path carries the data and
  `NOT EXISTS` when none does, whatever the other tests bind.

  Projecting a path that crosses a list attribute inside a node's data returns
  every match as one JSON array cell, and `null` where nothing matches. Paths
  through different repeated steps of one variable are bound independently, so
  selecting both returns every combination of their nodes.
- **Aggregates read every data item of their path** in the rows `FROM` and
  `WHERE` select: `MAX(<systolic>)` over an observation measured twice sees
  both events, and `COUNT` over a list attribute counts its elements. An item
  counts when some assignment of the `WHERE` bindings, with the steps the
  item shares fixed to its own nodes, makes the clause true, so
  `SELECT COUNT(<systolic>) ... WHERE NOT (<systolic> < 140)` counts exactly
  the readings that selecting `<systolic>` would return. List elements inside
  one node are not filtered one by one: `COUNT(<links path>)` under a `WHERE`
  on that path counts every link of a row the clause keeps. An aggregate adds
  no rows, so `COUNT(*)` counts the rows the `WHERE` clause keeps whatever
  other aggregates sit beside it. `COUNT(DISTINCT)` compares numbers by value
  (`120` and `120.0` are one value), a stored JSON `null` counts as no value,
  and `AVG` returns a decimal mean.
- **`ORDER BY`**, **`LIMIT`**, and **`OFFSET`** behave as you expect; quantities
  order by their openEHR magnitude semantics.
- **Date/time comparisons accept reduced precision** on both sides: openEHR
  admits partial values (`2019`, `1985-06`), and the engine compares them by
  flooring to the first instant they contain (a partial date assumes the first
  month/day, a partial time assumes zero). A comparison value that is not an
  ISO 8601 date/time at all is refused with a 400.

## Running a query over HTTP

The query API lives under the base path at `/query/aql`. The simplest form is a
`POST` with a JSON body:

```shell
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/json' \
  -d '{"q":"SELECT e/ehr_id/value FROM EHR e"}' \
  http://localhost:8080/ferroehr/rest/openehr/v1/query/aql
```

The body fields are:

| Field | Meaning |
|---|---|
| `q` | The AQL text (**required**). |
| `offset` | Rows to skip (default 0). |
| `fetch` | Maximum rows to return. |
| `query_parameters` | An object of named parameter values (see below). |

There is also a `GET /query/aql` form taking `q`, `offset`, `fetch`, an optional
`ehr_id`, and `query_parameters` as query-string parameters, convenient for
simple, cacheable reads.

`offset`, `fetch`, `ehr_id` and named parameters are accepted in the query string
on the `POST` forms too. A value supplied in **both** the body and the URL must
agree; a disagreement is a **400 Bad Request** rather than a silent choice
between them.

The query API is **JSON only** (`Accept: application/json`).

### Scoping to an EHR

You can restrict a query to one EHR without writing the constraint into the AQL:
pass an `ehr_id` query-string parameter, or the `openehr-ehr-id` request header.
Both forms work on **every** execution endpoint: ad-hoc and stored, `GET` and
`POST` alike. (`openEHR-EHR-id` is the deprecated spelling of the same header and
still resolves, HTTP header names being case-insensitive.)

If a request carries **both** forms they must name the **same** EHR; a request
whose parameter and header name **different** EHRs is self-contradictory and is
rejected with a **400 Bad Request**. So is a request repeating the header with
two different values. An empty header value counts as "not supplied" and
conflicts with nothing.

The id must exist: a **malformed** id is a **400**, and a well-formed id that
matches **no EHR** is an honest **404 Not Found** rather than an empty result
set, so a typo cannot masquerade as "no data".

## The result set

A query returns a `RESULT_SET`: a description of the columns and an array of row
tuples.

```json
{
  "meta": {
    "_type": "RESULTSET",
    "_schema_version": "1.0.0",
    "_created": "…",
    "_executed_aql": "SELECT e/ehr_id/value FROM EHR e"
  },
  "q": "SELECT e/ehr_id/value FROM EHR e",
  "columns": [
    { "name": "#0", "path": "/ehr_id/value" }
  ],
  "rows": [
    [ "7d44b88c-4199-4bad-9764-5da0e2a97441" ],
    [ "b1e2c3d4-5678-90ab-cdef-1234567890ab" ]
  ]
}
```

Each entry in `columns` names the column (the `AS` alias, or `#<index>` when you
did not alias it) and its `path`. Each row in `rows` is an array of cells, one
per column in column order. A cell can be a scalar or a full RM object (for
example `{"_type":"DV_TEXT","value":"Labs"}`) depending on what you selected. A
stored-query execution additionally carries the query's `name`.

The `meta` block's `_executed_aql` field is the AQL the server actually ran, with
your named parameters substituted in as literals. Paste it straight back into an
ad-hoc query when debugging a parameterised call. The top-level `q` keeps the
text exactly as you submitted it, and `_created` stamps when this response was
produced.

Query responses carry a weak **`ETag`** that is a content digest of the result
set: two runs returning identical results carry the identical tag, so a client
can cheaply detect "nothing changed" between polls. The digest deliberately
covers the query, the executed AQL, the columns and the rows, and **not** the
per-response `_created` stamp, so an unchanged result does not mint a new tag
every second.

## Parameters

Parameterise a query with named placeholders (a name preceded by a dollar sign)
and supply the values in `query_parameters`. This is the safe way to inject
values, with no string concatenation:

```shell
curl -u ferroehr:ferroehr -H 'Content-Type: application/json' -d '{
  "q": "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = $name",
  "query_parameters": { "name": "Vital signs" }
}' http://localhost:8080/ferroehr/rest/openehr/v1/query/aql
```

On the URL, each parameter can also be its **own query-string key**, which is
the form the openEHR request documentation shows and is usually easier to build
by hand:

```shell
curl -u ferroehr:ferroehr --get \
  --data-urlencode 'q=SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = $name' \
  --data-urlencode 'name=Vital signs' \
  http://localhost:8080/ferroehr/rest/openehr/v1/query/aql
```

Four details about the URL form: the leading `$` is optional (`$name` and `name`
bind the same parameter); a value in double quotes binds the JSON string it
denotes, so `uid="123"` binds the text `123`, as `{"uid": "123"}` does in a
`POST` body (a value that only starts with a quote keeps its text); any other
value that parses as a JSON scalar binds as that type (`36` as a number, `true`
as a boolean) while everything else binds as text; and the reserved keys `q`, `offset`, `fetch`, `ehr_id` and `query_parameters` are
request controls, never parameters. Where the same name appears both as its own
key and inside `query_parameters`, the named key wins.

## Stored queries

You can register a query once, under a qualified name and version, and execute it
by name later. Storing is done through the definition API with the AQL as a
plain-text body; executing is done through the query API.

```shell
# Store a query as org.example::bp_over, version 1.0.0
curl -u ferroehr:ferroehr -X PUT -i \
  -H 'Content-Type: text/plain' \
  --data-binary 'SELECT o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/query/org.example::bp_over/1.0.0

# List and fetch stored queries
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/query/org.example::bp_over

# Execute it
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/query/org.example::bp_over/1.0.0
```

**Storing.** Both store forms answer **200 OK** with an empty body and a
`Location` header naming exactly the version that was written:

- `PUT /definition/query/{name}/{version}` stores at that exact SemVer. The pair
  is immutable (re-storing an existing `(name, version)` is a **409 Conflict**,
  never an overwrite) and a partial or malformed version segment is a **400**.
- `PUT /definition/query/{name}` (no version) stores or updates at the default
  version `1.0.0`.

The body must be `text/plain`: declaring another media type is a **415**, and an
absent `Content-Type` reads as the plain-text body. The AQL is parsed at store
time, so a syntactically invalid query is rejected **400**; the server never
stores a query it cannot execute. An optional `query_type` parameter names the
formalism, default `AQL` (case-insensitive); anything else is rejected **400**
with an honest "unsupported formalism" message rather than a misleading "invalid
AQL".

**Naming.** The qualified name is `[{namespace}::]{query-name}`, and the
three-part `{namespace}::{formalism}::{query-name}` form is recognised as well.
The namespace is optional: a bare name is stored under the assumed namespace
`misc`, so `my_compositions` and `misc::my_compositions` are the same query and
listings show the qualified form. Identity is case-insensitive while the casing
you stored is preserved. The query-name `aql` is **reserved** (case-insensitive; it would collide with
the ad-hoc `/query/aql` route) and is rejected with a
**400**.

**Reading and executing.**

- `GET /definition/query/{name}` lists the queries under that name as a prefix
  pattern; `GET /definition/query` (no name at all) lists every stored query, a
  FerroEHR convenience, since the openEHR API defines only the named form.
- `GET /definition/query/{name}/{version}` fetches one, by exact SemVer or by
  prefix (`1`, `1.0` → the highest matching stored version).
- `GET|POST /query/{name}` executes the **latest** version;
  `GET|POST /query/{name}/{version}` executes that version, again by exact SemVer
  or prefix. Both take the same `offset`, `fetch`, `ehr_id` and named parameters
  as ad-hoc queries; a `POST` body of `{}` executes a parameterless stored query.

Deleting a stored query is an admin operation; see
[Admin & messaging APIs](operations-admin-apis.md).

## Cohort queries across the pseudonymisation boundary

Some questions are about a population rather than a patient: the blood pressures
of everyone in a city, the medication history of an age band, the readmissions
of the people one organisation cares for. Answering them normally means someone
producing a list of patient identifiers first, which is exactly the disclosure
the split between clinical and demographic storage exists to avoid.

`POST /query/cohort` answers them without producing that list. It is a FerroEHR
extension — no openEHR spec governs it: AQL is defined over the clinical domain
and has no demographic source, and ITS-REST publishes an ad-hoc and a stored
query and nothing else.

**How it runs.** Three steps, each on its own database credential:

1. A predicate from the deployment's allow-list selects **parties** in the
   demographic domain. It reads one archetype leaf and returns identifiers,
   nothing else — no name, address or national identifier leaves that domain.
2. The linkage domain resolves those parties to the **EHRs** they are the
   subject of. Identifiers in, identifiers out.
3. Your AQL runs on the clinical pool, scoped to exactly those EHRs.

The three statements never meet. With `[storage.party] url` and `[storage.linkage] url`
set, each pool authenticates as a role revoked from the other two, and the
server refuses to boot if one can read across. You never receive a party
identifier: the response carries what your AQL projected, plus counts.

**The allow-list.** A cohort predicate can only name something the deployment
declared, under `[cohort.predicates]` — the surface is off until at least one is
bound, and the route answers `404` until then. Five keys are bindable:
`city`, `postcode_area`, `sex`, `age_band` and `organisation`. Each binding
names the archetype of the leaf's nearest archetyped ancestor, the leaf's
at-code, and how a value is matched. See
[Privacy & data minimisation](installation/config-privacy.md#cohort) for the
keys and their defaults.

**Running one.**

```shell
curl -u ferroehr:ferroehr -X POST \
  -H 'Content-Type: application/json' \
  -d '{
        "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
        "cohort": [{ "name": "city", "value": "Groningen" }],
        "purpose": "secondary-use",
        "fetch": 100
      }' \
  http://localhost:8080/ferroehr/rest/openehr/v1/query/cohort
```

`q`, `offset`, `fetch` and `query_parameters` mean what they mean on the ad-hoc
body. There is no `ehr_id`: the cohort **is** the scope, and supplying one is a
`400`. Several `cohort` entries **intersect** — city *and* sex, not city *or*
sex.

The response is an ordinary `RESULT_SET` with one extra block:

```json
{
  "meta": {
    "_type": "RESULTSET",
    "cohort": {
      "size": 412,
      "served_ehrs": 388,
      "suppressed": false,
      "definition": "9f2c…"
    }
  },
  "rows": [["8849fd1e-…::local::1"]]
}
```

`size` is how many EHRs the cohort resolved to, `served_ehrs` how many the rows
actually came from, and `definition` a SHA-256 digest of the predicate list —
the cohort's identity, carrying none of its values.

**Small-cell suppression.** A result set narrow enough to name one person
re-identifies that person out of content you hold no individual entitlement to.
When fewer EHRs are served than `cohort.small_cell_threshold` (default `5`), the
rows are withheld and `suppressed` is `true`. The response says so rather than
looking like an empty cohort: a caller who cannot tell the two apart will keep
widening the predicate until the boundary leaks. Set the threshold to `0` to
disable suppression.

**The access record.** Every execution — suppressed and failed ones included —
records one `linkage`-domain access event naming the cohort by its digest, the
purpose you declared, and how many EHRs were served. A crossing of this boundary
that nobody can reconstruct afterwards is what the access log exists to prevent.
The predicate values are never recorded: they *are* the cohort, and a trail
carrying them would be a second copy of it.

**Limits.** A predicate matching more than `cohort.max_cohort_size` parties
(default `100000`) is refused `422` rather than truncated — a silently shortened
cohort is a wrong denominator with nothing on the wire to say so. An unbound
predicate name, a malformed age band (the form is `<min>-<max>` whole years,
both inclusive) or a request with no predicate at all is a `400`. An empty
cohort runs no AQL and answers an empty result set.

**Measured.** The committed record `docs/benchmarks/cohort/record.json`
in the repository (produced by the ignored `cohort_bench` test, not by the
conformance instrument) measures the whole call, predicate to result set, over
a corpus of 100 000 parties, each the subject of one EHR holding one
composition, seeded through the service API, with the database in a local
container. Selecting one composition path per EHR:

{{#include ../generated/cohort-bench.md}}

The difference between the statement times and the wall clock is result
assembly and the access records. Both statements run on indexes (`idx_dem_node_archetype` on the demographic side, the current-version
and EHR indexes on the clinical side); no sequential scan appears in any of the
recorded plans. The record names the commit it was measured at; a re-run
replaces it rather than appending.

## Queries rooted at a demographic class

The Demographic API reads a party by its identifier and has no search. To find
a person, an organisation or a role by what it holds, such as an identifier in
its `details`, FerroEHR serves a subset of the experimental AQL demographic
extension on the same `POST /query/aql` and `GET /query/aql` routes. No openEHR
specification governs it: QUERY 1.1 lets `FROM` name any Reference Model class
but gives semantics only to the clinical repository, so everything below is
FerroEHR's own design, and the extension may change.

```
SELECT p/uid/value
FROM PERSON p
WHERE p/details[at0001]/items[at0002]/value/id = $identifier
ORDER BY p/uid/value
```

**The grammar served.** The query is ordinary AQL 1.1 syntax with these rules on
the `FROM` clause:

- The outermost class is `PERSON`, `ORGANISATION` or `ROLE`, optionally under a
  `VERSION` class. It is the only demographic class at the root of the query.
- Below it, `CONTAINS` may name the content classes a party holds: `CLUSTER`,
  `ELEMENT`, the item structures, and the party containers `PARTY_IDENTITY`,
  `CONTACT`, `ADDRESS` and `CAPABILITY`, with archetype and node predicates as
  usual, for example
  `CONTAINS CLUSTER c[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2]`.
- `SELECT`, `WHERE` and `ORDER BY` take paths inside the party: its `details`
  tree, its `identities`, `contacts` and `capabilities`, `p/uid/value`, and
  the whole party (`SELECT p`). Named parameters, `LIMIT`/`OFFSET`, the REST
  `offset` and `fetch`, and the functions and aggregates of the clinical
  envelope apply unchanged.

**What a row is.** The query reads the latest trunk version of each party. A
logically deleted party, and a party under a restriction of processing, answers
no row. `p/uid/value` is the version identifier the Demographic API returns.

**What is refused.** Each of these is a **400 Bad Request** naming the
construct, so you can tell a refusal from an empty result:

- `GROUP`, `AGENT`, `PARTY_RELATIONSHIP`, and the abstract `PARTY` and `ACTOR`;
- a demographic class anywhere but the root, or a second one;
- a query that names `EHR` or a clinical versioned object such as
  `COMPOSITION` anywhere but directly under the root (see the join below);
- an `ehr_id` scope (query parameter or `openehr-ehr-id` header), because a
  party is in no EHR.

**Joining a party to its EHR.** An `EHR` operand directly under the root
reaches the EHR the party is the subject of, through the party-to-EHR map the
linkage service keeps:

```
SELECT p/uid/value, c/name/value
FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1]
WHERE p/details[at0001]/items[at0002]/value/id = $identifier
ORDER BY c/context/start_time/value DESC
```

Party classes may sit beside the `EHR` in an `AND`
(`PERSON p CONTAINS (CLUSTER id[…] AND EHR e CONTAINS COMPOSITION c)`). A row
is a party row together with a clinical row of an EHR that party is the subject
of, so a party with no EHR, and an EHR whose subject has no party, answer no
row. FerroEHR runs the query as three statements, one on each database
credential: the party half, the map, and the clinical half scoped to the EHRs
the map names. It then joins, orders and pages the rows. The rules that follow:

- each column, each `WHERE` condition joined by `AND`, and each `ORDER BY` key
  reads one side only, the party or the EHR; a condition comparing the two, or
  an `OR` across them, is a **400**;
- aggregates and `SELECT DISTINCT` are a **400**;
- each half, and the joined rows, may hold at most `query.max_result_rows`
  rows; above that the query is a **400** asking you to narrow it, never a
  shortened answer;
- the clinical half carries your per-EHR access decision and patient scope as
  any query does, and the party read and the map read each leave an access
  record in their own domain.

**Who may run it.** The query runs under the authorisation of a Demographic API
read: the role check that admits a party read admits the query, and the per-EHR
access decision and the attribute-based patient scope, which govern EHR
content, do not apply to parties. The SMART `aql-…` scopes that govern every
query still apply. The query runs on the demographic database credential, and
its access record is filed under the demographic domain with the number of rows
served.

## Version scope: LATEST_VERSION and ALL_VERSIONS

By default a query sees the **latest** version of each object. FerroEHR also
supports querying the **entire version history**, a capability many CDRs lack.
Wrap a source in `VERSION` and choose the scope:

```text
SELECT v/commit_audit/time_committed, c/name/value
FROM EHR e
    CONTAINS VERSION v[ALL_VERSIONS]
        CONTAINS COMPOSITION c
```

`LATEST_VERSION` (the default) reads only current trunk versions;
`ALL_VERSIONS` reads across history (including branch versions, where a version
tree has any) so you can see how a record changed over time. A version
predicate on `commit_audit/time_committed` reads the trunk version current at
that instant. The `VERSION` variable also exposes commit metadata: the audit, the
committed time, and the version uid.

## Terminology in queries

Value filters can be backed by terminology in three ways:

- `TERMINOLOGY('expand', …)` as (or inside) a `matches` operand expands a value
  set so a coded field matches any code in it, rather than listing codes by
  hand;
- `TERMINOLOGY('validate'|'subsumes', …) = true` as a boolean condition
  evaluates a code-membership or subsumption test once per query;
- a terminology URI operand (`matches { terminology://… }`) expands the set the
  URI identifies.

These require a terminology source; if no terminology server is configured,
the in-process openEHR bundle is used. See
[Terminology servers](beyond-core/terminology.md) for wiring the FHIR
terminology server.

Two rejections to expect here: only `expand` may stand as a `matches` operand
(the other operations have no value-list meaning), and only `validate` and
`subsumes` have boolean semantics, so `TERMINOLOGY('lookup', …) = true` is
refused rather than guessed at. A value set the configured server does not know
is a **400** naming it; a terminology server that fails mid-query is a **500**,
kept distinct from a bad query on purpose.

## Pagination and limits

Combine `LIMIT`/`OFFSET` in the AQL with the `fetch`/`offset` request parameters
to page through large result sets. Three bounds interact, and it is worth knowing
which one you hit:

| Bound | Where it comes from | Effect |
|---|---|---|
| `LIMIT`/`OFFSET`, or `fetch`/`offset` | your query or request | Exactly the window you asked for. |
| [`query.max_result_rows`](installation/config-integrations.md#query) | server config, default `10000` | The largest page one execution serves: the page of a query nothing else bounds, and the maximum a `LIMIT` or `fetch` may ask for. `0` means unbounded. |
| [`query.timeout_ms`](installation/config-integrations.md#query) | server config, default `30000` | Per-query database execution budget. **On by default**; `0` disables it. |

A query that exceeds the time budget returns **408 Request Timeout**; narrow it
(add archetype constraints, an `ehr_id` scope, or a `WHERE` filter) rather than
retrying it unchanged. A `LIMIT` or `fetch` larger than the row ceiling returns
**400 Bad Request** naming the ceiling; the page is never silently shortened,
because a client paging with its own `fetch` as the stride would skip rows
without noticing. Page with `offset` and a `fetch` at or below the ceiling. A
result that stops at the ceiling without either bound set is the default page:
page explicitly to read past it.

### Always order a query you intend to page

`OFFSET` skips rows of a result, and without `ORDER BY` a result has no defined
row order. Two requests for consecutive pages can then repeat a row or miss one
entirely, and nothing in the response says so. Give any query you page an
`ORDER BY` on something unique, such as `c/uid/value`.

Ordering also changes what paging costs, in the direction that helps you. An
ordered query sorts the whole matched set to answer any page, so the first page
already pays most of the work and a deep page costs little more than a shallow
one. An unordered query skips rows one at a time, so its cost grows with the
offset, from almost nothing on page one. Either way what dominates is how much
your `FROM`/`CONTAINS` matched, not how deep you paged.

One more reason to order a paged query, if the deployment runs attribute-based
authorization: the authorization decision is made over every EHR and template
the query would touch, not over the page it served, so that set is collected
whatever the page size. An ordered query pays nothing extra for it, because it
was producing the whole matched set anyway. An unordered page that looks cheap
carries the whole collection on top of it. Narrow the query, and order it.

> [!TIP]
> The more specific your `FROM`/`CONTAINS` (name the archetype, scope by
> `ehr_id`), the faster the query: those constraints map to indexed columns,
> while broad "everything that contains anything" queries do the most work.
> Repeated identical query text reuses a cached plan, so parameterising a
> recurring query beats rewriting its literals.

## What is supported

FerroEHR implements the core AQL 1.1 envelope and rejects out-of-envelope
constructs with an explicit, typed error rather than silently returning wrong
results. Supported today includes:

- `SELECT` of paths, literals, aliases, `DISTINCT`, and the aggregates `COUNT`
  (including `COUNT(DISTINCT)`), `MIN`, `MAX`, `SUM`, `AVG`. `MIN` and `MAX`
  order their operand by type (a quantity by its openEHR magnitude, a date/time
  chronologically, text lexically) so they work over non-numeric leaves;
- `FROM` over `EHR`, `VERSION` (`LATEST_VERSION` / `ALL_VERSIONS`), and the RM
  structure classes, with archetype and name predicates, and over `PERSON`,
  `ORGANISATION` and `ROLE` as the experimental demographic extension;
- `CONTAINS` trees with `AND`, `OR`, and `NOT CONTAINS`;
- `WHERE` comparisons on typed leaves (with openEHR magnitude ordering for
  quantities), `EXISTS`, `LIKE`, `MATCHES` value lists, and range predicates;
- `ORDER BY` typed leaves, `LIMIT`/`OFFSET`, `TOP n`, named query parameters,
  and the `ehr_id`, `offset`, and `fetch` request parameters;
- the single-row functions: `LENGTH`, `SUBSTRING`, `POSITION`, the string
  `CONTAINS`, `CONCAT`/`CONCAT_WS`, `ABS`, `MOD`, `CEIL`, `FLOOR`, `ROUND`, and
  `CURRENT_DATE`/`CURRENT_TIME`/`CURRENT_DATE_TIME`/`NOW`/`CURRENT_TIMEZONE`;
- terminology-backed `TERMINOLOGY()` operands and terminology-URI `matches`
  operands (see above).

### What is refused, and why

Every refusal below is a typed error naming the construct, so you never get a
silently incorrect answer:

- **Demographic forms outside the served subset.** `GROUP`, `AGENT`,
  `PARTY_RELATIONSHIP`, `PARTY` and `ACTOR`, a demographic class below the root,
  and a query mixing a demographic root with `EHR` or clinical content (see
  [Queries rooted at a demographic class](#queries-rooted-at-a-demographic-class)).
- **`TOP … BACKWARD`.** `TOP` is deprecated as of AQL 1.1.0 and the direction
  variant is not implemented; the error carries the specification's own rewrite
  (`ORDER BY <path> DESC LIMIT n`). `TOP` and `LIMIT` in one query are also
  refused; pick one.
- **Regex and `OR` node predicates.** `[{/…/}]` is archetype-definition syntax,
  not AQL value matching, and a disjunctive node predicate is outside the
  accepted subset.
- **Branch version addressing.** A version predicate may address trunk versions;
  reading branch content is done by scoping `ALL_VERSIONS`, not by naming a
  branch.
- **Reading a variable declared under `NOT CONTAINS`.** `NOT CONTAINS` states
  that no such node exists, so a path through its variable has nothing to
  read.
- **`SELECT DISTINCT` ordered by an unselected expression.** De-duplication and
  sorting by something the projection dropped have no defined meaning together;
  sort by a selected column.
- **Analysis failures**, which are precise: an unknown RM
  class or attribute, an unbound `$parameter`, a duplicate `FROM` variable name,
  `LIMIT 0`, a negative `OFFSET`, wrong function arity, `SUM`/`AVG` over a
  non-numeric path, and an `archetype_node_id` criterion that is neither an
  archetype identifier nor a node code. Variable names are case-insensitive, as
  the specification requires.

### The specification generation is an acceptance boundary

Which Reference Model classes and attributes a query may name depends on the
deployment's [`spec_profile`](installation/configuration.md#spec_profile). On the
default `development` profile the query surface is the full RM 1.2.0 model. On
`stable` (RM 1.1.0), a `FROM` class or a path attribute that only a newer
generation defines is refused with **400 Bad Request**, and the message names
both the offending class or attribute and the active profile. A server of that
generation would answer "unknown", so returning rows instead would silently
overclaim the profile the deployment advertises.

Nothing else about querying changes with the profile: paths, predicates, and the
result-set shape are identical.
