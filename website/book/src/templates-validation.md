# Templates & validation

A template is what tells FerroEHR what clinical data to accept. Before you can
commit a composition, you upload the **Operational Template (OPT)** it conforms
to; from that template the server derives everything it needs to validate
incoming data and to describe the data's shape to client applications. This
chapter covers uploading and retrieving templates in both ADL generations, the
derived WebTemplate, the convenience FLAT and STRUCTURED composition formats, and
how validation behaves on commit. If templates and archetypes are new to you,
read the [openEHR primer](concepts/openehr-primer.md) first.

<!-- toc -->

## Uploading an OPT 1.4 template

FerroEHR ingests templates in the **OPT 1.4** XML format. Upload one with
`Content-Type: application/xml`:

```shell
curl -u ferroehr:ferroehr -i \
  -H 'Content-Type: application/xml' \
  --data-binary @vital_signs.opt \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4
```

The upload takes OPT XML and nothing else: a request declaring another payload
type (`Content-Type: application/json`, say) is refused with **415 Unsupported
Media Type** before the template is parsed. `text/xml` works the same as
`application/xml`, and omitting the header altogether is fine; the endpoint has
only one body format.

A successful upload returns **201 Created** with the id in `ETag` and
`Location`. Add `Prefer: return=identifier` when you only need the id back; the
response body is then the JSON identifier object:

```json
{ "template_id": "vital_signs" }
```

(`return=representation` returns the stored OPT XML; the default is an empty
body.) Uploading a template whose id already exists returns **409 Conflict**;
templates are immutable once loaded. Template ids are compared
**case-insensitively** (the stored casing is preserved), so uploading a
case-variant of an existing id (`Vital_Signs` against a stored `vital_signs`)
is also a **409 Conflict**, not a second template.

### What "invalid template" means, and which status you get

The upload runs two distinct gates. Both answer **400 Bad Request**, the only
refusal ITS-REST 1.1.0 declares for the upload besides the 409 duplicate:

- **A payload that is not well-formed XML:** the server could not read it as
  XML at all.
- **A well-formed document that is not a valid operational template.** This
  covers both the structural check (foreign or
  duplicated top-level elements, a decoded OPT with an empty `template_id` or
  `concept`) and the full archetype-model artefact-validity catalogue:
  reference-model conformance of every constrained type and attribute,
  occurrence/cardinality consistency, terminology code definedness and language
  consistency, archetype-identifier well-formedness, and constraint pattern
  validity (temporal and duration patterns, boolean satisfiability, assumed
  values inside their own constraints). The response body lists the offending
  archetype-model rule codes (`VCARM`, `VATID`, `VCORM`, `Pattern_validity`, …)
  in the problem's `errors`, each with a human-readable `detail`; a request
  with `Prefer: return=representation` receives them
  ([Problem types](using-the-api/problems.md#validation-failed)).

The second gate also validates the template's **own meta-data** against the RM
resource-package invariants: the template's `language` (and every description
detail's) must be in the openEHR `languages` code set, `is_controlled` must
agree with the presence of a `revision_history`, and a `description` needs a
non-empty `original_author`, `lifecycle_state`, and at least one detail with a
non-empty `purpose`, with details keyed by distinct languages. Those refusals
carry the RM invariant name (for example
`RESOURCE_DESCRIPTION.Lifecycle_state_valid`) in `errors`. ADL 1.4
archetype source uploads enforce the same family; there, an empty
`purpose`/`use`/`misuse` string is reported as a named warning rather than
refused, because the empty string is how real-world 1.4 authoring spells
absence.

**A refusal lists everything it found, not the first thing.** `errors`
carries one entry per violation, so a template with an empty `use` and an empty
`misuse` reports both, and a code list with several duplicated codes names each
one. Fix the list, upload once. Two things are worth knowing about the shape of
that list:

- A constrained type the reference model does not have, or an attribute its
  parent type does not declare, makes every rule below it meaningless. Those
  violations say that the tree under them went unchecked, so a short list is
  never a claim that nothing else is wrong. Fix them and upload again.
- A template with more than 200 violations reports the first 200 and a final
  `TRUNCATED` entry saying so.

### What an OPT 1.4 export must carry

One requirement of the OPT 1.4 XML format catches exported templates. Inside a
`C_DV_ORDINAL`, every `list` entry is a `DV_ORDINAL` whose `symbol` is a
`DV_CODED_TEXT`, and a `DV_CODED_TEXT` carries a `<value>` element beside its
`defining_code`. The element may be empty, `<value/>`: the rubric a user sees is
resolved from the template's `term_definitions`, not from this text. CKM's
exporter writes the empty element, so its templates load. Archetype Designer
omits it, and such a template is refused with **400**, naming the `<symbol>`
element, its line, and the fix. Add `<value/>` to each ordinal symbol and upload
again.

Whether the schema should require the element on an ordinal symbol is a question
for openEHR, tracked at
<https://github.com/FerroHEALTH/FerroEHR/issues/3401>.

## Uploading ADL 2 artefacts

ADL 2 artefacts (archetypes, templates and operational templates) are accepted
as `text/plain` source on `…/definition/template/adl2` and validated by the full
ADL 2 engine: the source is parsed, then checked against the AOM2 validity
catalogue (phase-1 basic integrity, reference-model conformance, and, for a
specialised artefact whose parent is already loaded, specialisation
conformance).

Both failure modes answer **400 Bad Request**, the only refusal ITS-REST 1.1.0
declares for the upload besides the 409 duplicate:

- **A source that does not parse** carries the ADL syntax (`S`-prefixed) error
  codes.
- **A source that parses but fails validation** lists the AOM2 validity
  (`V`-prefixed) rule codes in its `errors`: `VARD`, `VCORM`, `VCARM`, `VACSD`
  and friends.

Every archetype a template or archetype fills a slot with or references
through `use_archetype` must already be loaded (AOM2 rule `VARXR`), so upload
the archetypes before the templates built on them. A template naming one the
CDR does not hold is refused with a 400 whose `errors` carry one `VARXR` entry
per missing archetype id.

A reference may name its archetype down to the major version only
(`openEHR-EHR-OBSERVATION.bp.v1`). With several releases of that archetype
loaded, it resolves to the newest release of that major version (`v1.10.0`
before `v1.9.0`, and never `v2.0.0`), a release before a release candidate. A
reference with a minor or patch version (`.v1.2`, `.v1.2.3`) narrows the match
to it, and an alpha or beta version is matched only by its full id. A
reference to a major version no loaded release carries is a missing archetype.
Two major versions of one archetype are two archetypes, so one template may
reference both (`.v1` and `.v2`), each resolving to its own release. A
reference resolves within one namespace: the one it names, or, when it names
none, the namespace of the archetype or template that makes it.

A duplicate artefact id returns **409 Conflict**.

A loaded ADL 2 template is retrieved in either of two representations, chosen by
`Accept`: the stored ADL 2 **source** (`text/plain`, the default) or the
**operational template as JSON** (`application/json`). `Accept: application/xml`
has no declared body here and is a **406**. A template whose constituent
archetype was deleted after its upload cannot be compiled: its JSON form and
its example answer **422**, and with `Prefer: return=representation` the
`errors` list one `VARXR` entry per missing archetype, its `detail` the
archetype id. Upload the archetype again to restore it. A partial `template_id` resolves to
the latest matching version, and a separate route addresses a version
explicitly:

```shell
# The ADL 2 source, verbatim
curl -u ferroehr:ferroehr -H 'Accept: text/plain' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl2/openEHR-EHR-COMPOSITION.t_vitals.v1.0.0

# The operational template as JSON, latest version matching the partial id
curl -u ferroehr:ferroehr -H 'Accept: application/json' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl2/openEHR-EHR-COMPOSITION.t_vitals.v1

# Resolved inside one SemVer version (exact, or a `{major}` / `{major}.{minor}` prefix)
curl -u ferroehr:ferroehr -H 'Accept: text/plain' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl2/openEHR-EHR-COMPOSITION.t_vitals/1.0
```

A version on that route that is no SemVer and no `{major}` or `{major}.{minor}`
prefix (`v1.x`, `01`) is a **400**; a well-formed version that is not stored is
a **404**.

> [!NOTE]
> The Basic-authenticated examples on this page run against the quickstart,
> whose user holds `user/template-*.cruds`. The template family is
> scope-governed on every deployment: a caller, Basic or bearer, whose scopes
> grant no `template-` access answers `403`. See
> [SMART App Launch](smart-app-launch.md#what-each-route-requires).

## Listing templates

```shell
# List all templates
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4

# Filter and page the list
curl -u ferroehr:ferroehr \
  'http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4?template_id=vital*&offset=0&fetch=20'

# Get the canonical OPT XML for one template
curl -u ferroehr:ferroehr -H 'Accept: application/xml' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4/vital_signs
```

Both list endpoints (ADL 1.4 and ADL 2) accept the same three filters
(`template_id`, `concept`, and `version`, each a glob pattern with `*` wildcards)
plus `offset` (rows to skip, default 0) and `fetch` (maximum rows; absent or 0 =
all). The filters AND together. An offset past the end of the match set is an
empty list, not an error.

> [!NOTE]
> When the `version` parameter is absent, the list collapses to the **latest
> version** of each template (the highest `.vN` axis of its `template_id`). Pass
> `version=*` to list every stored version, or a partial glob (`version=*.v1*`)
> to select specific ones.

## Deleting a template

Deletes are deliberately not part of the openEHR definition API, and FerroEHR
keeps them off the provisioning routes:

- an **OPT 1.4** template is removed with
  `DELETE /ferroehr/rest/extensions/v1/admin/template/{template_id}`,
  behind the admin gate (see
  [Admin & messaging APIs](operations-admin-apis.md));
- an **ADL 2** artefact is removed with
  `DELETE /ferroehr/rest/extensions/v1/definition/artefact/adl2/{artefact_id}`.

Both refuse with **409 Conflict** while any committed version still references
the template, and the message names how many versions still hold the reference.
A physical delete can never orphan committed clinical data. The count spans
archived content too, so an archived composition's template cannot be deleted out
from under it. Delete the compositions first.

The ADL 2 delete also refuses with **409 Conflict** while another stored
artefact still needs the one you delete, and the message names every one of
them. An artefact needs another when it does not compile without it: a
template or archetype that fills a slot with it or references it through
`use_archetype`, a specialised archetype whose parent it is, and every
artefact built on one of those. So an archetype a stored template uses is not
deleted, and a specialised archetype keeps its parent. Delete the dependents
first; an archetype nothing uses deletes at once. A stored
operational template inlines its archetypes and needs none of them.

## Archetype sources (a FerroEHR extension)

Beyond operational templates, FerroEHR can also hold the **source archetypes**
they were built from, so a deployment can keep its definition material in one
place. ITS-REST defines no archetype routes, so they sit under the extensions
root, `/ferroehr/rest/extensions/v1`, beside the openEHR API, and every route
below is relative to it:

| Route | What it does |
|---|---|
| `GET|POST /definition/archetype/adl1.4` | List the stored ADL 1.4 archetype ids; upload one as `text/plain` ADL source |
| `GET|DELETE /definition/archetype/adl1.4/{archetype_id}` | Retrieve the stored source verbatim; remove it |
| `GET /definition/archetype/adl2`, `…/adl2/count` | List the stored ADL 2 archetypes, and count them |
| `GET /definition/artefact/adl2`, `…/adl2/count` | List and count *all* stored ADL 2 artefacts (archetypes, templates, OPTs) |
| `DELETE /definition/artefact/adl2/{artefact_id}` | Remove one ADL 2 artefact no other stored artefact needs |

Properties worth knowing before you build on these:

- **They are our own design.** The openEHR REST API provisions operational
  templates only: it declares no archetype resource, no count, and no delete.
  These routes realize the openEHR *service model*'s archetype operations, which
  the released wire never surfaced, so they are excluded from openEHR wire
  conformance.
- **The archetype id is the client's, never assigned.** An ADL 1.4 upload reads
  it out of the source's own `archetype` header, and a re-upload of the same id
  **replaces** it (there is no conflict branch: the service operation is
  replace-or-create). Success is **201** with the id in the body and a
  `Location`.
- **Ids match case-insensitively**, and a malformed id on a read is simply a
  **404**; nothing is stored under it.
- **Writes respect the read-only role.** A principal carrying the configured
  read-only role is refused **403** before the store is touched. Uploads that do
  not parse or fail the phase-1 validity catalogue are **422**.

## The WebTemplate

The **WebTemplate** is a JSON description of a template that is far easier for
application code to consume than raw OPT XML: it lists every field with its
path, type, cardinality, allowed values, and labels, which is exactly what you
need to render a form or map data. Request it with the WebTemplate media type:

```shell
curl -u ferroehr:ferroehr \
  -H 'Accept: application/openehr.wt+json' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4/vital_signs
```

The WebTemplate is part of openEHR's own **Simplified Formats**
sub-specification of ITS-REST 1.1.0, and FerroEHR emits its metadata format
version `2.3` as that specification defines it, so tooling written against the
standard model works unchanged.

`Accept: application/json` on the same URL returns that identical WebTemplate
document (it is the only JSON representation of a template) but labelled
`Content-Type: application/json`, the type you asked for. Use
`Accept: application/xml` for the canonical OPT instead.

You can also fetch an **example composition** for a template (a skeleton
instance you can fill in) from
`GET /definition/template/adl1.4/{template_id}/example`. The same endpoint exists
for ADL 2 templates at `GET /definition/template/adl2/{template_id}/example`: the
stored operational template is turned into a WebTemplate and walked into an
example composition. Both serve any of canonical JSON/XML, FLAT, or STRUCTURED
(via `Accept`) and take the same two query parameters: `type`
(`input`/`output`) and `detail_level` (`required`/`medium`/`complete`).

The example carries every attribute and node the template makes mandatory, so
it passes the server's own validation. A template whose mandatory content
only an archetype slot admits cannot be filled by any example.

A demographic template (one rooted at `PERSON`, `ORGANISATION`, `GROUP`,
`AGENT` or `ROLE`) gets an example party of that class instead, in canonical
JSON or XML. An example `ROLE` carries a placeholder `performer`, which the
reference model makes mandatory. The Simplified Formats map no party class,
so `Accept: application/openehr.wt.flat+json` or `…structured+json` on a party
example answers `406 Not Acceptable`.

A template whose root is neither a `COMPOSITION` nor a party class (an
OBSERVATION or a SECTION, for example) has no example: the endpoint answers
`400 Bad Request` naming the root, because the example the specification
defines is an instance of a versioned-object class such as `COMPOSITION`,
which such a template does not describe.

## Composition formats

When committing or retrieving a composition, the canonical openEHR JSON (or XML)
is always available, but two flatter formats are offered for convenience, keyed
to a template:

- **FLAT (simSDT):** `application/openehr.wt.flat+json`. The whole composition
  as a single flat map of `path|attribute → value`, which is compact and easy to
  produce from a form. For example:

  ```json
  {
    "vital_signs/blood_pressure/any_event:0/systolic|magnitude": 120,
    "vital_signs/blood_pressure/any_event:0/systolic|unit": "mm[Hg]",
    "vital_signs/blood_pressure/any_event:0/diastolic|magnitude": 80,
    "vital_signs/blood_pressure/any_event:0/diastolic|unit": "mm[Hg]",
    "vital_signs/language|code": "en",
    "vital_signs/language|terminology": "ISO_639-1"
  }
  ```

- **STRUCTURED (structSDT):** `application/openehr.wt.structured+json`. The
  same data as a nested JSON tree that mirrors the template structure, rather
  than a flat map.

Send the matching `Content-Type` when committing, or the matching `Accept` when
retrieving, and the server converts between the flat/structured form and the
canonical composition. Both are defined by openEHR's **Simplified Formats**
sub-specification of ITS-REST 1.1.0. That specification text is the only
authority the server follows here, and there is no vendor-compatibility mode. The
canonical JSON and XML remain the full-fidelity wire format. They work against
both ADL 1.4 and ADL 2 templates: a FLAT or STRUCTURED commit keyed to an
ADL 2-registered template resolves and is validated against that template's
archetype constraints exactly as an ADL 1.4 commit is.

> [!NOTE]
> The FLAT and STRUCTURED formats are always relative to a template: the paths
> are template paths, and the target template comes from the
> `openehr-template-id` request header, without which a commit is a **400**. Use
> them for form-driven capture; use canonical JSON/XML for full-fidelity
> exchange and archival.

### Optional RM attributes (`_`-prefixed keys)

Beyond the template's own fields, FLAT and STRUCTURED carry the optional
reference-model attributes as `_`-prefixed path segments, round-tripping in both
directions. Indexed families take an `:n` suffix; sub-fields ride the usual
`|attribute` pipes:

```json
{
  "vital_signs/blood_pressure/_uid": "9fcc1c70-…",
  "vital_signs/blood_pressure/_link:0|type": "problem",
  "vital_signs/blood_pressure/_link:0|target": "ehr://…",
  "vital_signs/blood_pressure/any_event:0/systolic/_null_flavour|code": "253",
  "vital_signs/blood_pressure/any_event:0/systolic/_normal_range/lower|magnitude": 90,
  "vital_signs/blood_pressure/_other_participation:0|function": "witness",
  "vital_signs/blood_pressure/_other_participation:0|name": "Dr. Marcus Johnson"
}
```

Which families apply depends on the node they hang off:

| Host | Families |
|---|---|
| Any locatable node | `_uid`, `_link:n`, `_feeder_audit` |
| Any entry (observation, evaluation, instruction, action, admin entry) | `_work_flow_id`, `_guideline_id`, `_provider`, `_other_participation:n` |
| An instruction | `_expiry_time`, `_wf_definition` |
| An action | `_instruction_details`, and `_reason:n` on its state transition |
| An element | `_null_flavour`, `_null_reason` |
| A data value | `_mapping:n`, `_normal_range`, `_other_reference_ranges:n`, `_accuracy`, `_language`, `_encoding`, `_charset`, `_thumbnail` |
| Any party reference (a provider, a participation, the subject) | `_identifier:n` |
| The composition's event context | `_health_care_facility`, `_participation:n` |

An ACTION's `_instruction_details` carries exactly three suffixes on the field
itself: the instruction's path within its composition, that composition's uid,
and the activity id:

```json
{
  "encounter/procedure/_instruction_details|path": "/content[openEHR-EHR-INSTRUCTION.request.v1]",
  "encounter/procedure/_instruction_details|composition_uid": "4cdc3017-d8c5-4cd3-9900-f3bb7171d006",
  "encounter/procedure/_instruction_details|activity_id": "activities[at0001]"
}
```

An interval event additionally carries `|sample_count` (the number of samples the
interval summarises) alongside its `/width` and `/math_function` fields.

An element that records *why* a value is missing carries `_null_flavour` (and
optionally `_null_reason`) **instead of** a value (the reference model makes the
two mutually exclusive), and both directions preserve it:

```json
{
  "vital_signs/blood_pressure/any_event:0/systolic/_null_flavour|code": "253",
  "vital_signs/blood_pressure/any_event:0/systolic/_null_flavour|value": "unknown",
  "vital_signs/blood_pressure/any_event:0/systolic/_null_flavour|terminology": "openehr",
  "vital_signs/blood_pressure/any_event:0/systolic/_null_reason": "not asked"
}
```

### Who the entry is about: `subject`

Every entry (observation, evaluation, instruction, action, admin entry) records a
subject. It defaults to the owner of the EHR, and while it stays the default it
does not appear on the wire at all. When an entry is about someone else (a
relative, a donor, a fetus) spell it out with the party suffixes:

```json
{
  "family_history/family_history/subject|name": "Susan Doe",
  "family_history/family_history/subject|id": "199",
  "family_history/family_history/subject|id_scheme": "HOSPITAL-NS",
  "family_history/family_history/subject|id_namespace": "HOSPITAL-NS",
  "family_history/family_history/subject/relationship|code": "10",
  "family_history/family_history/subject/relationship|value": "mother",
  "family_history/family_history/subject/relationship|terminology": "openehr"
}
```

A `/relationship` sub-path makes the subject a *related party*; additional
identifiers ride the `/_identifier:n` family; `|_type: "PARTY_SELF"` marks a
subject that is the EHR owner yet still carries an external reference.

### Context fields: `ctx/` shortcut or full path

The composition's event context is normally set through the `ctx/` shortcuts
(`ctx/time`, `ctx/setting`, `ctx/language`, …). The equivalent full paths are
accepted on input too, and take precedence over the shortcut defaults:
`…/context/start_time`, `…/context/setting`, the `_end_time` and `_location`
forms, and an entry's `…/language` and `…/encoding`. A bare
`…/context/setting|code` is resolved against the openEHR *setting* value set,
exactly as `ctx/setting` is. A full path also feeds the defaults its shortcut
feeds: `…/context/start_time` sets the default history origin of every
observation and the time of every event that gives none, exactly as `ctx/time`
does. When you give both spellings, the full path sets the stored field and the
`ctx/` key sets the defaults. Output always uses the `ctx/` form for these, so a
round trip through FLAT is stable. A path the Simplified Formats specification
does not define is rejected with an error, never silently dropped.

### Embedding canonical JSON with `|raw`

When one node needs full fidelity inside an otherwise-FLAT commit, write the
node's canonical JSON verbatim under the `|raw` suffix:

```json
{
  "vital_signs/blood_pressure/any_event:0/systolic|raw": {
    "_type": "DV_QUANTITY", "magnitude": 120, "unit": "mm[Hg]"
  }
}
```

The embedded object **must** carry `_type` (without it, the key is treated as a
normal leaf). Retrieval decomposes a value into regular FLAT keys wherever the
Simplified Formats define them, and returns `|raw` only for a value they cannot
express: a stored value whose type differs from the template's, or a
`DV_STATE`.

### `DV_STATE` has no FLAT form

The Simplified Formats RM mapping chapter has a table for each data value type
it maps, and none for `DV_STATE` (RM `value: DV_CODED_TEXT`, `is_terminal:
Boolean`). No specification governs a FLAT or STRUCTURED spelling for it, and
FerroEHR invents none: a write that decomposes a `DV_STATE` node (`|code`,
`|value`, a bare value) is refused with an error that names the node, and
the node is sent and returned as `|raw` canonical JSON instead:

```json
{
  "my_template/my_observation:0/process_state|raw": {
    "_type": "DV_STATE",
    "value": {
      "_type": "DV_CODED_TEXT", "value": "completed",
      "defining_code": {
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
        "code_string": "at0012"
      }
    },
    "is_terminal": true
  }
}
```

The FLAT and STRUCTURED examples leave a `DV_STATE` node out, and an element
whose value the template leaves open does not offer `DV_STATE` among its
alternatives. The gap is reported upstream in #3936.

### Coded text and open value sets

A coded field whose template value set is **open** accepts free text under the
`|other` suffix. `|other` may not be combined with `|code`, `|value`,
`|terminology`, or `|preferred_term` on the same leaf, and is rejected when the
value set is closed.

### Duplicate node names

When a template contains sibling nodes with the same name, the generated
WebTemplate/FLAT path ids are disambiguated with underscore suffixes counted from
1 (`blood_pressure`, `blood_pressure_1`, `blood_pressure_2`) as the
specification prescribes. There is no vendor-compatibility mode: the
specification's numbering is the only numbering, and vendor-only DV_QUANTITY
suffixes such as `|unit_system` and `|unit_display_name` are not accepted. If
your tooling was written against another server's path ids, map them on the
client side.

## Validation on commit

Every composition is validated against its template at commit time. This is
where the template earns its keep. The server checks:

- **structure:** required sections and fields are present, and cardinality and
  occurrence constraints are respected;
- **leaf values:** data types, units, value ranges, string patterns, decimal
  precision, and date/time constraints match the template;
- **terminology:** coded values are members of the value sets the template
  binds, using the bundled openEHR terminology or a configured external FHIR
  terminology server (see [Terminology servers](beyond-core/terminology.md)).

A second pass runs beside it and refuses the same way: the clinical side's
data-minimisation rules read the subject reference, the party proxies and every
string leaf of the body, and a finding is a `422` naming the RM path and the
rule that matched, never the offending value. That pass is configured
independently of the template, and the shipped default already refuses a
national identifier anywhere in clinical content. See
[Privacy & data minimisation](installation/config-privacy.md).

### Date and time patterns

A template can constrain which parts of a date, time or date-time value must be
present. The pattern takes the form `yyyy-mm-ddTHH:MM:SS`, and each part is
written one of three ways:

| In the pattern | Meaning |
|---|---|
| a letter field (`yyyy`, `mm`, `dd`, `HH`, `MM`, `SS`) | the part is required |
| `??` | the part is optional |
| `XX` | the part is not allowed |

So `yyyy-mm-ddTXX:XX:XX` accepts `2022-02-03` and refuses
`2022-02-03T04:05:06`, and `HH:MM:??` accepts both `10:30` and `10:30:15`.
A refusal names the parts in words and repeats the pattern with this key:

```text
temporal value '2022-02-03T04:05:06' does not satisfy the pattern
'yyyy-mm-ddTXX:XX:XX': the template allows a date only; a time is not allowed
here. Pattern key: a letter field (yyyy, mm, dd, HH, MM, SS) is required, ?? is
optional, XX is not allowed.
```

If a composition is well-formed but breaks its template, the commit fails with
**422 Unprocessable Entity**, a `validation-failed` problem whose `errors`, sent
when the request carries `Prefer: return=representation`, hold one
`{"path": …, "detail": …}` per offending node, so a client can show the user
exactly what to fix. A syntactically malformed request instead gets **400 Bad Request**.
The error shapes are described in
[Content negotiation & errors](using-the-api/content-negotiation.md#error-responses).

A version committed as `553|incomplete|` relaxes what must be *present* while
still checking everything that is; see the lifecycle notes in
[Resource walkthroughs](using-the-api/resources.md#composition).

## Next

- [Using the API → Resource walkthroughs](using-the-api/resources.md) — the full
  composition create/update/delete lifecycle.
- [Querying with AQL](querying-aql.md) — reading the data back out by path.
